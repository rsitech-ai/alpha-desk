use domain_types::ValueError;
use thiserror::Error;

use crate::signal::SignalLifecycleState;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SignalError {
    #[error("incomplete evidence: {0:?}")]
    IncompleteEvidence(Vec<String>),
    #[error("malformed {what}: {reason}")]
    Malformed {
        what: &'static str,
        reason: &'static str,
    },
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
        match error {
            market_intelligence::MarketError::Malformed { what, reason } => {
                Self::Malformed { what, reason }
            }
            market_intelligence::MarketError::MissingInput { name } => {
                Self::IncompleteEvidence(vec![name.to_owned()])
            }
            market_intelligence::MarketError::EmptyIdentifier { field } => {
                Self::EmptyIdentifier { field }
            }
            other @ (market_intelligence::MarketError::Unsupported { .. }
            | market_intelligence::MarketError::InsufficientHistory { .. }
            | market_intelligence::MarketError::RedDataHealth { .. }
            | market_intelligence::MarketError::EmptyDenominator
            | market_intelligence::MarketError::ScaleMismatch
            | market_intelligence::MarketError::Overflow
            | market_intelligence::MarketError::DivisionByZero
            | market_intelligence::MarketError::OutOfRange
            | market_intelligence::MarketError::Feature(_)) => Self::Domain(other.to_string()),
        }
    }
}
