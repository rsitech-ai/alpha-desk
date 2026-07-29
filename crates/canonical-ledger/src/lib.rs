#![forbid(unsafe_code)]

mod error;
mod ledger;
mod reducer;
mod state;

pub use error::{LedgerError, ReducerError, StateKeyError};
pub use ledger::{ApplyOutcome, CanonicalLedger, LedgerLimits, StateCheckpoint, StateDelta};
pub use reducer::{ApplyContext, EventReducer};
pub use state::{AppliedMutation, StateImage, StateKey, StateMutation, StateView};

pub const CRATE_BOOTSTRAPPED: bool = true;
