#![forbid(unsafe_code)]

mod alert_policy;
mod dedup;
mod errors;
mod evidence;
mod families;
mod invalidation;
mod lifecycle;
mod signal;
mod utility;

pub use alert_policy::{AlertDecision, AlertPolicy};
pub use dedup::{DedupKey, IndependenceClass, MaterialChange, dedup_key, originator_hash};
pub use errors::SignalError;
pub use evidence::EvidenceBundle;
pub use families::{
    FamilyThresholds, FragilityAsymmetryEvaluator, SignalContext, SignalEvaluation,
    SignalEvaluator, SmartCrowdDivergenceEvaluator, SmartFlowAccelerationEvaluator,
};
pub use invalidation::{
    InvalidationObservation, InvalidationRule, InvalidationStatus, any_triggered, evaluate_rule,
};
pub use lifecycle::{SignalLifecycleEvent, append_event, fold_lifecycle, transition_allowed};
pub use signal::{Signal, SignalActor, SignalConfirmationClass, SignalLifecycleState, SignalType};
pub use utility::canonical_utility;
