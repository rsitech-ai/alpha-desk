#![forbid(unsafe_code)]

mod book;
mod execution;
mod fixture;

pub use book::{
    BookDiff, BookHealth, L2Level, LifecycleEvent, LifecycleKind, OrderBook, RestingOrder,
};
pub use execution::{
    ExecutionError, ExecutionEstimate, ExecutionRequest, FEE_SCHEDULE_NONE,
    FEE_SCHEDULE_TAKER_100BPS_V1, quote_execution,
};
pub use fixture::{
    BOOK_FIXTURE_SCHEMA, BookFixture, BookFixtureError, BookReplayReport, parse_book_fixture,
    replay_book_fixture,
};

pub const CRATE_BOOTSTRAPPED: bool = false;
