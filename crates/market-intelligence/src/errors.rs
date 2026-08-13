use domain_types::ValueError;
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum MarketError {
    #[error("empty identifier {field}")]
    EmptyIdentifier { field: &'static str },
    #[error("unsupported {what}")]
    Unsupported { what: &'static str },
    #[error("malformed {what}: {reason}")]
    Malformed {
        what: &'static str,
        reason: &'static str,
    },
    #[error("insufficient history for {what}")]
    InsufficientHistory { what: &'static str },
    #[error("red data health refuses {what}")]
    RedDataHealth { what: &'static str },
    #[error("ambiguous or empty denominator")]
    EmptyDenominator,
    #[error("missing required input {name}")]
    MissingInput { name: &'static str },
    #[error("scale mismatch")]
    ScaleMismatch,
    #[error("fixed-point overflow")]
    Overflow,
    #[error("division by zero")]
    DivisionByZero,
    #[error("value out of range")]
    OutOfRange,
    #[error("{0}")]
    Feature(String),
}

impl From<ValueError> for MarketError {
    fn from(error: ValueError) -> Self {
        match error {
            ValueError::DivisionByZero => Self::DivisionByZero,
            ValueError::Overflow => Self::Overflow,
            ValueError::ScaleMismatch { .. } => Self::ScaleMismatch,
            ValueError::OutOfRange => Self::OutOfRange,
            ValueError::Empty
            | ValueError::Invalid
            | ValueError::ExcessPrecision { .. }
            | ValueError::ScaleOutOfRange { .. }
            | ValueError::DownwardExactRescale { .. } => Self::Malformed {
                what: "decimal",
                reason: "invalid exact value",
            },
        }
    }
}

impl From<feature_core::FeatureError> for MarketError {
    fn from(error: feature_core::FeatureError) -> Self {
        Self::Feature(error.to_string())
    }
}

impl From<wallet_intelligence::IntelligenceError> for MarketError {
    fn from(error: wallet_intelligence::IntelligenceError) -> Self {
        Self::Feature(error.to_string())
    }
}
