use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use thiserror::Error;

const INVALID_SCOPE: &str = "health:invalid";

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HealthState {
    Green,
    Amber,
    Red,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HealthAssessment {
    pub scope: String,
    pub state: HealthState,
    pub reason_code: String,
    pub observed_at_micros: i64,
    pub suppresses: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum HealthError {
    #[error("{field} must not be empty")]
    EmptyIdentifier { field: &'static str },
    #[error("{field} must not contain empty colon-delimited segments")]
    EmptySegment { field: &'static str },
    #[error("{field} must not contain control characters")]
    ControlCharacter { field: &'static str },
    #[error("{field} must not have leading or trailing whitespace")]
    SurroundingWhitespace { field: &'static str },
    #[error("observed_at_micros must be non-negative")]
    NegativeObservedTime,
}

impl HealthAssessment {
    #[must_use]
    pub fn green(scope: impl Into<String>) -> Self {
        match Self::try_green(scope) {
            Ok(assessment) => assessment,
            Err(error) => Self::invalid(error),
        }
    }

    #[must_use]
    pub fn amber(scope: impl Into<String>, reason_code: impl Into<String>) -> Self {
        match Self::try_amber(scope, reason_code) {
            Ok(assessment) => assessment,
            Err(error) => Self::invalid(error),
        }
    }

    #[must_use]
    pub fn red(scope: impl Into<String>, reason_code: impl Into<String>) -> Self {
        match Self::try_red(scope, reason_code) {
            Ok(assessment) => assessment,
            Err(error) => Self::invalid(error),
        }
    }

    pub fn try_green(scope: impl Into<String>) -> Result<Self, HealthError> {
        Self::build(
            scope.into(),
            HealthState::Green,
            "healthy".to_owned(),
            0,
            std::iter::empty::<String>(),
        )
    }

    pub fn try_amber(
        scope: impl Into<String>,
        reason_code: impl Into<String>,
    ) -> Result<Self, HealthError> {
        Self::build(
            scope.into(),
            HealthState::Amber,
            reason_code.into(),
            0,
            std::iter::empty::<String>(),
        )
    }

    pub fn try_red(
        scope: impl Into<String>,
        reason_code: impl Into<String>,
    ) -> Result<Self, HealthError> {
        Self::build(
            scope.into(),
            HealthState::Red,
            reason_code.into(),
            0,
            std::iter::empty::<String>(),
        )
    }

    pub fn try_green_at<I, S>(
        scope: impl Into<String>,
        observed_at_micros: i64,
        suppresses: I,
    ) -> Result<Self, HealthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::build(
            scope.into(),
            HealthState::Green,
            "healthy".to_owned(),
            observed_at_micros,
            suppresses,
        )
    }

    pub fn try_amber_at<I, S>(
        scope: impl Into<String>,
        reason_code: impl Into<String>,
        observed_at_micros: i64,
        suppresses: I,
    ) -> Result<Self, HealthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::build(
            scope.into(),
            HealthState::Amber,
            reason_code.into(),
            observed_at_micros,
            suppresses,
        )
    }

    pub fn try_red_at<I, S>(
        scope: impl Into<String>,
        reason_code: impl Into<String>,
        observed_at_micros: i64,
        suppresses: I,
    ) -> Result<Self, HealthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::build(
            scope.into(),
            HealthState::Red,
            reason_code.into(),
            observed_at_micros,
            suppresses,
        )
    }

    fn build<I, S>(
        scope: String,
        state: HealthState,
        reason_code: String,
        observed_at_micros: i64,
        suppresses: I,
    ) -> Result<Self, HealthError>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        validate_identifier(&scope, "scope", true)?;
        validate_identifier(&reason_code, "reason_code", false)?;
        if observed_at_micros < 0 {
            return Err(HealthError::NegativeObservedTime);
        }

        let mut suppression_set = BTreeSet::new();
        for suppression in suppresses {
            let suppression = suppression.into();
            validate_identifier(&suppression, "suppression", true)?;
            suppression_set.insert(suppression);
        }
        if state == HealthState::Red {
            suppression_set.extend(suppression_dependencies(&scope));
        }

        Ok(Self {
            scope,
            state,
            reason_code,
            observed_at_micros,
            suppresses: suppression_set.into_iter().collect(),
        })
    }

    fn invalid(error: HealthError) -> Self {
        let reason_code = match error {
            HealthError::NegativeObservedTime => "invalid_observed_time",
            HealthError::EmptyIdentifier { field: "scope" }
            | HealthError::EmptySegment { field: "scope" }
            | HealthError::ControlCharacter { field: "scope" }
            | HealthError::SurroundingWhitespace { field: "scope" } => "invalid_scope",
            HealthError::EmptyIdentifier {
                field: "reason_code",
            }
            | HealthError::ControlCharacter {
                field: "reason_code",
            }
            | HealthError::SurroundingWhitespace {
                field: "reason_code",
            }
            | HealthError::EmptySegment {
                field: "reason_code",
            } => "invalid_reason_code",
            _ => "invalid_suppression",
        };
        Self {
            scope: INVALID_SCOPE.to_owned(),
            state: HealthState::Red,
            reason_code: reason_code.to_owned(),
            observed_at_micros: 0,
            suppresses: Vec::new(),
        }
    }

    #[must_use]
    pub fn aggregate<I>(required: I) -> Self
    where
        I: IntoIterator<Item = Self>,
    {
        let assessments: Vec<Self> = required.into_iter().collect();
        if assessments.is_empty() {
            return Self {
                scope: "health:aggregate".to_owned(),
                state: HealthState::Red,
                reason_code: "no_required_dependencies".to_owned(),
                observed_at_micros: 0,
                suppresses: Vec::new(),
            };
        }

        let state = assessments
            .iter()
            .map(|assessment| assessment.state)
            .max()
            .map_or(HealthState::Red, |value| value);
        let observed_at_micros = assessments
            .iter()
            .map(|assessment| assessment.observed_at_micros)
            .max()
            .map_or(0, |value| value);
        let mut reasons: Vec<(&str, &str)> = assessments
            .iter()
            .filter(|assessment| assessment.state != HealthState::Green)
            .map(|assessment| (assessment.scope.as_str(), assessment.reason_code.as_str()))
            .collect();
        reasons.sort_by(|left, right| left.0.cmp(right.0).then(left.1.cmp(right.1)));
        let reason_code = if reasons.is_empty() {
            "healthy".to_owned()
        } else {
            reasons
                .into_iter()
                .map(|(scope, reason)| format!("{scope}={reason}"))
                .collect::<Vec<_>>()
                .join(";")
        };
        let suppresses = assessments
            .iter()
            .flat_map(|assessment| assessment.suppresses.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();

        Self {
            scope: "health:aggregate".to_owned(),
            state,
            reason_code,
            observed_at_micros,
            suppresses,
        }
    }

    #[must_use]
    pub fn suppresses(&self, scope: &str) -> bool {
        self.suppresses
            .binary_search_by(|candidate| candidate.as_str().cmp(scope))
            .is_ok()
    }
}

fn suppression_dependencies(scope: &str) -> Vec<String> {
    scope
        .strip_prefix("book:")
        .filter(|market| !market.is_empty() && !market.contains(':'))
        .map_or_else(Vec::new, |market| vec![format!("market:{market}:capacity")])
}

fn validate_identifier(
    value: &str,
    field: &'static str,
    reject_empty_segments: bool,
) -> Result<(), HealthError> {
    if value.trim().is_empty() {
        return Err(HealthError::EmptyIdentifier { field });
    }
    if value.trim() != value {
        return Err(HealthError::SurroundingWhitespace { field });
    }
    if value.chars().any(char::is_control) {
        return Err(HealthError::ControlCharacter { field });
    }
    if reject_empty_segments && value.split(':').any(str::is_empty) {
        return Err(HealthError::EmptySegment { field });
    }
    Ok(())
}
