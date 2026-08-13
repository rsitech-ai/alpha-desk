use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use domain_types::{BlockHeight, ChainId, SourceId};
use hl_protocol::{
    BlockSource, SourceAdmission, SourceCursor, SourceError, SourceRequestContext, SourceTrust,
};
use storage_ports::{CaptureProgressStore, CursorPolicy, ProgressError};
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::adapters::{
    NodeBlockDirectoryConfig, NodeBlockDirectorySource, NodeFileConfig, NodeLineFileSource,
};
use crate::app::CaptureRuntimeHealth;
use crate::auxiliary_checkpoint::AuxiliaryArchiveCheckpoint;
use crate::coordinator::{CaptureCoordinator, CoordinatorError};
use crate::spool::{
    CloseReceipt, DurabilityPolicy, SourceSpool, SourceSpoolAppendDisposition, SourceSpoolConfig,
    SpoolError, SpoolRead, SpoolReader, SpoolRotationPolicy,
};
use crate::{
    AppError, BacklogError, BacklogRead, CaptureConfig, CommittedNodePipeline,
    CommittedNodePipelineConfig, DiskReserveError, DiskReserveGuard, DiskSpaceProbe,
    FilesystemDiskSpaceProbe, OwnedTask, PipelineError, PipelineOutcome, RawSegmentArchive,
    RawSegmentArchiveConfig, RawSegmentArchiveError, RawSegmentArchiveVerification,
    RawSpoolArchiveEvidence, SourceAdapterConfig, SpoolBacklog,
};

const SPOOL_SCHEMA_VERSION: &str = "spool-v1";
const PRIMARY_ACQUISITION_TASK_NAME: &str = "primary-node-acquisition";
const INDEPENDENT_ACQUISITION_TASK_NAME: &str = "independent-node-acquisition";
const DRAIN_TASK_NAME: &str = "committed-source-drain";
const AUXILIARY_ACQUISITION_TASK_NAME: &str = "auxiliary-node-acquisition-supervisor";
const WRITE_HEADROOM_BYTES: u64 = 1024 * 1024;
const RAW_ARCHIVE_BATCH_BYTES: u64 = 64 * 1024 * 1024;
const BACKLOG_POLL_DELAY: Duration = Duration::from_millis(250);
const RETRY_BACKOFF_BASE_MILLIS: u64 = 250;
const RETRY_BACKOFF_CEILING_BASE_MILLIS: u64 = 25_000;
const RETRY_JITTER_MIN_BPS: u64 = 8_000;
const RETRY_JITTER_SPAN_BPS: u64 = 4_001;
const BASIS_POINTS: u64 = 10_000;
const MAX_AUXILIARY_SOURCES: usize = 16;
const RAW_ARCHIVE_BATCH_RECORDS: usize = 4_096;

fn prepare_source_spool_path(
    configured_path: &Path,
    source_id: &SourceId,
) -> Result<PathBuf, SourceRuntimeError> {
    let parent = configured_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or(SourceRuntimeError::InvalidConfig)?;
    if configured_path.file_name() != Some(std::ffi::OsStr::new(source_id.as_str())) {
        return Err(SourceRuntimeError::InvalidConfig);
    }
    std::fs::create_dir_all(parent).map_err(|_| SourceRuntimeError::InvalidConfig)?;
    let canonical_parent =
        std::fs::canonicalize(parent).map_err(|_| SourceRuntimeError::InvalidConfig)?;
    match std::fs::symlink_metadata(configured_path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(SourceRuntimeError::InvalidConfig);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(configured_path).map_err(|_| SourceRuntimeError::InvalidConfig)?;
        }
        Err(_) => return Err(SourceRuntimeError::InvalidConfig),
    }
    let canonical_source =
        std::fs::canonicalize(configured_path).map_err(|_| SourceRuntimeError::InvalidConfig)?;
    if canonical_source.parent() != Some(canonical_parent.as_path()) {
        return Err(SourceRuntimeError::InvalidConfig);
    }
    Ok(configured_path.to_path_buf())
}

fn auxiliary_commit_policy(
    durability: crate::config::DurabilityPolicy,
    queue_capacity: usize,
    backpressure_timeout: Duration,
) -> Result<(usize, Duration), SourceRuntimeError> {
    match durability {
        crate::config::DurabilityPolicy::FsyncEveryRecord => Ok((1, backpressure_timeout)),
        crate::config::DurabilityPolicy::Batched {
            max_records,
            max_delay_millis,
        } => {
            let max_records =
                usize::try_from(max_records).map_err(|_| SourceRuntimeError::InvalidConfig)?;
            Ok((
                max_records.min(queue_capacity),
                Duration::from_millis(max_delay_millis),
            ))
        }
    }
}

fn group_commit_due(
    pending_len: usize,
    max_records: usize,
    deadline: Option<Instant>,
    now: Instant,
) -> bool {
    pending_len != 0
        && (pending_len >= max_records || deadline.is_some_and(|deadline| deadline <= now))
}

fn archive_lineage_identity(path: &Path) -> Result<String, SourceRuntimeError> {
    let canonical = std::fs::canonicalize(path).map_err(|_| SourceRuntimeError::InvalidConfig)?;
    Ok(hex::encode(
        blake3::hash(canonical.to_string_lossy().as_bytes()).as_bytes(),
    ))
}

#[derive(Debug)]
struct RetryBackoff {
    source_seed: [u8; 32],
    attempt: u32,
}

impl RetryBackoff {
    fn new(source_id: &SourceId) -> Self {
        Self {
            source_seed: *blake3::hash(source_id.as_str().as_bytes()).as_bytes(),
            attempt: 0,
        }
    }

    fn next_delay(&mut self) -> Duration {
        let exponent = self.attempt.min(16);
        let multiplier = 1_u64 << exponent;
        let base_millis = RETRY_BACKOFF_BASE_MILLIS
            .saturating_mul(multiplier)
            .min(RETRY_BACKOFF_CEILING_BASE_MILLIS);
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"hyperliquid-alpha-desk/capture-retry-jitter/v1");
        hasher.update(&self.source_seed);
        hasher.update(&self.attempt.to_le_bytes());
        let digest = hasher.finalize();
        let random = u64::from_le_bytes(
            digest.as_bytes()[..8]
                .try_into()
                .expect("BLAKE3 digest has at least eight bytes"),
        );
        let jitter_bps = RETRY_JITTER_MIN_BPS + (random % RETRY_JITTER_SPAN_BPS);
        let delay_millis = base_millis.saturating_mul(jitter_bps) / BASIS_POINTS;
        self.attempt = self.attempt.saturating_add(1);
        Duration::from_millis(delay_millis)
    }

    fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[derive(Debug, Clone)]
struct NodeSourceTaskConfig {
    role: CommittedSourceRole,
    chain_id: ChainId,
    source_id: SourceId,
    source_version: String,
    admission: SourceAdmission,
    parser_version: String,
    source_path: PathBuf,
    stream_name: String,
    first_height: BlockHeight,
    start_height: u64,
    poll_interval: Duration,
    queue_capacity: usize,
    max_payload_bytes: usize,
    spool_path: PathBuf,
    archive_path: PathBuf,
    segment_target_bytes: u64,
    rotation_interval: Duration,
    backpressure_timeout: Duration,
    max_pending_blocks: usize,
    retained_committed_blocks: usize,
    disk_reserve_bytes: u64,
}

#[derive(Debug, Clone)]
struct AuxiliaryNodeSourceTaskConfig {
    chain_id: ChainId,
    source_id: SourceId,
    source_version: String,
    parser_version: String,
    source_path: PathBuf,
    stream_name: String,
    stream: hl_protocol::node::v1::NodeStreamKind,
    poll_interval: Duration,
    max_payload_bytes: usize,
    spool_path: PathBuf,
    archive_path: PathBuf,
    segment_target_bytes: u64,
    rotation_interval: Duration,
    backpressure_timeout: Duration,
    archive_commit_max_records: usize,
    archive_commit_max_delay: Duration,
    disk_reserve_bytes: u64,
}

#[derive(Debug)]
struct PendingAuxiliaryAcknowledgement {
    cursor: SourceCursor,
    received_wall_micros: i64,
    quarantine_reason: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RecoveredAuxiliaryEvidence {
    last_received_wall_micros: i64,
    quarantine_reason: Option<String>,
}

fn segment_auxiliary_evidence(
    segment: &CloseReceipt,
    source_id: &SourceId,
    source_version: &str,
    prior_quarantine_reason: Option<String>,
) -> Result<Option<RecoveredAuxiliaryEvidence>, SourceRuntimeError> {
    let mut last_received_wall_micros = None;
    let mut quarantine_reason = prior_quarantine_reason;
    segment
        .verify_current()
        .map_err(SourceRuntimeError::Spool)?;
    let reader = SpoolReader::open(segment.segment_path()).map_err(SourceRuntimeError::Spool)?;
    if reader.header().source_id() != source_id
        || reader.header().source_version() != source_version
    {
        return Err(SourceRuntimeError::InvalidConfig);
    }
    let mut records = reader.stream().map_err(SourceRuntimeError::Spool)?;
    loop {
        let record = match records.next_record().map_err(SourceRuntimeError::Spool)? {
            SpoolRead::Record(record) => record,
            SpoolRead::EndOfFile => break,
            SpoolRead::IncompleteTail { record_offset } => {
                return Err(SourceRuntimeError::Spool(SpoolError::IncompleteTail {
                    record_offset,
                }));
            }
        };
        last_received_wall_micros = Some(record.received().wall_micros());
        if let Some(reason) = record
            .parser_schema_version()
            .strip_prefix("quarantine-v1:")
            .filter(|reason| !reason.is_empty())
        {
            crate::status::validate_reason_code(reason)
                .map_err(|_| SourceRuntimeError::InvalidConfig)?;
            quarantine_reason = Some(reason.to_owned());
        }
    }
    Ok(
        last_received_wall_micros.map(|last_received_wall_micros| RecoveredAuxiliaryEvidence {
            last_received_wall_micros,
            quarantine_reason,
        }),
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommittedSourceRole {
    Primary,
    Independent,
}

impl CommittedSourceRole {
    const fn acquisition_task_name(self) -> &'static str {
        match self {
            Self::Primary => PRIMARY_ACQUISITION_TASK_NAME,
            Self::Independent => INDEPENDENT_ACQUISITION_TASK_NAME,
        }
    }

    const fn source_class(self) -> crate::CommittedSourceClass {
        match self {
            Self::Primary => crate::CommittedSourceClass::LocallyVerifiedCommitted,
            Self::Independent => crate::CommittedSourceClass::IndependentCommitted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum SourceNotification {
    Durable,
    VisibleGap {
        role: CommittedSourceRole,
        source_id: SourceId,
        height: BlockHeight,
    },
}

#[derive(Debug)]
struct CommittedDrainConfig {
    primary: NodeSourceTaskConfig,
    independent: Option<NodeSourceTaskConfig>,
    failover_store: Arc<crate::FailoverStore>,
    failover_decision: Option<crate::FailoverDecision>,
}

pub(crate) fn committed_node_tasks(
    config: &CaptureConfig,
    progress: Arc<dyn CaptureProgressStore>,
    coordinator: Arc<CaptureCoordinator>,
    raw_archive: Arc<dyn RawSegmentArchive>,
    failover_store: Arc<crate::FailoverStore>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<Vec<OwnedTask>, SourceRuntimeError> {
    let mut primary = None;
    let mut independent = None;
    for source in config.sources() {
        let Some(SourceAdapterConfig::NodeBlockDirectory {
            path,
            stream_name,
            start_height,
            poll_interval_millis,
            replica_cmds_style: _,
        }) = source.adapter()
        else {
            continue;
        };
        let admission = source
            .admission()
            .map_err(|_| SourceRuntimeError::InvalidConfig)?;
        if !admission.can_advance_committed_watermark() {
            return Err(SourceRuntimeError::InvalidConfig);
        }
        let role = match source.trust() {
            SourceTrust::LocallyVerifiedCommitted => CommittedSourceRole::Primary,
            SourceTrust::IndependentCommitted => CommittedSourceRole::Independent,
            _ => return Err(SourceRuntimeError::InvalidConfig),
        };
        let selected = NodeSourceTaskConfig {
            role,
            chain_id: config.runtime().chain_id(),
            source_id: SourceId::new(source.id().to_owned())
                .map_err(|_| SourceRuntimeError::InvalidConfig)?,
            source_version: source.source_version().to_owned(),
            admission,
            parser_version: config.parser_version().to_owned(),
            source_path: path.clone(),
            stream_name: stream_name.clone(),
            first_height: config.runtime().first_height(),
            start_height: *start_height,
            poll_interval: Duration::from_millis(*poll_interval_millis),
            queue_capacity: source.queue_capacity(),
            max_payload_bytes: source.max_payload_bytes(),
            spool_path: config.spool().path().join(source.id()),
            archive_path: config.runtime().archive_path().to_path_buf(),
            segment_target_bytes: config.spool().segment_target_bytes(),
            rotation_interval: Duration::from_secs(config.spool().rotation_interval_seconds()),
            backpressure_timeout: Duration::from_millis(
                config.runtime().backpressure_timeout_millis(),
            ),
            max_pending_blocks: config.runtime().max_pending_blocks(),
            retained_committed_blocks: config.runtime().retained_committed_blocks(),
            disk_reserve_bytes: config.runtime().disk_reserve_bytes(),
        };
        let slot = match role {
            CommittedSourceRole::Primary => &mut primary,
            CommittedSourceRole::Independent => &mut independent,
        };
        if slot.replace(selected).is_some() {
            return Err(SourceRuntimeError::InvalidConfig);
        }
    }
    let primary = primary.ok_or(SourceRuntimeError::InvalidConfig)?;
    let failover_decision = failover_store
        .load()
        .map_err(SourceRuntimeError::Failover)?;
    match (&failover_decision, &independent) {
        (Some(decision), Some(independent)) => decision
            .validate_topology(
                &primary.chain_id,
                &primary.source_id,
                &independent.source_id,
            )
            .map_err(SourceRuntimeError::Failover)?,
        (Some(_), None) => return Err(SourceRuntimeError::InvalidConfig),
        (None, _) => {}
    }
    health.configure_committed_sources(independent.is_some(), failover_decision.as_ref());
    let notification_capacity = independent
        .as_ref()
        .map_or(primary.queue_capacity, |source| {
            primary.queue_capacity.max(source.queue_capacity)
        });
    let (notification, backlog_notifications) = mpsc::channel(notification_capacity);
    let drain_cancellation = cancellation.child_token();
    let mut tasks = Vec::with_capacity(if independent.is_some() { 3 } else { 2 });
    for acquisition_config in [Some(primary.clone()), independent.clone()]
        .into_iter()
        .flatten()
    {
        let task_name = acquisition_config.role.acquisition_task_name();
        let acquisition_archive = Arc::clone(&raw_archive);
        let acquisition_notification = notification.clone();
        let acquisition_health = Arc::clone(&health);
        let acquisition_cancellation = cancellation.child_token();
        tasks.push(OwnedTask::new(task_name, async move {
            run_committed_node_acquisition(
                acquisition_config,
                acquisition_archive,
                acquisition_notification,
                acquisition_health,
                acquisition_cancellation,
            )
            .await
            .map_err(|error| AppError::TaskFailed {
                task: task_name,
                reason_code: error.reason_code(),
            })
        }));
    }
    drop(notification);
    tasks.push(OwnedTask::new(DRAIN_TASK_NAME, async move {
        run_committed_node_drain(
            CommittedDrainConfig {
                primary,
                independent,
                failover_store,
                failover_decision,
            },
            progress,
            coordinator,
            backlog_notifications,
            health,
            drain_cancellation,
        )
        .await
        .map_err(|error| AppError::TaskFailed {
            task: DRAIN_TASK_NAME,
            reason_code: error.reason_code(),
        })
    }));
    Ok(tasks)
}

pub(crate) fn auxiliary_node_task(
    config: &CaptureConfig,
    raw_archive: Arc<dyn RawSegmentArchive>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<Option<OwnedTask>, SourceRuntimeError> {
    let mut sources = Vec::new();
    for source in config.sources() {
        let Some(SourceAdapterConfig::NodeLine {
            path,
            stream_name,
            stream,
            poll_interval_millis,
        }) = source.adapter()
        else {
            continue;
        };
        let admission = source
            .admission()
            .map_err(|_| SourceRuntimeError::InvalidConfig)?;
        if admission.can_advance_committed_watermark() {
            return Err(SourceRuntimeError::InvalidConfig);
        }
        let backpressure_timeout =
            Duration::from_millis(config.runtime().backpressure_timeout_millis());
        let (archive_commit_max_records, archive_commit_max_delay) = auxiliary_commit_policy(
            *config.spool().provisional_durability(),
            source.queue_capacity(),
            backpressure_timeout,
        )?;
        sources.push(AuxiliaryNodeSourceTaskConfig {
            chain_id: config.runtime().chain_id(),
            source_id: SourceId::new(source.id().to_owned())
                .map_err(|_| SourceRuntimeError::InvalidConfig)?,
            source_version: source.source_version().to_owned(),
            parser_version: config.parser_version().to_owned(),
            source_path: path.clone(),
            stream_name: stream_name.clone(),
            stream: *stream,
            poll_interval: Duration::from_millis(*poll_interval_millis),
            max_payload_bytes: source.max_payload_bytes(),
            spool_path: config.spool().path().join(source.id()),
            archive_path: config.runtime().archive_path().to_path_buf(),
            segment_target_bytes: config.spool().segment_target_bytes(),
            rotation_interval: Duration::from_secs(config.spool().rotation_interval_seconds()),
            backpressure_timeout,
            archive_commit_max_records,
            archive_commit_max_delay,
            disk_reserve_bytes: config.runtime().disk_reserve_bytes(),
        });
    }
    if sources.is_empty() {
        return Ok(None);
    }
    if sources.len() > MAX_AUXILIARY_SOURCES {
        return Err(SourceRuntimeError::InvalidConfig);
    }
    health.configure_auxiliary_sources(
        &sources
            .iter()
            .map(|source| source.source_id.as_str().to_owned())
            .collect::<Vec<_>>(),
    );
    Ok(Some(OwnedTask::new(
        AUXILIARY_ACQUISITION_TASK_NAME,
        async move {
            run_auxiliary_node_acquisitions(sources, raw_archive, health, cancellation)
                .await
                .map_err(|error| AppError::TaskFailed {
                    task: AUXILIARY_ACQUISITION_TASK_NAME,
                    reason_code: error.reason_code(),
                })
        },
    )))
}

async fn run_auxiliary_node_acquisitions(
    configs: Vec<AuxiliaryNodeSourceTaskConfig>,
    raw_archive: Arc<dyn RawSegmentArchive>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<(), SourceRuntimeError> {
    let mut tasks = JoinSet::new();
    let mut task_sources = HashMap::new();
    for config in configs {
        let archive = Arc::clone(&raw_archive);
        let source_health = Arc::clone(&health);
        let source_cancellation = cancellation.child_token();
        let source_id = config.source_id.clone();
        let task_source_id = source_id.clone();
        let handle = tasks.spawn(async move {
            let result =
                run_auxiliary_node_acquisition(config, archive, source_health, source_cancellation)
                    .await;
            (source_id, result)
        });
        task_sources.insert(handle.id(), task_source_id);
    }
    supervise_auxiliary_tasks(tasks, task_sources, health, cancellation).await
}

async fn supervise_auxiliary_tasks(
    mut tasks: JoinSet<(SourceId, Result<(), SourceRuntimeError>)>,
    mut task_sources: HashMap<tokio::task::Id, SourceId>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<(), SourceRuntimeError> {
    let mut first_error = None;
    loop {
        tokio::select! {
            () = cancellation.cancelled(), if first_error.is_none() => {}
            result = tasks.join_next_with_id(), if !tasks.is_empty() => {
                match result {
                    Some(Ok((task_id, (source_id, Ok(()))))) if cancellation.is_cancelled() => {
                        task_sources.remove(&task_id);
                        tracing::info!(source_id = %source_id.as_str(), "auxiliary source stopped");
                    }
                    Some(Ok((task_id, (source_id, Ok(()))))) => {
                        task_sources.remove(&task_id);
                        health.latch_auxiliary(source_id.as_str(), SourceRuntimeError::UnexpectedSourceExit.reason_code());
                        tracing::error!(source_id = %source_id.as_str(), "auxiliary source exited before shutdown");
                        first_error = Some(SourceRuntimeError::UnexpectedSourceExit);
                    }
                    Some(Ok((task_id, (source_id, Err(error))))) => {
                        task_sources.remove(&task_id);
                        health.latch_auxiliary(source_id.as_str(), error.reason_code());
                        tracing::error!(source_id = %source_id.as_str(), reason_code = error.reason_code(), "auxiliary source failed");
                        first_error = Some(error);
                    }
                    Some(Err(error)) => {
                        if let Some(source_id) = task_sources.remove(&error.id()) {
                            health.latch_auxiliary(source_id.as_str(), SourceRuntimeError::BlockingTask.reason_code());
                        }
                        first_error = Some(SourceRuntimeError::BlockingTask);
                    }
                    None => break,
                }
            }
        }
        if cancellation.is_cancelled() || first_error.is_some() {
            cancellation.cancel();
            while let Some(result) = tasks.join_next_with_id().await {
                match result {
                    Ok((task_id, (source_id, Err(error)))) => {
                        task_sources.remove(&task_id);
                        health.latch_auxiliary(source_id.as_str(), error.reason_code());
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                    Ok((task_id, _)) => {
                        task_sources.remove(&task_id);
                    }
                    Err(error) => {
                        if let Some(source_id) = task_sources.remove(&error.id()) {
                            health.latch_auxiliary(
                                source_id.as_str(),
                                SourceRuntimeError::BlockingTask.reason_code(),
                            );
                        }
                        if first_error.is_none() {
                            first_error = Some(SourceRuntimeError::BlockingTask);
                        }
                    }
                }
            }
            break;
        }
    }
    first_error.map_or(Ok(()), Err)
}

async fn run_auxiliary_node_acquisition(
    config: AuxiliaryNodeSourceTaskConfig,
    raw_archive: Arc<dyn RawSegmentArchive>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<(), SourceRuntimeError> {
    run_auxiliary_node_acquisition_with_probe(config, raw_archive, health, cancellation, |config| {
        FilesystemDiskSpaceProbe::open([config.spool_path.clone(), config.archive_path.clone()])
    })
    .await
}

async fn run_auxiliary_node_acquisition_with_probe<P, F>(
    config: AuxiliaryNodeSourceTaskConfig,
    raw_archive: Arc<dyn RawSegmentArchive>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
    probe_factory: F,
) -> Result<(), SourceRuntimeError>
where
    P: DiskSpaceProbe,
    F: FnOnce(&AuxiliaryNodeSourceTaskConfig) -> Result<P, DiskReserveError>,
{
    let spool_path = prepare_source_spool_path(&config.spool_path, &config.source_id)?;
    let archive_identity = archive_lineage_identity(&config.archive_path)?;
    let mut checkpoint = AuxiliaryArchiveCheckpoint::load(
        &spool_path,
        &config.source_id,
        &config.source_version,
        &archive_identity,
    )
    .map_err(SourceRuntimeError::Spool)?;
    if let Some(recovered) = &checkpoint {
        let spool_evidence = RawSpoolArchiveEvidence::try_new(
            recovered
                .spool_manifest_blake3()
                .map_err(SourceRuntimeError::Spool)?,
            recovered
                .spool_segment_blake3()
                .map_err(SourceRuntimeError::Spool)?,
            recovered
                .first_local_sequence()
                .map_err(SourceRuntimeError::Spool)?,
            recovered.last_cursor().map_err(SourceRuntimeError::Spool)?,
            recovered
                .last_local_sequence()
                .map_err(SourceRuntimeError::Spool)?,
            recovered.record_count(),
        )
        .map_err(SourceRuntimeError::RawArchive)?;
        let verification = RawSegmentArchiveVerification::new(
            config.chain_id.clone(),
            config.source_id.clone(),
            spool_evidence,
            recovered
                .raw_manifest_ids()
                .map_err(SourceRuntimeError::Spool)?,
        );
        raw_archive
            .verify_archived_segment(&verification)
            .await
            .map_err(SourceRuntimeError::RawArchive)?;
        recovered
            .cleanup_archived_segment(&spool_path)
            .map_err(SourceRuntimeError::Spool)?;
    }
    let mut spool_config = SourceSpoolConfig::try_new_with_cursor_policy(
        spool_path.clone(),
        config.source_id.clone(),
        config.source_version.clone(),
        SPOOL_SCHEMA_VERSION,
        *blake3::hash(env!("CARGO_PKG_VERSION").as_bytes()).as_bytes(),
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(config.segment_target_bytes, config.rotation_interval)
            .map_err(SourceRuntimeError::Spool)?,
        CursorPolicy::MonotonicByteOffset,
    )
    .map_err(SourceRuntimeError::Spool)?;
    if let Some(recovered) = &checkpoint {
        spool_config = spool_config
            .with_baseline(recovered.baseline().map_err(SourceRuntimeError::Spool)?)
            .map_err(SourceRuntimeError::Spool)?;
    }
    let created_at = now_micros()?;
    let mut spool =
        tokio::task::spawn_blocking(move || SourceSpool::open(spool_config, created_at))
            .await
            .map_err(|_| SourceRuntimeError::BlockingTask)?
            .map_err(SourceRuntimeError::Spool)?;
    let disk_guard = DiskReserveGuard::try_new(probe_factory(&config)?, config.disk_reserve_bytes)?;
    let raw_archive_config = RawSegmentArchiveConfig::try_new(
        config.max_payload_bytes,
        RAW_ARCHIVE_BATCH_RECORDS,
        RAW_ARCHIVE_BATCH_BYTES,
    )
    .map_err(SourceRuntimeError::RawArchive)?;
    disk_guard.ensure_write(WRITE_HEADROOM_BYTES)?;
    let (returned_spool, sealed) = tokio::task::spawn_blocking(move || {
        let mut owned = spool;
        let result = owned.seal_active(created_at);
        (owned, result)
    })
    .await
    .map_err(|_| SourceRuntimeError::BlockingTask)?;
    spool = returned_spool;
    sealed.map_err(SourceRuntimeError::Spool)?;
    for segment in spool.closed_segments().to_vec() {
        let evidence = segment_auxiliary_evidence(
            &segment,
            &config.source_id,
            &config.source_version,
            checkpoint
                .as_ref()
                .and_then(AuxiliaryArchiveCheckpoint::quarantine_reason)
                .map(ToOwned::to_owned),
        )?
        .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
        let archive_summary = archive_closed_segment(
            raw_archive.as_ref(),
            &disk_guard,
            &config.chain_id,
            &segment,
            raw_archive_config,
        )
        .await?;
        let checkpoint_directory = spool_path.clone();
        let checkpoint_segment = segment.clone();
        let checkpoint_archive_identity = archive_identity.clone();
        let new_checkpoint = tokio::task::spawn_blocking(move || {
            AuxiliaryArchiveCheckpoint::publish(
                &checkpoint_directory,
                &checkpoint_archive_identity,
                &checkpoint_segment,
                archive_summary.manifest_ids(),
                evidence.last_received_wall_micros,
                evidence.quarantine_reason,
            )
        })
        .await
        .map_err(|_| SourceRuntimeError::BlockingTask)?
        .map_err(SourceRuntimeError::Spool)?;
        spool
            .forget_archived_segment(&segment)
            .map_err(SourceRuntimeError::Spool)?;
        new_checkpoint
            .cleanup_archived_segment(&spool_path)
            .map_err(SourceRuntimeError::Spool)?;
        checkpoint = Some(new_checkpoint);
    }
    if let Some(recovered) = &checkpoint {
        let durable_cursor = spool
            .last_durable_cursor()
            .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
        let local_sequence = spool
            .last_local_sequence()
            .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
        health.record_auxiliary_recovered(
            config.source_id.as_str(),
            durable_cursor,
            local_sequence.get(),
            recovered.last_received_wall_micros(),
            recovered.quarantine_reason(),
        );
    }
    let adapter_config = NodeFileConfig::new_bounded(
        config.source_path.clone(),
        config.stream_name.clone(),
        config.stream,
        config.source_id.clone(),
        config.source_version.clone(),
        config.parser_version.clone(),
        config.max_payload_bytes,
        config.poll_interval,
        config.archive_commit_max_records,
    )
    .map_err(SourceRuntimeError::Source)?;
    let mut retry_backoff = RetryBackoff::new(&config.source_id);
    let opened_source = open_auxiliary_node_source(
        &adapter_config,
        spool.last_durable_cursor().cloned(),
        &config.source_id,
        &health,
        &cancellation,
        &mut retry_backoff,
    )
    .await;
    let Some(mut source) = (match opened_source {
        Ok(source) => source,
        Err(error) => {
            close_spool(
                spool,
                raw_archive.as_ref(),
                &disk_guard,
                &config.chain_id,
                raw_archive_config,
            )
            .await?;
            return Err(error);
        }
    }) else {
        close_spool(
            spool,
            raw_archive.as_ref(),
            &disk_guard,
            &config.chain_id,
            raw_archive_config,
        )
        .await?;
        return Ok(());
    };
    record_auxiliary_tail(&health, &config.source_id, &source)?;
    let mut pending = Vec::<PendingAuxiliaryAcknowledgement>::new();
    let mut group_commit_deadline = None;

    loop {
        let now = Instant::now();
        if group_commit_due(
            pending.len(),
            config.archive_commit_max_records,
            group_commit_deadline,
            now,
        ) {
            spool = flush_auxiliary_observations(
                spool,
                &mut source,
                &mut pending,
                raw_archive.as_ref(),
                &disk_guard,
                &config,
                raw_archive_config,
                &health,
                &mut checkpoint,
                &archive_identity,
            )
            .await?;
            group_commit_deadline = None;
            continue;
        }
        let request_deadline = now
            .checked_add(config.backpressure_timeout)
            .ok_or(SourceRuntimeError::InvalidConfig)?;
        let deadline = if pending.is_empty() {
            request_deadline
        } else {
            group_commit_deadline.unwrap_or(request_deadline)
        };
        let context = SourceRequestContext::new(cancellation.child_token(), deadline);
        let observation = match source.next_observation(&context).await {
            Ok(observation) => observation,
            Err(SourceError::Cancelled) => {
                spool = flush_auxiliary_observations(
                    spool,
                    &mut source,
                    &mut pending,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config,
                    raw_archive_config,
                    &health,
                    &mut checkpoint,
                    &archive_identity,
                )
                .await?;
                close_spool(
                    spool,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config.chain_id,
                    raw_archive_config,
                )
                .await?;
                return Ok(());
            }
            Err(SourceError::BackpressureTimeout) => {
                if group_commit_due(
                    pending.len(),
                    config.archive_commit_max_records,
                    group_commit_deadline,
                    Instant::now(),
                ) {
                    spool = flush_auxiliary_observations(
                        spool,
                        &mut source,
                        &mut pending,
                        raw_archive.as_ref(),
                        &disk_guard,
                        &config,
                        raw_archive_config,
                        &health,
                        &mut checkpoint,
                        &archive_identity,
                    )
                    .await?;
                    group_commit_deadline = None;
                } else {
                    record_auxiliary_tail(&health, &config.source_id, &source)?;
                }
                continue;
            }
            Err(SourceError::TemporaryDisconnect(_)) => {
                spool = flush_auxiliary_observations(
                    spool,
                    &mut source,
                    &mut pending,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config,
                    raw_archive_config,
                    &health,
                    &mut checkpoint,
                    &archive_identity,
                )
                .await?;
                group_commit_deadline = None;
                let Some(reopened) = open_auxiliary_node_source(
                    &adapter_config,
                    spool.last_durable_cursor().cloned(),
                    &config.source_id,
                    &health,
                    &cancellation,
                    &mut retry_backoff,
                )
                .await?
                else {
                    close_spool(
                        spool,
                        raw_archive.as_ref(),
                        &disk_guard,
                        &config.chain_id,
                        raw_archive_config,
                    )
                    .await?;
                    return Ok(());
                };
                source = reopened;
                record_auxiliary_tail(&health, &config.source_id, &source)?;
                continue;
            }
            Err(error @ (SourceError::MalformedPayload(_) | SourceError::SchemaDrift(_))) => {
                let Some(quarantine) = source.pending_quarantine().cloned() else {
                    spool = flush_auxiliary_observations(
                        spool,
                        &mut source,
                        &mut pending,
                        raw_archive.as_ref(),
                        &disk_guard,
                        &config,
                        raw_archive_config,
                        &health,
                        &mut checkpoint,
                        &archive_identity,
                    )
                    .await?;
                    close_spool(
                        spool,
                        raw_archive.as_ref(),
                        &disk_guard,
                        &config.chain_id,
                        raw_archive_config,
                    )
                    .await?;
                    return Err(SourceRuntimeError::Source(error));
                };
                let parser_disposition = format!("quarantine-v1:{}", quarantine.reason_code());
                let observation = hl_protocol::SourceObservation::new(
                    config.source_id.clone(),
                    config.source_version.clone(),
                    quarantine.observation_class(),
                    quarantine.cursor().clone(),
                    quarantine.received(),
                    parser_disposition,
                    quarantine.payload().clone(),
                    Vec::new(),
                    config.max_payload_bytes,
                )
                .map_err(|_| SourceRuntimeError::InvalidQuarantineEvidence)?;
                let cursor = observation.cursor().clone();
                let received_wall_micros = observation.received().wall_micros();
                let (returned_spool, _) = append_auxiliary_observation(
                    spool,
                    observation,
                    &disk_guard,
                    raw_archive.as_ref(),
                    &config.chain_id,
                    &config.source_id,
                )
                .await?;
                spool = returned_spool;
                pending.push(PendingAuxiliaryAcknowledgement {
                    cursor: cursor.clone(),
                    received_wall_micros,
                    quarantine_reason: Some(error.reason_code()),
                });
                record_auxiliary_buffered(
                    &health,
                    &config.source_id,
                    &source,
                    &spool,
                    pending.len(),
                )?;
                spool = flush_auxiliary_observations(
                    spool,
                    &mut source,
                    &mut pending,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config,
                    raw_archive_config,
                    &health,
                    &mut checkpoint,
                    &archive_identity,
                )
                .await?;
                group_commit_deadline = None;
                tracing::warn!(
                    source_id = %config.source_id.as_str(),
                    cursor_epoch = cursor.epoch(),
                    cursor_offset = cursor.offset(),
                    reason_code = error.reason_code(),
                    "persisted quarantined auxiliary source record"
                );
                continue;
            }
            Err(error) => {
                spool = flush_auxiliary_observations(
                    spool,
                    &mut source,
                    &mut pending,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config,
                    raw_archive_config,
                    &health,
                    &mut checkpoint,
                    &archive_identity,
                )
                .await?;
                close_spool(
                    spool,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config.chain_id,
                    raw_archive_config,
                )
                .await?;
                return Err(SourceRuntimeError::Source(error));
            }
        };
        let cursor = observation.cursor().clone();
        let received_wall_micros = observation.received().wall_micros();
        let (returned_spool, rotated) = append_auxiliary_observation(
            spool,
            observation,
            &disk_guard,
            raw_archive.as_ref(),
            &config.chain_id,
            &config.source_id,
        )
        .await?;
        spool = returned_spool;
        pending.push(PendingAuxiliaryAcknowledgement {
            cursor,
            received_wall_micros,
            quarantine_reason: None,
        });
        if group_commit_deadline.is_none() {
            group_commit_deadline = Some(
                Instant::now()
                    .checked_add(config.archive_commit_max_delay)
                    .ok_or(SourceRuntimeError::InvalidConfig)?,
            );
        }
        record_auxiliary_buffered(&health, &config.source_id, &source, &spool, pending.len())?;
        if rotated || pending.len() >= config.archive_commit_max_records {
            spool = flush_auxiliary_observations(
                spool,
                &mut source,
                &mut pending,
                raw_archive.as_ref(),
                &disk_guard,
                &config,
                raw_archive_config,
                &health,
                &mut checkpoint,
                &archive_identity,
            )
            .await?;
            group_commit_deadline = None;
        }
    }
}

async fn open_auxiliary_node_source(
    config: &NodeFileConfig,
    durable_cursor: Option<SourceCursor>,
    source_id: &SourceId,
    health: &CaptureRuntimeHealth,
    cancellation: &CancellationToken,
    retry_backoff: &mut RetryBackoff,
) -> Result<Option<NodeLineFileSource>, SourceRuntimeError> {
    loop {
        match NodeLineFileSource::open(config.clone(), durable_cursor.clone()) {
            Ok(source) => {
                retry_backoff.reset();
                health.recover_auxiliary_retry(source_id.as_str());
                return Ok(Some(source));
            }
            Err(SourceError::TemporaryDisconnect(_)) => {
                health.retry_auxiliary(source_id.as_str(), "source.temporary_disconnect");
                let delay = retry_backoff.next_delay();
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(None),
                    () = tokio::time::sleep(delay) => {}
                }
            }
            Err(error) => return Err(SourceRuntimeError::Source(error)),
        }
    }
}

fn record_auxiliary_tail(
    health: &CaptureRuntimeHealth,
    source_id: &SourceId,
    source: &NodeLineFileSource,
) -> Result<(), SourceRuntimeError> {
    let state = source.tail_state().map_err(SourceRuntimeError::Source)?;
    health.record_auxiliary_tail(
        source_id.as_str(),
        state.active_cursor_epoch(),
        state.unread_bytes(),
        state.partial_line(),
    );
    Ok(())
}

fn record_auxiliary_durable(
    health: &CaptureRuntimeHealth,
    source_id: &SourceId,
    source: &NodeLineFileSource,
    spool: &SourceSpool,
    received_wall_micros: i64,
    quarantine_reason: Option<&str>,
) -> Result<(), SourceRuntimeError> {
    let state = source.tail_state().map_err(SourceRuntimeError::Source)?;
    let durable_cursor = state
        .durable_cursor()
        .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
    let local_sequence = spool
        .last_local_sequence()
        .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
    health.record_auxiliary_durable(
        source_id.as_str(),
        durable_cursor.epoch(),
        state.active_cursor_epoch(),
        durable_cursor.offset(),
        local_sequence.get(),
        state.unread_bytes(),
        state.partial_line(),
        received_wall_micros,
        quarantine_reason,
    );
    Ok(())
}

fn record_auxiliary_buffered(
    health: &CaptureRuntimeHealth,
    source_id: &SourceId,
    source: &NodeLineFileSource,
    spool: &SourceSpool,
    pending: usize,
) -> Result<(), SourceRuntimeError> {
    let state = source.tail_state().map_err(SourceRuntimeError::Source)?;
    let spool_records = spool
        .last_local_sequence()
        .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
    let unarchived_records =
        u64::try_from(pending).map_err(|_| SourceRuntimeError::MissingDurabilityReceipt)?;
    health.record_auxiliary_buffered(
        source_id.as_str(),
        state.active_cursor_epoch(),
        spool_records.get(),
        unarchived_records,
        state.unread_bytes(),
        state.partial_line(),
    );
    Ok(())
}

async fn append_auxiliary_observation<P: DiskSpaceProbe>(
    spool: SourceSpool,
    observation: hl_protocol::SourceObservation,
    disk_guard: &DiskReserveGuard<P>,
    raw_archive: &dyn RawSegmentArchive,
    chain_id: &ChainId,
    source_id: &SourceId,
) -> Result<(SourceSpool, bool), SourceRuntimeError> {
    if spool
        .last_durable_cursor()
        .is_some_and(|previous| previous.epoch() != observation.cursor().epoch())
        && raw_archive
            .contains_archived_epoch(chain_id, source_id, observation.cursor().epoch())
            .await
            .map_err(SourceRuntimeError::RawArchive)?
    {
        return Err(SourceRuntimeError::Spool(SpoolError::CursorRegression));
    }
    disk_guard.ensure_write(anticipated_write_bytes(observation.payload().len())?)?;
    let durable_at = now_micros()?;
    let (spool, append) = tokio::task::spawn_blocking(move || {
        let mut owned = spool;
        let result = owned.append(&observation, durable_at);
        (owned, result)
    })
    .await
    .map_err(|_| SourceRuntimeError::BlockingTask)?;
    let append = append.map_err(SourceRuntimeError::Spool)?;
    if append.disposition() == SourceSpoolAppendDisposition::Duplicate {
        return Err(SourceRuntimeError::DuplicateObservation);
    }
    let rotated = append.closed_segment().is_some();
    Ok((spool, rotated))
}

#[allow(clippy::too_many_arguments)]
async fn flush_auxiliary_observations<P: DiskSpaceProbe>(
    mut spool: SourceSpool,
    source: &mut NodeLineFileSource,
    pending: &mut Vec<PendingAuxiliaryAcknowledgement>,
    raw_archive: &dyn RawSegmentArchive,
    disk_guard: &DiskReserveGuard<P>,
    config: &AuxiliaryNodeSourceTaskConfig,
    raw_archive_config: RawSegmentArchiveConfig,
    health: &CaptureRuntimeHealth,
    checkpoint: &mut Option<AuxiliaryArchiveCheckpoint>,
    archive_identity: &str,
) -> Result<SourceSpool, SourceRuntimeError> {
    if pending.is_empty() {
        return Ok(spool);
    }
    let closed_at = now_micros()?;
    let (returned_spool, sealed) = tokio::task::spawn_blocking(move || {
        let result = spool.seal_active(closed_at);
        (spool, result)
    })
    .await
    .map_err(|_| SourceRuntimeError::BlockingTask)?;
    spool = returned_spool;
    sealed.map_err(SourceRuntimeError::Spool)?;
    let segments = spool.closed_segments().to_vec();
    if segments.is_empty() {
        return Err(SourceRuntimeError::MissingDurabilityReceipt);
    }
    for segment in segments {
        let evidence = segment_auxiliary_evidence(
            &segment,
            &config.source_id,
            &config.source_version,
            checkpoint
                .as_ref()
                .and_then(AuxiliaryArchiveCheckpoint::quarantine_reason)
                .map(ToOwned::to_owned),
        )?
        .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
        let archive_summary = archive_closed_segment(
            raw_archive,
            disk_guard,
            &config.chain_id,
            &segment,
            raw_archive_config,
        )
        .await?;
        let checkpoint_directory = config.spool_path.clone();
        let checkpoint_segment = segment.clone();
        let checkpoint_archive_identity = archive_identity.to_owned();
        let next_checkpoint = tokio::task::spawn_blocking(move || {
            AuxiliaryArchiveCheckpoint::publish(
                &checkpoint_directory,
                &checkpoint_archive_identity,
                &checkpoint_segment,
                archive_summary.manifest_ids(),
                evidence.last_received_wall_micros,
                evidence.quarantine_reason,
            )
        })
        .await
        .map_err(|_| SourceRuntimeError::BlockingTask)?
        .map_err(SourceRuntimeError::Spool)?;
        spool
            .forget_archived_segment(&segment)
            .map_err(SourceRuntimeError::Spool)?;
        next_checkpoint
            .cleanup_archived_segment(&config.spool_path)
            .map_err(SourceRuntimeError::Spool)?;
        *checkpoint = Some(next_checkpoint);
    }
    let last = pending
        .last()
        .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
    let last_received_wall_micros = last.received_wall_micros;
    let last_quarantine_reason = checkpoint
        .as_ref()
        .and_then(AuxiliaryArchiveCheckpoint::quarantine_reason);
    for acknowledgement in pending.drain(..) {
        match acknowledgement.quarantine_reason {
            Some(_) => source
                .acknowledge_quarantine_durable(&acknowledgement.cursor)
                .map_err(SourceRuntimeError::Source)?,
            None => source
                .acknowledge_durable(&acknowledgement.cursor)
                .map_err(SourceRuntimeError::Source)?,
        }
    }
    record_auxiliary_durable(
        health,
        &config.source_id,
        source,
        &spool,
        last_received_wall_micros,
        last_quarantine_reason,
    )?;
    Ok(spool)
}

async fn run_committed_node_acquisition(
    config: NodeSourceTaskConfig,
    raw_archive: Arc<dyn RawSegmentArchive>,
    notification: mpsc::Sender<SourceNotification>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<(), SourceRuntimeError> {
    run_committed_node_acquisition_with_probe(
        config,
        raw_archive,
        notification,
        health,
        cancellation,
        |config| {
            FilesystemDiskSpaceProbe::open([config.spool_path.clone(), config.archive_path.clone()])
        },
    )
    .await
}

async fn run_committed_node_acquisition_with_probe<P, F>(
    config: NodeSourceTaskConfig,
    raw_archive: Arc<dyn RawSegmentArchive>,
    notification: mpsc::Sender<SourceNotification>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
    probe_factory: F,
) -> Result<(), SourceRuntimeError>
where
    P: DiskSpaceProbe,
    F: FnOnce(&NodeSourceTaskConfig) -> Result<P, DiskReserveError>,
{
    let spool_path = prepare_source_spool_path(&config.spool_path, &config.source_id)?;
    let spool_config = SourceSpoolConfig::try_new(
        spool_path,
        config.source_id.clone(),
        config.source_version.clone(),
        SPOOL_SCHEMA_VERSION,
        *blake3::hash(env!("CARGO_PKG_VERSION").as_bytes()).as_bytes(),
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(config.segment_target_bytes, config.rotation_interval)
            .map_err(SourceRuntimeError::Spool)?,
    )
    .map_err(SourceRuntimeError::Spool)?;
    let created_at = now_micros()?;
    let mut spool =
        tokio::task::spawn_blocking(move || SourceSpool::open(spool_config, created_at))
            .await
            .map_err(|_| SourceRuntimeError::BlockingTask)?
            .map_err(SourceRuntimeError::Spool)?;
    let disk_guard = DiskReserveGuard::try_new(probe_factory(&config)?, config.disk_reserve_bytes)?;
    let raw_archive_config = RawSegmentArchiveConfig::try_new(
        config.max_payload_bytes,
        RAW_ARCHIVE_BATCH_RECORDS,
        RAW_ARCHIVE_BATCH_BYTES,
    )
    .map_err(SourceRuntimeError::RawArchive)?;
    let initial_capacity = disk_guard.ensure_write(WRITE_HEADROOM_BYTES)?;
    health.record_disk_capacity(initial_capacity.free_basis_points());
    let (returned_spool, sealed) = tokio::task::spawn_blocking(move || {
        let mut owned = spool;
        let result = owned.seal_active(created_at);
        (owned, result)
    })
    .await
    .map_err(|_| SourceRuntimeError::BlockingTask)?;
    spool = returned_spool;
    sealed.map_err(SourceRuntimeError::Spool)?;
    for segment in spool.closed_segments().to_vec() {
        archive_closed_segment(
            raw_archive.as_ref(),
            &disk_guard,
            &config.chain_id,
            &segment,
            raw_archive_config,
        )
        .await?;
    }
    let adapter_config = NodeBlockDirectoryConfig::new(
        config.source_path,
        config.stream_name,
        config.source_id.clone(),
        config.source_version,
        config.parser_version,
        config.start_height,
        config.max_payload_bytes,
        config.poll_interval,
    )
    .map_err(SourceRuntimeError::Source)?;
    let mut source =
        NodeBlockDirectorySource::open(adapter_config, spool.last_durable_cursor().cloned())
            .map_err(SourceRuntimeError::Source)?;
    health.record_source_healthy(config.role.source_class());

    loop {
        let deadline = Instant::now()
            .checked_add(config.backpressure_timeout)
            .ok_or(SourceRuntimeError::InvalidConfig)?;
        let context = SourceRequestContext::new(cancellation.child_token(), deadline);
        let observation = match source.next_observation(&context).await {
            Ok(observation) => observation,
            Err(SourceError::Cancelled) => {
                close_spool(
                    spool,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config.chain_id,
                    raw_archive_config,
                )
                .await?;
                return Ok(());
            }
            Err(SourceError::BackpressureTimeout) => continue,
            Err(SourceError::RangeUnavailable) => {
                let gap_height = match spool.last_durable_cursor() {
                    Some(cursor) => cursor
                        .offset()
                        .checked_add(1)
                        .ok_or(SourceRuntimeError::InvalidConfig)?,
                    None => config.start_height,
                };
                close_spool(
                    spool,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config.chain_id,
                    raw_archive_config,
                )
                .await?;
                health.record_source_gap(config.role.source_class());
                let gap = SourceNotification::VisibleGap {
                    role: config.role,
                    source_id: config.source_id.clone(),
                    height: BlockHeight::new(gap_height),
                };
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(()),
                    result = notification.send(gap) => {
                        if result.is_err() {
                            return Err(SourceRuntimeError::NotificationClosed);
                        }
                    }
                }
                cancellation.cancelled().await;
                return Ok(());
            }
            Err(error) => {
                close_spool(
                    spool,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config.chain_id,
                    raw_archive_config,
                )
                .await?;
                return Err(SourceRuntimeError::Source(error));
            }
        };
        let anticipated_write = anticipated_write_bytes(observation.payload().len())?;
        let disk_capacity = disk_guard.ensure_write(anticipated_write)?;
        let durable_at = now_micros()?;
        let observation_for_spool = observation.clone();
        let (returned_spool, append) = tokio::task::spawn_blocking(move || {
            let mut owned = spool;
            let result = owned.append(&observation_for_spool, durable_at);
            (owned, result)
        })
        .await
        .map_err(|_| SourceRuntimeError::BlockingTask)?;
        spool = returned_spool;
        let (receipt, closed_segment) = append.map_err(SourceRuntimeError::Spool)?.into_parts();
        let receipt = receipt.ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
        health.record_capture(
            config.role.source_class(),
            BlockHeight::new(receipt.durable_cursor.offset()),
            disk_capacity.free_basis_points(),
        );
        if let Some(segment) = closed_segment {
            archive_closed_segment(
                raw_archive.as_ref(),
                &disk_guard,
                &config.chain_id,
                &segment,
                raw_archive_config,
            )
            .await?;
        }
        source
            .acknowledge_durable(&receipt.durable_cursor)
            .map_err(SourceRuntimeError::Source)?;
        let _ = notification.try_send(SourceNotification::Durable);
    }
}

async fn run_committed_node_drain(
    config: CommittedDrainConfig,
    progress: Arc<dyn CaptureProgressStore>,
    coordinator: Arc<CaptureCoordinator>,
    mut notifications: mpsc::Receiver<SourceNotification>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<(), SourceRuntimeError> {
    let CommittedDrainConfig {
        primary,
        independent,
        failover_store,
        failover_decision,
    } = config;
    let mut active_role = if failover_decision.is_some() {
        CommittedSourceRole::Independent
    } else {
        CommittedSourceRole::Primary
    };
    if active_role == CommittedSourceRole::Independent && independent.is_none() {
        return Err(SourceRuntimeError::InvalidConfig);
    }
    let mut primary_gap = None;
    let mut independent_gap = None;
    let mut retry_backoff = RetryBackoff::new(match active_role {
        CommittedSourceRole::Primary => &primary.source_id,
        CommittedSourceRole::Independent => {
            &independent
                .as_ref()
                .ok_or(SourceRuntimeError::InvalidConfig)?
                .source_id
        }
    });
    loop {
        if cancellation.is_cancelled() {
            return Ok(());
        }
        let active_config = match active_role {
            CommittedSourceRole::Primary => &primary,
            CommittedSourceRole::Independent => independent
                .as_ref()
                .ok_or(SourceRuntimeError::InvalidConfig)?,
        };
        match drain_backlog_once(
            active_config,
            progress.as_ref(),
            coordinator.as_ref(),
            health.as_ref(),
        )
        .await
        {
            Ok(()) => {
                retry_backoff.reset();
                if active_role == CommittedSourceRole::Primary {
                    if let Some(gap_height) = primary_gap {
                        match attempt_failover(
                            &primary,
                            independent.as_ref(),
                            failover_store.as_ref(),
                            progress.as_ref(),
                            gap_height,
                        )
                        .await
                        {
                            Ok(Some(decision)) => {
                                active_role = CommittedSourceRole::Independent;
                                let source = independent
                                    .as_ref()
                                    .ok_or(SourceRuntimeError::InvalidConfig)?;
                                retry_backoff = RetryBackoff::new(&source.source_id);
                                health.activate_independent(&decision);
                                continue;
                            }
                            Ok(None) => {
                                health
                                    .set_latched("capture_failover.independent_height_unavailable");
                            }
                            Err(DrainSessionError::Retryable(reason_code)) => {
                                health.set_retryable(reason_code);
                            }
                            Err(DrainSessionError::Latched(reason_code)) => {
                                health.set_latched(reason_code);
                            }
                            Err(DrainSessionError::Fatal(error)) => return Err(error),
                        }
                    } else {
                        health.set_ready();
                    }
                } else if independent_gap.is_some() {
                    health.set_latched("capture_failover.independent_range_unavailable");
                } else {
                    health.set_ready();
                }
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(()),
                    notification = notifications.recv() => {
                        match notification {
                            Some(SourceNotification::Durable) => {}
                            Some(SourceNotification::VisibleGap {
                                role,
                                source_id,
                                height,
                            }) => {
                                tracing::warn!(
                                    source_role = ?role,
                                    source_id = %source_id.as_str(),
                                    gap_height = height.get(),
                                    "committed source exposed a visible height gap"
                                );
                                let expected_source = match role {
                                    CommittedSourceRole::Primary => &primary.source_id,
                                    CommittedSourceRole::Independent => &independent
                                        .as_ref()
                                        .ok_or(SourceRuntimeError::InvalidConfig)?
                                        .source_id,
                                };
                                if source_id != *expected_source {
                                    return Err(SourceRuntimeError::InvalidConfig);
                                }
                                match role {
                                    CommittedSourceRole::Primary => primary_gap = Some(height),
                                    CommittedSourceRole::Independent => {
                                        independent_gap = Some(height);
                                    }
                                }
                            }
                            None => return Err(SourceRuntimeError::NotificationClosed),
                        }
                    }
                    () = tokio::time::sleep(BACKLOG_POLL_DELAY) => {}
                }
            }
            Err(DrainSessionError::Retryable(reason_code)) => {
                health.set_retryable(reason_code);
                let retry_delay = retry_backoff.next_delay();
                tokio::select! {
                    () = cancellation.cancelled() => return Ok(()),
                    () = tokio::time::sleep(retry_delay) => {}
                }
            }
            Err(DrainSessionError::Latched(reason_code)) => {
                health.set_latched(reason_code);
                cancellation.cancelled().await;
                return Ok(());
            }
            Err(DrainSessionError::Fatal(error)) => {
                health.set_latched(error.reason_code());
                return Err(error);
            }
        }
    }
}

async fn attempt_failover(
    primary: &NodeSourceTaskConfig,
    independent: Option<&NodeSourceTaskConfig>,
    failover_store: &crate::FailoverStore,
    progress: &dyn CaptureProgressStore,
    failover_height: BlockHeight,
) -> Result<Option<crate::FailoverDecision>, DrainSessionError> {
    let independent = independent.ok_or(DrainSessionError::Latched(
        "capture_failover.independent_source_missing",
    ))?;
    let next_expected = match progress.next_expected_height(&primary.chain_id).await {
        Ok(height) => height,
        Err(ProgressError::Storage(_)) => {
            return Err(DrainSessionError::Retryable("capture_progress.storage"));
        }
        Err(error) => {
            return Err(DrainSessionError::Fatal(SourceRuntimeError::Progress(
                error,
            )));
        }
    };
    if next_expected < failover_height {
        return Ok(None);
    }
    if next_expected > failover_height {
        return Err(DrainSessionError::Fatal(SourceRuntimeError::InvalidConfig));
    }
    let spool_path = independent.spool_path.clone();
    let source_id = independent.source_id.clone();
    let source_version = independent.source_version.clone();
    let max_payload_bytes = independent.max_payload_bytes;
    let expected_offset = failover_height.get();
    let probe = tokio::task::spawn_blocking(move || {
        let mut backlog = SpoolBacklog::open(
            spool_path,
            source_id,
            source_version,
            expected_offset,
            max_payload_bytes,
        )?;
        backlog.next_observation()
    })
    .await
    .map_err(|_| DrainSessionError::Fatal(SourceRuntimeError::BlockingTask))?;
    match probe {
        Ok(BacklogRead::Observation(observation))
            if observation.cursor().offset() == failover_height.get() => {}
        Ok(BacklogRead::Observation(_)) => {
            return Err(DrainSessionError::Fatal(SourceRuntimeError::InvalidConfig));
        }
        Ok(BacklogRead::CaughtUp { .. }) => return Ok(None),
        Err(BacklogError::Spool(SpoolError::Io { source, .. }))
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            return Ok(None);
        }
        Err(error @ BacklogError::Gap { .. }) => {
            tracing::error!(
                reason_code = error.reason_code(),
                failover_height = failover_height.get(),
                "independent committed spool cannot fill the primary gap"
            );
            return Err(DrainSessionError::Latched(
                "capture_failover.independent_range_unavailable",
            ));
        }
        Err(error) => return Err(classify_backlog_error(error)),
    }
    let decision = crate::FailoverDecision::try_new(
        primary.chain_id.clone(),
        primary.source_id.clone(),
        independent.source_id.clone(),
        failover_height,
        crate::FailoverReason::PrimaryRangeUnavailable,
    )
    .map_err(|error| DrainSessionError::Fatal(SourceRuntimeError::Failover(error)))?;
    let store_path = failover_store.path().to_path_buf();
    let recorded = decision.clone();
    tokio::task::spawn_blocking(move || crate::FailoverStore::new(store_path)?.record(&recorded))
        .await
        .map_err(|_| DrainSessionError::Fatal(SourceRuntimeError::BlockingTask))?
        .map_err(|error| DrainSessionError::Fatal(SourceRuntimeError::Failover(error)))?;
    Ok(Some(decision))
}

async fn drain_backlog_once(
    config: &NodeSourceTaskConfig,
    progress: &dyn CaptureProgressStore,
    coordinator: &CaptureCoordinator,
    health: &CaptureRuntimeHealth,
) -> Result<(), DrainSessionError> {
    match progress
        .initialize_chain(&config.chain_id, config.first_height)
        .await
    {
        Ok(_) => {}
        Err(ProgressError::Storage(_)) => {
            return Err(DrainSessionError::Retryable("capture_progress.storage"));
        }
        Err(error) => {
            return Err(DrainSessionError::Fatal(SourceRuntimeError::Progress(
                error,
            )));
        }
    }
    match coordinator
        .recover_startup(&config.chain_id, config.max_pending_blocks)
        .await
    {
        Ok(_) => {}
        Err(CoordinatorError::Publication | CoordinatorError::Progress) => {
            return Err(DrainSessionError::Retryable(
                "capture_runtime.downstream_unavailable",
            ));
        }
        Err(error) => {
            return Err(DrainSessionError::Fatal(SourceRuntimeError::Coordinator(
                error,
            )));
        }
    }
    let first_height = match progress.next_expected_height(&config.chain_id).await {
        Ok(height) => height,
        Err(ProgressError::Storage(_)) => {
            return Err(DrainSessionError::Retryable("capture_progress.storage"));
        }
        Err(error) => {
            return Err(DrainSessionError::Fatal(SourceRuntimeError::Progress(
                error,
            )));
        }
    };
    health.record_next_expected(first_height);
    let pipeline_config = CommittedNodePipelineConfig::try_new(
        config.chain_id.clone(),
        config.source_id.clone(),
        config.source_version.clone(),
        config.admission,
        first_height,
        config.max_pending_blocks,
        config.retained_committed_blocks,
    )
    .map_err(|error| DrainSessionError::Fatal(SourceRuntimeError::Pipeline(error)))?;
    let spool_path = config.spool_path.clone();
    let source_id = config.source_id.clone();
    let source_version = config.source_version.clone();
    let max_payload_bytes = config.max_payload_bytes;
    let backlog = tokio::task::spawn_blocking(move || {
        SpoolBacklog::open(
            spool_path,
            source_id,
            source_version,
            first_height.get(),
            max_payload_bytes,
        )
    })
    .await
    .map_err(|_| DrainSessionError::Fatal(SourceRuntimeError::BlockingTask))?
    .map_err(classify_backlog_error)?;
    let mut backlog = backlog;
    let mut pipeline = CommittedNodePipeline::new(pipeline_config, coordinator);

    loop {
        let (returned_backlog, read) = tokio::task::spawn_blocking(move || {
            let mut owned = backlog;
            let result = owned.next_observation();
            (owned, result)
        })
        .await
        .map_err(|_| DrainSessionError::Fatal(SourceRuntimeError::BlockingTask))?;
        backlog = returned_backlog;
        let observation = match read.map_err(classify_backlog_error)? {
            BacklogRead::CaughtUp { .. } => return Ok(()),
            BacklogRead::Observation(observation) => observation,
        };
        let offset = observation.cursor().offset();
        match pipeline.process_spooled(&observation).await {
            Ok(PipelineOutcome::Committed { .. } | PipelineOutcome::Duplicate { .. }) => {
                backlog.acknowledge(offset).map_err(|error| {
                    DrainSessionError::Fatal(SourceRuntimeError::Backlog(error))
                })?;
                let next_expected = offset
                    .checked_add(1)
                    .ok_or(DrainSessionError::Fatal(SourceRuntimeError::InvalidConfig))?;
                health.record_next_expected(BlockHeight::new(next_expected));
            }
            Ok(PipelineOutcome::Gap { .. } | PipelineOutcome::AwaitingEvidence) => {
                return Err(DrainSessionError::Latched(
                    "capture_pipeline.awaiting_evidence",
                ));
            }
            Err(PipelineError::Commit(
                "capture_coordinator.publication" | "capture_coordinator.progress",
            )) => {
                return Err(DrainSessionError::Retryable(
                    "capture_runtime.downstream_unavailable",
                ));
            }
            Err(
                error @ (PipelineError::Mapping(_)
                | PipelineError::SourceParse
                | PipelineError::Quarantined { .. }
                | PipelineError::HistoricalVerificationUnavailable
                | PipelineError::OperatorResolutionRequired),
            ) => return Err(DrainSessionError::Latched(error.reason_code())),
            Err(error) => {
                return Err(DrainSessionError::Fatal(SourceRuntimeError::Pipeline(
                    error,
                )));
            }
        }
    }
}

fn classify_backlog_error(error: BacklogError) -> DrainSessionError {
    match &error {
        BacklogError::Spool(SpoolError::IncompleteTail { .. }) => {
            DrainSessionError::Retryable("capture_spool.incomplete_tail")
        }
        BacklogError::Spool(SpoolError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            DrainSessionError::Retryable("capture_spool.io")
        }
        BacklogError::Gap { .. } => DrainSessionError::Latched(error.reason_code()),
        _ => DrainSessionError::Fatal(SourceRuntimeError::Backlog(error)),
    }
}

#[derive(Debug)]
enum DrainSessionError {
    Retryable(&'static str),
    Latched(&'static str),
    Fatal(SourceRuntimeError),
}

async fn close_spool<P: DiskSpaceProbe>(
    spool: SourceSpool,
    raw_archive: &dyn RawSegmentArchive,
    disk_guard: &DiskReserveGuard<P>,
    chain_id: &ChainId,
    raw_archive_config: RawSegmentArchiveConfig,
) -> Result<(), SourceRuntimeError> {
    let closed_at = now_micros()?;
    let closed = tokio::task::spawn_blocking(move || spool.shutdown(closed_at))
        .await
        .map_err(|_| SourceRuntimeError::BlockingTask)?
        .map_err(SourceRuntimeError::Spool)?;
    if let Some(segment) = closed {
        archive_closed_segment(
            raw_archive,
            disk_guard,
            chain_id,
            &segment,
            raw_archive_config,
        )
        .await?;
    }
    Ok(())
}

async fn archive_closed_segment<P: DiskSpaceProbe>(
    raw_archive: &dyn RawSegmentArchive,
    disk_guard: &DiskReserveGuard<P>,
    chain_id: &ChainId,
    segment: &CloseReceipt,
    raw_archive_config: RawSegmentArchiveConfig,
) -> Result<crate::RawSegmentArchiveSummary, SourceRuntimeError> {
    let anticipated = segment
        .manifest()
        .file_size_bytes()
        .checked_add(WRITE_HEADROOM_BYTES)
        .ok_or(SourceRuntimeError::Disk(DiskReserveError::SizeOverflow))?;
    disk_guard.ensure_write(anticipated)?;
    let summary = raw_archive
        .archive_segment(chain_id, segment, raw_archive_config)
        .await
        .map_err(SourceRuntimeError::RawArchive)?;
    Ok(summary)
}

fn anticipated_write_bytes(payload_bytes: usize) -> Result<u64, SourceRuntimeError> {
    u64::try_from(payload_bytes)
        .map_err(|_| SourceRuntimeError::Disk(DiskReserveError::SizeOverflow))?
        .checked_add(WRITE_HEADROOM_BYTES)
        .ok_or(SourceRuntimeError::Disk(DiskReserveError::SizeOverflow))
}

fn now_micros() -> Result<i64, SourceRuntimeError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SourceRuntimeError::Clock)?;
    i64::try_from(elapsed.as_micros()).map_err(|_| SourceRuntimeError::Clock)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SourceRuntimeError {
    #[error("committed node source runtime configuration is invalid")]
    InvalidConfig,
    #[error("committed-source failover state failed: {0}")]
    Failover(#[source] crate::FailoverError),
    #[error("committed node source progress failed: {0}")]
    Progress(#[source] ProgressError),
    #[error("committed node capture coordinator failed: {0}")]
    Coordinator(#[source] CoordinatorError),
    #[error("committed node durable backlog failed: {0}")]
    Backlog(#[source] BacklogError),
    #[error("committed node source spool failed: {0}")]
    Spool(#[source] SpoolError),
    #[error("committed node source adapter failed: {0}")]
    Source(#[source] SourceError),
    #[error("committed node canonical pipeline failed: {0}")]
    Pipeline(#[from] PipelineError),
    #[error("committed node raw archive failed: {0}")]
    RawArchive(#[source] RawSegmentArchiveError),
    #[error("committed node disk reserve failed: {0}")]
    Disk(#[from] DiskReserveError),
    #[error("committed node append produced no durability receipt")]
    MissingDurabilityReceipt,
    #[error("auxiliary quarantine evidence cannot be represented durably")]
    InvalidQuarantineEvidence,
    #[error("committed node backlog notification channel closed")]
    NotificationClosed,
    #[error("committed node blocking task failed")]
    BlockingTask,
    #[error("configured source exited before capture shutdown")]
    UnexpectedSourceExit,
    #[error("auxiliary adapter emitted an already-spooled observation")]
    DuplicateObservation,
    #[error("committed node runtime clock failed")]
    Clock,
}

impl SourceRuntimeError {
    const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "capture_source.invalid_config",
            Self::Failover(error) => error.reason_code(),
            Self::Progress(error) => error.reason_code(),
            Self::Coordinator(error) => error.reason_code(),
            Self::Backlog(error) => error.reason_code(),
            Self::Spool(error) => error.reason_code(),
            Self::Source(error) => error.reason_code(),
            Self::Pipeline(error) => error.reason_code(),
            Self::RawArchive(error) => error.reason_code(),
            Self::Disk(error) => error.reason_code(),
            Self::MissingDurabilityReceipt => "capture_source.missing_durability_receipt",
            Self::InvalidQuarantineEvidence => "capture_source.invalid_quarantine_evidence",
            Self::NotificationClosed => "capture_source.notification_closed",
            Self::BlockingTask => "capture_source.blocking_task",
            Self::UnexpectedSourceExit => "capture_source.unexpected_exit",
            Self::DuplicateObservation => "capture_source.duplicate_observation",
            Self::Clock => "capture_source.clock",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use async_trait::async_trait;
    use bytes::Bytes;
    use canonical_archive::{ArchiveConfig, LocalParquetArchive};
    use domain_types::{BlockHeight, ChainId, KnownTime, SourceId};
    use hl_protocol::{
        ObservationClass, ReceiveTimestamps, SourceAdmission, SourceCursor, SourceObservation,
        SourceTrust,
    };
    use storage_ports::{
        CaptureProgressStore, LocalRecordSequence, LocalRecordSequenceRange, RawObservationArchive,
    };
    use tempfile::TempDir;
    use tokio::sync::{Notify, mpsc};
    use tokio::task::JoinSet;
    use tokio_util::sync::CancellationToken;

    use crate::adapters::NodeFileConfig;
    use crate::app::CaptureRuntimeHealth;
    use crate::progress::InMemoryProgressStore;
    use crate::spool::{
        DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolRotationPolicy, inspect_spool,
    };
    use crate::{
        AuxiliaryQualificationState, AuxiliarySourceHealth, BlockingRawSegmentArchive,
        CaptureConfig, DiskReserveError, DiskReserveGuard, DiskSpaceProbe, FailoverStore,
        RawSegmentArchive, RestartReconstruction,
    };

    use super::{
        AuxiliaryNodeSourceTaskConfig, CommittedSourceRole, DrainSessionError,
        NodeSourceTaskConfig, RetryBackoff, SourceNotification, SourceRuntimeError,
        append_auxiliary_observation, attempt_failover, auxiliary_commit_policy,
        auxiliary_node_task, group_commit_due, open_auxiliary_node_source,
        prepare_source_spool_path, run_auxiliary_node_acquisition_with_probe,
        run_committed_node_acquisition_with_probe, supervise_auxiliary_tasks,
    };

    #[derive(Debug, Clone, Copy)]
    struct TestDiskSpaceProbe;

    #[derive(Debug)]
    struct PendingRawArchive {
        started: Arc<Notify>,
    }

    #[derive(Debug)]
    struct FailingRawArchive;

    struct GateRawArchive {
        inner: Arc<dyn RawSegmentArchive>,
        calls: AtomicUsize,
        block_on_call: usize,
        blocked: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl RawSegmentArchive for GateRawArchive {
        async fn archive_segment(
            &self,
            chain_id: &ChainId,
            segment: &crate::spool::CloseReceipt,
            config: crate::RawSegmentArchiveConfig,
        ) -> Result<crate::RawSegmentArchiveSummary, crate::RawSegmentArchiveError> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            if call == self.block_on_call {
                self.blocked.notify_one();
                self.release.notified().await;
            }
            self.inner.archive_segment(chain_id, segment, config).await
        }

        async fn verify_archived_segment(
            &self,
            verification: &crate::RawSegmentArchiveVerification,
        ) -> Result<(), crate::RawSegmentArchiveError> {
            self.inner.verify_archived_segment(verification).await
        }

        async fn contains_archived_epoch(
            &self,
            chain_id: &ChainId,
            source_id: &SourceId,
            cursor_epoch: &str,
        ) -> Result<bool, crate::RawSegmentArchiveError> {
            self.inner
                .contains_archived_epoch(chain_id, source_id, cursor_epoch)
                .await
        }
    }

    #[tokio::test]
    async fn auxiliary_supervisor_latches_unexpected_exit_to_the_exact_source() {
        let source_id = SourceId::new("node-fills-exit").unwrap();
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[source_id.as_str().to_owned()]);
        let mut tasks = JoinSet::new();
        let task_source_id = source_id.clone();
        let handle = tasks.spawn(async move { (task_source_id, Ok(())) });
        let task_sources = HashMap::from([(handle.id(), source_id.clone())]);

        let error = supervise_auxiliary_tasks(
            tasks,
            task_sources,
            Arc::clone(&health),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.reason_code(), "capture_source.unexpected_exit");
        let status = health.auxiliary_source_status(source_id.as_str()).unwrap();
        assert_eq!(status.health(), AuxiliarySourceHealth::Latched);
        assert_eq!(
            status.last_error_reason(),
            Some("capture_source.unexpected_exit")
        );
    }

    #[tokio::test]
    async fn auxiliary_supervisor_attributes_panics_to_the_exact_source() {
        let source_id = SourceId::new("node-fills-panic").unwrap();
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[source_id.as_str().to_owned()]);
        let mut tasks: JoinSet<(SourceId, Result<(), SourceRuntimeError>)> = JoinSet::new();
        let task_source_id = source_id.clone();
        let handle = tasks.spawn(async move {
            let _ = task_source_id;
            panic!("synthetic source panic");
        });
        let task_sources = HashMap::from([(handle.id(), source_id.clone())]);

        let error = supervise_auxiliary_tasks(
            tasks,
            task_sources,
            Arc::clone(&health),
            CancellationToken::new(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.reason_code(), "capture_source.blocking_task");
        let status = health.auxiliary_source_status(source_id.as_str()).unwrap();
        assert_eq!(status.health(), AuxiliarySourceHealth::Latched);
        assert_eq!(
            status.last_error_reason(),
            Some("capture_source.blocking_task")
        );
    }

    #[tokio::test]
    async fn auxiliary_supervisor_latches_an_error_racing_with_peer_cancellation() {
        let first_source = SourceId::new("node-fills-first-error").unwrap();
        let draining_source = SourceId::new("node-fills-drain-error").unwrap();
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[
            first_source.as_str().to_owned(),
            draining_source.as_str().to_owned(),
        ]);
        let cancellation = CancellationToken::new();
        let mut tasks: JoinSet<(SourceId, Result<(), SourceRuntimeError>)> = JoinSet::new();
        let first_task_source = first_source.clone();
        let first_handle = tasks.spawn(async move {
            (
                first_task_source,
                Err(SourceRuntimeError::DuplicateObservation),
            )
        });
        let draining_task_source = draining_source.clone();
        let draining_cancellation = cancellation.child_token();
        let draining_handle = tasks.spawn(async move {
            draining_cancellation.cancelled().await;
            (draining_task_source, Err(SourceRuntimeError::Clock))
        });
        let task_sources = HashMap::from([
            (first_handle.id(), first_source.clone()),
            (draining_handle.id(), draining_source.clone()),
        ]);

        let error =
            supervise_auxiliary_tasks(tasks, task_sources, Arc::clone(&health), cancellation)
                .await
                .unwrap_err();

        assert_eq!(error.reason_code(), "capture_source.duplicate_observation");
        let first_status = health
            .auxiliary_source_status(first_source.as_str())
            .unwrap();
        assert_eq!(first_status.health(), AuxiliarySourceHealth::Latched);
        assert_eq!(
            first_status.last_error_reason(),
            Some("capture_source.duplicate_observation")
        );
        let draining_status = health
            .auxiliary_source_status(draining_source.as_str())
            .unwrap();
        assert_eq!(draining_status.health(), AuxiliarySourceHealth::Latched);
        assert_eq!(
            draining_status.last_error_reason(),
            Some("capture_source.clock")
        );
    }

    #[test]
    fn source_spool_path_rejects_a_symlinked_source_directory_escape() {
        let root = TempDir::new().unwrap();
        let spool_root = root.path().join("spool");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&spool_root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let source_id = SourceId::new("node-fills").unwrap();
        let source_path = spool_root.join(source_id.as_str());
        std::os::unix::fs::symlink(&outside, &source_path).unwrap();

        let error = prepare_source_spool_path(&source_path, &source_id)
            .expect_err("source symlink must not escape the configured spool root");

        assert_eq!(error.reason_code(), "capture_source.invalid_config");
        assert!(outside.read_dir().unwrap().next().is_none());
    }

    #[async_trait]
    impl RawSegmentArchive for PendingRawArchive {
        async fn archive_segment(
            &self,
            _chain_id: &ChainId,
            _segment: &crate::spool::CloseReceipt,
            _config: crate::RawSegmentArchiveConfig,
        ) -> Result<crate::RawSegmentArchiveSummary, crate::RawSegmentArchiveError> {
            self.started.notify_one();
            std::future::pending().await
        }

        async fn verify_archived_segment(
            &self,
            _verification: &crate::RawSegmentArchiveVerification,
        ) -> Result<(), crate::RawSegmentArchiveError> {
            Err(crate::RawSegmentArchiveError::VerificationMismatch)
        }

        async fn contains_archived_epoch(
            &self,
            _chain_id: &ChainId,
            _source_id: &SourceId,
            _cursor_epoch: &str,
        ) -> Result<bool, crate::RawSegmentArchiveError> {
            Ok(false)
        }
    }

    #[async_trait]
    impl RawSegmentArchive for FailingRawArchive {
        async fn archive_segment(
            &self,
            _chain_id: &ChainId,
            _segment: &crate::spool::CloseReceipt,
            _config: crate::RawSegmentArchiveConfig,
        ) -> Result<crate::RawSegmentArchiveSummary, crate::RawSegmentArchiveError> {
            Err(crate::RawSegmentArchiveError::VerificationMismatch)
        }

        async fn verify_archived_segment(
            &self,
            _verification: &crate::RawSegmentArchiveVerification,
        ) -> Result<(), crate::RawSegmentArchiveError> {
            Err(crate::RawSegmentArchiveError::VerificationMismatch)
        }

        async fn contains_archived_epoch(
            &self,
            _chain_id: &ChainId,
            _source_id: &SourceId,
            _cursor_epoch: &str,
        ) -> Result<bool, crate::RawSegmentArchiveError> {
            Ok(false)
        }
    }

    impl DiskSpaceProbe for TestDiskSpaceProbe {
        fn minimum_available_bytes(&self) -> Result<u64, DiskReserveError> {
            Ok(u64::MAX)
        }

        fn minimum_free_basis_points(&self) -> Result<u16, DiskReserveError> {
            Ok(10_000)
        }
    }

    #[test]
    fn retry_backoff_is_bounded_deterministic_and_source_staggered() {
        let source_a = SourceId::new("primary-node").unwrap();
        let source_b = SourceId::new("independent-node").unwrap();
        let mut first = RetryBackoff::new(&source_a);
        let mut repeated = RetryBackoff::new(&source_a);
        let mut independent = RetryBackoff::new(&source_b);

        let first_delays: Vec<_> = (0..32).map(|_| first.next_delay()).collect();
        let repeated_delays: Vec<_> = (0..32).map(|_| repeated.next_delay()).collect();
        let independent_delays: Vec<_> = (0..32).map(|_| independent.next_delay()).collect();

        assert_eq!(first_delays, repeated_delays);
        assert_ne!(first_delays, independent_delays);
        assert!(
            first_delays
                .iter()
                .all(|delay| *delay >= Duration::from_millis(200)
                    && *delay <= Duration::from_secs(30))
        );
        assert!(
            first_delays
                .iter()
                .skip(8)
                .all(|delay| *delay >= Duration::from_secs(15))
        );
    }

    #[test]
    fn a_success_resets_the_retry_sequence() {
        let source = SourceId::new("primary-node").unwrap();
        let mut backoff = RetryBackoff::new(&source);
        let first = backoff.next_delay();
        let _ = backoff.next_delay();
        let _ = backoff.next_delay();

        backoff.reset();

        assert_eq!(backoff.next_delay(), first);
    }

    #[tokio::test]
    async fn auxiliary_duplicate_is_latched_instead_of_being_acknowledged_again() {
        let root = TempDir::new().unwrap();
        let source_id = SourceId::new("node-fills-duplicate").unwrap();
        let mut spool = SourceSpool::open(
            SourceSpoolConfig::try_new_with_cursor_policy(
                root.path().join("spool/node-fills-duplicate"),
                source_id.clone(),
                "hyperliquid-node-v1",
                "spool-v1",
                [0x44; 32],
                DurabilityPolicy::FsyncEveryRecord,
                SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(60)).unwrap(),
                storage_ports::CursorPolicy::MonotonicByteOffset,
            )
            .unwrap(),
            1_000,
        )
        .unwrap();
        let observation = SourceObservation::new(
            source_id,
            "hyperliquid-node-v1",
            ObservationClass::AuxiliaryLedger,
            SourceCursor::new("node-file-epoch", 47).unwrap(),
            ReceiveTimestamps::new(1_000, 1_000).unwrap(),
            "parser-v1",
            Bytes::from_static(b"duplicate"),
            Vec::new(),
            1_024,
        )
        .unwrap();
        spool.append(&observation, 1_001).unwrap();
        let disk_guard = DiskReserveGuard::try_new(TestDiskSpaceProbe, 1).unwrap();

        let raw_archive = PendingRawArchive {
            started: Arc::new(Notify::new()),
        };
        let error = append_auxiliary_observation(
            spool,
            observation,
            &disk_guard,
            &raw_archive,
            &ChainId::new("mainnet").unwrap(),
            &SourceId::new("node-fills-duplicate").unwrap(),
        )
        .await
        .unwrap_err();

        assert_eq!(error.reason_code(), "capture_source.duplicate_observation");
    }

    fn write_block(root: &Path, height: u64, payload: &[u8]) {
        let directory = root.join("1721000000").join("20260729");
        fs::create_dir_all(&directory).unwrap();
        fs::write(directory.join(height.to_string()), payload).unwrap();
    }

    fn acquisition_config(
        root: &TempDir,
        role: CommittedSourceRole,
        source_id: &str,
        source_path: &Path,
    ) -> NodeSourceTaskConfig {
        let trust = match role {
            CommittedSourceRole::Primary => SourceTrust::LocallyVerifiedCommitted,
            CommittedSourceRole::Independent => SourceTrust::IndependentCommitted,
        };
        NodeSourceTaskConfig {
            role,
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new(source_id).unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            admission: SourceAdmission::new(trust, ObservationClass::CommittedBlock).unwrap(),
            parser_version: "parser-v1".to_owned(),
            source_path: source_path.to_path_buf(),
            stream_name: format!("{source_id}-replica-cmds"),
            first_height: BlockHeight::new(100),
            start_height: 100,
            poll_interval: Duration::from_millis(5),
            queue_capacity: 32,
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool").join(source_id),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(100),
            max_pending_blocks: 32,
            retained_committed_blocks: 32,
            disk_reserve_bytes: 1,
        }
    }

    #[tokio::test]
    async fn primary_visible_gap_parks_only_primary_while_independent_spooling_continues() {
        let root = TempDir::new().unwrap();
        let primary_source = root.path().join("primary-source");
        let independent_source = root.path().join("independent-source");
        write_block(&primary_source, 100, b"primary-100");
        write_block(&primary_source, 102, b"primary-102");
        write_block(&independent_source, 100, b"independent-100");
        write_block(&independent_source, 101, b"independent-101");
        write_block(&independent_source, 102, b"independent-102");

        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "dual-acquisition-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive;
        let raw_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let health = Arc::new(CaptureRuntimeHealth::new());
        let cancellation = CancellationToken::new();
        let (notifications, mut received) = mpsc::channel(32);
        let primary_config = acquisition_config(
            &root,
            CommittedSourceRole::Primary,
            "primary-node",
            &primary_source,
        );
        let independent_config = acquisition_config(
            &root,
            CommittedSourceRole::Independent,
            "independent-node",
            &independent_source,
        );
        let primary_spool = primary_config.spool_path.clone();
        let independent_spool = independent_config.spool_path.clone();

        let primary = tokio::spawn(run_committed_node_acquisition_with_probe(
            primary_config,
            Arc::clone(&raw_archive),
            notifications.clone(),
            Arc::clone(&health),
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        let independent = tokio::spawn(run_committed_node_acquisition_with_probe(
            independent_config,
            raw_archive,
            notifications,
            health,
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));

        let gap_result = tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if let Some(notification @ SourceNotification::VisibleGap { .. }) =
                    received.recv().await
                {
                    return notification;
                }
            }
        })
        .await;
        let gap = match gap_result {
            Ok(gap) => gap,
            Err(error) => {
                let primary_finished = primary.is_finished();
                let independent_finished = independent.is_finished();
                let primary_records =
                    inspect_spool(&primary_spool).map(|inspection| inspection.records());
                let independent_records =
                    inspect_spool(&independent_spool).map(|inspection| inspection.records());
                cancellation.cancel();
                let primary_result = primary.await;
                let independent_result = independent.await;
                panic!(
                    "primary gap signal: {error}; primary_finished={primary_finished}; \
                     independent_finished={independent_finished}; primary_records={primary_records:?}; \
                     independent_records={independent_records:?}; primary_result={primary_result:?}; \
                     independent_result={independent_result:?}"
                );
            }
        };
        assert_eq!(
            gap,
            SourceNotification::VisibleGap {
                role: CommittedSourceRole::Primary,
                source_id: SourceId::new("primary-node").unwrap(),
                height: BlockHeight::new(101),
            }
        );

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                let primary_records = inspect_spool(&primary_spool)
                    .map(|inspection| inspection.records())
                    .unwrap_or(0);
                let independent_records = inspect_spool(&independent_spool)
                    .map(|inspection| inspection.records())
                    .unwrap_or(0);
                if primary_records == 1 && independent_records == 3 {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("both source spools reach expected durable cursors");
        assert!(!primary.is_finished(), "primary acquisition must park");
        assert!(
            !independent.is_finished(),
            "independent acquisition must keep polling"
        );

        cancellation.cancel();
        primary.await.unwrap().unwrap();
        independent.await.unwrap().unwrap();
        assert_eq!(inspect_spool(primary_spool).unwrap().records(), 1);
        assert_eq!(inspect_spool(independent_spool).unwrap().records(), 3);
    }

    fn write_spool_observation(config: &NodeSourceTaskConfig, height: u64) {
        let mut spool = SourceSpool::open(
            SourceSpoolConfig::try_new(
                config.spool_path.clone(),
                config.source_id.clone(),
                config.source_version.clone(),
                "spool-v1",
                [0x42; 32],
                DurabilityPolicy::FsyncEveryRecord,
                SpoolRotationPolicy::try_new(1024 * 1024, Duration::from_secs(60)).unwrap(),
            )
            .unwrap(),
            1_000,
        )
        .unwrap();
        let observation = SourceObservation::new(
            config.source_id.clone(),
            config.source_version.clone(),
            ObservationClass::CommittedBlock,
            SourceCursor::new("independent-epoch", height).unwrap(),
            ReceiveTimestamps::new(1_000, height).unwrap(),
            "parser-v1",
            Bytes::from_static(b"independent-evidence"),
            Vec::new(),
            config.max_payload_bytes,
        )
        .unwrap();
        spool.append(&observation, 1_001).unwrap();
        spool.shutdown(1_002).unwrap();
    }

    #[tokio::test]
    async fn failover_is_recorded_only_after_exact_independent_evidence_is_durable() {
        let root = TempDir::new().unwrap();
        let source_root = root.path().join("unused-source");
        fs::create_dir(&source_root).unwrap();
        let primary = acquisition_config(
            &root,
            CommittedSourceRole::Primary,
            "primary-node",
            &source_root,
        );
        let independent = acquisition_config(
            &root,
            CommittedSourceRole::Independent,
            "independent-node",
            &source_root,
        );
        let progress = InMemoryProgressStore::new(16).unwrap();
        progress
            .initialize_chain(&primary.chain_id, BlockHeight::new(100))
            .await
            .unwrap();
        let state_path = root
            .path()
            .canonicalize()
            .unwrap()
            .join("state/failover.json");
        let store = FailoverStore::new(state_path).unwrap();

        assert!(
            attempt_failover(
                &primary,
                Some(&independent),
                &store,
                &progress,
                BlockHeight::new(100),
            )
            .await
            .unwrap()
            .is_none()
        );
        assert!(store.load().unwrap().is_none());

        write_spool_observation(&independent, 100);
        let decision = attempt_failover(
            &primary,
            Some(&independent),
            &store,
            &progress,
            BlockHeight::new(100),
        )
        .await
        .unwrap()
        .expect("exact independent evidence activates failover");
        assert_eq!(decision.failover_height(), BlockHeight::new(100));
        assert_eq!(store.load().unwrap(), Some(decision));
    }

    #[tokio::test]
    async fn independent_spool_gap_does_not_create_a_failover_decision() {
        let root = TempDir::new().unwrap();
        let source_root = root.path().join("unused-source");
        fs::create_dir(&source_root).unwrap();
        let primary = acquisition_config(
            &root,
            CommittedSourceRole::Primary,
            "primary-node",
            &source_root,
        );
        let independent = acquisition_config(
            &root,
            CommittedSourceRole::Independent,
            "independent-node",
            &source_root,
        );
        write_spool_observation(&independent, 101);
        let progress = InMemoryProgressStore::new(16).unwrap();
        progress
            .initialize_chain(&primary.chain_id, BlockHeight::new(100))
            .await
            .unwrap();
        let store = FailoverStore::new(
            root.path()
                .canonicalize()
                .unwrap()
                .join("state/failover.json"),
        )
        .unwrap();

        assert!(matches!(
            attempt_failover(
                &primary,
                Some(&independent),
                &store,
                &progress,
                BlockHeight::new(100),
            )
            .await,
            Err(DrainSessionError::Latched(
                "capture_failover.independent_range_unavailable"
            ))
        ));
        assert!(store.load().unwrap().is_none());
    }

    #[tokio::test]
    async fn auxiliary_node_line_is_spooled_archived_resumed_and_cleanly_cancelled() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-fills");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() == Some(&b'\n') {
            fill.pop();
        }
        let mut source_bytes = fill.clone();
        source_bytes.push(b'\n');
        for _ in 0..2 {
            source_bytes.extend_from_slice(&fill);
            source_bytes.push(b'\n');
        }
        fs::write(&source_path, &source_bytes).unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-acquisition-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let raw_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-fills").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path,
            stream_name: "node-fills".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/node-fills"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(100),
            archive_commit_max_records: 32,
            archive_commit_max_delay: Duration::from_millis(100),
            disk_reserve_bytes: 1,
        };
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);

        let first_cancellation = CancellationToken::new();
        let first = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            Arc::clone(&raw_archive),
            Arc::clone(&health),
            first_cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        if wait_for_raw_observations(archive.as_ref(), 3)
            .await
            .is_err()
        {
            if first.is_finished() {
                panic!("auxiliary acquisition exited early: {:?}", first.await);
            }
            panic!("auxiliary acquisition did not archive the first observation");
        }
        first_cancellation.cancel();
        first.await.unwrap().unwrap();

        let first_status = health.auxiliary_source_status("node-fills").unwrap();
        assert_eq!(first_status.health(), AuxiliarySourceHealth::Healthy);
        assert_eq!(first_status.local_sequence(), Some(3));

        let restarted_health = Arc::new(CaptureRuntimeHealth::new());
        restarted_health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let second_cancellation = CancellationToken::new();
        let second = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            Arc::clone(&raw_archive),
            Arc::clone(&restarted_health),
            second_cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = restarted_health
                    .auxiliary_source_status("node-fills")
                    .unwrap();
                if status.health() == AuxiliarySourceHealth::Healthy
                    && status.local_sequence() == Some(3)
                    && status.tail_cursor_epoch().is_some()
                    && status.restart_reconstruction() == RestartReconstruction::Complete
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("idle restart restores verified durable status");
        let mut four_lines = source_bytes;
        four_lines.extend_from_slice(&fill);
        four_lines.push(b'\n');
        fs::write(&config.source_path, four_lines).unwrap();
        if wait_for_raw_observations(archive.as_ref(), 4)
            .await
            .is_err()
        {
            if second.is_finished() {
                panic!(
                    "restarted auxiliary acquisition exited early: {:?}",
                    second.await
                );
            }
            panic!("restarted auxiliary acquisition did not archive the second observation");
        }
        second_cancellation.cancel();
        second.await.unwrap().unwrap();

        let replayed = archive
            .read_observations_by_sequence(
                &config.chain_id,
                &config.source_id,
                LocalRecordSequenceRange::try_new(
                    LocalRecordSequence::try_new(1).unwrap(),
                    LocalRecordSequence::try_new(4).unwrap(),
                )
                .unwrap(),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(replayed.len(), 4);
        assert!(replayed[0].observation().cursor().offset() > 1);
        assert!(
            replayed[1].observation().cursor().offset()
                > replayed[0].observation().cursor().offset()
        );
        assert_eq!(inspect_spool(config.spool_path).unwrap().records(), 0);
        assert_eq!(archive.inspect().unwrap().objects().len(), 2);
        let source_status = restarted_health
            .auxiliary_source_status("node-fills")
            .unwrap();
        assert_eq!(source_status.health(), AuxiliarySourceHealth::Healthy);
        assert_eq!(
            source_status.qualification(),
            AuxiliaryQualificationState::Unqualified
        );
        assert_eq!(source_status.local_sequence(), Some(4));
        assert_eq!(source_status.spool_records(), 4);
        assert_eq!(source_status.unarchived_records(), 0);
        assert_eq!(source_status.unread_bytes(), Some(0));
        assert!(!source_status.partial_line());
        assert_eq!(
            source_status.restart_reconstruction(),
            RestartReconstruction::Complete
        );
        assert!(source_status.last_error_reason().is_none());
    }

    #[tokio::test]
    async fn checkpoint_rejects_an_archive_recreated_at_the_same_path() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-fills");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() != Some(&b'\n') {
            fill.push(b'\n');
        }
        fs::write(&source_path, fill).unwrap();
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-fills-archive-lineage").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path,
            stream_name: "node-fills-archive-lineage".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/node-fills-archive-lineage"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(20),
            archive_commit_max_records: 1,
            archive_commit_max_delay: Duration::from_millis(20),
            disk_reserve_bytes: 1,
        };
        let archive = Arc::new(
            LocalParquetArchive::open(
                &config.archive_path,
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-archive-lineage-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let raw_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            Arc::clone(&raw_archive),
            health,
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        wait_for_raw_observations(archive.as_ref(), 1)
            .await
            .expect("initial archive evidence is published");
        cancellation.cancel();
        task.await.unwrap().unwrap();
        drop(raw_archive);
        drop(archive);

        fs::remove_dir_all(&config.archive_path).unwrap();
        let replacement = Arc::new(
            LocalParquetArchive::open(
                &config.archive_path,
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-archive-lineage-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let replacement_port: Arc<dyn RawObservationArchive> = replacement;
        let error = run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            Arc::new(BlockingRawSegmentArchive::new(replacement_port)),
            Arc::new(CaptureRuntimeHealth::new()),
            CancellationToken::new(),
            |_| Ok(TestDiskSpaceProbe),
        )
        .await
        .unwrap_err();

        assert_eq!(error.reason_code(), "archive.io");
        assert_eq!(inspect_spool(&config.spool_path).unwrap().records(), 0);
        assert!(
            config
                .spool_path
                .join("auxiliary-archive-checkpoint-v1.json")
                .is_file()
        );
    }

    #[tokio::test]
    async fn rotation_exposes_atomic_epochs_and_rejects_a_pruned_epoch_recurrence() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-fills");
        let retained_a = root.path().join("node-fills-epoch-a");
        let retained_b = root.path().join("node-fills-epoch-b");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() == Some(&b'\n') {
            fill.pop();
        }
        let mut epoch_a = Vec::new();
        for _ in 0..3 {
            epoch_a.extend_from_slice(&fill);
            epoch_a.push(b'\n');
        }
        fs::write(&source_path, &epoch_a).unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-rotation-runtime-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let blocking_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let archive_blocked = Arc::new(Notify::new());
        let archive_release = Arc::new(Notify::new());
        let gated_archive: Arc<dyn RawSegmentArchive> = Arc::new(GateRawArchive {
            inner: Arc::clone(&blocking_archive),
            calls: AtomicUsize::new(0),
            block_on_call: 2,
            blocked: Arc::clone(&archive_blocked),
            release: Arc::clone(&archive_release),
        });
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-fills-rotation").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path: source_path.clone(),
            stream_name: "node-fills-rotation".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/node-fills-rotation"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(100),
            archive_commit_max_records: 3,
            archive_commit_max_delay: Duration::from_secs(2),
            disk_reserve_bytes: 1,
        };
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            gated_archive,
            Arc::clone(&health),
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        wait_for_raw_observations(archive.as_ref(), 3)
            .await
            .expect("epoch A reaches verified archive");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = health
                    .auxiliary_source_status(config.source_id.as_str())
                    .unwrap();
                if status.local_sequence() == Some(3) && status.cursor_epoch().is_some() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("epoch A durable status is published");
        let durable_a = health
            .auxiliary_source_status(config.source_id.as_str())
            .unwrap()
            .cursor_epoch()
            .unwrap()
            .to_owned();

        let mut append_a = OpenOptions::new().append(true).open(&source_path).unwrap();
        append_a.write_all(&fill).unwrap();
        append_a.write_all(b"\n").unwrap();
        append_a.flush().unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = health
                    .auxiliary_source_status(config.source_id.as_str())
                    .unwrap();
                if status.spool_records() == 4 && status.unarchived_records() == 1 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("one epoch A record remains pending");
        drop(append_a);
        fs::rename(&source_path, &retained_a).unwrap();
        let mut epoch_b = fill.clone();
        epoch_b.push(b'\n');
        fs::write(&source_path, epoch_b).unwrap();
        tokio::time::timeout(Duration::from_secs(2), archive_blocked.notified())
            .await
            .expect("rotation flush reaches the controlled archive boundary");

        let pending_rotation = health
            .auxiliary_source_status(config.source_id.as_str())
            .unwrap();
        assert_eq!(pending_rotation.cursor_epoch(), Some(durable_a.as_str()));
        assert_ne!(
            pending_rotation.tail_cursor_epoch(),
            pending_rotation.cursor_epoch()
        );
        assert_eq!(pending_rotation.local_sequence(), Some(3));
        assert_eq!(pending_rotation.spool_records(), 5);
        assert_eq!(pending_rotation.unarchived_records(), 2);
        assert!(
            pending_rotation.unarchived_records()
                <= u64::try_from(config.archive_commit_max_records).unwrap()
        );

        archive_release.notify_one();
        wait_for_raw_observations(archive.as_ref(), 5)
            .await
            .expect("both epochs archive in sequence");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = health
                    .auxiliary_source_status(config.source_id.as_str())
                    .unwrap();
                if status.local_sequence() == Some(5) && status.unarchived_records() == 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("epoch B durable status is published");
        let durable_b = health
            .auxiliary_source_status(config.source_id.as_str())
            .unwrap();
        assert_ne!(durable_b.cursor_epoch(), Some(durable_a.as_str()));
        assert_eq!(durable_b.cursor_epoch(), durable_b.tail_cursor_epoch());
        assert_eq!(durable_b.local_sequence(), Some(5));
        assert_eq!(durable_b.unarchived_records(), 0);
        assert_eq!(inspect_spool(&config.spool_path).unwrap().records(), 0);
        assert!(
            blocking_archive
                .contains_archived_epoch(&config.chain_id, &config.source_id, &durable_a)
                .await
                .unwrap()
        );

        fs::rename(&source_path, &retained_b).unwrap();
        fs::rename(&retained_a, &source_path).unwrap();
        let error = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("live recurrence fails without waiting for shutdown")
            .unwrap()
            .unwrap_err();
        assert_eq!(error.reason_code(), "spool.cursor_regression");
        assert_eq!(archive.inspect().unwrap().raw_observations(), 5);
    }

    #[tokio::test]
    async fn partial_suffix_is_observable_and_archives_only_after_completion() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-fills-partial");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() == Some(&b'\n') {
            fill.pop();
        }
        let split = fill.len() / 2;
        let mut initial = fill.clone();
        initial.push(b'\n');
        initial.extend_from_slice(&fill[..split]);
        fs::write(&source_path, initial).unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-partial-runtime-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-fills-partial").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path: source_path.clone(),
            stream_name: "node-fills-partial".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/node-fills-partial"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(20),
            archive_commit_max_records: 1,
            archive_commit_max_delay: Duration::from_millis(20),
            disk_reserve_bytes: 1,
        };
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            Arc::new(BlockingRawSegmentArchive::new(raw_port)),
            Arc::clone(&health),
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        wait_for_raw_observations(archive.as_ref(), 1)
            .await
            .expect("complete prefix archives");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = health
                    .auxiliary_source_status(config.source_id.as_str())
                    .unwrap();
                if status.local_sequence() == Some(1) && status.partial_line() {
                    assert_eq!(status.unread_bytes(), Some(u64::try_from(split).unwrap()));
                    assert_eq!(status.unarchived_records(), 0);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("partial suffix is visible without being archived");

        let mut writer = OpenOptions::new().append(true).open(&source_path).unwrap();
        writer.write_all(&fill[split..]).unwrap();
        writer.write_all(b"\n").unwrap();
        writer.flush().unwrap();
        wait_for_raw_observations(archive.as_ref(), 2)
            .await
            .expect("completed suffix archives exactly once");
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = health
                    .auxiliary_source_status(config.source_id.as_str())
                    .unwrap();
                if status.local_sequence() == Some(2) && !status.partial_line() {
                    assert_eq!(status.unread_bytes(), Some(0));
                    assert_eq!(status.unarchived_records(), 0);
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("partial status clears after exact completion");
        cancellation.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn oversized_line_latches_exact_source_without_cursor_or_archive_advance() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-fills-oversized");
        let mut oversized = vec![b'x'; 1_025];
        oversized.push(b'\n');
        fs::write(&source_path, oversized).unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-oversized-runtime-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let raw_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-fills-oversized").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path,
            stream_name: "node-fills-oversized".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1_024,
            spool_path: root.path().join("spool/node-fills-oversized"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(20),
            archive_commit_max_records: 1,
            archive_commit_max_delay: Duration::from_millis(20),
            disk_reserve_bytes: 1,
        };
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);

        let cancellation = CancellationToken::new();
        let mut tasks: JoinSet<(SourceId, Result<(), SourceRuntimeError>)> = JoinSet::new();
        let task_config = config.clone();
        let task_source_id = config.source_id.clone();
        let task_health = Arc::clone(&health);
        let task_cancellation = cancellation.child_token();
        let handle = tasks.spawn(async move {
            let result = run_auxiliary_node_acquisition_with_probe(
                task_config,
                raw_archive,
                task_health,
                task_cancellation,
                |_| Ok(TestDiskSpaceProbe),
            )
            .await;
            (task_source_id, result)
        });
        let task_sources = HashMap::from([(handle.id(), config.source_id.clone())]);
        let error =
            supervise_auxiliary_tasks(tasks, task_sources, Arc::clone(&health), cancellation)
                .await
                .unwrap_err();

        assert_eq!(error.reason_code(), "source.malformed_payload");
        let status = health
            .auxiliary_source_status(config.source_id.as_str())
            .unwrap();
        assert_eq!(status.health(), AuxiliarySourceHealth::Latched);
        assert_eq!(status.last_error_reason(), Some("source.malformed_payload"));
        assert_eq!(status.local_sequence(), None);
        assert_eq!(status.spool_records(), 0);
        assert_eq!(status.unarchived_records(), 0);
        assert_eq!(archive.inspect().unwrap().raw_observations(), 0);
        assert_eq!(inspect_spool(&config.spool_path).unwrap().records(), 0);
    }

    #[tokio::test]
    async fn auxiliary_group_commit_delay_is_anchored_to_the_first_pending_record() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("continuous-node-fills");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() == Some(&b'\n') {
            fill.pop();
        }
        let mut first_line = fill.clone();
        first_line.push(b'\n');
        fs::write(&source_path, first_line).unwrap();
        let archive_path = root.path().join("archive");
        fs::create_dir_all(&archive_path).unwrap();
        let archive_started = Arc::new(Notify::new());
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("continuous-node-fills").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path: source_path.clone(),
            stream_name: "continuous-node-fills".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(2),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/continuous-node-fills"),
            archive_path,
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(250),
            archive_commit_max_records: 128,
            archive_commit_max_delay: Duration::from_millis(60),
            disk_reserve_bytes: 1,
        };
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let acquisition = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config,
            Arc::new(PendingRawArchive {
                started: Arc::clone(&archive_started),
            }),
            Arc::clone(&health),
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        let producer_fill = fill;
        let producer = tokio::spawn(async move {
            let mut file = OpenOptions::new().append(true).open(source_path).unwrap();
            for _ in 0..100 {
                tokio::time::sleep(Duration::from_millis(15)).await;
                file.write_all(&producer_fill).unwrap();
                file.write_all(b"\n").unwrap();
                file.flush().unwrap();
            }
        });

        tokio::time::timeout(Duration::from_secs(1), archive_started.notified())
            .await
            .expect("first pending group reaches archive before continuous input stops");
        assert!(
            !producer.is_finished(),
            "continuous producer must still be active at the absolute flush boundary"
        );
        let status = health
            .auxiliary_source_status("continuous-node-fills")
            .unwrap();
        assert_eq!(status.local_sequence(), None);
        assert!(status.spool_records() > 0);
        assert_eq!(status.unarchived_records(), status.spool_records());

        producer.abort();
        let _ = producer.await;
        cancellation.cancel();
        acquisition.abort();
        assert!(acquisition.await.unwrap_err().is_cancelled());
    }

    #[tokio::test]
    async fn auxiliary_schema_drift_is_spooled_and_archived_before_cursor_acknowledgement() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-misc-events");
        let mut transfer = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/source/node-v1/transfer.json"),
        )
        .unwrap();
        if transfer.last() == Some(&b'\n') {
            transfer.pop();
        }
        let mut unknown = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/source/node-v1/unknown-variant.json"),
        )
        .unwrap();
        if unknown.last() == Some(&b'\n') {
            unknown.pop();
        }
        let mut source_bytes = transfer.clone();
        source_bytes.push(b'\n');
        source_bytes.extend_from_slice(&unknown);
        source_bytes.push(b'\n');
        source_bytes.extend_from_slice(&transfer);
        source_bytes.push(b'\n');
        fs::write(&source_path, source_bytes).unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-quarantine-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let raw_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-misc-events").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path,
            stream_name: "node-misc-events".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::MiscEvents,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/node-misc-events"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(100),
            archive_commit_max_records: 32,
            archive_commit_max_delay: Duration::from_millis(100),
            disk_reserve_bytes: 1,
        };
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let task = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            raw_archive,
            Arc::clone(&health),
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        if wait_for_raw_observations(archive.as_ref(), 3)
            .await
            .is_err()
        {
            if task.is_finished() {
                panic!("quarantine acquisition exited early: {:?}", task.await);
            }
            panic!("normal/quarantine/normal records were not archived");
        }
        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("quarantine acquisition stops after cancellation")
            .unwrap()
            .unwrap();

        let replayed = archive
            .read_observations_by_sequence(
                &config.chain_id,
                &config.source_id,
                LocalRecordSequenceRange::try_new(
                    LocalRecordSequence::try_new(1).unwrap(),
                    LocalRecordSequence::try_new(3).unwrap(),
                )
                .unwrap(),
            )
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let quarantined = replayed[1].observation();
        assert_eq!(quarantined.payload().as_ref(), unknown);
        assert_eq!(
            quarantined.parser_schema_version(),
            "quarantine-v1:source.schema_drift"
        );
        assert_eq!(inspect_spool(&config.spool_path).unwrap().records(), 0);
        let source_status = health.auxiliary_source_status("node-misc-events").unwrap();
        assert_eq!(source_status.health(), AuxiliarySourceHealth::Quarantined);
        assert_eq!(source_status.local_sequence(), Some(3));
        assert_eq!(
            source_status.quarantine_reason(),
            Some("source.schema_drift")
        );
        assert_eq!(source_status.last_error_reason(), None);

        let mut restart_config = config;
        restart_config.archive_commit_max_records = 1;
        let restarted_health = Arc::new(CaptureRuntimeHealth::new());
        restarted_health
            .configure_auxiliary_sources(&[restart_config.source_id.as_str().to_owned()]);
        let restart_cancellation = CancellationToken::new();
        let restarted = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            restart_config.clone(),
            Arc::new(BlockingRawSegmentArchive::new(archive.clone())),
            Arc::clone(&restarted_health),
            restart_cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if restarted.is_finished() {
                    panic!("restarted quarantine acquisition exited before status recovery");
                }
                let status = restarted_health
                    .auxiliary_source_status("node-misc-events")
                    .unwrap();
                if status.health() == AuxiliarySourceHealth::Quarantined
                    && status.local_sequence() == Some(3)
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("restart reconstructs the durable quarantine latch");
        restart_cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), restarted)
            .await
            .expect("restarted quarantine acquisition stops after cancellation")
            .unwrap()
            .unwrap();

        fs::remove_file(&restart_config.source_path).unwrap();
        let missing_health = Arc::new(CaptureRuntimeHealth::new());
        missing_health.configure_auxiliary_sources(&[restart_config.source_id.as_str().to_owned()]);
        let missing_cancellation = CancellationToken::new();
        let missing = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            restart_config,
            Arc::new(BlockingRawSegmentArchive::new(archive)),
            Arc::clone(&missing_health),
            missing_cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = missing_health
                    .auxiliary_source_status("node-misc-events")
                    .unwrap();
                if status.health() == AuxiliarySourceHealth::Quarantined
                    && status.quarantine_reason() == Some("source.schema_drift")
                    && status.last_error_reason() == Some("source.temporary_disconnect")
                {
                    break;
                }
                assert!(!missing.is_finished(), "missing quarantined source exited");
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("quarantine cause remains distinct from a restart outage");
        missing_cancellation.cancel();
        missing.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn auxiliary_source_retries_a_missing_startup_file_until_it_is_available() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("late-node-fills");
        let source_id = SourceId::new("late-node-fills").unwrap();
        let adapter = NodeFileConfig::new(
            source_path.clone(),
            "late-node-fills",
            hl_protocol::node::v1::NodeStreamKind::Fills,
            source_id.clone(),
            "hyperliquid-node-v1",
            "parser-v1",
            1024 * 1024,
            Duration::from_millis(5),
        )
        .unwrap();
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let task_health = Arc::clone(&health);
        let task_cancellation = cancellation.child_token();
        let task_source_id = source_id.clone();
        let task = tokio::spawn(async move {
            let mut retry = RetryBackoff::new(&task_source_id);
            open_auxiliary_node_source(
                &adapter,
                None,
                &task_source_id,
                task_health.as_ref(),
                &task_cancellation,
                &mut retry,
            )
            .await
        });

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let status = health.auxiliary_source_status(source_id.as_str()).unwrap();
                if status.last_error_reason() == Some("source.temporary_disconnect") {
                    return;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("missing source is observable as retrying");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() != Some(&b'\n') {
            fill.push(b'\n');
        }
        fs::write(source_path, fill).unwrap();

        let opened = tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("source opens after appearing")
            .unwrap()
            .unwrap();
        assert!(opened.is_some());
    }

    #[test]
    fn configured_node_line_creates_an_owned_runtime_task_and_starting_status() {
        let root = TempDir::new().unwrap();
        let mut source = include_str!("../../../config/capture.example.toml").to_owned();
        source.push_str(&format!(
            r#"

[[sources]]
id = "node-fills"
source_version = "hyperliquid-node-v1"
trust = "locally-verified-committed"
class = "auxiliary-ledger"
queue_capacity = 32
max_payload_bytes = 1048576
adapter = {{ kind = "node-line", path = "{}", stream_name = "node-fills", stream = "fills", poll_interval_millis = 5 }}
"#,
            root.path().join("node-fills").display()
        ));
        let config = CaptureConfig::from_toml(&source).unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-task-wiring-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive;
        let raw_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let health = Arc::new(CaptureRuntimeHealth::new());

        let task = auxiliary_node_task(
            &config,
            raw_archive,
            Arc::clone(&health),
            CancellationToken::new(),
        )
        .unwrap();

        assert!(
            task.is_some(),
            "configured NodeLine source must not be inert"
        );
        let source = health.auxiliary_source_status("node-fills").unwrap();
        assert_eq!(source.health(), AuxiliarySourceHealth::Starting);
        assert_eq!(
            source.qualification(),
            AuxiliaryQualificationState::Unqualified
        );
    }

    #[tokio::test]
    async fn crash_after_spool_seal_before_archive_recovers_without_loss_or_duplicate() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-fills");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() != Some(&b'\n') {
            fill.push(b'\n');
        }
        fs::write(&source_path, fill).unwrap();
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-fills-crash").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path,
            stream_name: "node-fills-crash".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/node-fills-crash"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(20),
            archive_commit_max_records: 32,
            archive_commit_max_delay: Duration::from_millis(20),
            disk_reserve_bytes: 1,
        };
        fs::create_dir_all(&config.archive_path).unwrap();
        let started = Arc::new(Notify::new());
        let stalled_archive: Arc<dyn RawSegmentArchive> = Arc::new(PendingRawArchive {
            started: Arc::clone(&started),
        });
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let crashed = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            stalled_archive,
            Arc::clone(&health),
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        tokio::time::timeout(Duration::from_secs(2), started.notified())
            .await
            .expect("archive boundary reached");
        let pre_archive_status = health
            .auxiliary_source_status(config.source_id.as_str())
            .unwrap();
        assert_eq!(pre_archive_status.local_sequence(), None);
        assert_eq!(pre_archive_status.spool_records(), 1);
        assert_eq!(pre_archive_status.unarchived_records(), 1);
        crashed.abort();
        assert!(crashed.await.unwrap_err().is_cancelled());
        assert_eq!(inspect_spool(&config.spool_path).unwrap().records(), 1);

        let archive = Arc::new(
            LocalParquetArchive::open(
                &config.archive_path,
                ArchiveConfig::deterministic_fixture(
                    "auxiliary-crash-recovery-test",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let recovered_archive: Arc<dyn RawSegmentArchive> =
            Arc::new(BlockingRawSegmentArchive::new(raw_port));
        let restart_cancellation = CancellationToken::new();
        let restarted_health = Arc::new(CaptureRuntimeHealth::new());
        restarted_health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let restarted = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            recovered_archive,
            Arc::clone(&restarted_health),
            restart_cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        if wait_for_raw_observations(archive.as_ref(), 1)
            .await
            .is_err()
        {
            if restarted.is_finished() {
                panic!("crash recovery exited early: {:?}", restarted.await);
            }
            panic!("crash recovery did not archive the sealed spool segment");
        }
        restart_cancellation.cancel();
        restarted.await.unwrap().unwrap();
        assert_eq!(archive.inspect().unwrap().raw_observations(), 1);
        assert_eq!(inspect_spool(&config.spool_path).unwrap().records(), 0);
        let recovered_status = restarted_health
            .auxiliary_source_status("node-fills-crash")
            .unwrap();
        assert_eq!(recovered_status.health(), AuxiliarySourceHealth::Healthy);
        assert_eq!(recovered_status.local_sequence(), Some(1));
        assert_eq!(recovered_status.unarchived_records(), 0);

        let temporarily_missing_path = config.source_path.with_extension("temporarily-missing");
        fs::rename(&config.source_path, &temporarily_missing_path).unwrap();
        let missing_health = Arc::new(CaptureRuntimeHealth::new());
        missing_health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let missing_cancellation = CancellationToken::new();
        let missing = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            Arc::new(BlockingRawSegmentArchive::new(archive.clone())),
            Arc::clone(&missing_health),
            missing_cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = missing_health
                    .auxiliary_source_status("node-fills-crash")
                    .unwrap();
                if status.local_sequence() == Some(1)
                    && status.last_error_reason() == Some("source.temporary_disconnect")
                {
                    assert_eq!(status.health(), AuxiliarySourceHealth::Starting);
                    assert!(status.cursor_epoch().is_some());
                    assert!(status.durable_offset().is_some());
                    assert_eq!(status.tail_cursor_epoch(), status.cursor_epoch());
                    assert_eq!(
                        status.restart_reconstruction(),
                        RestartReconstruction::Incomplete
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("missing restart source retains verified durable status while retrying");
        fs::rename(&temporarily_missing_path, &config.source_path).unwrap();
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                let status = missing_health
                    .auxiliary_source_status("node-fills-crash")
                    .unwrap();
                if status.health() == AuxiliarySourceHealth::Healthy
                    && status.local_sequence() == Some(1)
                    && status.last_error_reason().is_none()
                    && status.restart_reconstruction() == RestartReconstruction::Complete
                {
                    break;
                }
                assert!(!missing.is_finished(), "recovered idle source exited");
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("unchanged idle source clears its temporary outage after reopening");
        assert_eq!(archive.inspect().unwrap().raw_observations(), 1);
        missing_cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), missing)
            .await
            .expect("missing-source retry stops after cancellation")
            .unwrap()
            .unwrap();
    }

    #[test]
    fn batched_provisional_durability_is_the_auxiliary_group_commit_window() {
        let (records, delay) = auxiliary_commit_policy(
            crate::config::DurabilityPolicy::Batched {
                max_records: 128,
                max_delay_millis: 100,
            },
            32,
            Duration::from_secs(5),
        )
        .unwrap();
        assert_eq!(records, 32);
        assert_eq!(delay, Duration::from_millis(100));

        let (immediate_records, immediate_delay) = auxiliary_commit_policy(
            crate::config::DurabilityPolicy::FsyncEveryRecord,
            32,
            Duration::from_millis(20),
        )
        .unwrap();
        assert_eq!(immediate_records, 1);
        assert_eq!(immediate_delay, Duration::from_millis(20));
        assert!(!group_commit_due(
            1,
            128,
            Some(std::time::Instant::now() + Duration::from_millis(50)),
            std::time::Instant::now()
        ));
        assert!(group_commit_due(128, 128, None, std::time::Instant::now()));
    }

    #[tokio::test]
    async fn raw_archive_failure_produces_zero_acknowledgements() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("node-fills-archive-fail");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() != Some(&b'\n') {
            fill.push(b'\n');
        }
        fs::write(&source_path, fill).unwrap();
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("node-fills-archive-fail").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path,
            stream_name: "node-fills-archive-fail".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/node-fills-archive-fail"),
            archive_path: root.path().join("archive"),
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(20),
            archive_commit_max_records: 1,
            archive_commit_max_delay: Duration::from_millis(20),
            disk_reserve_bytes: 1,
        };
        fs::create_dir_all(&config.archive_path).unwrap();
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let error = run_auxiliary_node_acquisition_with_probe(
            config.clone(),
            Arc::new(FailingRawArchive),
            Arc::clone(&health),
            CancellationToken::new(),
            |_| Ok(TestDiskSpaceProbe),
        )
        .await
        .unwrap_err();

        assert_eq!(
            error.reason_code(),
            "capture_raw_archive.verification_mismatch"
        );
        let status = health
            .auxiliary_source_status(config.source_id.as_str())
            .unwrap();
        assert!(status.local_sequence().is_none());
        assert!(status.durable_offset().is_none());
        assert_eq!(status.spool_records(), 1);
        assert_eq!(status.unarchived_records(), 1);
        assert_eq!(inspect_spool(&config.spool_path).unwrap().records(), 1);
    }

    #[tokio::test]
    async fn auxiliary_group_commit_waits_for_configured_delay_despite_shorter_backpressure() {
        let root = TempDir::new().unwrap();
        let source_path = root.path().join("delayed-node-fills");
        let mut fill = fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/source/node-v1/fill.json"),
        )
        .unwrap();
        if fill.last() == Some(&b'\n') {
            fill.pop();
        }
        let mut first_line = fill.clone();
        first_line.push(b'\n');
        fs::write(&source_path, first_line).unwrap();
        let archive_path = root.path().join("archive");
        fs::create_dir_all(&archive_path).unwrap();
        let archive_started = Arc::new(Notify::new());
        let config = AuxiliaryNodeSourceTaskConfig {
            chain_id: ChainId::new("mainnet").unwrap(),
            source_id: SourceId::new("delayed-node-fills").unwrap(),
            source_version: "hyperliquid-node-v1".to_owned(),
            parser_version: "parser-v1".to_owned(),
            source_path,
            stream_name: "delayed-node-fills".to_owned(),
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            poll_interval: Duration::from_millis(5),
            max_payload_bytes: 1024 * 1024,
            spool_path: root.path().join("spool/delayed-node-fills"),
            archive_path,
            segment_target_bytes: 1024 * 1024,
            rotation_interval: Duration::from_secs(60),
            backpressure_timeout: Duration::from_millis(20),
            archive_commit_max_records: 128,
            archive_commit_max_delay: Duration::from_millis(150),
            disk_reserve_bytes: 1,
        };
        let health = Arc::new(CaptureRuntimeHealth::new());
        health.configure_auxiliary_sources(&[config.source_id.as_str().to_owned()]);
        let cancellation = CancellationToken::new();
        let acquisition = tokio::spawn(run_auxiliary_node_acquisition_with_probe(
            config,
            Arc::new(PendingRawArchive {
                started: Arc::clone(&archive_started),
            }),
            Arc::clone(&health),
            cancellation.child_token(),
            |_| Ok(TestDiskSpaceProbe),
        ));

        let early =
            tokio::time::timeout(Duration::from_millis(70), archive_started.notified()).await;
        assert!(
            early.is_err(),
            "group commit must not flush on a shorter backpressure timeout"
        );
        tokio::time::timeout(Duration::from_millis(250), archive_started.notified())
            .await
            .expect("absolute group-commit delay still flushes the pending window");
        let status = health
            .auxiliary_source_status("delayed-node-fills")
            .unwrap();
        assert!(status.local_sequence().is_none());
        assert_eq!(status.spool_records(), status.unarchived_records());
        assert!(status.spool_records() > 0);

        cancellation.cancel();
        acquisition.abort();
        let _ = acquisition.await;
    }

    async fn wait_for_raw_observations(
        archive: &LocalParquetArchive,
        expected: u64,
    ) -> Result<(), tokio::time::error::Elapsed> {
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if archive.inspect().unwrap().raw_observations() == expected {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
    }
}
