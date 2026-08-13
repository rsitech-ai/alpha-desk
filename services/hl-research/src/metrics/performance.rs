use domain_types::{Decimal, RoundingMode};
use serde::Serialize;

use crate::error::ResearchError;
use crate::estimator::{METRIC_SCALE, mean};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PerformanceMetrics {
    pub n: usize,
    pub expectancy: String,
    pub median: String,
    pub hit_rate: String,
    pub mean_abs_error: String,
    pub sharpe: &'static str,
    pub significance: &'static str,
}

pub fn score_predictions(
    predictions: &[Decimal],
    outcomes: &[Decimal],
) -> Result<PerformanceMetrics, ResearchError> {
    if predictions.len() != outcomes.len() || predictions.is_empty() {
        return Err(ResearchError::InsufficientTrain { field: "metrics" });
    }
    let expectancy = mean(outcomes)?;
    let median = median(outcomes)?;
    let mut hits = 0_i128;
    let mut abs_errors = Vec::with_capacity(outcomes.len());
    for (prediction, outcome) in predictions.iter().zip(outcomes) {
        if same_sign(*prediction, *outcome) {
            hits += 1;
        }
        abs_errors.push(abs_diff(*prediction, *outcome)?);
    }
    let hit_rate = Decimal::from_raw(hits, 0)
        .map_err(|_| ResearchError::Metric { field: "hit_rate" })?
        .checked_div(
            Decimal::from_raw(
                i128::try_from(outcomes.len())
                    .map_err(|_| ResearchError::Metric { field: "hit_rate" })?,
                0,
            )
            .map_err(|_| ResearchError::Metric { field: "hit_rate" })?,
            METRIC_SCALE,
            RoundingMode::TowardZero,
        )
        .map_err(|_| ResearchError::Metric { field: "hit_rate" })?;
    Ok(PerformanceMetrics {
        n: outcomes.len(),
        expectancy: expectancy.to_string(),
        median: median.to_string(),
        hit_rate: hit_rate.to_string(),
        mean_abs_error: mean(&abs_errors)?.to_string(),
        sharpe: "not_claimed",
        significance: "not_claimed",
    })
}

fn median(values: &[Decimal]) -> Result<Decimal, ResearchError> {
    let mut ordered = values.to_vec();
    ordered.sort();
    let mid = ordered.len() / 2;
    if ordered.len() % 2 == 1 {
        Ok(ordered[mid])
    } else {
        mean(&[ordered[mid - 1], ordered[mid]])
    }
}

fn same_sign(left: Decimal, right: Decimal) -> bool {
    (left.raw() == 0 && right.raw() == 0) || left.raw().signum() == right.raw().signum()
}

fn abs_diff(left: Decimal, right: Decimal) -> Result<Decimal, ResearchError> {
    if left >= right {
        left.checked_sub(right)
            .map_err(|_| ResearchError::Metric { field: "mae" })
    } else {
        right
            .checked_sub(left)
            .map_err(|_| ResearchError::Metric { field: "mae" })
    }
}
