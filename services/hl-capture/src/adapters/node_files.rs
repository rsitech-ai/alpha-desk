use std::fs::{self, File, Metadata};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bytes::Bytes;
use domain_types::SourceId;
use hl_protocol::node::state_snapshot::PERIODIC_SNAPSHOT_STRIDE;
use hl_protocol::node::v1::NodeStreamKind;
use hl_protocol::{
    BlockSource, ParseWarning, SourceCursor, SourceError, SourceObservation, SourceRequestContext,
};

use super::node_stream::{NodeReceiveClock, SystemNodeClock, valid_identity};

const READ_CHUNK_BYTES: usize = 16 * 1024;
const MAX_NODE_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
const GAP_PROBE_HEIGHTS: u64 = 64;

#[derive(Debug)]
pub(super) struct OpenNodeFile {
    pub file: File,
    pub identity: FileIdentity,
    pub epoch: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct FileIdentity {
    device: u64,
    inode: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum LineRead {
    Complete { payload: Vec<u8>, end_offset: u64 },
    EndOfFile,
    Partial,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PathState {
    Same,
    Replaced,
    Missing,
}

pub(super) fn open_node_file(
    path: &Path,
    stream_name: &str,
    max_payload_bytes: usize,
) -> Result<OpenNodeFile, SourceError> {
    let metadata = safe_regular_metadata(path)?;
    let identity = file_identity(&metadata);
    let mut file = File::open(path)
        .map_err(|_| SourceError::TemporaryDisconnect("node output file is unavailable".into()))?;
    let opening_record = match read_line(&mut file, 0, max_payload_bytes)? {
        LineRead::Complete { payload, .. } => payload,
        LineRead::EndOfFile | LineRead::Partial => {
            return Err(SourceError::TemporaryDisconnect(
                "node output has no complete opening record".into(),
            ));
        }
    };
    let epoch = epoch(stream_name, identity, &opening_record);
    Ok(OpenNodeFile {
        file,
        identity,
        epoch,
    })
}

pub(super) fn read_line(
    file: &mut File,
    offset: u64,
    max_payload_bytes: usize,
) -> Result<LineRead, SourceError> {
    file.seek(SeekFrom::Start(offset))
        .map_err(|_| SourceError::TemporaryDisconnect("node output seek failed".into()))?;
    let mut payload = Vec::new();
    let mut chunk = [0_u8; READ_CHUNK_BYTES];
    loop {
        let read = file
            .read(&mut chunk)
            .map_err(|_| SourceError::TemporaryDisconnect("node output read failed".into()))?;
        if read == 0 {
            return if payload.is_empty() {
                Ok(LineRead::EndOfFile)
            } else {
                Ok(LineRead::Partial)
            };
        }
        if let Some(newline) = chunk[..read].iter().position(|byte| *byte == b'\n') {
            let record_bytes = newline;
            if payload
                .len()
                .checked_add(record_bytes)
                .is_none_or(|length| length > max_payload_bytes)
            {
                return Err(SourceError::MalformedPayload(
                    "node output record exceeds its configured limit".into(),
                ));
            }
            payload.extend_from_slice(&chunk[..newline]);
            let end_offset =
                offset
                    .checked_add(u64::try_from(payload.len()).map_err(|_| {
                        SourceError::MalformedPayload("node offset overflow".into())
                    })?)
                    .and_then(|value| value.checked_add(1))
                    .ok_or_else(|| SourceError::MalformedPayload("node offset overflow".into()))?;
            return Ok(LineRead::Complete {
                payload,
                end_offset,
            });
        }
        if payload
            .len()
            .checked_add(read)
            .is_none_or(|length| length > max_payload_bytes)
        {
            return Err(SourceError::MalformedPayload(
                "node output record exceeds its configured limit".into(),
            ));
        }
        payload.extend_from_slice(&chunk[..read]);
    }
}

pub(super) fn path_state(path: &Path, active: FileIdentity) -> Result<PathState, SourceError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(SourceError::Configuration(
                    "node output path must remain a regular non-symlink file".into(),
                ));
            }
            if file_identity(&metadata) == active {
                Ok(PathState::Same)
            } else {
                Ok(PathState::Replaced)
            }
        }
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(PathState::Missing),
        Err(_) => Err(SourceError::TemporaryDisconnect(
            "node output metadata is unavailable".into(),
        )),
    }
}

pub(super) fn validate_resume_boundary(file: &mut File, offset: u64) -> Result<(), SourceError> {
    let size = file
        .metadata()
        .map_err(|_| SourceError::TemporaryDisconnect("node output metadata failed".into()))?
        .len();
    if offset > size {
        return Err(SourceError::CursorRegression);
    }
    if offset == 0 {
        return Ok(());
    }
    file.seek(SeekFrom::Start(offset - 1))
        .map_err(|_| SourceError::TemporaryDisconnect("node resume seek failed".into()))?;
    let mut delimiter = [0_u8; 1];
    file.read_exact(&mut delimiter)
        .map_err(|_| SourceError::TemporaryDisconnect("node resume read failed".into()))?;
    if delimiter == *b"\n" {
        Ok(())
    } else {
        Err(SourceError::CursorRegression)
    }
}

fn safe_regular_metadata(path: &Path) -> Result<Metadata, SourceError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| SourceError::TemporaryDisconnect("node output file is unavailable".into()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(SourceError::Configuration(
            "node output path must be a regular non-symlink file".into(),
        ));
    }
    Ok(metadata)
}

#[cfg(unix)]
fn file_identity(metadata: &Metadata) -> FileIdentity {
    use std::os::unix::fs::MetadataExt;

    FileIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    }
}

#[cfg(not(unix))]
fn file_identity(metadata: &Metadata) -> FileIdentity {
    FileIdentity {
        device: 0,
        inode: metadata.len(),
    }
}

fn epoch(stream_name: &str, identity: FileIdentity, opening_record: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"alpha-desk-node-file-epoch-v1\0");
    hasher.update(stream_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(&identity.device.to_le_bytes());
    hasher.update(&identity.inode.to_le_bytes());
    hasher.update(blake3::hash(opening_record).as_bytes());
    format!("node-file-v1:{}", hasher.finalize().to_hex())
}

#[derive(Debug, Clone)]
pub struct NodeBlockDirectoryConfig {
    root: PathBuf,
    stream_name: String,
    source_id: SourceId,
    source_version: String,
    parser_schema_version: String,
    start_height: u64,
    max_payload_bytes: usize,
    poll_interval: Duration,
}

impl NodeBlockDirectoryConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        stream_name: impl Into<String>,
        source_id: SourceId,
        source_version: impl Into<String>,
        parser_schema_version: impl Into<String>,
        start_height: u64,
        max_payload_bytes: usize,
        poll_interval: Duration,
    ) -> Result<Self, SourceError> {
        let stream_name = stream_name.into();
        let source_version = source_version.into();
        let parser_schema_version = parser_schema_version.into();
        if !root.is_absolute()
            || !valid_identity(&stream_name)
            || !valid_identity(&source_version)
            || !valid_identity(&parser_schema_version)
            || !(1..=MAX_NODE_PAYLOAD_BYTES).contains(&max_payload_bytes)
            || poll_interval.is_zero()
            || Instant::now().checked_add(poll_interval).is_none()
        {
            return Err(SourceError::Configuration(
                "invalid node block-directory configuration".into(),
            ));
        }
        Ok(Self {
            root,
            stream_name,
            source_id,
            source_version,
            parser_schema_version,
            start_height,
            max_payload_bytes,
            poll_interval,
        })
    }
}

#[derive(Debug)]
pub struct NodeBlockDirectorySource<C = SystemNodeClock> {
    config: NodeBlockDirectoryConfig,
    root_identity: FileIdentity,
    epoch: String,
    last_read_height: Option<u64>,
    durable_cursor: Option<SourceCursor>,
    pending_emitted_cursor: Option<SourceCursor>,
    clock: C,
}

impl NodeBlockDirectorySource<SystemNodeClock> {
    pub fn open(
        config: NodeBlockDirectoryConfig,
        durable_cursor: Option<SourceCursor>,
    ) -> Result<Self, SourceError> {
        Self::open_with_clock(config, durable_cursor, SystemNodeClock::default())
    }
}

impl<C: NodeReceiveClock> NodeBlockDirectorySource<C> {
    pub fn open_with_clock(
        config: NodeBlockDirectoryConfig,
        durable_cursor: Option<SourceCursor>,
        clock: C,
    ) -> Result<Self, SourceError> {
        let metadata = safe_directory_metadata(&config.root)?;
        let root_identity = file_identity(&metadata);
        let epoch = directory_epoch(&config.stream_name, root_identity);
        if durable_cursor
            .as_ref()
            .is_some_and(|cursor| cursor.epoch() != epoch)
        {
            return Err(SourceError::CursorRegression);
        }
        let first_needed_height = match &durable_cursor {
            Some(cursor) => cursor
                .offset()
                .checked_add(1)
                .ok_or(SourceError::CursorRegression)?,
            None => config.start_height,
        };
        find_block_path(&config, first_needed_height)?;
        let last_read_height = durable_cursor.as_ref().map(SourceCursor::offset);
        Ok(Self {
            config,
            root_identity,
            epoch,
            last_read_height,
            durable_cursor,
            pending_emitted_cursor: None,
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

    async fn wait_for_progress(&self, context: &SourceRequestContext) -> Result<(), SourceError> {
        context.check()?;
        let now = tokio::time::Instant::now();
        let deadline = context.backpressure_deadline();
        let wake = now
            .checked_add(self.config.poll_interval)
            .map_or(deadline, |poll| poll.min(deadline));
        tokio::select! {
            () = context.cancellation().cancelled() => Err(SourceError::Cancelled),
            () = tokio::time::sleep_until(wake) => context.check(),
        }
    }

    async fn next_block(&self) -> Result<Option<(u64, PathBuf)>, SourceError> {
        let config = self.config.clone();
        let root_identity = self.root_identity;
        let last_read_height = self.last_read_height;
        tokio::task::spawn_blocking(move || {
            find_next_block(&config, root_identity, last_read_height)
        })
        .await
        .map_err(|_| SourceError::Configuration("node block discovery task failed".into()))?
    }

    async fn read_block(
        &mut self,
        height: u64,
        path: PathBuf,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError> {
        let max_payload_bytes = self.config.max_payload_bytes;
        let payload =
            tokio::task::spawn_blocking(move || read_stable_block(&path, max_payload_bytes))
                .await
                .map_err(|_| SourceError::Configuration("node block read task failed".into()))??;
        context.check()?;
        let cursor = SourceCursor::new(self.epoch.clone(), height)
            .map_err(|_| SourceError::MalformedPayload("node block cursor is invalid".into()))?;
        let bytes = Bytes::from(payload);
        let observation = SourceObservation::new(
            self.config.source_id.clone(),
            self.config.source_version.clone(),
            hl_protocol::ObservationClass::CommittedBlock,
            cursor.clone(),
            self.clock.now()?,
            self.config.parser_schema_version.clone(),
            bytes,
            Vec::<ParseWarning>::new(),
            self.config.max_payload_bytes,
        )
        .map_err(|_| SourceError::MalformedPayload("node block observation is invalid".into()))?;
        self.last_read_height = Some(height);
        self.pending_emitted_cursor = Some(cursor);
        Ok(observation)
    }
}

#[async_trait]
impl<C: NodeReceiveClock> BlockSource for NodeBlockDirectorySource<C> {
    async fn next_observation(
        &mut self,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError> {
        loop {
            context.check()?;
            if self.pending_emitted_cursor.is_some() {
                return Err(SourceError::BackpressureTimeout);
            }
            if let Some((height, path)) = self.next_block().await? {
                context.check()?;
                return self.read_block(height, path, context).await;
            }
            self.wait_for_progress(context).await?;
        }
    }

    fn source_id(&self) -> &SourceId {
        &self.config.source_id
    }

    fn committed_cursor(&self) -> Option<&SourceCursor> {
        self.durable_cursor.as_ref()
    }
}

fn find_next_block(
    config: &NodeBlockDirectoryConfig,
    root_identity: FileIdentity,
    last_read_height: Option<u64>,
) -> Result<Option<(u64, PathBuf)>, SourceError> {
    let metadata = safe_directory_metadata(&config.root)?;
    if file_identity(&metadata) != root_identity {
        return Err(SourceError::CursorRegression);
    }
    let expected = match last_read_height {
        Some(last) => last.checked_add(1).ok_or(SourceError::CursorRegression)?,
        None => config.start_height,
    };
    let leaves = discover_block_leaves(&config.root)?;
    if let Some(path) = find_block_path_in_leaves(config, &leaves, expected)? {
        return Ok(Some((expected, path)));
    }
    for delta in 1..=GAP_PROBE_HEIGHTS {
        let Some(probe) = expected.checked_add(delta) else {
            break;
        };
        if find_block_path_in_leaves(config, &leaves, probe)?.is_some() {
            return Err(SourceError::RangeUnavailable);
        }
    }
    Ok(None)
}

fn find_block_path(
    config: &NodeBlockDirectoryConfig,
    height: u64,
) -> Result<Option<PathBuf>, SourceError> {
    let leaves = discover_block_leaves(&config.root)?;
    find_block_path_in_leaves(config, &leaves, height)
}

fn find_block_path_in_leaves(
    config: &NodeBlockDirectoryConfig,
    leaves: &[PathBuf],
    height: u64,
) -> Result<Option<PathBuf>, SourceError> {
    let mut candidates = Vec::new();
    for leaf in leaves {
        let candidate = leaf.join(height.to_string());
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SourceError::Configuration(
                        "node block path is not a regular file".into(),
                    ));
                }
                candidates.push(candidate);
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(SourceError::TemporaryDisconnect(
                    "node block metadata failed".into(),
                ));
            }
        }
    }
    candidates.sort();
    let Some(selected) = candidates.last().cloned() else {
        return Ok(None);
    };
    if candidates.len() > 1 {
        let expected = read_stable_block(&selected, config.max_payload_bytes)?;
        let expected_hash = blake3::hash(&expected);
        for candidate in &candidates[..candidates.len() - 1] {
            let payload = read_stable_block(candidate, config.max_payload_bytes)?;
            if blake3::hash(&payload) != expected_hash {
                return Err(SourceError::SchemaDrift(
                    "conflicting duplicate node block height".into(),
                ));
            }
        }
    }
    Ok(Some(selected))
}

fn discover_block_leaves(root: &Path) -> Result<Vec<PathBuf>, SourceError> {
    let mut leaves = Vec::new();
    for session in strict_subdirectories(root, is_decimal_name)? {
        for date in strict_subdirectories(&session, is_date_name)? {
            leaves.push(date);
        }
    }
    leaves.sort();
    Ok(leaves)
}

fn strict_subdirectories(
    root: &Path,
    valid_name: fn(&str) -> bool,
) -> Result<Vec<PathBuf>, SourceError> {
    let entries = fs::read_dir(root)
        .map_err(|_| {
            SourceError::TemporaryDisconnect("node block directory is unavailable".into())
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| SourceError::TemporaryDisconnect("node block directory scan failed".into()))?;
    let mut directories = Vec::with_capacity(entries.len());
    for entry in entries {
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            SourceError::Configuration("node block directory name is not UTF-8".into())
        })?;
        let file_type = entry
            .file_type()
            .map_err(|_| SourceError::TemporaryDisconnect("node block type failed".into()))?;
        if file_type.is_symlink() || !file_type.is_dir() || !valid_name(name) {
            return Err(SourceError::Configuration(
                "node block directory layout is invalid".into(),
            ));
        }
        directories.push(entry.path());
    }
    directories.sort();
    Ok(directories)
}

fn is_decimal_name(name: &str) -> bool {
    !name.is_empty() && name.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_date_name(name: &str) -> bool {
    name.len() == 8 && is_decimal_name(name)
}

fn read_stable_block(path: &Path, max_payload_bytes: usize) -> Result<Vec<u8>, SourceError> {
    let before = safe_regular_metadata(path)?;
    if before.len() == 0
        || before.len()
            > u64::try_from(max_payload_bytes)
                .map_err(|_| SourceError::Configuration("node payload limit overflow".into()))?
    {
        return Err(SourceError::MalformedPayload(
            "node block size is outside its configured limit".into(),
        ));
    }
    let mut file = File::open(path)
        .map_err(|_| SourceError::TemporaryDisconnect("node block file is unavailable".into()))?;
    let mut payload = Vec::new();
    file.by_ref()
        .take(
            u64::try_from(max_payload_bytes)
                .map_err(|_| SourceError::Configuration("node payload limit overflow".into()))?
                + 1,
        )
        .read_to_end(&mut payload)
        .map_err(|_| SourceError::TemporaryDisconnect("node block read failed".into()))?;
    if payload.len() > max_payload_bytes {
        return Err(SourceError::MalformedPayload(
            "node block exceeds its configured limit".into(),
        ));
    }
    let after = file
        .metadata()
        .map_err(|_| SourceError::TemporaryDisconnect("node block metadata failed".into()))?;
    if before.len() != after.len()
        || file_identity(&before) != file_identity(&after)
        || before.modified().ok() != after.modified().ok()
    {
        return Err(SourceError::TemporaryDisconnect(
            "node block changed while it was read".into(),
        ));
    }
    Ok(payload)
}

fn safe_directory_metadata(path: &Path) -> Result<Metadata, SourceError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        SourceError::TemporaryDisconnect("node block directory is unavailable".into())
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SourceError::Configuration(
            "node block root must be a regular non-symlink directory".into(),
        ));
    }
    Ok(metadata)
}

fn directory_epoch(stream_name: &str, identity: FileIdentity) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"alpha-desk-node-block-directory-epoch-v1\0");
    hasher.update(stream_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(&identity.device.to_le_bytes());
    hasher.update(&identity.inode.to_le_bytes());
    format!("node-block-dir-v1:{}", hasher.finalize().to_hex())
}

// ponytail: gitbook documents ABCI as `{date}/{height}.rmp` every 10,000
// heights. L4 snapshots are JSON `[[coin, [bids, asks]], ...]` computed from
// ABCI state; the on-disk directory is not named. This adapter uses the same
// `{date}/{height}.json` layout until the node repo documents the path.
#[derive(Debug, Clone)]
pub struct NodeSnapshotDirectoryConfig {
    root: PathBuf,
    stream_name: String,
    stream: NodeStreamKind,
    source_id: SourceId,
    source_version: String,
    parser_schema_version: String,
    start_height: u64,
    max_payload_bytes: usize,
    poll_interval: Duration,
}

impl NodeSnapshotDirectoryConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        stream_name: impl Into<String>,
        stream: NodeStreamKind,
        source_id: SourceId,
        source_version: impl Into<String>,
        parser_schema_version: impl Into<String>,
        start_height: u64,
        max_payload_bytes: usize,
        poll_interval: Duration,
    ) -> Result<Self, SourceError> {
        let stream_name = stream_name.into();
        let source_version = source_version.into();
        let parser_schema_version = parser_schema_version.into();
        if !root.is_absolute()
            || !stream.is_whole_file_snapshot()
            || !start_height.is_multiple_of(PERIODIC_SNAPSHOT_STRIDE)
            || !valid_identity(&stream_name)
            || !valid_identity(&source_version)
            || !valid_identity(&parser_schema_version)
            || !(1..=MAX_NODE_PAYLOAD_BYTES).contains(&max_payload_bytes)
            || poll_interval.is_zero()
            || Instant::now().checked_add(poll_interval).is_none()
        {
            return Err(SourceError::Configuration(
                "invalid node snapshot-directory configuration".into(),
            ));
        }
        Ok(Self {
            root,
            stream_name,
            stream,
            source_id,
            source_version,
            parser_schema_version,
            start_height,
            max_payload_bytes,
            poll_interval,
        })
    }
}

#[derive(Debug)]
pub struct NodeSnapshotDirectorySource<C = SystemNodeClock> {
    config: NodeSnapshotDirectoryConfig,
    root_identity: FileIdentity,
    epoch: String,
    last_read_height: Option<u64>,
    durable_cursor: Option<SourceCursor>,
    pending_emitted_cursor: Option<SourceCursor>,
    clock: C,
}

impl NodeSnapshotDirectorySource<SystemNodeClock> {
    pub fn open(
        config: NodeSnapshotDirectoryConfig,
        durable_cursor: Option<SourceCursor>,
    ) -> Result<Self, SourceError> {
        Self::open_with_clock(config, durable_cursor, SystemNodeClock::default())
    }
}

impl<C: NodeReceiveClock> NodeSnapshotDirectorySource<C> {
    pub fn open_with_clock(
        config: NodeSnapshotDirectoryConfig,
        durable_cursor: Option<SourceCursor>,
        clock: C,
    ) -> Result<Self, SourceError> {
        let metadata = safe_directory_metadata(&config.root)?;
        let root_identity = file_identity(&metadata);
        let epoch = snapshot_directory_epoch(&config.stream_name, root_identity);
        if durable_cursor
            .as_ref()
            .is_some_and(|cursor| cursor.epoch() != epoch)
        {
            return Err(SourceError::CursorRegression);
        }
        let first_needed_height = match &durable_cursor {
            Some(cursor) => {
                if !cursor.offset().is_multiple_of(PERIODIC_SNAPSHOT_STRIDE) {
                    return Err(SourceError::CursorRegression);
                }
                cursor
                    .offset()
                    .checked_add(PERIODIC_SNAPSHOT_STRIDE)
                    .ok_or(SourceError::CursorRegression)?
            }
            None => config.start_height,
        };
        find_snapshot_path(&config, first_needed_height)?;
        let last_read_height = durable_cursor.as_ref().map(SourceCursor::offset);
        Ok(Self {
            config,
            root_identity,
            epoch,
            last_read_height,
            durable_cursor,
            pending_emitted_cursor: None,
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

    async fn wait_for_progress(&self, context: &SourceRequestContext) -> Result<(), SourceError> {
        context.check()?;
        let now = tokio::time::Instant::now();
        let deadline = context.backpressure_deadline();
        let wake = now
            .checked_add(self.config.poll_interval)
            .map_or(deadline, |poll| poll.min(deadline));
        tokio::select! {
            () = context.cancellation().cancelled() => Err(SourceError::Cancelled),
            () = tokio::time::sleep_until(wake) => context.check(),
        }
    }

    async fn next_snapshot(&self) -> Result<Option<(u64, PathBuf)>, SourceError> {
        let config = self.config.clone();
        let root_identity = self.root_identity;
        let last_read_height = self.last_read_height;
        tokio::task::spawn_blocking(move || {
            find_next_snapshot(&config, root_identity, last_read_height)
        })
        .await
        .map_err(|_| SourceError::Configuration("node snapshot discovery task failed".into()))?
    }

    async fn read_snapshot(
        &mut self,
        height: u64,
        path: PathBuf,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError> {
        let max_payload_bytes = self.config.max_payload_bytes;
        let payload =
            tokio::task::spawn_blocking(move || read_stable_block(&path, max_payload_bytes))
                .await
                .map_err(|_| {
                    SourceError::Configuration("node snapshot read task failed".into())
                })??;
        context.check()?;
        let cursor = SourceCursor::new(self.epoch.clone(), height)
            .map_err(|_| SourceError::MalformedPayload("node snapshot cursor is invalid".into()))?;
        let observation_class = self.config.stream.observation_class();
        let observation = SourceObservation::new(
            self.config.source_id.clone(),
            self.config.source_version.clone(),
            observation_class,
            cursor.clone(),
            self.clock.now()?,
            self.config.parser_schema_version.clone(),
            Bytes::from(payload),
            Vec::<ParseWarning>::new(),
            self.config.max_payload_bytes,
        )
        .map_err(|_| {
            SourceError::MalformedPayload("node snapshot observation is invalid".into())
        })?;
        self.last_read_height = Some(height);
        self.pending_emitted_cursor = Some(cursor);
        Ok(observation)
    }
}

#[async_trait]
impl<C: NodeReceiveClock> BlockSource for NodeSnapshotDirectorySource<C> {
    async fn next_observation(
        &mut self,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError> {
        loop {
            context.check()?;
            if self.pending_emitted_cursor.is_some() {
                return Err(SourceError::BackpressureTimeout);
            }
            if let Some((height, path)) = self.next_snapshot().await? {
                context.check()?;
                return self.read_snapshot(height, path, context).await;
            }
            self.wait_for_progress(context).await?;
        }
    }

    fn source_id(&self) -> &SourceId {
        &self.config.source_id
    }

    fn committed_cursor(&self) -> Option<&SourceCursor> {
        self.durable_cursor.as_ref()
    }
}

fn find_next_snapshot(
    config: &NodeSnapshotDirectoryConfig,
    root_identity: FileIdentity,
    last_read_height: Option<u64>,
) -> Result<Option<(u64, PathBuf)>, SourceError> {
    let metadata = safe_directory_metadata(&config.root)?;
    if file_identity(&metadata) != root_identity {
        return Err(SourceError::CursorRegression);
    }
    let expected = match last_read_height {
        Some(last) => last
            .checked_add(PERIODIC_SNAPSHOT_STRIDE)
            .ok_or(SourceError::CursorRegression)?,
        None => config.start_height,
    };
    let dates = discover_snapshot_dates(&config.root)?;
    if let Some(path) = find_snapshot_path_in_dates(config, &dates, expected)? {
        return Ok(Some((expected, path)));
    }
    for delta in 1..=GAP_PROBE_HEIGHTS {
        let Some(probe) = expected.checked_add(delta.saturating_mul(PERIODIC_SNAPSHOT_STRIDE))
        else {
            break;
        };
        if find_snapshot_path_in_dates(config, &dates, probe)?.is_some() {
            return Err(SourceError::RangeUnavailable);
        }
    }
    Ok(None)
}

fn find_snapshot_path(
    config: &NodeSnapshotDirectoryConfig,
    height: u64,
) -> Result<Option<PathBuf>, SourceError> {
    let dates = discover_snapshot_dates(&config.root)?;
    find_snapshot_path_in_dates(config, &dates, height)
}

fn find_snapshot_path_in_dates(
    config: &NodeSnapshotDirectoryConfig,
    dates: &[PathBuf],
    height: u64,
) -> Result<Option<PathBuf>, SourceError> {
    let Some(extension) = config.stream.snapshot_file_extension() else {
        return Err(SourceError::Configuration(
            "snapshot directory stream is not a whole-file snapshot".into(),
        ));
    };
    let file_name = format!("{height}.{extension}");
    let mut candidates = Vec::new();
    for date in dates {
        let candidate = date.join(&file_name);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(SourceError::Configuration(
                        "node snapshot path is not a regular file".into(),
                    ));
                }
                candidates.push(candidate);
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(SourceError::TemporaryDisconnect(
                    "node snapshot metadata failed".into(),
                ));
            }
        }
    }
    candidates.sort();
    let Some(selected) = candidates.last().cloned() else {
        return Ok(None);
    };
    if candidates.len() > 1 {
        let expected = read_stable_block(&selected, config.max_payload_bytes)?;
        let expected_hash = blake3::hash(&expected);
        for candidate in &candidates[..candidates.len() - 1] {
            let payload = read_stable_block(candidate, config.max_payload_bytes)?;
            if blake3::hash(&payload) != expected_hash {
                return Err(SourceError::SchemaDrift(
                    "conflicting duplicate node snapshot height".into(),
                ));
            }
        }
    }
    Ok(Some(selected))
}

fn discover_snapshot_dates(root: &Path) -> Result<Vec<PathBuf>, SourceError> {
    strict_subdirectories(root, is_date_name)
}

fn snapshot_directory_epoch(stream_name: &str, identity: FileIdentity) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"alpha-desk-node-snapshot-directory-epoch-v1\0");
    hasher.update(stream_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(&identity.device.to_le_bytes());
    hasher.update(&identity.inode.to_le_bytes());
    format!("node-snapshot-dir-v1:{}", hasher.finalize().to_hex())
}
