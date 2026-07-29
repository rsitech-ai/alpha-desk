use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use domain_types::{BlockHeight, ChainId, SourceId};
use hl_protocol::{BlockSource, SourceAdmission, SourceError, SourceRequestContext};
use storage_ports::CaptureProgressStore;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::adapters::{NodeBlockDirectoryConfig, NodeBlockDirectorySource};
use crate::coordinator::CaptureCoordinator;
use crate::spool::{
    CloseReceipt, DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolError, SpoolRead,
    SpoolReader, SpoolRecord, SpoolRotationPolicy,
};
use crate::{
    AppError, CaptureConfig, CommittedNodePipeline, CommittedNodePipelineConfig, DiskReserveError,
    DiskReserveGuard, FilesystemDiskSpaceProbe, OwnedTask, PipelineError, PipelineOutcome,
    RawSegmentArchive, RawSegmentArchiveConfig, RawSegmentArchiveError, SourceAdapterConfig,
};

const SPOOL_SCHEMA_VERSION: &str = "spool-v1";
const SOURCE_TASK_NAME: &str = "primary-node-source";
const WRITE_HEADROOM_BYTES: u64 = 1024 * 1024;
const RAW_ARCHIVE_BATCH_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Clone)]
struct NodeSourceTaskConfig {
    chain_id: ChainId,
    source_id: SourceId,
    source_version: String,
    admission: SourceAdmission,
    parser_version: String,
    source_path: PathBuf,
    stream_name: String,
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

pub(crate) fn primary_node_task(
    config: &CaptureConfig,
    progress: Arc<dyn CaptureProgressStore>,
    coordinator: Arc<CaptureCoordinator>,
    raw_archive: Arc<dyn RawSegmentArchive>,
    cancellation: CancellationToken,
) -> Result<OwnedTask, SourceRuntimeError> {
    let mut selected = None;
    for source in config.sources() {
        let Some(SourceAdapterConfig::NodeBlockDirectory {
            path,
            stream_name,
            start_height,
            poll_interval_millis,
        }) = source.adapter()
        else {
            continue;
        };
        let admission = source
            .admission()
            .map_err(|_| SourceRuntimeError::InvalidConfig)?;
        if !admission.can_advance_committed_watermark() || selected.is_some() {
            return Err(SourceRuntimeError::InvalidConfig);
        }
        selected = Some(NodeSourceTaskConfig {
            chain_id: config.runtime().chain_id(),
            source_id: SourceId::new(source.id().to_owned())
                .map_err(|_| SourceRuntimeError::InvalidConfig)?,
            source_version: source.source_version().to_owned(),
            admission,
            parser_version: config.parser_version().to_owned(),
            source_path: path.clone(),
            stream_name: stream_name.clone(),
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
        });
    }
    let selected = selected.ok_or(SourceRuntimeError::InvalidConfig)?;
    Ok(OwnedTask::new(SOURCE_TASK_NAME, async move {
        run_primary_node(
            selected,
            progress,
            coordinator,
            raw_archive,
            cancellation.child_token(),
        )
        .await
        .map_err(|error| AppError::TaskFailed {
            task: SOURCE_TASK_NAME,
            reason_code: error.reason_code(),
        })
    }))
}

async fn run_primary_node(
    config: NodeSourceTaskConfig,
    progress: Arc<dyn CaptureProgressStore>,
    coordinator: Arc<CaptureCoordinator>,
    raw_archive: Arc<dyn RawSegmentArchive>,
    cancellation: CancellationToken,
) -> Result<(), SourceRuntimeError> {
    let first_height = progress
        .next_expected_height(&config.chain_id)
        .await
        .map_err(|_| SourceRuntimeError::Progress)?;
    let pipeline_config = CommittedNodePipelineConfig::try_new(
        config.chain_id.clone(),
        config.source_id.clone(),
        config.source_version.clone(),
        config.admission,
        first_height,
        config.max_pending_blocks,
        config.retained_committed_blocks,
    )?;
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
    let disk_guard = DiskReserveGuard::try_new(
        FilesystemDiskSpaceProbe::open([config.spool_path.clone(), config.archive_path.clone()])?,
        config.disk_reserve_bytes,
    )?;
    let raw_archive_config = RawSegmentArchiveConfig::try_new(
        config.max_payload_bytes,
        config.queue_capacity,
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
        archive_closed_segment(
            raw_archive.as_ref(),
            &disk_guard,
            &config.chain_id,
            &segment,
            raw_archive_config,
        )
        .await?;
    }
    let mut pipeline = CommittedNodePipeline::new(pipeline_config, coordinator.as_ref());

    for path in spool.verified_segment_paths().to_vec() {
        replay_spool_segment(path, first_height, &config, &disk_guard, &mut pipeline).await?;
    }

    let adapter_config = NodeBlockDirectoryConfig::new(
        config.source_path,
        config.stream_name,
        config.source_id,
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
        disk_guard.ensure_write(anticipated_write)?;
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
        disk_guard.ensure_write(anticipated_write)?;
        source
            .acknowledge_durable(&receipt.durable_cursor)
            .map_err(SourceRuntimeError::Source)?;
        match pipeline.process_spooled(&observation).await {
            Ok(outcome) => require_advancing_outcome(outcome)?,
            Err(error) => {
                close_spool(
                    spool,
                    raw_archive.as_ref(),
                    &disk_guard,
                    &config.chain_id,
                    raw_archive_config,
                )
                .await?;
                return Err(SourceRuntimeError::Pipeline(error));
            }
        }
    }
}

async fn replay_spool_segment(
    path: PathBuf,
    first_height: BlockHeight,
    config: &NodeSourceTaskConfig,
    disk_guard: &DiskReserveGuard<FilesystemDiskSpaceProbe>,
    pipeline: &mut CommittedNodePipeline<'_, CaptureCoordinator>,
) -> Result<(), SourceRuntimeError> {
    let (sender, mut receiver) = mpsc::channel::<SpoolRecord>(1);
    let reader = tokio::task::spawn_blocking(move || -> Result<(), SpoolError> {
        let reader = SpoolReader::open(path)?;
        let mut records = reader.stream()?;
        loop {
            match records.next_record()? {
                SpoolRead::Record(record) => {
                    if sender.blocking_send(record).is_err() {
                        return Ok(());
                    }
                }
                SpoolRead::EndOfFile => return Ok(()),
                SpoolRead::IncompleteTail { record_offset } => {
                    return Err(SpoolError::IncompleteTail { record_offset });
                }
            }
        }
    });
    let processing: Result<(), SourceRuntimeError> = async {
        while let Some(record) = receiver.recv().await {
            if record.cursor().offset() < first_height.get() {
                continue;
            }
            let observation = record
                .into_observation(
                    config.source_id.clone(),
                    config.source_version.clone(),
                    config.max_payload_bytes,
                )
                .map_err(|_| SourceRuntimeError::InvalidSpoolObservation)?;
            disk_guard.ensure_write(anticipated_write_bytes(observation.payload().len())?)?;
            require_advancing_outcome(pipeline.process_spooled(&observation).await?)?;
        }
        Ok(())
    }
    .await;
    drop(receiver);
    let reader = reader
        .await
        .map_err(|_| SourceRuntimeError::BlockingTask)?
        .map_err(SourceRuntimeError::Spool);
    processing?;
    reader
}

async fn close_spool(
    spool: SourceSpool,
    raw_archive: &dyn RawSegmentArchive,
    disk_guard: &DiskReserveGuard<FilesystemDiskSpaceProbe>,
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

async fn archive_closed_segment(
    raw_archive: &dyn RawSegmentArchive,
    disk_guard: &DiskReserveGuard<FilesystemDiskSpaceProbe>,
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

fn require_advancing_outcome(outcome: PipelineOutcome) -> Result<(), SourceRuntimeError> {
    match outcome {
        PipelineOutcome::Committed { .. } | PipelineOutcome::Duplicate { .. } => Ok(()),
        PipelineOutcome::Gap { .. } => Err(SourceRuntimeError::Gap),
        PipelineOutcome::AwaitingEvidence => Err(SourceRuntimeError::AwaitingEvidence),
    }
}

fn now_micros() -> Result<i64, SourceRuntimeError> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| SourceRuntimeError::Clock)?;
    i64::try_from(elapsed.as_micros()).map_err(|_| SourceRuntimeError::Clock)
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum SourceRuntimeError {
    #[error("primary node source runtime configuration is invalid")]
    InvalidConfig,
    #[error("primary node source progress is unavailable")]
    Progress,
    #[error("primary node source spool failed: {0}")]
    Spool(#[source] SpoolError),
    #[error("primary node source adapter failed: {0}")]
    Source(#[source] SourceError),
    #[error("primary node canonical pipeline failed: {0}")]
    Pipeline(#[from] PipelineError),
    #[error("primary node raw archive failed: {0}")]
    RawArchive(#[source] RawSegmentArchiveError),
    #[error("primary node disk reserve failed: {0}")]
    Disk(#[from] DiskReserveError),
    #[error("primary node spool record is not a valid observation")]
    InvalidSpoolObservation,
    #[error("primary node committed append produced no durability receipt")]
    MissingDurabilityReceipt,
    #[error("primary node source contains an unresolved block gap")]
    Gap,
    #[error("primary node source is awaiting unsupported evidence")]
    AwaitingEvidence,
    #[error("primary node blocking task failed")]
    BlockingTask,
    #[error("primary node runtime clock failed")]
    Clock,
}

impl SourceRuntimeError {
    const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "capture_source.invalid_config",
            Self::Progress => "capture_source.progress",
            Self::Spool(error) => error.reason_code(),
            Self::Source(error) => error.reason_code(),
            Self::Pipeline(error) => error.reason_code(),
            Self::RawArchive(error) => error.reason_code(),
            Self::Disk(error) => error.reason_code(),
            Self::InvalidSpoolObservation => "capture_source.invalid_spool_observation",
            Self::MissingDurabilityReceipt => "capture_source.missing_durability_receipt",
            Self::Gap => "capture_source.gap",
            Self::AwaitingEvidence => "capture_source.awaiting_evidence",
            Self::BlockingTask => "capture_source.blocking_task",
            Self::Clock => "capture_source.clock",
        }
    }
}
