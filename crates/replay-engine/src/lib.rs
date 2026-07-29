#![forbid(unsafe_code)]

mod engine;
mod error;
mod receipt;
mod request;

pub use engine::{ReplayCancellation, SerialReplayEngine};
pub use error::{ReplayError, ReplayProgress, ReplayRequestError};
pub use receipt::{ReplayOutcome, ReplayReceipt, ReplayStatus};
pub use request::{ReplayLimits, ReplayRequest};

pub const CRATE_BOOTSTRAPPED: bool = true;
