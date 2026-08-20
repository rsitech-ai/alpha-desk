use domain_types::Decimal;
use serde::Serialize;

use crate::error::ResearchError;
use crate::estimator::mean;

pub const MIN_BOOTSTRAP_N: usize = 8;
pub const MIN_BOUND_N: usize = 30;
pub const DEFAULT_REPLICATES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BootstrapReport {
    pub schema_version: &'static str,
    pub n: usize,
    pub replicates: usize,
    pub block_length: usize,
    pub mean: Option<String>,
    pub lower_bound: Option<String>,
    pub significance: &'static str,
    pub withheld_reason: Option<&'static str>,
}

pub fn stationary_block_bootstrap(
    outcomes: &[Decimal],
    block_length: usize,
    replicates: usize,
    seed: u64,
) -> Result<BootstrapReport, ResearchError> {
    if block_length == 0 || replicates == 0 {
        return Err(ResearchError::Metric {
            field: "bootstrap.policy",
        });
    }
    if outcomes.len() < MIN_BOOTSTRAP_N || block_length > outcomes.len() {
        return Ok(withheld(
            outcomes.len(),
            replicates,
            block_length,
            "insufficient_independent_outcomes",
        ));
    }
    let sample_mean = mean(outcomes)?;
    if outcomes.len() < MIN_BOUND_N {
        return Ok(BootstrapReport {
            schema_version: "hl.research.bootstrap.v1",
            n: outcomes.len(),
            replicates,
            block_length,
            mean: Some(sample_mean.to_string()),
            lower_bound: None,
            significance: "not_claimed",
            withheld_reason: Some("insufficient_independent_outcomes"),
        });
    }

    let mut rng = SplitMix64::new(seed);
    let mut means = Vec::with_capacity(replicates);
    for _ in 0..replicates {
        let replicate = resample_blocks(outcomes, block_length, &mut rng)?;
        means.push(mean(&replicate)?);
    }
    means.sort();
    let index = replicates * 5 / 100;
    let lower = means.get(index).ok_or(ResearchError::Metric {
        field: "bootstrap.percentile",
    })?;
    Ok(BootstrapReport {
        schema_version: "hl.research.bootstrap.v1",
        n: outcomes.len(),
        replicates,
        block_length,
        mean: Some(sample_mean.to_string()),
        lower_bound: Some(lower.to_string()),
        significance: "not_claimed",
        withheld_reason: Some("no_locked_holdout"),
    })
}

fn withheld(
    n: usize,
    replicates: usize,
    block_length: usize,
    reason: &'static str,
) -> BootstrapReport {
    BootstrapReport {
        schema_version: "hl.research.bootstrap.v1",
        n,
        replicates,
        block_length,
        mean: None,
        lower_bound: None,
        significance: "not_claimed",
        withheld_reason: Some(reason),
    }
}

fn resample_blocks(
    outcomes: &[Decimal],
    block_length: usize,
    rng: &mut SplitMix64,
) -> Result<Vec<Decimal>, ResearchError> {
    let starts = outcomes.len() - block_length + 1;
    let mut sample = Vec::with_capacity(outcomes.len());
    while sample.len() < outcomes.len() {
        let start = rng.index(starts)?;
        let take = block_length.min(outcomes.len() - sample.len());
        sample.extend(outcomes[start..start + take].iter().copied());
    }
    Ok(sample)
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9E37_79B9_7F4A_7C15)
    }

    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn index(&mut self, n: usize) -> Result<usize, ResearchError> {
        if n == 0 {
            return Err(ResearchError::Metric {
                field: "bootstrap.rng",
            });
        }
        let bound = u64::try_from(n).map_err(|_| ResearchError::Metric {
            field: "bootstrap.rng",
        })?;
        usize::try_from(self.next_u64() % bound).map_err(|_| ResearchError::Metric {
            field: "bootstrap.rng",
        })
    }
}
