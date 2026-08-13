use domain_types::{Decimal, RoundingMode};
use serde::{Deserialize, Serialize};

use crate::error::ResearchError;

pub const METRIC_SCALE: u8 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EstimatorClass {
    MeanOutcome,
    UnivariateLinear,
}

impl EstimatorClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MeanOutcome => "mean_outcome",
            Self::UnivariateLinear => "univariate_linear",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FittedEstimator {
    pub class: EstimatorClass,
    pub intercept: String,
    pub weights: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinearModel {
    class: EstimatorClass,
    intercept: Decimal,
    weights: Vec<Decimal>,
}

impl LinearModel {
    #[must_use]
    pub fn class(&self) -> EstimatorClass {
        self.class
    }

    pub fn predict(&self, features: &[Decimal]) -> Result<Decimal, ResearchError> {
        match self.class {
            EstimatorClass::MeanOutcome => Ok(self.intercept),
            EstimatorClass::UnivariateLinear => {
                if self.weights.len() != 1 || features.len() != 1 {
                    return Err(ResearchError::UnsupportedEstimator);
                }
                let term = self.weights[0]
                    .checked_mul(features[0], METRIC_SCALE, RoundingMode::TowardZero)
                    .map_err(|_| ResearchError::Metric {
                        field: "predict.mul",
                    })?;
                self.intercept
                    .checked_add(term)
                    .map_err(|_| ResearchError::Metric {
                        field: "predict.add",
                    })
            }
        }
    }

    #[must_use]
    pub fn report(&self) -> FittedEstimator {
        FittedEstimator {
            class: self.class,
            intercept: self.intercept.to_string(),
            weights: self.weights.iter().map(ToString::to_string).collect(),
        }
    }
}

pub fn fit(
    class: EstimatorClass,
    features: &[Vec<Decimal>],
    outcomes: &[Decimal],
) -> Result<LinearModel, ResearchError> {
    if features.len() != outcomes.len() {
        return Err(ResearchError::InvalidFixture);
    }
    match class {
        EstimatorClass::MeanOutcome => fit_mean(outcomes),
        EstimatorClass::UnivariateLinear => fit_univariate(features, outcomes),
    }
}

pub fn mean(values: &[Decimal]) -> Result<Decimal, ResearchError> {
    if values.is_empty() {
        return Err(ResearchError::InsufficientTrain { field: "mean" });
    }
    let mut acc = zero()?;
    for value in values {
        let aligned = value
            .rescale(METRIC_SCALE, RoundingMode::TowardZero)
            .map_err(|_| ResearchError::Metric {
                field: "mean.scale",
            })?;
        acc = acc
            .checked_add(aligned)
            .map_err(|_| ResearchError::Metric { field: "mean.add" })?;
    }
    let count = Decimal::from_raw(
        i128::try_from(values.len()).map_err(|_| ResearchError::Metric { field: "mean.n" })?,
        0,
    )
    .map_err(|_| ResearchError::Metric { field: "mean.n" })?;
    acc.checked_div(count, METRIC_SCALE, RoundingMode::TowardZero)
        .map_err(|_| ResearchError::Metric { field: "mean.div" })
}

fn fit_mean(outcomes: &[Decimal]) -> Result<LinearModel, ResearchError> {
    Ok(LinearModel {
        class: EstimatorClass::MeanOutcome,
        intercept: mean(outcomes)?,
        weights: Vec::new(),
    })
}

fn fit_univariate(
    features: &[Vec<Decimal>],
    outcomes: &[Decimal],
) -> Result<LinearModel, ResearchError> {
    if features.len() < 2 {
        return Err(ResearchError::InsufficientTrain {
            field: "univariate_linear",
        });
    }
    let mut xs = Vec::with_capacity(features.len());
    for row in features {
        if row.len() != 1 {
            return Err(ResearchError::UnsupportedEstimator);
        }
        xs.push(
            row[0]
                .rescale(METRIC_SCALE, RoundingMode::TowardZero)
                .map_err(|_| ResearchError::Metric { field: "ols.x" })?,
        );
    }
    let ys: Vec<Decimal> = outcomes
        .iter()
        .map(|value| {
            value
                .rescale(METRIC_SCALE, RoundingMode::TowardZero)
                .map_err(|_| ResearchError::Metric { field: "ols.y" })
        })
        .collect::<Result<_, _>>()?;
    let xbar = mean(&xs)?;
    let ybar = mean(&ys)?;
    let mut covariance = zero()?;
    let mut variance = zero()?;
    for (x, y) in xs.iter().zip(&ys) {
        let dx = x
            .checked_sub(xbar)
            .map_err(|_| ResearchError::Metric { field: "ols.dx" })?;
        let dy = y
            .checked_sub(ybar)
            .map_err(|_| ResearchError::Metric { field: "ols.dy" })?;
        let cov_term = dx
            .checked_mul(dy, METRIC_SCALE, RoundingMode::TowardZero)
            .map_err(|_| ResearchError::Metric { field: "ols.cov" })?;
        let var_term = dx
            .checked_mul(dx, METRIC_SCALE, RoundingMode::TowardZero)
            .map_err(|_| ResearchError::Metric { field: "ols.var" })?;
        covariance = covariance
            .checked_add(cov_term)
            .map_err(|_| ResearchError::Metric { field: "ols.cov" })?;
        variance = variance
            .checked_add(var_term)
            .map_err(|_| ResearchError::Metric { field: "ols.var" })?;
    }
    if variance.raw() == 0 {
        return Err(ResearchError::UnmodeledVariance {
            field: "univariate_linear",
        });
    }
    let slope = covariance
        .checked_div(variance, METRIC_SCALE, RoundingMode::TowardZero)
        .map_err(|_| ResearchError::Metric { field: "ols.slope" })?;
    let intercept_term = slope
        .checked_mul(xbar, METRIC_SCALE, RoundingMode::TowardZero)
        .map_err(|_| ResearchError::Metric {
            field: "ols.intercept",
        })?;
    let intercept = ybar
        .checked_sub(intercept_term)
        .map_err(|_| ResearchError::Metric {
            field: "ols.intercept",
        })?;
    Ok(LinearModel {
        class: EstimatorClass::UnivariateLinear,
        intercept,
        weights: vec![slope],
    })
}

fn zero() -> Result<Decimal, ResearchError> {
    Decimal::from_raw(0, METRIC_SCALE).map_err(|_| ResearchError::Metric { field: "zero" })
}
