#![forbid(unsafe_code)]

mod bootstrap;
mod multiple_testing;
mod performance;

pub use bootstrap::{BootstrapReport, DEFAULT_REPLICATES, stationary_block_bootstrap};
pub use multiple_testing::{MultipleTestingReport, claim_discovery, diagnose_family};
pub use performance::{PerformanceMetrics, score_predictions};
