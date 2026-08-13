use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GraphError {
    #[error("empty identifier {field}")]
    EmptyIdentifier { field: &'static str },
    #[error("unsupported {what}")]
    Unsupported { what: &'static str },
    #[error("malformed {what}: {reason}")]
    Malformed {
        what: &'static str,
        reason: &'static str,
    },
    #[error("insufficient evidence families for merge")]
    InsufficientEvidenceFamilies,
    #[error("conflicting entity link refused: {reason}")]
    ConflictingLink { reason: &'static str },
    #[error("temporal inversion")]
    TemporalInversion,
    #[error("overflow")]
    Overflow,
    #[error("{0}")]
    Feature(String),
}

impl From<feature_core::FeatureError> for GraphError {
    fn from(error: feature_core::FeatureError) -> Self {
        Self::Feature(error.to_string())
    }
}

impl From<domain_types::ValueError> for GraphError {
    fn from(error: domain_types::ValueError) -> Self {
        match error {
            domain_types::ValueError::Overflow => Self::Overflow,
            domain_types::ValueError::OutOfRange => Self::Malformed {
                what: "probability",
                reason: "out of range",
            },
            domain_types::ValueError::Empty
            | domain_types::ValueError::Invalid
            | domain_types::ValueError::ExcessPrecision { .. }
            | domain_types::ValueError::ScaleOutOfRange { .. }
            | domain_types::ValueError::ScaleMismatch { .. }
            | domain_types::ValueError::DownwardExactRescale { .. }
            | domain_types::ValueError::DivisionByZero => Self::Malformed {
                what: "value",
                reason: "invalid",
            },
        }
    }
}
