use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Component, Path, PathBuf},
    time::Duration,
};

use async_nats::{
    ConnectOptions,
    header::NATS_MESSAGE_ID,
    jetstream::{self, consumer::pull::Config as PullConfig, context::ContextBuilder},
};
use canonical_events::{BlockEnvelope, CanonicalEventEnvelope, ConfirmationClass};
use canonical_ledger::{EventReducer, LedgerLimits, StateImageLimits};
use domain_types::{BlockHeight, ChainId};
use futures_util::StreamExt as _;
use sha2::{Digest as _, Sha256};

use crate::{
    DurableApplyError, DurableApplyOutcome, LocalReplayError,
    publication::{
        BLOCK_MARKER_SCHEMA_V1, CANONICAL_STREAM, CanonicalSubject, CommittedBlockMarker,
        HEADER_ARCHIVE_MANIFEST_SHA256, HEADER_ARCHIVE_RECEIPT, HEADER_BLOCK_HASH,
        HEADER_BLOCK_HEIGHT, HEADER_CHAIN, HEADER_PUBLICATION_SHA256, HEADER_SCHEMA,
        decode_committed_block_marker, encode_committed_block_marker, encode_event_payload,
        subject_for_event_kind,
    },
    state_runtime::{ResumeMode, StateRuntime, align_watermarks},
};

const MAX_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_ACK_INFLIGHT: usize = 100_000;
const MAX_FETCH_BATCH: usize = 10_000;
const MAX_PENDING_BLOCKS: usize = 64;
const MAX_SECRET_BYTES: u64 = 16_384;
const DEFAULT_DURABLE_NAME: &str = "hl-core-file-replay";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JetStreamReplayReport {
    pub applied: u64,
    pub already_applied: u64,
    pub last_height: Option<BlockHeight>,
    pub state_hash: [u8; 32],
    pub live_qualified: bool,
    pub stage_2_qualified: bool,
}

impl JetStreamReplayReport {
    fn empty(state_hash: [u8; 32], last_height: Option<BlockHeight>) -> Self {
        Self {
            applied: 0,
            already_applied: 0,
            last_height,
            state_hash,
            live_qualified: false,
            stage_2_qualified: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalDelivery {
    pub subject: CanonicalSubject,
    pub message_id: String,
    pub schema: String,
    pub chain_id: String,
    pub block_height: u64,
    pub block_hash: [u8; 32],
    pub archive_receipt_id: String,
    pub archive_manifest_sha256: [u8; 32],
    pub publication_sha256: [u8; 32],
    pub payload: Vec<u8>,
}

impl CanonicalDelivery {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        subject: CanonicalSubject,
        message_id: impl Into<String>,
        schema: impl Into<String>,
        chain_id: impl Into<String>,
        block_height: u64,
        block_hash: [u8; 32],
        archive_receipt_id: impl Into<String>,
        archive_manifest_sha256: [u8; 32],
        payload: Vec<u8>,
    ) -> Result<Self, JetStreamReplayError> {
        let message_id = message_id.into();
        let schema = schema.into();
        let chain_id = chain_id.into();
        let archive_receipt_id = archive_receipt_id.into();
        validate_identity(&message_id)?;
        validate_identity(&schema)?;
        validate_identity(&chain_id)?;
        validate_identity(&archive_receipt_id)?;
        if message_id != message_id.to_ascii_lowercase() {
            return Err(JetStreamReplayError::Decode(
                crate::publication::BlockMarkerError::InvalidIdentity,
            ));
        }
        if payload.is_empty() {
            return Err(JetStreamReplayError::Decode(
                crate::publication::BlockMarkerError::PayloadSize,
            ));
        }
        let publication_sha256 = Sha256::digest(&payload).into();
        Ok(Self {
            subject,
            message_id,
            schema,
            chain_id,
            block_height,
            block_hash,
            archive_receipt_id,
            archive_manifest_sha256,
            publication_sha256,
            payload,
        })
    }
}

pub fn committed_block_delivery(
    block: &BlockEnvelope,
    receipt: &storage_ports::ArchiveReceipt,
) -> Result<CanonicalDelivery, JetStreamReplayError> {
    let payload = encode_committed_block_marker(block, receipt)?;
    CanonicalDelivery::try_new(
        CanonicalSubject::BlockCommitted,
        format!("blk_{}", hex::encode(block.canonical_block_hash())),
        BLOCK_MARKER_SCHEMA_V1,
        block.chain_id().as_str(),
        block.block_height().get(),
        block.canonical_block_hash(),
        receipt.receipt_id(),
        receipt.manifest_sha256(),
        payload,
    )
}

pub fn committed_event_delivery(
    event: &CanonicalEventEnvelope,
    block: &BlockEnvelope,
    receipt: &storage_ports::ArchiveReceipt,
) -> Result<CanonicalDelivery, JetStreamReplayError> {
    CanonicalDelivery::try_new(
        subject_for_event_kind(event.event_kind()),
        event.event_id().as_str(),
        event.schema_version(),
        block.chain_id().as_str(),
        block.block_height().get(),
        block.canonical_block_hash(),
        receipt.receipt_id(),
        receipt.manifest_sha256(),
        encode_event_payload(event)?,
    )
}

pub trait CanonicalPullSource {
    fn fetch(
        &mut self,
        max_messages: usize,
    ) -> impl Future<Output = Result<Vec<CanonicalDelivery>, JetStreamReplayError>> + Send;

    fn ack(
        &mut self,
        message_ids: &[String],
    ) -> impl Future<Output = Result<(), JetStreamReplayError>> + Send;
}

#[derive(Debug, Default)]
pub struct InMemoryCanonicalSource {
    queued: VecDeque<CanonicalDelivery>,
    acked: BTreeSet<String>,
}

impl InMemoryCanonicalSource {
    #[must_use]
    pub fn new(deliveries: impl IntoIterator<Item = CanonicalDelivery>) -> Self {
        Self {
            queued: deliveries.into_iter().collect(),
            acked: BTreeSet::new(),
        }
    }

    #[must_use]
    pub const fn acked(&self) -> &BTreeSet<String> {
        &self.acked
    }
}

impl CanonicalPullSource for InMemoryCanonicalSource {
    async fn fetch(
        &mut self,
        max_messages: usize,
    ) -> Result<Vec<CanonicalDelivery>, JetStreamReplayError> {
        let mut batch = Vec::new();
        for _ in 0..max_messages {
            let Some(delivery) = self.queued.pop_front() else {
                break;
            };
            batch.push(delivery);
        }
        Ok(batch)
    }

    async fn ack(&mut self, message_ids: &[String]) -> Result<(), JetStreamReplayError> {
        for message_id in message_ids {
            self.acked.insert(message_id.clone());
        }
        Ok(())
    }
}

pub struct JetStreamPullSource {
    consumer: jetstream::consumer::Consumer<PullConfig>,
    inflight: BTreeMap<String, jetstream::Message>,
    fetch_batch: usize,
}

impl JetStreamPullSource {
    pub async fn connect(config: JetStreamReplayConfig) -> Result<Self, JetStreamReplayError> {
        let options = match &config.authentication {
            JetStreamReplayAuth::Anonymous => ConnectOptions::new(),
            JetStreamReplayAuth::CredentialsFile(path) => {
                read_protected_secret(path).map_err(|_| JetStreamReplayError::Transport)?;
                ConnectOptions::with_credentials_file(path)
                    .await
                    .map_err(|_| JetStreamReplayError::Transport)?
            }
            JetStreamReplayAuth::UserPasswordFile {
                username,
                password_path,
            } => {
                let password = read_protected_secret(password_path)
                    .map_err(|_| JetStreamReplayError::Transport)?;
                ConnectOptions::with_user_and_password(username.clone(), password)
            }
        }
        .name("alpha-desk-hl-core")
        .connection_timeout(config.connect_timeout)
        .request_timeout(Some(config.acknowledgement_timeout));

        let client = options
            .connect(config.server_url.as_str())
            .await
            .map_err(|_| JetStreamReplayError::Transport)?;
        let context = ContextBuilder::new()
            .timeout(config.acknowledgement_timeout)
            .ack_timeout(config.acknowledgement_timeout)
            .max_ack_inflight(config.max_ack_inflight)
            .build(client);
        let stream = context
            .get_stream(CANONICAL_STREAM)
            .await
            .map_err(|_| JetStreamReplayError::Transport)?;
        let consumer = stream
            .get_or_create_consumer(
                &config.durable_name,
                PullConfig {
                    durable_name: Some(config.durable_name.clone()),
                    ack_policy: jetstream::consumer::AckPolicy::Explicit,
                    ack_wait: config.acknowledgement_timeout,
                    max_deliver: 8,
                    max_ack_pending: i64::try_from(config.max_ack_inflight)
                        .map_err(|_| JetStreamReplayError::Transport)?,
                    backoff: vec![
                        Duration::from_millis(200),
                        Duration::from_secs(1),
                        Duration::from_secs(5),
                    ],
                    ..PullConfig::default()
                },
            )
            .await
            .map_err(|_| JetStreamReplayError::Transport)?;
        Ok(Self {
            consumer,
            inflight: BTreeMap::new(),
            fetch_batch: config.fetch_batch,
        })
    }
}

impl CanonicalPullSource for JetStreamPullSource {
    async fn fetch(
        &mut self,
        max_messages: usize,
    ) -> Result<Vec<CanonicalDelivery>, JetStreamReplayError> {
        let max_messages = max_messages.min(self.fetch_batch).max(1);
        let mut batch = self
            .consumer
            .fetch()
            .max_messages(max_messages)
            .messages()
            .await
            .map_err(|_| JetStreamReplayError::Transport)?;
        let mut deliveries = Vec::new();
        while let Some(message) = batch.next().await {
            let message = message.map_err(|_| JetStreamReplayError::Transport)?;
            let delivery = delivery_from_jetstream(&message)?;
            if self
                .inflight
                .insert(delivery.message_id.clone(), message)
                .is_some()
            {
                return Err(JetStreamReplayError::PendingLimit);
            }
            deliveries.push(delivery);
        }
        Ok(deliveries)
    }

    async fn ack(&mut self, message_ids: &[String]) -> Result<(), JetStreamReplayError> {
        for message_id in message_ids {
            let message = self
                .inflight
                .remove(message_id)
                .ok_or(JetStreamReplayError::Transport)?;
            message
                .ack()
                .await
                .map_err(|_| JetStreamReplayError::Transport)?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JetStreamReplayConfig {
    server_url: String,
    authentication: JetStreamReplayAuth,
    connect_timeout: Duration,
    acknowledgement_timeout: Duration,
    max_ack_inflight: usize,
    durable_name: String,
    fetch_batch: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JetStreamReplayAuth {
    Anonymous,
    CredentialsFile(PathBuf),
    UserPasswordFile {
        username: String,
        password_path: PathBuf,
    },
}

impl JetStreamReplayConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        server_url: impl Into<String>,
        authentication: JetStreamReplayAuth,
        connect_timeout: Duration,
        acknowledgement_timeout: Duration,
        max_ack_inflight: usize,
        durable_name: impl Into<String>,
        fetch_batch: usize,
    ) -> Result<Self, JetStreamReplayConfigError> {
        let server_url = server_url.into();
        validate_server_url(&server_url)?;
        match &authentication {
            JetStreamReplayAuth::Anonymous => {}
            JetStreamReplayAuth::CredentialsFile(path) => validate_credentials_path(path)?,
            JetStreamReplayAuth::UserPasswordFile {
                username,
                password_path,
            } => {
                validate_username(username)?;
                validate_credentials_path(password_path)?;
            }
        }
        if connect_timeout.is_zero() || connect_timeout > MAX_TIMEOUT {
            return Err(JetStreamReplayConfigError::InvalidConnectTimeout);
        }
        if acknowledgement_timeout.is_zero() || acknowledgement_timeout > MAX_TIMEOUT {
            return Err(JetStreamReplayConfigError::InvalidAcknowledgementTimeout);
        }
        if !(1..=MAX_ACK_INFLIGHT).contains(&max_ack_inflight) {
            return Err(JetStreamReplayConfigError::InvalidMaxAckInflight);
        }
        let durable_name = durable_name.into();
        if durable_name.is_empty() {
            return Err(JetStreamReplayConfigError::InvalidDurableName);
        }
        validate_username(&durable_name)
            .map_err(|_| JetStreamReplayConfigError::InvalidDurableName)?;
        if !(1..=MAX_FETCH_BATCH).contains(&fetch_batch) {
            return Err(JetStreamReplayConfigError::InvalidFetchBatch);
        }
        Ok(Self {
            server_url,
            authentication,
            connect_timeout,
            acknowledgement_timeout,
            max_ack_inflight,
            durable_name,
            fetch_batch,
        })
    }

    #[must_use]
    pub fn default_durable_name() -> &'static str {
        DEFAULT_DURABLE_NAME
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum JetStreamReplayConfigError {
    #[error("NATS server URL is invalid or contains inline credentials")]
    UnsafeServerUrl,
    #[error("NATS credentials path must be absolute and normalized")]
    UnsafeCredentialsPath,
    #[error("NATS authentication username is invalid")]
    InvalidUsername,
    #[error("NATS connection timeout is outside the supported bound")]
    InvalidConnectTimeout,
    #[error("JetStream acknowledgement timeout is outside the supported bound")]
    InvalidAcknowledgementTimeout,
    #[error("JetStream maximum in-flight acknowledgement count is outside the supported bound")]
    InvalidMaxAckInflight,
    #[error("JetStream durable consumer name is invalid")]
    InvalidDurableName,
    #[error("JetStream fetch batch size is outside the supported bound")]
    InvalidFetchBatch,
}

impl JetStreamReplayConfigError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnsafeServerUrl => "core.jetstream_config.unsafe_server_url",
            Self::UnsafeCredentialsPath => "core.jetstream_config.unsafe_credentials_path",
            Self::InvalidUsername => "core.jetstream_config.invalid_username",
            Self::InvalidConnectTimeout => "core.jetstream_config.invalid_connect_timeout",
            Self::InvalidAcknowledgementTimeout => {
                "core.jetstream_config.invalid_acknowledgement_timeout"
            }
            Self::InvalidMaxAckInflight => "core.jetstream_config.invalid_max_ack_inflight",
            Self::InvalidDurableName => "core.jetstream_config.invalid_durable_name",
            Self::InvalidFetchBatch => "core.jetstream_config.invalid_fetch_batch",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JetStreamReplayError {
    #[error(transparent)]
    Config(#[from] JetStreamReplayConfigError),
    #[error(transparent)]
    Decode(#[from] crate::publication::BlockMarkerError),
    #[error("JetStream publication hash does not match the payload")]
    HashMismatch,
    #[error("JetStream delivered an incomplete canonical block")]
    IncompleteBlock,
    #[error("JetStream pending block buffer exceeded its bound")]
    PendingLimit,
    #[error("JetStream connection or acknowledgement failed")]
    Transport,
    #[error(transparent)]
    Replay(#[from] LocalReplayError),
    #[error("JetStream replay applied-block counter overflowed")]
    Overflow,
}

impl JetStreamReplayError {
    #[must_use]
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Config(error) => error.reason_code(),
            Self::Decode(error) => error.reason_code(),
            Self::HashMismatch => "core.jetstream_hash_mismatch",
            Self::IncompleteBlock => "core.jetstream_incomplete_block",
            Self::PendingLimit => "core.jetstream_pending_limit",
            Self::Transport => "core.jetstream_transport",
            Self::Replay(LocalReplayError::Durable(DurableApplyError::Ledger(error))) => {
                error.reason_code()
            }
            Self::Replay(error) => error.reason_code(),
            Self::Overflow => "core.replay_overflow",
        }
    }
}

pub struct JetStreamReplaySession<R, S> {
    runtime: StateRuntime<R, S>,
    assembler: BlockAssembler,
    fetch_batch: usize,
}

impl<R: EventReducer, S: storage_ports::AtomicStateStore> JetStreamReplaySession<R, S> {
    pub fn open(
        chain_id: ChainId,
        first_height: BlockHeight,
        reducer: R,
        limits: LedgerLimits,
        store: S,
        image_limits: StateImageLimits,
    ) -> Result<Self, JetStreamReplayError> {
        Ok(Self {
            runtime: StateRuntime::open(
                chain_id,
                first_height,
                reducer,
                limits,
                store,
                image_limits,
                ResumeMode::Durable,
                None,
            )?,
            assembler: BlockAssembler::new(),
            fetch_batch: 64,
        })
    }

    #[must_use]
    pub fn ledger(&self) -> &canonical_ledger::CanonicalLedger<R> {
        self.runtime.ledger()
    }

    #[must_use]
    pub fn runtime(&self) -> &StateRuntime<R, S> {
        &self.runtime
    }

    pub fn runtime_mut(&mut self) -> &mut StateRuntime<R, S> {
        &mut self.runtime
    }

    pub async fn consume_available<Src: CanonicalPullSource>(
        &mut self,
        source: &mut Src,
    ) -> Result<JetStreamReplayReport, JetStreamReplayError> {
        let mut report = JetStreamReplayReport::empty(
            self.runtime.ledger().state_hash(),
            self.runtime
                .ledger()
                .checkpoint()
                .map(|checkpoint| checkpoint.block_height()),
        );
        loop {
            let batch = source.fetch(self.fetch_batch).await?;
            if batch.is_empty() {
                if self.assembler.has_pending() {
                    return Err(JetStreamReplayError::IncompleteBlock);
                }
                break;
            }
            for delivery in batch {
                if let Some(assembled) = self.assembler.push(delivery)? {
                    match self.runtime.session_mut().apply_next(&assembled.block)? {
                        DurableApplyOutcome::Applied { .. } => {
                            report.applied = report
                                .applied
                                .checked_add(1)
                                .ok_or(JetStreamReplayError::Overflow)?;
                            let state_height = self
                                .runtime
                                .ledger()
                                .checkpoint()
                                .map(|checkpoint| checkpoint.block_height())
                                .ok_or(JetStreamReplayError::Replay(
                                    LocalReplayError::WatermarkMisaligned,
                                ))?;
                            align_watermarks(
                                assembled.block.block_height(),
                                assembled.block.block_height(),
                                state_height,
                            )?;
                        }
                        DurableApplyOutcome::AlreadyApplied(_) => {
                            report.already_applied = report.already_applied.saturating_add(1);
                        }
                    }
                    report.last_height = Some(assembled.block.block_height());
                    report.state_hash = self.runtime.ledger().state_hash();
                    source.ack(&assembled.message_ids).await?;
                }
            }
        }
        Ok(report)
    }
}

struct BlockAssembler {
    pending: BTreeMap<[u8; 32], PendingBlock>,
}

struct PendingBlock {
    height: u64,
    chain_id: String,
    marker: Option<CommittedBlockMarker>,
    events: BTreeMap<String, CanonicalEventEnvelope>,
    message_ids: Vec<String>,
}

struct AssembledBlock {
    block: BlockEnvelope,
    message_ids: Vec<String>,
}

impl BlockAssembler {
    fn new() -> Self {
        Self {
            pending: BTreeMap::new(),
        }
    }

    fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }

    fn push(
        &mut self,
        delivery: CanonicalDelivery,
    ) -> Result<Option<AssembledBlock>, JetStreamReplayError> {
        let digest: [u8; 32] = Sha256::digest(&delivery.payload).into();
        if digest != delivery.publication_sha256 {
            return Err(JetStreamReplayError::HashMismatch);
        }
        if !self.pending.contains_key(&delivery.block_hash)
            && self.pending.len() >= MAX_PENDING_BLOCKS
        {
            return Err(JetStreamReplayError::PendingLimit);
        }
        let pending = self
            .pending
            .entry(delivery.block_hash)
            .or_insert_with(|| PendingBlock {
                height: delivery.block_height,
                chain_id: delivery.chain_id.clone(),
                marker: None,
                events: BTreeMap::new(),
                message_ids: Vec::new(),
            });
        if pending.height != delivery.block_height || pending.chain_id != delivery.chain_id {
            return Err(JetStreamReplayError::HashMismatch);
        }
        pending.message_ids.push(delivery.message_id.clone());
        match delivery.subject {
            CanonicalSubject::BlockCommitted => {
                if pending.marker.is_some() {
                    return Err(JetStreamReplayError::Decode(
                        crate::publication::BlockMarkerError::Malformed,
                    ));
                }
                if delivery.schema != BLOCK_MARKER_SCHEMA_V1 {
                    return Err(JetStreamReplayError::Decode(
                        crate::publication::BlockMarkerError::UnsupportedSchema,
                    ));
                }
                let marker = decode_committed_block_marker(&delivery.payload)?;
                if marker.canonical_block_hash != delivery.block_hash
                    || marker.block_height.get() != delivery.block_height
                    || marker.chain_id.as_str() != delivery.chain_id
                    || marker.archive_receipt_id != delivery.archive_receipt_id
                    || marker.archive_manifest_sha256 != delivery.archive_manifest_sha256
                {
                    return Err(JetStreamReplayError::HashMismatch);
                }
                if !matches!(
                    marker.confirmation_class,
                    ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent
                ) {
                    return Err(JetStreamReplayError::Decode(
                        crate::publication::BlockMarkerError::NotCommitted,
                    ));
                }
                pending.marker = Some(marker);
            }
            CanonicalSubject::EventFill
            | CanonicalSubject::EventOrder
            | CanonicalSubject::EventLedger
            | CanonicalSubject::EventMarketMeta
            | CanonicalSubject::EventOracle => {
                let event = CanonicalEventEnvelope::decode(&delivery.payload)
                    .map_err(|_| crate::publication::BlockMarkerError::Malformed)?;
                let digest: [u8; 32] = Sha256::digest(&delivery.payload).into();
                if event.event_id().as_str() != delivery.message_id
                    || event.block_height().get() != delivery.block_height
                    || event.chain_id().as_str() != delivery.chain_id
                    || subject_for_event_kind(event.event_kind()) != delivery.subject
                    || digest != delivery.publication_sha256
                {
                    return Err(JetStreamReplayError::HashMismatch);
                }
                if pending
                    .events
                    .insert(event.event_id().as_str().to_owned(), event)
                    .is_some()
                {
                    return Err(JetStreamReplayError::Decode(
                        crate::publication::BlockMarkerError::Malformed,
                    ));
                }
            }
        }
        let Some(marker) = pending.marker.as_ref() else {
            return Ok(None);
        };
        if pending.events.len() > marker.events.len() {
            return Err(JetStreamReplayError::Decode(
                crate::publication::BlockMarkerError::Malformed,
            ));
        }
        if pending.events.keys().any(|event_id| {
            marker
                .events
                .iter()
                .all(|listed| listed.event_id != *event_id)
        }) {
            return Err(JetStreamReplayError::Decode(
                crate::publication::BlockMarkerError::Malformed,
            ));
        }
        if pending.events.len() != marker.events.len() {
            return Ok(None);
        }
        let mut events = Vec::with_capacity(marker.events.len());
        for listed in &marker.events {
            let event = pending
                .events
                .get(&listed.event_id)
                .ok_or(JetStreamReplayError::IncompleteBlock)?;
            let encoded = encode_event_payload(event)?;
            let envelope_sha256: [u8; 32] = Sha256::digest(&encoded).into();
            if event.event_kind() != listed.event_kind
                || event.payload_hash() != listed.payload_hash
                || envelope_sha256 != listed.envelope_sha256
            {
                return Err(JetStreamReplayError::HashMismatch);
            }
            events.push(event.clone());
        }
        let block = BlockEnvelope::try_new(
            marker.chain_id.clone(),
            marker.block_height,
            marker.block_time,
            marker.confirmation_class,
            events,
            marker.source_block_hashes.clone(),
        )
        .map_err(|_| crate::publication::BlockMarkerError::Malformed)?;
        if block.canonical_block_hash() != marker.canonical_block_hash {
            return Err(JetStreamReplayError::HashMismatch);
        }
        let pending = self
            .pending
            .remove(&delivery.block_hash)
            .expect("assembled pending block");
        Ok(Some(AssembledBlock {
            block,
            message_ids: pending.message_ids,
        }))
    }
}

fn delivery_from_jetstream(
    message: &jetstream::Message,
) -> Result<CanonicalDelivery, JetStreamReplayError> {
    let headers = message
        .headers
        .as_ref()
        .ok_or(crate::publication::BlockMarkerError::Malformed)?;
    let subject = CanonicalSubject::parse(message.subject.as_str())?;
    let message_id = headers
        .get(NATS_MESSAGE_ID)
        .map(|value| value.as_str().to_owned())
        .ok_or(crate::publication::BlockMarkerError::Malformed)?;
    let schema = header(headers, HEADER_SCHEMA)?;
    let chain_id = header(headers, HEADER_CHAIN)?;
    let block_height = header(headers, HEADER_BLOCK_HEIGHT)?
        .parse::<u64>()
        .map_err(|_| crate::publication::BlockMarkerError::Malformed)?;
    let block_hash_header = header(headers, HEADER_BLOCK_HASH)?;
    let block_hash = parse_hash32(&block_hash_header)?;
    let archive_receipt_id = header(headers, HEADER_ARCHIVE_RECEIPT)?;
    let archive_manifest_header = header(headers, HEADER_ARCHIVE_MANIFEST_SHA256)?;
    let archive_manifest_sha256 = parse_hash32(&archive_manifest_header)?;
    let publication_header = header(headers, HEADER_PUBLICATION_SHA256)?;
    let publication_sha256 = parse_hash32(&publication_header)?;
    let payload = message.payload.to_vec();
    let digest: [u8; 32] = Sha256::digest(&payload).into();
    if digest != publication_sha256 {
        return Err(JetStreamReplayError::HashMismatch);
    }
    let delivery = CanonicalDelivery::try_new(
        subject,
        message_id,
        schema,
        chain_id,
        block_height,
        block_hash,
        archive_receipt_id,
        archive_manifest_sha256,
        payload,
    )?;
    if delivery.publication_sha256 != publication_sha256 {
        return Err(JetStreamReplayError::HashMismatch);
    }
    Ok(delivery)
}

fn header(
    headers: &async_nats::HeaderMap,
    name: &str,
) -> Result<String, crate::publication::BlockMarkerError> {
    headers
        .get(name)
        .map(|value| value.as_str().to_owned())
        .ok_or(crate::publication::BlockMarkerError::Malformed)
}

fn parse_hash32(value: &str) -> Result<[u8; 32], crate::publication::BlockMarkerError> {
    let bytes = hex::decode(value).map_err(|_| crate::publication::BlockMarkerError::Malformed)?;
    <[u8; 32]>::try_from(bytes).map_err(|_| crate::publication::BlockMarkerError::Malformed)
}

fn validate_identity(value: &str) -> Result<(), JetStreamReplayError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 512
        || value.chars().any(char::is_control)
    {
        Err(JetStreamReplayError::Decode(
            crate::publication::BlockMarkerError::InvalidIdentity,
        ))
    } else {
        Ok(())
    }
}

fn validate_server_url(value: &str) -> Result<(), JetStreamReplayConfigError> {
    let valid_scheme = value.starts_with("nats://") || value.starts_with("tls://");
    if !valid_scheme
        || value.trim() != value
        || value.contains('@')
        || value.chars().any(char::is_control)
    {
        return Err(JetStreamReplayConfigError::UnsafeServerUrl);
    }
    let authority = value
        .split_once("://")
        .map(|(_, authority)| authority)
        .unwrap_or_default();
    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
    {
        Err(JetStreamReplayConfigError::UnsafeServerUrl)
    } else {
        Ok(())
    }
}

fn validate_credentials_path(path: &Path) -> Result<(), JetStreamReplayConfigError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::CurDir | Component::ParentDir))
    {
        Err(JetStreamReplayConfigError::UnsafeCredentialsPath)
    } else {
        Ok(())
    }
}

fn validate_username(value: &str) -> Result<(), JetStreamReplayConfigError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 128
        || value.chars().any(char::is_control)
    {
        Err(JetStreamReplayConfigError::InvalidUsername)
    } else {
        Ok(())
    }
}

fn read_protected_secret(path: &Path) -> Result<String, std::io::Error> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > MAX_SECRET_BYTES
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "unsafe secret file",
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "secret file permissions are too broad",
        ));
    }
    let bytes = fs::read(path)?;
    let value = String::from_utf8(bytes)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "secret UTF-8"))?;
    let value = value.strip_suffix('\n').unwrap_or(&value);
    let value = value.strip_suffix('\r').unwrap_or(value);
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid secret",
        ));
    }
    Ok(value.to_owned())
}
