use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use canonical_events::BlockEnvelope;
use domain_types::{BlockHeight, BlockRange, ChainId, KnownTime};
use storage_ports::{
    ArchiveError, ArchiveReceipt, ArchivedBlockPlan, CanonicalArchive, CaptureCursor,
    CaptureProgressStore, PlannedPublication, ProgressError, PublicationAcknowledgement,
};

use crate::bus::{
    CanonicalPublisher, CommittedPublicationBatch, PublicationError, PublicationMessage,
};

#[async_trait]
pub trait CaptureArchive: Send + Sync {
    async fn append_block(&self, block: &BlockEnvelope) -> Result<ArchiveReceipt, ArchiveError>;

    async fn load_block(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<BlockEnvelope, ArchiveError>;
}

#[derive(Clone)]
pub struct BlockingCanonicalArchive {
    archive: Arc<dyn CanonicalArchive>,
}

impl std::fmt::Debug for BlockingCanonicalArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BlockingCanonicalArchive")
            .finish_non_exhaustive()
    }
}

impl BlockingCanonicalArchive {
    #[must_use]
    pub fn new(archive: Arc<dyn CanonicalArchive>) -> Self {
        Self { archive }
    }
}

#[async_trait]
impl CaptureArchive for BlockingCanonicalArchive {
    async fn append_block(&self, block: &BlockEnvelope) -> Result<ArchiveReceipt, ArchiveError> {
        let archive = Arc::clone(&self.archive);
        let block = block.clone();
        tokio::task::spawn_blocking(move || archive.append_block(&block))
            .await
            .map_err(|_| ArchiveError::Io("joining archive append worker"))?
    }

    async fn load_block(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<BlockEnvelope, ArchiveError> {
        let archive = Arc::clone(&self.archive);
        let chain_id = chain_id.clone();
        tokio::task::spawn_blocking(move || {
            let range = BlockRange::new(block_height, block_height)
                .map_err(|_| ArchiveError::InvalidInput("single-block recovery range"))?;
            let mut blocks = archive.read_range(&chain_id, range)?;
            let block = blocks.next().ok_or(ArchiveError::RangeUnavailable)??;
            if blocks.next().is_some() {
                return Err(ArchiveError::ManifestVerification(
                    "single-block range returned extra rows",
                ));
            }
            Ok(block)
        })
        .await
        .map_err(|_| ArchiveError::Io("joining archive recovery worker"))?
    }
}

pub trait AcknowledgementClock: Send + Sync {
    fn acknowledged_at(
        &self,
        block_height: BlockHeight,
        ordinal: u32,
    ) -> Result<KnownTime, CoordinatorError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemAcknowledgementClock;

impl AcknowledgementClock for SystemAcknowledgementClock {
    fn acknowledged_at(
        &self,
        _block_height: BlockHeight,
        _ordinal: u32,
    ) -> Result<KnownTime, CoordinatorError> {
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| CoordinatorError::Clock)?
            .as_micros();
        let micros = i64::try_from(micros).map_err(|_| CoordinatorError::Clock)?;
        KnownTime::from_unix_micros(micros).map_err(|_| CoordinatorError::Clock)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorFaultPoint {
    AfterArchive,
    AfterJournal,
    AfterPublish { ordinal: u32 },
    AfterAcknowledgement { ordinal: u32 },
    AfterCursor,
}

pub trait CoordinatorFaultInjector: Send + Sync {
    fn check(&self, point: CoordinatorFaultPoint) -> Result<(), CoordinatorError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoCoordinatorFaults;

impl CoordinatorFaultInjector for NoCoordinatorFaults {
    fn check(&self, _point: CoordinatorFaultPoint) -> Result<(), CoordinatorError> {
        Ok(())
    }
}

pub struct CaptureCoordinator {
    archive: Arc<dyn CaptureArchive>,
    progress: Arc<dyn CaptureProgressStore>,
    publisher: Arc<dyn CanonicalPublisher>,
    clock: Arc<dyn AcknowledgementClock>,
    faults: Arc<dyn CoordinatorFaultInjector>,
}

impl std::fmt::Debug for CaptureCoordinator {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureCoordinator")
            .finish_non_exhaustive()
    }
}

impl CaptureCoordinator {
    #[must_use]
    pub fn new(
        archive: Arc<dyn CaptureArchive>,
        progress: Arc<dyn CaptureProgressStore>,
        publisher: Arc<dyn CanonicalPublisher>,
        clock: Arc<dyn AcknowledgementClock>,
        faults: Arc<dyn CoordinatorFaultInjector>,
    ) -> Self {
        Self {
            archive,
            progress,
            publisher,
            clock,
            faults,
        }
    }

    pub async fn process_block(
        &self,
        block: &BlockEnvelope,
    ) -> Result<CaptureCursor, CoordinatorError> {
        let receipt = self
            .archive
            .append_block(block)
            .await
            .map_err(|_| CoordinatorError::Archive)?;
        self.faults.check(CoordinatorFaultPoint::AfterArchive)?;

        let batch =
            CommittedPublicationBatch::try_new(block, &receipt).map_err(publication_error)?;
        let plan = build_plan(block, &receipt, &batch)?;
        self.progress
            .record_archived(&plan)
            .await
            .map_err(progress_error)?;
        self.faults.check(CoordinatorFaultPoint::AfterJournal)?;
        self.complete_batch(&plan, &batch).await
    }

    pub async fn recover_pending(
        &self,
        chain_id: &ChainId,
        limit: usize,
    ) -> Result<Vec<BlockHeight>, CoordinatorError> {
        let plans = self
            .progress
            .pending_blocks(chain_id, limit)
            .await
            .map_err(progress_error)?;
        let mut recovered = Vec::with_capacity(plans.len());
        for plan in plans {
            let block = self
                .archive
                .load_block(chain_id, plan.block_height())
                .await
                .map_err(|_| CoordinatorError::Archive)?;
            let receipt = receipt_from_plan(&plan)?;
            let batch =
                CommittedPublicationBatch::try_new(&block, &receipt).map_err(publication_error)?;
            verify_plan_matches_batch(&plan, &batch)?;
            self.complete_batch(&plan, &batch).await?;
            recovered.push(plan.block_height());
        }
        Ok(recovered)
    }

    pub async fn recover_startup(
        &self,
        chain_id: &ChainId,
        limit: usize,
    ) -> Result<Vec<BlockHeight>, CoordinatorError> {
        if limit == 0 {
            return Err(CoordinatorError::InvalidRecoveryLimit);
        }
        let mut recovered = Vec::with_capacity(limit);
        for _ in 0..limit {
            let expected = self
                .progress
                .next_expected_height(chain_id)
                .await
                .map_err(progress_error)?;
            let cursor = if let Some(plan) = self
                .progress
                .load_archived_block(chain_id, expected)
                .await
                .map_err(progress_error)?
            {
                let block = self
                    .archive
                    .load_block(chain_id, expected)
                    .await
                    .map_err(|_| CoordinatorError::Archive)?;
                let receipt = receipt_from_plan(&plan)?;
                let batch = CommittedPublicationBatch::try_new(&block, &receipt)
                    .map_err(publication_error)?;
                verify_plan_matches_batch(&plan, &batch)?;
                self.complete_batch(&plan, &batch).await?
            } else {
                let block = match self.archive.load_block(chain_id, expected).await {
                    Ok(block) => block,
                    Err(ArchiveError::RangeUnavailable) => break,
                    Err(_) => return Err(CoordinatorError::Archive),
                };
                self.process_block(&block).await?
            };
            if cursor.committed_block_height() != expected {
                return Err(CoordinatorError::RecoveryMismatch);
            }
            recovered.push(expected);
        }
        Ok(recovered)
    }

    async fn complete_batch(
        &self,
        plan: &ArchivedBlockPlan,
        batch: &CommittedPublicationBatch,
    ) -> Result<CaptureCursor, CoordinatorError> {
        verify_plan_matches_batch(plan, batch)?;
        let acknowledgements = self
            .progress
            .load_acknowledgements(plan.chain_id(), plan.block_height())
            .await
            .map_err(progress_error)?;
        let mut acknowledged_by_ordinal = BTreeMap::new();
        for acknowledgement in acknowledgements {
            if acknowledged_by_ordinal
                .insert(acknowledgement.ordinal(), acknowledgement)
                .is_some()
            {
                return Err(CoordinatorError::RecoveryMismatch);
            }
        }

        for (index, message) in batch.iter().enumerate() {
            let ordinal =
                u32::try_from(index).map_err(|_| CoordinatorError::PublicationPlanOverflow)?;
            if let Some(acknowledgement) = acknowledged_by_ordinal.get(&ordinal) {
                verify_acknowledgement(message, ordinal, acknowledgement)?;
                continue;
            }
            let acknowledgement = self
                .publisher
                .publish(message)
                .await
                .map_err(publication_error)?;
            self.faults
                .check(CoordinatorFaultPoint::AfterPublish { ordinal })?;
            let acknowledged_at = self.clock.acknowledged_at(plan.block_height(), ordinal)?;
            let durable_acknowledgement = PublicationAcknowledgement::try_new(
                ordinal,
                acknowledgement.message_id(),
                message.subject().as_str(),
                acknowledgement.publication_sha256(),
                acknowledgement.stream(),
                acknowledgement.stream_sequence(),
                acknowledgement.duplicate(),
                acknowledged_at,
            )
            .map_err(progress_error)?;
            self.progress
                .record_acknowledgement(
                    plan.chain_id(),
                    plan.block_height(),
                    &durable_acknowledgement,
                )
                .await
                .map_err(progress_error)?;
            self.faults
                .check(CoordinatorFaultPoint::AfterAcknowledgement { ordinal })?;
        }
        let cursor = self
            .progress
            .advance_cursor(plan.chain_id(), plan.block_height())
            .await
            .map_err(progress_error)?;
        self.faults.check(CoordinatorFaultPoint::AfterCursor)?;
        Ok(cursor)
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum CoordinatorError {
    #[error("canonical archive operation failed")]
    Archive,
    #[error("canonical publication operation failed")]
    Publication,
    #[error("capture progress operation failed")]
    Progress,
    #[error("capture acknowledgement clock failed")]
    Clock,
    #[error("capture publication plan exceeds the supported ordinal domain")]
    PublicationPlanOverflow,
    #[error("capture recovery content does not match the durable publication plan")]
    RecoveryMismatch,
    #[error("capture recovery limit must be greater than zero")]
    InvalidRecoveryLimit,
    #[error("deterministic coordinator fault injected at {0:?}")]
    InjectedFault(CoordinatorFaultPoint),
}

impl CoordinatorError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Archive => "capture_coordinator.archive",
            Self::Publication => "capture_coordinator.publication",
            Self::Progress => "capture_coordinator.progress",
            Self::Clock => "capture_coordinator.clock",
            Self::PublicationPlanOverflow => "capture_coordinator.publication_plan_overflow",
            Self::RecoveryMismatch => "capture_coordinator.recovery_mismatch",
            Self::InvalidRecoveryLimit => "capture_coordinator.invalid_recovery_limit",
            Self::InjectedFault(_) => "capture_coordinator.injected_fault",
        }
    }
}

fn build_plan(
    block: &BlockEnvelope,
    receipt: &ArchiveReceipt,
    batch: &CommittedPublicationBatch,
) -> Result<ArchivedBlockPlan, CoordinatorError> {
    let publications = batch
        .iter()
        .enumerate()
        .map(|(index, message)| {
            PlannedPublication::try_new(
                u32::try_from(index).map_err(|_| CoordinatorError::PublicationPlanOverflow)?,
                message.message_id(),
                message.subject().as_str(),
                message.publication_sha256(),
            )
            .map_err(progress_error)
        })
        .collect::<Result<Vec<_>, _>>()?;
    ArchivedBlockPlan::try_new(
        block.chain_id().clone(),
        block.block_height(),
        block.canonical_block_hash(),
        receipt.receipt_id(),
        receipt.manifest_id().clone(),
        receipt.object_sha256(),
        receipt.manifest_sha256(),
        receipt.schema_fingerprint(),
        publications,
        receipt.durable_at(),
    )
    .map_err(progress_error)
}

fn receipt_from_plan(plan: &ArchivedBlockPlan) -> Result<ArchiveReceipt, CoordinatorError> {
    ArchiveReceipt::try_new(
        plan.archive_receipt_id(),
        plan.archive_manifest_id().clone(),
        plan.block_height(),
        plan.canonical_block_hash(),
        plan.archive_object_sha256(),
        plan.archive_manifest_sha256(),
        plan.archive_schema_fingerprint(),
        plan.archived_at(),
    )
    .map_err(|_| CoordinatorError::RecoveryMismatch)
}

fn verify_plan_matches_batch(
    plan: &ArchivedBlockPlan,
    batch: &CommittedPublicationBatch,
) -> Result<(), CoordinatorError> {
    if plan.publications().len() != batch.iter().count() {
        return Err(CoordinatorError::RecoveryMismatch);
    }
    for (index, (planned, message)) in plan.publications().iter().zip(batch.iter()).enumerate() {
        let ordinal =
            u32::try_from(index).map_err(|_| CoordinatorError::PublicationPlanOverflow)?;
        if planned.ordinal() != ordinal
            || planned.message_id() != message.message_id()
            || planned.subject() != message.subject().as_str()
            || planned.publication_sha256() != message.publication_sha256()
            || message.chain_id() != plan.chain_id()
            || message.block_height() != plan.block_height()
            || message.canonical_block_hash() != plan.canonical_block_hash()
            || message.archive_receipt_id() != plan.archive_receipt_id()
            || message.archive_manifest_sha256() != plan.archive_manifest_sha256()
        {
            return Err(CoordinatorError::RecoveryMismatch);
        }
    }
    Ok(())
}

fn verify_acknowledgement(
    message: &PublicationMessage,
    ordinal: u32,
    acknowledgement: &PublicationAcknowledgement,
) -> Result<(), CoordinatorError> {
    if acknowledgement.ordinal() != ordinal
        || acknowledgement.message_id() != message.message_id()
        || acknowledgement.subject() != message.subject().as_str()
        || acknowledgement.publication_sha256() != message.publication_sha256()
        || acknowledgement.stream() != message.stream()
    {
        return Err(CoordinatorError::RecoveryMismatch);
    }
    Ok(())
}

fn publication_error(_error: PublicationError) -> CoordinatorError {
    CoordinatorError::Publication
}

fn progress_error(_error: ProgressError) -> CoordinatorError {
    CoordinatorError::Progress
}

pub use crate::adapters::info_rest::{
    InfoCaptureCoordinator, InfoCaptureOutcome, InfoFaultPoint, NoInfoFaults,
};
