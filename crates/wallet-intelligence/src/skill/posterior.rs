use domain_types::{
    BasisPoints, CalibrationStatus, ClosedInterval, Horizon, KnownTime, MarketId, ProtocolTime,
    RegimeId,
};
use serde::{Deserialize, Serialize};

use crate::{
    Applicability, ApplicabilitySupport, IntelligenceError, IntelligenceSubject,
    math::{integer_sqrt, logistic_ppm},
    skill::{SkillPrior, current_freshness, effective_sample_size_milli},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillEstimate {
    pub posterior_mean_bps: BasisPoints,
    pub credible_interval_bps: ClosedInterval<BasisPoints>,
    pub probability_positive: domain_types::ProbabilityPpm,
    pub effective_sample_size_milli: u64,
    pub freshness: domain_types::ProbabilityPpm,
    pub calibration: CalibrationStatus,
    pub applicability: Applicability,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillVector {
    pub directional: SkillEstimate,
    pub entry_timing: SkillEstimate,
    pub exit_timing: SkillEstimate,
    pub execution: SkillEstimate,
    pub market_making: SkillEstimate,
    pub carry: SkillEstimate,
    pub risk_discipline: SkillEstimate,
    pub consistency: SkillEstimate,
    pub regime_fit: SkillEstimate,
    pub current_relevance: SkillEstimate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillObservation {
    pub markout_bps: i64,
    pub observed_at: ProtocolTime,
    pub market_id: MarketId,
    pub horizon: Horizon,
    pub regime_id: Option<RegimeId>,
    pub segment_id: u32,
}

pub fn estimate_skill(
    subject: &IntelligenceSubject,
    observations: &[SkillObservation],
    prior: &SkillPrior,
    known_at: KnownTime,
    as_of: Option<ProtocolTime>,
    change_point_segment: Option<u32>,
) -> Result<SkillVector, IntelligenceError> {
    let _ = subject;
    let filtered: Vec<&SkillObservation> = observations
        .iter()
        .filter(|observation| as_of.is_none_or(|cutoff| observation.observed_at <= cutoff))
        .filter(|observation| {
            change_point_segment.is_none_or(|segment| observation.segment_id == segment)
        })
        .collect();
    if filtered.is_empty() {
        return Err(IntelligenceError::InsufficientHistory { what: "skill" });
    }
    let directional = estimate_component(&filtered, prior, known_at, "directional")?;
    let execution = estimate_component(&filtered, prior, known_at, "execution")?;
    let mut current_relevance = directional.clone();
    current_relevance.posterior_mean_bps =
        scale_bps_by_ppm(directional.posterior_mean_bps, directional.freshness)?;
    Ok(SkillVector {
        entry_timing: directional.clone(),
        exit_timing: directional.clone(),
        market_making: execution.clone(),
        carry: directional.clone(),
        risk_discipline: execution.clone(),
        consistency: execution.clone(),
        regime_fit: directional.clone(),
        current_relevance,
        directional,
        execution,
    })
}

fn estimate_component(
    observations: &[&SkillObservation],
    prior: &SkillPrior,
    known_at: KnownTime,
    label: &'static str,
) -> Result<SkillEstimate, IntelligenceError> {
    let values: Vec<i64> = observations
        .iter()
        .map(|observation| observation.markout_bps)
        .collect();
    let ess_milli = effective_sample_size_milli(&values)?;
    let mean = values
        .iter()
        .try_fold(0_i128, |acc, value| acc.checked_add(i128::from(*value)))
        .ok_or(IntelligenceError::Overflow)?
        / i128::try_from(values.len()).map_err(|_| IntelligenceError::Overflow)?;
    let kappa_n = i128::from(prior.kappa0_milli)
        .checked_add(i128::from(ess_milli))
        .ok_or(IntelligenceError::Overflow)?;
    let posterior = i128::from(prior.kappa0_milli)
        .checked_mul(i128::from(prior.mu0_bps))
        .and_then(|prior_mass| {
            i128::from(ess_milli)
                .checked_mul(mean)
                .and_then(|data_mass| prior_mass.checked_add(data_mass))
        })
        .and_then(|numerator| numerator.checked_div(kappa_n))
        .ok_or(IntelligenceError::Overflow)?;
    let posterior_mean = BasisPoints::from_raw(posterior, 0)?;
    let residual_ss = values
        .iter()
        .try_fold(0_i128, |acc, value| {
            let delta = i128::from(*value).checked_sub(mean)?;
            acc.checked_add(delta.checked_mul(delta)?)
        })
        .ok_or(IntelligenceError::Overflow)?;
    let std = integer_sqrt(
        u128::try_from((residual_ss / i128::try_from(values.len()).unwrap_or(1)).max(1))
            .map_err(|_| IntelligenceError::Overflow)?,
    );
    let se = if ess_milli == 0 {
        return Err(IntelligenceError::DivisionByZero);
    } else {
        i128::try_from(std).map_err(|_| IntelligenceError::Overflow)? * 1_000
            / i128::try_from(integer_sqrt(u128::from(ess_milli)).max(1))
                .map_err(|_| IntelligenceError::Overflow)?
    };
    let half_width = se.max(1);
    let lower = posterior
        .checked_sub(half_width)
        .ok_or(IntelligenceError::Overflow)?;
    let upper = posterior
        .checked_add(half_width)
        .ok_or(IntelligenceError::Overflow)?;
    let z_milli = if se == 0 {
        if posterior > 0 {
            8_000_i128
        } else {
            -8_000_i128
        }
    } else {
        posterior
            .checked_mul(1_000)
            .and_then(|value| value.checked_div(se))
            .ok_or(IntelligenceError::Overflow)?
    };
    let last = observations
        .iter()
        .map(|observation| observation.observed_at)
        .max_by_key(|time| time.unix_micros())
        .ok_or(IntelligenceError::InsufficientHistory { what: label })?;
    let freshness = current_freshness(last, known_at, prior)?;
    let calibration = prior.calibration(ess_milli);
    let support = if ess_milli < prior.min_ess_milli {
        ApplicabilitySupport::InsufficientEvidence
    } else {
        ApplicabilitySupport::Supported
    };
    let reason_codes = match support {
        ApplicabilitySupport::Supported => Vec::new(),
        ApplicabilitySupport::InsufficientEvidence => vec!["insufficient_ess".to_owned()],
        ApplicabilitySupport::Unsupported => vec!["unsupported".to_owned()],
    };
    Ok(SkillEstimate {
        posterior_mean_bps: posterior_mean,
        credible_interval_bps: ClosedInterval::new(
            BasisPoints::from_raw(lower, 0)?,
            BasisPoints::from_raw(upper.max(lower), 0)?,
        )?,
        probability_positive: logistic_ppm(
            i64::try_from(z_milli).map_err(|_| IntelligenceError::Overflow)?,
        )?,
        effective_sample_size_milli: ess_milli,
        freshness,
        calibration,
        applicability: Applicability::try_new(
            unique_markets(observations),
            unique_horizons(observations),
            unique_regimes(observations),
            support,
            reason_codes,
        )?,
    })
}

fn unique_markets(observations: &[&SkillObservation]) -> Vec<MarketId> {
    let mut markets: Vec<MarketId> = observations
        .iter()
        .map(|observation| observation.market_id.clone())
        .collect();
    markets.sort();
    markets.dedup();
    markets
}

fn unique_horizons(observations: &[&SkillObservation]) -> Vec<Horizon> {
    let mut horizons: Vec<Horizon> = observations
        .iter()
        .map(|observation| observation.horizon)
        .collect();
    horizons.sort_by_key(|horizon| horizon.as_micros());
    horizons.dedup();
    horizons
}

fn unique_regimes(observations: &[&SkillObservation]) -> Vec<RegimeId> {
    let mut regimes: Vec<RegimeId> = observations
        .iter()
        .filter_map(|observation| observation.regime_id.clone())
        .collect();
    regimes.sort();
    regimes.dedup();
    regimes
}

fn scale_bps_by_ppm(
    value: BasisPoints,
    probability: domain_types::ProbabilityPpm,
) -> Result<BasisPoints, IntelligenceError> {
    let scaled = probability.checked_scale_i128_toward_zero(value.raw())?;
    BasisPoints::from_raw(scaled, value.scale()).map_err(Into::into)
}
