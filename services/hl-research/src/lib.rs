#![forbid(unsafe_code)]

mod baselines;
mod claims;
mod corpus;
mod error;
mod estimator;
mod experiment;
mod ledger;
mod metrics;
mod promotion;
mod report;
mod shadow;
mod synthetic;
mod validation;

pub use baselines::{FOLD_ESTIMATOR_CLASSES, SYNTHETIC_BASELINES, UNMODELED_BASELINES};
pub use corpus::{CorpusClass, load_corpus_path, refuse_corpus_path};
pub use error::ResearchError;
pub use estimator::{EstimatorClass, FittedEstimator, LinearModel, fit};
pub use experiment::{ExperimentManifest, ExperimentRecord, ExperimentRegistry, ExperimentStatus};
pub use ledger::{VariantLedger, VariantRecord, VariantStatus, variant_identity};
pub use metrics::{
    BootstrapReport, CalibrationReport, CapacityPoint, MultipleTestingReport, PerformanceMetrics,
    calibrate_scores, claim_discovery, diagnose_family, kendall_tau, max_drawdown, ranked_capacity,
    score_predictions, stationary_block_bootstrap,
};
pub use promotion::{
    GateDecision, GateResult, HoldoutLock, PromotionEvidence, PromotionPolicy, PromotionReport,
    evaluate_promotion, lock_path_is_in_repo, promote, stamp_holdout_passed,
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
    run_promote_bytes, run_walk_forward_bytes,
};
