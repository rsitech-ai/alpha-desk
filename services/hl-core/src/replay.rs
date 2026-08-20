use canonical_events::BlockEnvelope;
use canonical_ledger::{CanonicalLedger, EventReducer, LedgerLimits, StateImageLimits};
use domain_types::{BlockHeight, ChainId};
use storage_ports::AtomicStateStore;

use crate::{
    DurableApplyError, DurableApplyOutcome, apply_block_durably,
    source::{BlockSourceError, CanonicalBlockSource},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalReplayReport {
    pub applied: u64,
    pub already_applied: u64,
    pub last_height: Option<BlockHeight>,
    pub state_hash: [u8; 32],
}

#[derive(Debug, thiserror::Error)]
pub enum LocalReplayError {
    #[error(transparent)]
    Durable(#[from] DurableApplyError),
    #[error(transparent)]
    Source(#[from] BlockSourceError),
    #[error("local replay ledger could not be constructed: {0}")]
    Ledger(canonical_ledger::LedgerError),
    #[error("durable state store failed during replay restore: {0}")]
    Store(storage_ports::StateStoreError),
    #[error("local replay applied-block counter overflowed")]
    Overflow,
}

impl LocalReplayError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Durable(error) => error.reason_code(),
            Self::Source(BlockSourceError::Qualification) => "core.replay_qualification",
            Self::Source(BlockSourceError::Decode(_) | BlockSourceError::Io) => {
                "core.replay_source"
            }
            Self::Ledger(_) => "core.replay_ledger",
            Self::Store(_) => "core.state_store",
            Self::Overflow => "core.replay_overflow",
        }
    }
}

pub struct LocalReplaySession<R, S> {
    ledger: CanonicalLedger<R>,
    store: S,
}

impl<R: EventReducer, S: AtomicStateStore> LocalReplaySession<R, S> {
    pub fn open(
        chain_id: ChainId,
        first_height: BlockHeight,
        reducer: R,
        limits: LedgerLimits,
        store: S,
        image_limits: StateImageLimits,
    ) -> Result<Self, LocalReplayError> {
        let ledger = match store
            .load_latest(image_limits)
            .map_err(LocalReplayError::Store)?
        {
            Some(image) => CanonicalLedger::try_from_state_image(image, reducer, limits)
                .map_err(LocalReplayError::Ledger)?,
            None => CanonicalLedger::try_new(chain_id, first_height, reducer, limits)
                .map_err(LocalReplayError::Ledger)?,
        };
        Ok(Self { ledger, store })
    }

    pub fn replay<Src: CanonicalBlockSource>(
        &mut self,
        source: &mut Src,
    ) -> Result<LocalReplayReport, LocalReplayError> {
        let mut applied = 0_u64;
        let mut already_applied = 0_u64;
        let mut last_height = self
            .ledger
            .checkpoint()
            .map(|checkpoint| checkpoint.block_height());
        while let Some(block) = source.next_block()? {
            match apply_block_durably(&mut self.ledger, &self.store, &block)? {
                DurableApplyOutcome::Applied { .. } => {
                    applied = applied.checked_add(1).ok_or(LocalReplayError::Overflow)?;
                    last_height = Some(block.block_height());
                }
                DurableApplyOutcome::AlreadyApplied(checkpoint) => {
                    already_applied = already_applied.saturating_add(1);
                    last_height = Some(checkpoint.block_height());
                }
            }
        }
        Ok(LocalReplayReport {
            applied,
            already_applied,
            last_height,
            state_hash: self.ledger.state_hash(),
        })
    }

    #[must_use]
    pub fn ledger(&self) -> &CanonicalLedger<R> {
        &self.ledger
    }
}

pub fn replay_block_durably<R: EventReducer, S: AtomicStateStore>(
    ledger: &mut CanonicalLedger<R>,
    store: &S,
    block: &BlockEnvelope,
) -> Result<DurableApplyOutcome, DurableApplyError> {
    apply_block_durably(ledger, store, block)
}
