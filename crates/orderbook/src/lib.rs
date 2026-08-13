#![forbid(unsafe_code)]

mod book;
mod execution;

pub use book::{
    BookDiff, BookHealth, L2Level, LifecycleEvent, LifecycleKind, OrderBook, RestingOrder,
};
pub use execution::{ExecutionError, ExecutionEstimate, ExecutionRequest, quote_execution};

pub const CRATE_BOOTSTRAPPED: bool = false;
