use canonical_events::BlockEnvelope;
use canonical_ledger::{
    CanonicalLedger, CheckpointCompatibility, EventReducer, LedgerLimits, StateImageLimits,
};
use domain_types::{BlockHeight, ChainId, CheckpointId};
use storage_ports::{ArchiveReceipt, AtomicStateStore, StateCheckpointStore};

use crate::{
    DurableApplyOutcome,
    checkpoint::load_checkpoint_ledger,
    health::FeatureHealth,
    input::CoreInputSubject,
    reconciliation::{InputDisposition, ReconciliationInbox},
    replay::{LocalReplayError, LocalReplaySession},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeMode {
    Genesis,
    Durable,
    Checkpoint(CheckpointId),
}

pub struct StateRuntime<R, S> {
    session: LocalReplaySession<R, S>,
    genesis_height: BlockHeight,
    health: FeatureHealth,
    reconciliation: ReconciliationInbox,
}

impl<R: EventReducer, S: AtomicStateStore> StateRuntime<R, S> {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        chain_id: ChainId,
        genesis_height: BlockHeight,
        reducer: R,
        limits: LedgerLimits,
        store: S,
        image_limits: StateImageLimits,
        mode: ResumeMode,
        checkpoints: Option<(&dyn StateCheckpointStore, &CheckpointCompatibility)>,
    ) -> Result<Self, LocalReplayError> {
        let session = match &mode {
            ResumeMode::Genesis => {
                if store.load_latest(image_limits)?.is_some() {
                    return Err(LocalReplayError::MidHistoryResume);
                }
                LocalReplaySession::open(
                    chain_id,
                    genesis_height,
                    reducer,
                    limits,
                    store,
                    image_limits,
                )?
            }
            ResumeMode::Durable => LocalReplaySession::open(
                chain_id,
                genesis_height,
                reducer,
                limits,
                store,
                image_limits,
            )?,
            ResumeMode::Checkpoint(checkpoint_id) => {
                let Some((checkpoint_store, compatibility)) = checkpoints else {
                    return Err(LocalReplayError::MidHistoryResume);
                };
                let ledger = load_checkpoint_ledger(
                    checkpoint_store,
                    checkpoint_id,
                    compatibility,
                    reducer,
                    limits,
                    image_limits,
                )?;
                if let Some(image) = store.load_latest(image_limits)?
                    && image.state_hash() != ledger.state_hash()
                {
                    return Err(LocalReplayError::Store(
                        storage_ports::StateStoreError::Conflict,
                    ));
                }
                LocalReplaySession::from_restored(ledger, store)
            }
        };
        if session.ledger().checkpoint().is_none() {
            let next = session
                .ledger()
                .next_height()
                .map_err(LocalReplayError::Ledger)?;
            if next != genesis_height {
                return Err(LocalReplayError::MidHistoryResume);
            }
        }
        Ok(Self {
            session,
            genesis_height,
            health: FeatureHealth::green(),
            reconciliation: ReconciliationInbox::default(),
        })
    }

    pub fn apply_committed(
        &mut self,
        block: &BlockEnvelope,
        receipt: &ArchiveReceipt,
    ) -> Result<DurableApplyOutcome, LocalReplayError> {
        align_watermarks(
            block.block_height(),
            receipt.block_height(),
            block.block_height(),
        )?;
        if receipt.canonical_block_hash() != block.canonical_block_hash() {
            return Err(LocalReplayError::WatermarkMisaligned);
        }
        let outcome = self.session.apply_next(block)?;
        if let DurableApplyOutcome::Applied { .. } = &outcome {
            let state_height = self
                .session
                .ledger()
                .checkpoint()
                .map(|checkpoint| checkpoint.block_height())
                .ok_or(LocalReplayError::WatermarkMisaligned)?;
            align_watermarks(block.block_height(), receipt.block_height(), state_height)?;
        }
        Ok(outcome)
    }

    pub fn ingest_subject(&mut self, subject: CoreInputSubject) -> InputDisposition {
        self.reconciliation.observe(subject)
    }

    #[must_use]
    pub fn health(&self) -> &FeatureHealth {
        &self.health
    }

    pub fn health_mut(&mut self) -> &mut FeatureHealth {
        &mut self.health
    }

    #[must_use]
    pub fn reconciliation(&self) -> &ReconciliationInbox {
        &self.reconciliation
    }

    #[must_use]
    pub const fn genesis_height(&self) -> BlockHeight {
        self.genesis_height
    }

    #[must_use]
    pub fn ledger(&self) -> &CanonicalLedger<R> {
        self.session.ledger()
    }

    pub fn session_mut(&mut self) -> &mut LocalReplaySession<R, S> {
        &mut self.session
    }
}

pub fn align_watermarks(
    block_height: BlockHeight,
    archive_height: BlockHeight,
    state_height: BlockHeight,
) -> Result<(), LocalReplayError> {
    if block_height == archive_height && archive_height == state_height {
        Ok(())
    } else {
        Err(LocalReplayError::WatermarkMisaligned)
    }
}

pub fn admit_resume_height(
    genesis_height: BlockHeight,
    requested_height: BlockHeight,
    has_checkpoint: bool,
) -> Result<(), LocalReplayError> {
    if requested_height == genesis_height || has_checkpoint {
        Ok(())
    } else {
        Err(LocalReplayError::MidHistoryResume)
    }
}
