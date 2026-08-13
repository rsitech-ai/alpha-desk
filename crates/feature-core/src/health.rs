use serde::{Deserialize, Serialize};

/// Point-in-time data-health flag for a feature snapshot.
///
/// This is independent of the telemetry crate so feature definitions stay
/// runtime-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HealthState {
    Green,
    Amber,
    Red,
}

impl HealthState {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Green => "GREEN",
            Self::Amber => "AMBER",
            Self::Red => "RED",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthAssessment {
    pub scope: String,
    pub state: HealthState,
    pub reason_code: String,
}

impl HealthAssessment {
    pub fn try_new(
        scope: impl Into<String>,
        state: HealthState,
        reason_code: impl Into<String>,
    ) -> Result<Self, crate::FeatureError> {
        let scope = scope.into();
        let reason_code = reason_code.into();
        if scope.trim().is_empty() {
            return Err(crate::FeatureError::EmptyIdentifier { field: "scope" });
        }
        if reason_code.trim().is_empty() {
            return Err(crate::FeatureError::EmptyIdentifier {
                field: "reason_code",
            });
        }
        if scope.trim() != scope || reason_code.trim() != reason_code {
            return Err(crate::FeatureError::Malformed {
                what: "health_assessment",
                reason: "surrounding whitespace",
            });
        }
        Ok(Self {
            scope,
            state,
            reason_code,
        })
    }
}
