#![forbid(unsafe_code)]

mod app;
mod checkpoint;
mod config;
mod consumer;
mod health;
mod input;
mod publication;
mod publisher;
mod reconciliation;
mod replay;
mod source;
mod state_runtime;

use canonical_events::BlockEnvelope;
use canonical_ledger::{
    CanonicalLedger, CorrectionRecord, EventReducer, LedgerError, PrepareOutcome, StateCheckpoint,
    StateDelta,
};
use storage_ports::{AtomicStateCommit, AtomicStateStore, StateCommitDisposition, StateStoreError};

pub use app::CoreApp;
pub use checkpoint::{load_checkpoint_ledger, publish_checkpoint};
pub use config::{ConfigError, CoreConfig, NatsConfig};
pub use consumer::{
    CanonicalDelivery, CanonicalPullSource, InMemoryCanonicalSource, JetStreamPullSource,
    JetStreamReplayAuth, JetStreamReplayConfig, JetStreamReplayConfigError, JetStreamReplayError,
    JetStreamReplayReport, JetStreamReplaySession, committed_block_delivery,
    committed_event_delivery,
};
pub use health::{
    DiskPressureError, DiskReserve, DiskSpaceProbe, FeatureHealth, HealthState, ShutdownFlag,
};
pub use input::CoreInputSubject;
pub use publication::{
    BLOCK_COMMITTED_SUBJECT, BLOCK_MARKER_SCHEMA_V1, BLOCK_PROVISIONAL_SUBJECT, BlockMarkerError,
    CANONICAL_STREAM, CanonicalSubject, CommittedBlockMarker, HEALTH_SOURCE_SUBJECT,
    SNAPSHOT_ACCOUNT_SUBJECT, SNAPSHOT_ECOSYSTEM_SUBJECT, SNAPSHOT_MARKET_SUBJECT,
    decode_committed_block_marker, encode_committed_block_marker, encode_event_payload,
    subject_for_event_kind,
};
pub use publisher::{
    InMemoryDeltaSink, PublishDisposition, PublishError, STATE_ACCOUNT_DELTA_SUBJECT,
    STATE_BOOK_DELTA_SUBJECT, STATE_DELTA_SCHEMA_V1, StateDeltaSink, encode_state_delta,
    publish_state_delta,
};
pub use reconciliation::{InputDisposition, QuarantineRecord, ReconciliationInbox};
pub use replay::{
    LocalBlockInspectReport, LocalReplayError, LocalReplayReport, LocalReplaySession,
    inspect_local_replay_block, replay_block_durably,
};
pub use source::{
    BlockSourceError, CanonicalBlockSource, DirectoryBlockSource, InMemoryBlockSource,
    LOCAL_REPLAY_BLOCK_SCHEMA, confirmation_label, decode_local_replay_block,
};
pub use state_runtime::{ResumeMode, StateRuntime, admit_resume_height, align_watermarks};

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
            Self::Ledger(error) => error.reason_code(),
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

/// Ingest a typed correction record. Application is unimplemented: the store is
/// not contacted and the ledger is not mutated.
pub fn ingest_correction_record<R, S: AtomicStateStore>(
    _ledger: &CanonicalLedger<R>,
    _store: &S,
    _record: &CorrectionRecord,
) -> Result<(), DurableApplyError> {
    Err(DurableApplyError::Ledger(
        LedgerError::CorrectionUnimplemented,
    ))
}
