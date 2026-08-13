#![forbid(unsafe_code)]

mod bootstrap;
mod calibration;
mod multiple_testing;
mod performance;

pub use bootstrap::{BootstrapReport, DEFAULT_REPLICATES, stationary_block_bootstrap};
pub use calibration::{CalibrationReport, calibrate_scores};
pub use multiple_testing::{MultipleTestingReport, claim_discovery, diagnose_family};
pub use performance::{
    CapacityPoint, PerformanceMetrics, kendall_tau, max_drawdown, ranked_capacity,
    score_predictions,
};
