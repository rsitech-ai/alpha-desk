use domain_types::{Decimal, RoundingMode};
use serde::Serialize;

use crate::error::ResearchError;
use crate::estimator::{METRIC_SCALE, mean, zero};

const CAPACITY_PPMS: [u32; 3] = [1_000_000, 500_000, 250_000];
const MIN_EXPECTED_SHORTFALL_N: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CapacityPoint {
    pub top_ppm: u32,
    pub n: usize,
    pub expectancy: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PerformanceMetrics {
    pub n: usize,
    pub expectancy: String,
    pub median: String,
    pub min_outcome: String,
    pub max_drawdown: String,
    pub information_coefficient: String,
    pub capacity_top: Vec<CapacityPoint>,
    pub hit_rate: String,
    pub mean_abs_error: String,
    pub expected_shortfall: Option<String>,
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
        median: median(outcomes)?.to_string(),
        min_outcome: min_value(outcomes)?.to_string(),
        max_drawdown: max_drawdown(outcomes)?.to_string(),
        information_coefficient: kendall_tau(predictions, outcomes)?,
        capacity_top: ranked_capacity(predictions, outcomes)?,
        hit_rate: hit_rate.to_string(),
        mean_abs_error: mean(&abs_errors)?.to_string(),
        expected_shortfall: expected_shortfall(outcomes)?,
        sharpe: "not_claimed",
        significance: "not_claimed",
    })
}

pub fn max_drawdown(outcomes: &[Decimal]) -> Result<Decimal, ResearchError> {
    let mut equity = zero()?;
    let mut peak = zero()?;
    let mut worst = zero()?;
    for outcome in outcomes {
        let aligned = outcome
            .rescale(METRIC_SCALE, RoundingMode::TowardZero)
            .map_err(|_| ResearchError::Metric { field: "drawdown" })?;
        equity = equity
            .checked_add(aligned)
            .map_err(|_| ResearchError::Metric { field: "drawdown" })?;
        if equity > peak {
            peak = equity;
        }
        let drawdown = peak
            .checked_sub(equity)
            .map_err(|_| ResearchError::Metric { field: "drawdown" })?;
        if drawdown > worst {
            worst = drawdown;
        }
    }
    Ok(worst)
}

pub fn kendall_tau(predictions: &[Decimal], outcomes: &[Decimal]) -> Result<String, ResearchError> {
    if predictions.len() != outcomes.len() {
        return Err(ResearchError::Metric { field: "ic.length" });
    }
    if predictions.len() < 2 {
        return Ok("not_claimed".to_owned());
    }
    let n =
        i128::try_from(predictions.len()).map_err(|_| ResearchError::Metric { field: "ic.n" })?;
    let pairs = n
        .checked_mul(n - 1)
        .and_then(|value| value.checked_div(2))
        .ok_or(ResearchError::Metric { field: "ic.pairs" })?;
    if pairs == 0 {
        return Ok("not_claimed".to_owned());
    }
    let mut concordant = 0_i128;
    let mut discordant = 0_i128;
    for left in 0..predictions.len() {
        for right in (left + 1)..predictions.len() {
            let pred = cmp_sign(predictions[left], predictions[right]);
            let outcome = cmp_sign(outcomes[left], outcomes[right]);
            let product = pred * outcome;
            if product > 0 {
                concordant += 1;
            } else if product < 0 {
                discordant += 1;
            }
        }
    }
    let numerator = Decimal::from_raw(concordant - discordant, 0)
        .map_err(|_| ResearchError::Metric { field: "ic.num" })?;
    let denominator =
        Decimal::from_raw(pairs, 0).map_err(|_| ResearchError::Metric { field: "ic.den" })?;
    Ok(numerator
        .checked_div(denominator, METRIC_SCALE, RoundingMode::TowardZero)
        .map_err(|_| ResearchError::Metric { field: "ic.div" })?
        .to_string())
}

pub fn ranked_capacity(
    predictions: &[Decimal],
    outcomes: &[Decimal],
) -> Result<Vec<CapacityPoint>, ResearchError> {
    let mut order: Vec<usize> = (0..predictions.len()).collect();
    order.sort_by(|left, right| {
        predictions[*right]
            .cmp(&predictions[*left])
            .then(left.cmp(right))
    });
    let mut points = Vec::with_capacity(CAPACITY_PPMS.len());
    for top_ppm in CAPACITY_PPMS {
        let take = top_k(outcomes.len(), top_ppm);
        let selected: Vec<Decimal> = order
            .iter()
            .take(take)
            .map(|index| outcomes[*index])
            .collect();
        points.push(CapacityPoint {
            top_ppm,
            n: selected.len(),
            expectancy: mean(&selected)?.to_string(),
        });
    }
    Ok(points)
}

fn expected_shortfall(outcomes: &[Decimal]) -> Result<Option<String>, ResearchError> {
    if outcomes.len() < MIN_EXPECTED_SHORTFALL_N {
        return Ok(None);
    }
    let mut ordered = outcomes.to_vec();
    ordered.sort();
    let take = (outcomes.len() * 5 / 100).max(1);
    Ok(Some(mean(&ordered[..take])?.to_string()))
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

fn min_value(values: &[Decimal]) -> Result<Decimal, ResearchError> {
    values
        .iter()
        .copied()
        .min()
        .ok_or(ResearchError::Metric { field: "min" })
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

fn cmp_sign(left: Decimal, right: Decimal) -> i128 {
    match left.cmp(&right) {
        std::cmp::Ordering::Less => -1,
        std::cmp::Ordering::Equal => 0,
        std::cmp::Ordering::Greater => 1,
    }
}

fn top_k(n: usize, ppm: u32) -> usize {
    let ppm = usize::try_from(ppm.min(1_000_000)).unwrap_or(1_000_000);
    let scaled = n.saturating_mul(ppm) / 1_000_000;
    scaled.max(1).min(n)
}
