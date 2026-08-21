#![forbid(unsafe_code)]

mod book;
mod checkpoint;
mod execution;
mod fixture;
mod reconcile;
mod reconstruct;
mod store;

pub use book::{
    BookDiff, BookHealth, DEFAULT_MAX_ORDERS, L2Level, LifecycleEvent, LifecycleKind, OrderBook,
    RestingOrder, TriggerKind,
};
pub use checkpoint::L4CheckpointV1;
pub use execution::{
    ExecutionError, ExecutionEstimate, ExecutionRequest, FEE_SCHEDULE_NONE,
    FEE_SCHEDULE_TAKER_100BPS_V1, quote_execution,
};
pub use fixture::{
    BOOK_FIXTURE_SCHEMA, BookFixture, BookFixtureError, BookReplayReport, parse_book_fixture,
    replay_book_fixture,
};
pub use reconcile::{
    L2_RECONCILE_MAX_TIME_SKEW_MILLIS_V1, L2_RECONCILE_POLICY_V1, L2ReconcileDecision,
    L2ReconcilePolicyV1, reconcile_derived_l2,
};
pub use reconstruct::{L4Error, L4Reconstruction};
pub use store::{
    CF_CHECKPOINTS, CF_L2_BOOK, CF_L4_ORDERS, L4_CHECKPOINT_SCHEMA, L4_STORE_SCHEMA, L4StoreError,
    checkpoint_key, decode_l2_book, decode_resting_order, encode_l2_book, encode_resting_order,
    l2_book_key, l4_order_key,
};

pub const CRATE_BOOTSTRAPPED: bool = true;
