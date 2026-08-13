#![forbid(unsafe_code)]

mod replay;
mod source;

use canonical_events::BlockEnvelope;
use canonical_ledger::{
    CanonicalLedger, EventReducer, LedgerError, PrepareOutcome, StateCheckpoint, StateDelta,
};
use storage_ports::{AtomicStateCommit, AtomicStateStore, StateCommitDisposition, StateStoreError};

pub use replay::{LocalReplayError, LocalReplayReport, LocalReplaySession, replay_block_durably};
pub use source::{
    BlockSourceError, CanonicalBlockSource, DirectoryBlockSource, InMemoryBlockSource,
    LOCAL_REPLAY_BLOCK_SCHEMA, decode_local_replay_block,
};

#[derive(Debug)]
pub enum DurableApplyOutcome {
    Applied {
        delta: StateDelta,
        disposition: StateCommitDisposition,
    },
    AlreadyApplied(StateCheckpoint),
}

#[derive(Debug, thiserror::Error)]
pub enum DurableApplyError {
    #[error("canonical ledger rejected the block: {0}")]
    Ledger(#[from] LedgerError),
    #[error("prepared state violates the durable commit contract: {0}")]
    CommitContract(StateStoreError),
    #[error("durable state commit failed: {0}")]
    Store(StateStoreError),
    #[error("durable state store returned a receipt for another transition")]
    ReceiptMismatch,
    #[error("visible ledger could not accept an already-durable prepared transition: {0}")]
    PostCommitLedger(LedgerError),
}

impl DurableApplyError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Ledger(_) => "core.ledger",
            Self::CommitContract(_) => "core.state_commit_contract",
            Self::Store(_) => "core.state_store",
            Self::ReceiptMismatch => "core.state_receipt_mismatch",
            Self::PostCommitLedger(_) => "core.post_commit_ledger",
        }
    }
}

pub fn apply_block_durably<R: EventReducer, S: AtomicStateStore>(
    ledger: &mut CanonicalLedger<R>,
    store: &S,
    block: &BlockEnvelope,
) -> Result<DurableApplyOutcome, DurableApplyError> {
    let prepared = match ledger.prepare_block(block)? {
        PrepareOutcome::Ready(prepared) => prepared,
        PrepareOutcome::AlreadyApplied(checkpoint) => {
            return Ok(DurableApplyOutcome::AlreadyApplied(checkpoint));
        }
    };
    let disposition = {
        let commit = AtomicStateCommit::try_new(prepared.delta(), prepared.state_image())
            .map_err(DurableApplyError::CommitContract)?;
        let disposition = store.commit(&commit).map_err(DurableApplyError::Store)?;
        if !disposition.receipt().matches(&commit) {
            return Err(DurableApplyError::ReceiptMismatch);
        }
        disposition
    };
    let delta = ledger
        .commit_prepared(prepared)
        .map_err(DurableApplyError::PostCommitLedger)?;
    Ok(DurableApplyOutcome::Applied { delta, disposition })
}
