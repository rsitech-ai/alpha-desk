use domain_types::{Decimal, RoundingMode};
use serde::Serialize;

use crate::error::ResearchError;
use crate::estimator::{METRIC_SCALE, mean};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CalibrationReport {
    pub schema_version: &'static str,
    pub status: &'static str,
    pub serialized_as_probability: bool,
    pub brier: Option<String>,
    pub reliability_error: Option<String>,
    pub withheld_reason: Option<&'static str>,
}

pub fn calibrate_scores(
    scores: &[Decimal],
    outcomes: &[Decimal],
) -> Result<CalibrationReport, ResearchError> {
    if scores.len() != outcomes.len() || scores.is_empty() {
        return Err(ResearchError::InsufficientTrain {
            field: "calibration",
        });
    }
    if !scores.iter().all(in_unit_interval) || !outcomes.iter().all(in_unit_interval) {
        return Ok(uncalibrated(Some("scores_are_not_probabilities")));
    }
    let mut squared_errors = Vec::with_capacity(scores.len());
    for (score, outcome) in scores.iter().zip(outcomes) {
        let error = if *score >= *outcome {
            score
                .checked_sub(*outcome)
                .map_err(|_| ResearchError::Metric { field: "brier" })?
        } else {
            outcome
                .checked_sub(*score)
                .map_err(|_| ResearchError::Metric { field: "brier" })?
        };
        let squared = error
            .checked_mul(error, METRIC_SCALE, RoundingMode::TowardZero)
            .map_err(|_| ResearchError::Metric { field: "brier" })?;
        squared_errors.push(squared);
    }
    Ok(CalibrationReport {
        schema_version: "hl.research.calibration.v1",
        status: "uncalibrated",
        serialized_as_probability: false,
        brier: Some(mean(&squared_errors)?.to_string()),
        reliability_error: None,
        withheld_reason: Some("no_fitted_calibrator"),
    })
}

fn uncalibrated(reason: Option<&'static str>) -> CalibrationReport {
    CalibrationReport {
        schema_version: "hl.research.calibration.v1",
        status: "uncalibrated",
        serialized_as_probability: false,
        brier: None,
        reliability_error: None,
        withheld_reason: reason,
    }
}

fn in_unit_interval(value: &Decimal) -> bool {
    let Ok(aligned) = value.rescale(METRIC_SCALE, RoundingMode::TowardZero) else {
        return false;
    };
    aligned.raw() >= 0 && aligned.raw() <= 100_000_000
}
