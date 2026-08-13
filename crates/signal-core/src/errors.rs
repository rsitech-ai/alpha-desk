use domain_types::ValueError;
use thiserror::Error;

use crate::signal::SignalLifecycleState;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SignalError {
    #[error("incomplete evidence: {0:?}")]
    IncompleteEvidence(Vec<String>),
    #[error("invalid transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: SignalLifecycleState,
        to: SignalLifecycleState,
    },
    #[error("unsupported health")]
    UnsupportedHealth,
    #[error("contract violation: {0}")]
    ContractViolation(&'static str),
    #[error("empty identifier {field}")]
    EmptyIdentifier { field: &'static str },
    #[error("research-only signals cannot become live")]
    ResearchOnlyCannotGoLive,
    #[error("red dependency suppresses {what}")]
    Suppressed { what: &'static str },
    #[error("overflow")]
    Overflow,
    #[error("{0}")]
    Domain(String),
}

impl From<ValueError> for SignalError {
    fn from(error: ValueError) -> Self {
        Self::Domain(error.to_string())
    }
}

impl From<feature_core::FeatureError> for SignalError {
    fn from(error: feature_core::FeatureError) -> Self {
        Self::Domain(error.to_string())
    }
}

impl From<market_intelligence::MarketError> for SignalError {
    fn from(error: market_intelligence::MarketError) -> Self {
        Self::Domain(error.to_string())
    }
}
