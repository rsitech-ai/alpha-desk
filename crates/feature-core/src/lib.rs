#![forbid(unsafe_code)]

mod asof;
mod calculator;
mod errors;
mod feature;
mod health;
mod snapshot;
mod window;

pub use asof::{Bitemporal, asof, require_asof};
pub use calculator::{FeatureCalculator, FeatureContext, FeatureDelta, PitSnapshotCalculator};
pub use errors::FeatureError;
pub use feature::{
    EvidenceKind, EvidenceRef, FeatureKey, FeatureManifest, FeatureSubject, FeatureValue,
    MissingReason,
};
pub use health::{HealthAssessment, HealthState};
pub use snapshot::FeatureSnapshot;
pub use window::{
    RollingWindow, WINDOW_PARAMETER_VERSION, WindowAlgorithm, WindowSnapshot, WindowUpdate,
    window_debug_state,
};
