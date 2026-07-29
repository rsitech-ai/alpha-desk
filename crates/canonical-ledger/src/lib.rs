#![forbid(unsafe_code)]

mod checkpoint;
mod error;
mod ledger;
mod reducer;
mod state;
mod trade;
mod watermark_only;

pub use checkpoint::{CheckpointArtifact, CheckpointCompatibility};
pub use error::{CheckpointError, LedgerError, ReducerError, StateImageError, StateKeyError};
pub use ledger::{
    ApplyOutcome, CanonicalLedger, LedgerLimits, PrepareOutcome, PreparedBlock, StateCheckpoint,
    StateDelta,
};
pub use reducer::{ApplyContext, EventReducer};
pub use state::{
    AppliedMutation, StateImage, StateImageLimits, StateKey, StateMutation, StateView,
};
pub use trade::{
    CanonicalTradeReducerV1, TradeParticipantRecordV1, TradeReconciliationRecordV1,
    TradeStateError, TradeStateRecordV1,
};
pub use watermark_only::WatermarkOnlyReducerV1;

pub const CRATE_BOOTSTRAPPED: bool = true;
