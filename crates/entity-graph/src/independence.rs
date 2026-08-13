use domain_types::ProbabilityPpm;
use serde::{Deserialize, Serialize};

use crate::GraphError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndependenceInput {
    pub hard_cluster_share: ProbabilityPpm,
    pub follower_probability: ProbabilityPpm,
    pub coordinated_action_probability: ProbabilityPpm,
    pub evidence_quality: ProbabilityPpm,
}

pub fn independence_weight(input: &IndependenceInput) -> Result<ProbabilityPpm, GraphError> {
    product_ppm(&[
        input.hard_cluster_share,
        complement(input.follower_probability)?,
        complement(input.coordinated_action_probability)?,
        input.evidence_quality,
    ])
}

pub fn normalize_cohort(weights: &[ProbabilityPpm]) -> Result<Vec<ProbabilityPpm>, GraphError> {
    if weights.is_empty() {
        return Err(GraphError::Malformed {
            what: "independence",
            reason: "empty cohort",
        });
    }
    Ok(weights.to_vec())
}

pub fn effective_votes(weights: &[ProbabilityPpm]) -> Result<u32, GraphError> {
    let sum = weights
        .iter()
        .try_fold(0_u64, |acc, weight| {
            acc.checked_add(u64::from(weight.ppm()))
        })
        .ok_or(GraphError::Overflow)?;
    u32::try_from((sum + 500_000) / 1_000_000).map_err(|_| GraphError::Overflow)
}

fn complement(value: ProbabilityPpm) -> Result<ProbabilityPpm, GraphError> {
    ProbabilityPpm::from_ppm(1_000_000 - value.ppm()).map_err(Into::into)
}

fn product_ppm(values: &[ProbabilityPpm]) -> Result<ProbabilityPpm, GraphError> {
    if values.is_empty() {
        return Err(GraphError::Malformed {
            what: "ppm_product",
            reason: "empty factor list",
        });
    }
    let mut acc = 1_u128;
    for (index, value) in values.iter().enumerate() {
        acc = acc
            .checked_mul(u128::from(value.ppm()))
            .ok_or(GraphError::Overflow)?;
        if index > 0 {
            acc /= 1_000_000;
        }
    }
    ProbabilityPpm::from_ppm(u32::try_from(acc).map_err(|_| GraphError::Overflow)?)
        .map_err(Into::into)
}
