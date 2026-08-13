#![forbid(unsafe_code)]

mod error;
mod estimator;
mod experiment;
mod ledger;
mod metrics;
mod report;
mod shadow;
mod synthetic;
mod validation;

pub use error::ResearchError;
pub use estimator::{EstimatorClass, FittedEstimator, LinearModel, fit};
pub use experiment::{ExperimentManifest, ExperimentRecord, ExperimentRegistry, ExperimentStatus};
pub use ledger::{VariantLedger, VariantRecord, VariantStatus, variant_identity};
pub use metrics::{
    BootstrapReport, MultipleTestingReport, PerformanceMetrics, claim_discovery, diagnose_family,
    score_predictions, stationary_block_bootstrap,
};
pub use report::{ResearchReport, ResearchStatus};
pub use shadow::{
    ShadowCapture, ShadowCaptureReport, ShadowDecision, ShadowOutcome, run_shadow_capture_bytes,
};
pub use synthetic::{run_synthetic_bytes, run_synthetic_fixture};
pub use validation::{
    DatasetAccess, FoldAssignment, FoldEstimatorReport, HoldoutIsolationReport, HoldoutState,
    LabeledRow, ResearchDataset, ValidationFold, ValidationPolicy, WalkForwardReport,
    refuse_leaked_holdout_batch, run_evaluate_folds_bytes, run_holdout_isolation_bytes,
    run_walk_forward_bytes,
};
