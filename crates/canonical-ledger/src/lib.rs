#![forbid(unsafe_code)]

mod error;
mod ledger;
mod reducer;
mod state;
mod watermark_only;

pub use error::{LedgerError, ReducerError, StateKeyError};
pub use ledger::{ApplyOutcome, CanonicalLedger, LedgerLimits, StateCheckpoint, StateDelta};
pub use reducer::{ApplyContext, EventReducer};
pub use state::{AppliedMutation, StateImage, StateKey, StateMutation, StateView};
pub use watermark_only::WatermarkOnlyReducerV1;

pub const CRATE_BOOTSTRAPPED: bool = true;
