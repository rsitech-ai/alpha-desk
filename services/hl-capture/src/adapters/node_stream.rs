use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use bytes::Bytes;
use domain_types::SourceId;
use hl_protocol::node::v1::{NodeRecordV1, NodeStreamKind, parse_node_record};
use hl_protocol::{
    BlockSource, ParseWarning, ReceiveTimestamps, SourceCursor, SourceError, SourceObservation,
    SourceRequestContext,
};

use super::node_files::{
    LineRead, OpenNodeFile, PathState, open_node_file, path_state, read_line,
    validate_resume_boundary,
};

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_NODE_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct NodeFileConfig {
    path: PathBuf,
    stream_name: String,
    stream: NodeStreamKind,
    source_id: SourceId,
    source_version: String,
    parser_schema_version: String,
    max_payload_bytes: usize,
    poll_interval: Duration,
}

impl NodeFileConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        path: PathBuf,
        stream_name: impl Into<String>,
        stream: NodeStreamKind,
        source_id: SourceId,
        source_version: impl Into<String>,
        parser_schema_version: impl Into<String>,
        max_payload_bytes: usize,
        poll_interval: Duration,
    ) -> Result<Self, SourceError> {
        let stream_name = stream_name.into();
        let source_version = source_version.into();
        let parser_schema_version = parser_schema_version.into();
        if !path.is_absolute()
            || !valid_identity(&stream_name)
            || !valid_identity(&source_version)
            || !valid_identity(&parser_schema_version)
            || !(1..=MAX_NODE_PAYLOAD_BYTES).contains(&max_payload_bytes)
            || poll_interval.is_zero()
            || Instant::now().checked_add(poll_interval).is_none()
        {
            return Err(SourceError::Configuration(
                "invalid node file-source configuration".into(),
            ));
        }
        Ok(Self {
            path,
            stream_name,
            stream,
            source_id,
            source_version,
            parser_schema_version,
            max_payload_bytes,
            poll_interval,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn stream_name(&self) -> &str {
        &self.stream_name
    }

    #[must_use]
    pub const fn stream(&self) -> NodeStreamKind {
        self.stream
    }
}

pub trait NodeReceiveClock: Send + Sync {
    fn now(&self) -> Result<ReceiveTimestamps, SourceError>;
}

#[derive(Debug, Clone)]
pub struct SystemNodeClock {
    started: Instant,
}

impl Default for SystemNodeClock {
    fn default() -> Self {
        Self {
            started: Instant::now(),
        }
    }
}

impl NodeReceiveClock for SystemNodeClock {
    fn now(&self) -> Result<ReceiveTimestamps, SourceError> {
        let wall = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| SourceError::Configuration("system wall clock is invalid".into()))?;
        let wall_micros = i64::try_from(wall.as_micros())
            .map_err(|_| SourceError::Configuration("system wall clock overflow".into()))?;
        let monotonic_nanos = u64::try_from(self.started.elapsed().as_nanos())
            .map_err(|_| SourceError::Configuration("monotonic clock overflow".into()))?;
        ReceiveTimestamps::new(wall_micros, monotonic_nanos)
            .map_err(|_| SourceError::Configuration("system clock is invalid".into()))
    }
}

#[derive(Debug, Clone)]
pub struct NodeQuarantineRecord {
    cursor: SourceCursor,
    payload: Bytes,
    content_hash: blake3::Hash,
    reason_code: &'static str,
}

impl NodeQuarantineRecord {
    pub(super) fn new(cursor: SourceCursor, payload: Bytes, reason_code: &'static str) -> Self {
        let content_hash = blake3::hash(&payload);
        Self {
            cursor,
            payload,
            content_hash,
            reason_code,
        }
    }

    #[must_use]
    pub const fn cursor(&self) -> &SourceCursor {
        &self.cursor
    }

    #[must_use]
    pub const fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        self.content_hash
    }

    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }
}

#[derive(Debug, Clone)]
struct PendingQuarantine {
    record: NodeQuarantineRecord,
    error: SourceError,
}

#[derive(Debug)]
pub struct NodeLineFileSource<C = SystemNodeClock> {
    config: NodeFileConfig,
    active: OpenNodeFile,
    read_offset: u64,
    durable_cursor: Option<SourceCursor>,
    pending_emitted_cursor: Option<SourceCursor>,
    pending_quarantine: Option<PendingQuarantine>,
    clock: C,
}

impl NodeLineFileSource<SystemNodeClock> {
    pub fn open(
        config: NodeFileConfig,
        durable_cursor: Option<SourceCursor>,
    ) -> Result<Self, SourceError> {
        Self::open_with_clock(config, durable_cursor, SystemNodeClock::default())
    }
}

impl<C: NodeReceiveClock> NodeLineFileSource<C> {
    pub fn open_with_clock(
        config: NodeFileConfig,
        durable_cursor: Option<SourceCursor>,
        clock: C,
    ) -> Result<Self, SourceError> {
        let mut active = open_node_file(
            config.path(),
            config.stream_name(),
            config.max_payload_bytes,
        )?;
        let read_offset = if let Some(cursor) = &durable_cursor {
            if cursor.epoch() != active.epoch {
                return Err(SourceError::CursorRegression);
            }
            validate_resume_boundary(&mut active.file, cursor.offset())?;
            cursor.offset()
        } else {
            0
        };
        Ok(Self {
            config,
            active,
            read_offset,
            durable_cursor,
            pending_emitted_cursor: None,
            pending_quarantine: None,
            clock,
        })
    }

    pub fn acknowledge_durable(&mut self, cursor: &SourceCursor) -> Result<(), SourceError> {
        if self.pending_emitted_cursor.as_ref() != Some(cursor) {
            return Err(SourceError::CursorRegression);
        }
        self.durable_cursor = Some(cursor.clone());
        self.pending_emitted_cursor = None;
        Ok(())
    }

    pub fn acknowledge_quarantine_durable(
        &mut self,
        cursor: &SourceCursor,
    ) -> Result<(), SourceError> {
        let Some(pending) = &self.pending_quarantine else {
            return Err(SourceError::CursorRegression);
        };
        if pending.record.cursor() != cursor {
            return Err(SourceError::CursorRegression);
        }
        self.read_offset = cursor.offset();
        self.durable_cursor = Some(cursor.clone());
        self.pending_quarantine = None;
        Ok(())
    }

    #[must_use]
    pub fn pending_quarantine(&self) -> Option<&NodeQuarantineRecord> {
        self.pending_quarantine
            .as_ref()
            .map(|pending| &pending.record)
    }

    async fn handle_complete_line(
        &mut self,
        payload: Vec<u8>,
        end_offset: u64,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError> {
        let cursor = SourceCursor::new(self.active.epoch.clone(), end_offset)
            .map_err(|_| SourceError::MalformedPayload("node cursor is invalid".into()))?;
        let bytes = Bytes::from(payload);
        let parsed = parse_off_thread(self.config.stream, bytes.clone()).await;
        context.check()?;
        let parsed = match parsed {
            Ok(parsed) => parsed,
            Err(error @ (SourceError::MalformedPayload(_) | SourceError::SchemaDrift(_))) => {
                self.pending_quarantine = Some(PendingQuarantine {
                    record: NodeQuarantineRecord::new(cursor, bytes, error.reason_code()),
                    error: error.clone(),
                });
                return Err(error);
            }
            Err(error) => return Err(error),
        };
        let observation = SourceObservation::new(
            self.config.source_id.clone(),
            self.config.source_version.clone(),
            parsed.observation_class(),
            cursor.clone(),
            self.clock.now()?,
            self.config.parser_schema_version.clone(),
            parsed.into_payload(),
            Vec::<ParseWarning>::new(),
            self.config.max_payload_bytes,
        )
        .map_err(|_| SourceError::MalformedPayload("node observation is invalid".into()))?;
        self.read_offset = end_offset;
        self.pending_emitted_cursor = Some(cursor);
        Ok(observation)
    }

    async fn read_next_line(&self) -> Result<LineRead, SourceError> {
        let mut file = self.active.file.try_clone().map_err(|_| {
            SourceError::TemporaryDisconnect("node output file clone failed".into())
        })?;
        let offset = self.read_offset;
        let max_payload_bytes = self.config.max_payload_bytes;
        tokio::task::spawn_blocking(move || read_line(&mut file, offset, max_payload_bytes))
            .await
            .map_err(|_| SourceError::Configuration("node output read task failed".into()))?
    }

    async fn wait_for_progress(&self, context: &SourceRequestContext) -> Result<(), SourceError> {
        context.check()?;
        let now = Instant::now();
        let wake = now
            .checked_add(self.config.poll_interval)
            .map_or(context.backpressure_deadline(), |poll| {
                poll.min(context.backpressure_deadline())
            });
        tokio::select! {
            () = context.cancellation().cancelled() => Err(SourceError::Cancelled),
            () = tokio::time::sleep_until(wake.into()) => context.check(),
        }
    }

    fn rotate_if_ready(&mut self, state: LineRead) -> Result<bool, SourceError> {
        match path_state(self.config.path(), self.active.identity)? {
            PathState::Same | PathState::Missing => Ok(false),
            PathState::Replaced => match state {
                LineRead::Partial => Err(SourceError::MalformedPayload(
                    "node output rotated with an incomplete final record".into(),
                )),
                LineRead::EndOfFile => {
                    self.active = open_node_file(
                        self.config.path(),
                        self.config.stream_name(),
                        self.config.max_payload_bytes,
                    )?;
                    self.read_offset = 0;
                    self.pending_emitted_cursor = None;
                    Ok(true)
                }
                LineRead::Complete { .. } => Ok(false),
            },
        }
    }

    fn same_identity_size(&self) -> Result<u64, SourceError> {
        self.active
            .file
            .metadata()
            .map(|metadata| metadata.len())
            .map_err(|_| SourceError::TemporaryDisconnect("node output metadata failed".into()))
    }
}

#[async_trait]
impl<C: NodeReceiveClock> BlockSource for NodeLineFileSource<C> {
    async fn next_observation(
        &mut self,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError> {
        loop {
            context.check()?;
            if let Some(pending) = &self.pending_quarantine {
                return Err(pending.error.clone());
            }
            if self.pending_emitted_cursor.is_some() {
                return Err(SourceError::BackpressureTimeout);
            }
            if self.same_identity_size()? < self.read_offset {
                return Err(SourceError::CursorRegression);
            }
            let state = self.read_next_line().await?;
            context.check()?;
            match state {
                LineRead::Complete {
                    payload,
                    end_offset,
                } => {
                    return self
                        .handle_complete_line(payload, end_offset, context)
                        .await;
                }
                LineRead::EndOfFile | LineRead::Partial => {
                    if self.rotate_if_ready(state)? {
                        continue;
                    }
                    self.wait_for_progress(context).await?;
                }
            }
        }
    }

    fn source_id(&self) -> &SourceId {
        &self.config.source_id
    }

    fn committed_cursor(&self) -> Option<&SourceCursor> {
        self.durable_cursor.as_ref()
    }
}

pub(super) async fn parse_off_thread(
    stream: NodeStreamKind,
    payload: Bytes,
) -> Result<NodeRecordV1, SourceError> {
    tokio::task::spawn_blocking(move || parse_node_record(stream, payload))
        .await
        .map_err(|_| SourceError::Configuration("node parser task failed".into()))?
}

pub(super) fn valid_identity(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value.trim() == value
        && !value.chars().any(char::is_control)
}
