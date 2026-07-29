use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use domain_types::{ChainId, SourceId};
use hl_protocol::{
    BlockSource, ParseWarning, SourceAdmission, SourceError, SourceObservation,
    SourceRequestContext,
};
use storage_ports::CaptureProgressStore;
use tokio_util::sync::CancellationToken;

use crate::adapters::{NodeBlockDirectoryConfig, NodeBlockDirectorySource};
use crate::coordinator::CaptureCoordinator;
use crate::spool::{
    DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolError, SpoolReader, SpoolRotationPolicy,
};
use crate::{
    AppError, CaptureConfig, CommittedNodePipeline, CommittedNodePipelineConfig, OwnedTask,
    PipelineError, PipelineOutcome, SourceAdapterConfig,
};

const SPOOL_SCHEMA_VERSION: &str = "spool-v1";
const SOURCE_TASK_NAME: &str = "primary-node-source";

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
    max_payload_bytes: usize,
    spool_path: PathBuf,
    segment_target_bytes: u64,
    rotation_interval: Duration,
    backpressure_timeout: Duration,
    max_pending_blocks: usize,
    retained_committed_blocks: usize,
}

pub(crate) fn primary_node_task(
    config: &CaptureConfig,
    progress: Arc<dyn CaptureProgressStore>,
    coordinator: Arc<CaptureCoordinator>,
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
            max_payload_bytes: source.max_payload_bytes(),
            spool_path: config.spool().path().join(source.id()),
            segment_target_bytes: config.spool().segment_target_bytes(),
            rotation_interval: Duration::from_secs(config.spool().rotation_interval_seconds()),
            backpressure_timeout: Duration::from_millis(
                config.runtime().backpressure_timeout_millis(),
            ),
            max_pending_blocks: config.runtime().max_pending_blocks(),
            retained_committed_blocks: config.runtime().retained_committed_blocks(),
        });
    }
    let selected = selected.ok_or(SourceRuntimeError::InvalidConfig)?;
    Ok(OwnedTask::new(SOURCE_TASK_NAME, async move {
        run_primary_node(selected, progress, coordinator, cancellation.child_token())
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
    let mut pipeline = CommittedNodePipeline::new(pipeline_config, coordinator.as_ref());

    for path in spool.verified_segment_paths().to_vec() {
        let records = tokio::task::spawn_blocking(move || SpoolReader::open(path)?.read_all())
            .await
            .map_err(|_| SourceRuntimeError::BlockingTask)?
            .map_err(SourceRuntimeError::Spool)?;
        for record in records {
            if record.cursor().offset() < first_height.get() {
                continue;
            }
            let observation = SourceObservation::new(
                config.source_id.clone(),
                config.source_version.clone(),
                record.observation_class(),
                record.cursor().clone(),
                record.received(),
                record.parser_schema_version(),
                Bytes::copy_from_slice(record.payload()),
                Vec::<ParseWarning>::new(),
                config.max_payload_bytes,
            )
            .map_err(|_| SourceRuntimeError::InvalidSpoolObservation)?;
            require_advancing_outcome(pipeline.process_spooled(&observation).await?)?;
        }
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
                close_spool(spool).await?;
                return Ok(());
            }
            Err(SourceError::BackpressureTimeout) => continue,
            Err(error) => {
                close_spool(spool).await?;
                return Err(SourceRuntimeError::Source(error));
            }
        };
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
        let receipt = append
            .map_err(SourceRuntimeError::Spool)?
            .ok_or(SourceRuntimeError::MissingDurabilityReceipt)?;
        source
            .acknowledge_durable(&receipt.durable_cursor)
            .map_err(SourceRuntimeError::Source)?;
        match pipeline.process_spooled(&observation).await {
            Ok(outcome) => require_advancing_outcome(outcome)?,
            Err(error) => {
                close_spool(spool).await?;
                return Err(SourceRuntimeError::Pipeline(error));
            }
        }
    }
}

async fn close_spool(spool: SourceSpool) -> Result<(), SourceRuntimeError> {
    let closed_at = now_micros()?;
    tokio::task::spawn_blocking(move || spool.shutdown(closed_at))
        .await
        .map_err(|_| SourceRuntimeError::BlockingTask)?
        .map_err(SourceRuntimeError::Spool)?;
    Ok(())
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
            Self::InvalidSpoolObservation => "capture_source.invalid_spool_observation",
            Self::MissingDurabilityReceipt => "capture_source.missing_durability_receipt",
            Self::Gap => "capture_source.gap",
            Self::AwaitingEvidence => "capture_source.awaiting_evidence",
            Self::BlockingTask => "capture_source.blocking_task",
            Self::Clock => "capture_source.clock",
        }
    }
}
