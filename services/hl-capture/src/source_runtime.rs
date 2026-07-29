use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use domain_types::{BlockHeight, ChainId, SourceId};
use hl_protocol::{BlockSource, SourceAdmission, SourceError, SourceRequestContext, SourceTrust};
use storage_ports::{CaptureProgressStore, ProgressError};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::adapters::{NodeBlockDirectoryConfig, NodeBlockDirectorySource};
use crate::app::CaptureRuntimeHealth;
use crate::coordinator::{CaptureCoordinator, CoordinatorError};
use crate::spool::{
    CloseReceipt, DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolError, SpoolRotationPolicy,
};
use crate::{
    AppError, BacklogError, BacklogRead, CaptureConfig, CommittedNodePipeline,
    CommittedNodePipelineConfig, DiskReserveError, DiskReserveGuard, DiskSpaceProbe,
    FilesystemDiskSpaceProbe, OwnedTask, PipelineError, PipelineOutcome, RawSegmentArchive,
    RawSegmentArchiveConfig, RawSegmentArchiveError, SourceAdapterConfig, SpoolBacklog,
};

const SPOOL_SCHEMA_VERSION: &str = "spool-v1";
const PRIMARY_ACQUISITION_TASK_NAME: &str = "primary-node-acquisition";
const INDEPENDENT_ACQUISITION_TASK_NAME: &str = "independent-node-acquisition";
const DRAIN_TASK_NAME: &str = "committed-source-drain";
const WRITE_HEADROOM_BYTES: u64 = 1024 * 1024;
const RAW_ARCHIVE_BATCH_BYTES: u64 = 64 * 1024 * 1024;
const BACKLOG_POLL_DELAY: Duration = Duration::from_millis(250);
const RETRY_BACKOFF_BASE_MILLIS: u64 = 250;
const RETRY_BACKOFF_CEILING_BASE_MILLIS: u64 = 25_000;
const RETRY_JITTER_MIN_BPS: u64 = 8_000;
const RETRY_JITTER_SPAN_BPS: u64 = 4_001;
const BASIS_POINTS: u64 = 10_000;

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
    let spool_config = SourceSpoolConfig::try_new(
        config.spool_path.clone(),
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
        config.queue_capacity.min(4096),
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
) -> Result<(), SourceRuntimeError> {
    let anticipated = segment
        .manifest()
        .file_size_bytes()
        .checked_add(WRITE_HEADROOM_BYTES)
        .ok_or(SourceRuntimeError::Disk(DiskReserveError::SizeOverflow))?;
    disk_guard.ensure_write(anticipated)?;
    raw_archive
        .archive_segment(chain_id, segment, raw_archive_config)
        .await
        .map_err(SourceRuntimeError::RawArchive)?;
    Ok(())
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
    #[error("committed node backlog notification channel closed")]
    NotificationClosed,
    #[error("committed node blocking task failed")]
    BlockingTask,
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
            Self::NotificationClosed => "capture_source.notification_closed",
            Self::BlockingTask => "capture_source.blocking_task",
            Self::Clock => "capture_source.clock",
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use bytes::Bytes;
    use canonical_archive::{ArchiveConfig, LocalParquetArchive};
    use domain_types::{BlockHeight, ChainId, KnownTime, SourceId};
    use hl_protocol::{
        ObservationClass, ReceiveTimestamps, SourceAdmission, SourceCursor, SourceObservation,
        SourceTrust,
    };
    use storage_ports::{CaptureProgressStore, RawObservationArchive};
    use tempfile::TempDir;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    use crate::app::CaptureRuntimeHealth;
    use crate::progress::InMemoryProgressStore;
    use crate::spool::{
        DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolRotationPolicy, inspect_spool,
    };
    use crate::{
        BlockingRawSegmentArchive, DiskReserveError, DiskSpaceProbe, FailoverStore,
        RawSegmentArchive,
    };

    use super::{
        CommittedSourceRole, DrainSessionError, NodeSourceTaskConfig, RetryBackoff,
        SourceNotification, attempt_failover, run_committed_node_acquisition_with_probe,
    };

    #[derive(Debug, Clone, Copy)]
    struct TestDiskSpaceProbe;

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
}
