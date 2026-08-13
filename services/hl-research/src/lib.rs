#![forbid(unsafe_code)]

mod error;
mod experiment;
mod report;
mod shadow;
mod synthetic;
mod validation;

pub use error::ResearchError;
pub use experiment::{ExperimentManifest, ExperimentRecord, ExperimentRegistry, ExperimentStatus};
pub use report::{ResearchReport, ResearchStatus};
pub use shadow::{
    ShadowCapture, ShadowCaptureReport, ShadowDecision, ShadowOutcome, run_shadow_capture_bytes,
};
pub use synthetic::{run_synthetic_bytes, run_synthetic_fixture};
pub use validation::{
    DatasetAccess, FoldAssignment, HoldoutIsolationReport, HoldoutState, LabeledRow,
    ResearchDataset, ValidationFold, ValidationPolicy, WalkForwardReport,
    refuse_leaked_holdout_batch, run_holdout_isolation_bytes, run_walk_forward_bytes,
};
