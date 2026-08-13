use domain_types::{
    BasisPoints, BlockHeight, EventId, FeatureSetVersion, KnownTime, LatencyDistribution,
    ProbabilityPpm, ProtocolTime, UsdAmount,
};
use feature_core::{HealthAssessment, HealthState};
use serde::{Deserialize, Serialize};

use crate::{IntelligenceError, IntelligenceSubject, SkillVector, math::freshness_ppm};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CopyabilityClass {
    NotCopyable,
    LatencySensitive,
    CapacityLimited,
    ResearchOnly,
    Actionable,
}

impl CopyabilityClass {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::NotCopyable => "not_copyable",
            Self::LatencySensitive => "latency_sensitive",
            Self::CapacityLimited => "capacity_limited",
            Self::ResearchOnly => "research_only",
            Self::Actionable => "actionable",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioContextSummary {
    pub gross_exposure: UsdAmount,
    pub net_exposure: UsdAmount,
    pub same_market_exposure: UsdAmount,
    pub same_entity_exposure: UsdAmount,
    pub correlated_exposure: UsdAmount,
    pub snapshot_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyabilityRequest {
    pub subject: IntelligenceSubject,
    pub action_id: EventId,
    pub detection_latency: LatencyDistribution,
    pub bankroll: UsdAmount,
    pub max_participation: ProbabilityPpm,
    pub fee_schedule_id: domain_types::FeeScheduleId,
    pub portfolio_context: PortfolioContextSummary,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkoutHorizon {
    pub latency_micros: u64,
    pub net_return_bps: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyabilityInputs {
    pub request: CopyabilityRequest,
    pub markouts: Vec<MarkoutHorizon>,
    pub half_life_micros: u64,
    pub book_health: HealthAssessment,
    pub executable_depth: UsdAmount,
    pub fee_bps: i64,
    pub cost_threshold_bps: i64,
    pub impact_bps_per_participation_ppm: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyabilitySummary {
    pub class: CopyabilityClass,
    pub p10_net_return_bps: BasisPoints,
    pub p50_net_return_bps: BasisPoints,
    pub p90_net_return_bps: BasisPoints,
    pub fill_probability: ProbabilityPpm,
    pub alpha_remaining: ProbabilityPpm,
    pub assumptions_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapacitySummary {
    pub maximum_notional: UsdAmount,
    pub cost_threshold_bps: BasisPoints,
    pub stressed_maximum_notional: UsdAmount,
    pub book_as_of_block: BlockHeight,
    pub health: HealthAssessment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletIntelligenceVector {
    pub statistical_skill: SkillVector,
    pub copyability: CopyabilitySummary,
    pub capacity: CapacitySummary,
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub input_watermark: BlockHeight,
}

pub fn estimate_copyability(
    inputs: &CopyabilityInputs,
) -> Result<(CopyabilitySummary, CapacitySummary), IntelligenceError> {
    if inputs.book_health.state == HealthState::Red {
        return Err(IntelligenceError::RedDataHealth {
            what: "copyability",
        });
    }
    if inputs.markouts.len() < 2 || inputs.half_life_micros == 0 {
        return research_only(inputs, vec!["sparse_half_life".to_owned()]);
    }
    if inputs.fee_bps < 0 {
        return Err(IntelligenceError::Malformed {
            what: "copyability",
            reason: "fees must be non-negative",
        });
    }
    let p10 = interpolate(
        &inputs.markouts,
        inputs.request.detection_latency.p10_micros,
    )?;
    let p50 = interpolate(
        &inputs.markouts,
        inputs.request.detection_latency.p50_micros,
    )?;
    let p90 = interpolate(
        &inputs.markouts,
        inputs.request.detection_latency.p90_micros,
    )?;
    let alpha_remaining = freshness_ppm(
        inputs.request.detection_latency.p50_micros,
        inputs.half_life_micros,
    )?;
    let participation = inputs.request.max_participation.ppm();
    let size_ppm = if inputs.executable_depth.raw() <= 0 {
        return Err(IntelligenceError::DivisionByZero);
    } else {
        inputs
            .request
            .bankroll
            .raw()
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_div(inputs.executable_depth.raw()))
            .ok_or(IntelligenceError::Overflow)?
            .min(i128::from(participation))
    };
    let impact = inputs
        .impact_bps_per_participation_ppm
        .checked_mul(i64::try_from(size_ppm).map_err(|_| IntelligenceError::Overflow)?)
        .and_then(|value| value.checked_div(1_000_000))
        .ok_or(IntelligenceError::Overflow)?;
    let p50_after_cost = p50
        .checked_sub(inputs.fee_bps)
        .and_then(|value| value.checked_sub(impact))
        .ok_or(IntelligenceError::Overflow)?;
    let fill = fill_probability(
        inputs.request.bankroll,
        inputs.executable_depth,
        participation,
    )?;
    let capacity = capacity_curve(inputs)?;
    let class = classify(
        p10,
        p50_after_cost,
        p90,
        impact,
        inputs.cost_threshold_bps,
        fill,
        &inputs.markouts,
        inputs.request.detection_latency.p50_micros,
    );
    let mut hasher = blake3::Hasher::new();
    hasher.update(&inputs.request.detection_latency.p50_micros.to_le_bytes());
    hasher.update(&inputs.request.bankroll.raw().to_le_bytes());
    hasher.update(&inputs.fee_bps.to_le_bytes());
    hasher.update(&inputs.half_life_micros.to_le_bytes());
    hasher.update(inputs.request.fee_schedule_id.as_str().as_bytes());
    hasher.update(&inputs.request.portfolio_context.snapshot_hash);
    Ok((
        CopyabilitySummary {
            class,
            p10_net_return_bps: BasisPoints::from_raw(i128::from(p10), 0)?,
            p50_net_return_bps: BasisPoints::from_raw(i128::from(p50_after_cost), 0)?,
            p90_net_return_bps: BasisPoints::from_raw(i128::from(p90), 0)?,
            fill_probability: fill,
            alpha_remaining,
            assumptions_hash: *hasher.finalize().as_bytes(),
        },
        capacity,
    ))
}

fn research_only(
    inputs: &CopyabilityInputs,
    _reasons: Vec<String>,
) -> Result<(CopyabilitySummary, CapacitySummary), IntelligenceError> {
    Ok((
        CopyabilitySummary {
            class: CopyabilityClass::ResearchOnly,
            p10_net_return_bps: BasisPoints::from_raw(0, 0)?,
            p50_net_return_bps: BasisPoints::from_raw(0, 0)?,
            p90_net_return_bps: BasisPoints::from_raw(0, 0)?,
            fill_probability: ProbabilityPpm::from_ppm(0)?,
            alpha_remaining: ProbabilityPpm::from_ppm(0)?,
            assumptions_hash: [1_u8; 32],
        },
        CapacitySummary {
            maximum_notional: UsdAmount::from_raw(0, inputs.request.bankroll.scale())?,
            cost_threshold_bps: BasisPoints::from_raw(i128::from(inputs.cost_threshold_bps), 0)?,
            stressed_maximum_notional: UsdAmount::from_raw(0, inputs.request.bankroll.scale())?,
            book_as_of_block: BlockHeight::new(0),
            health: inputs.book_health.clone(),
        },
    ))
}

fn interpolate(points: &[MarkoutHorizon], latency_micros: u64) -> Result<i64, IntelligenceError> {
    let mut ordered = points.to_vec();
    ordered.sort_by_key(|point| point.latency_micros);
    if ordered
        .windows(2)
        .any(|pair| pair[0].latency_micros == pair[1].latency_micros)
    {
        return Err(IntelligenceError::Malformed {
            what: "copyability",
            reason: "duplicate markout horizons",
        });
    }
    if latency_micros <= ordered[0].latency_micros {
        return Ok(ordered[0].net_return_bps);
    }
    if latency_micros >= ordered[ordered.len() - 1].latency_micros {
        return Ok(ordered[ordered.len() - 1].net_return_bps);
    }
    for pair in ordered.windows(2) {
        if latency_micros >= pair[0].latency_micros && latency_micros <= pair[1].latency_micros {
            let span = pair[1]
                .latency_micros
                .checked_sub(pair[0].latency_micros)
                .ok_or(IntelligenceError::Overflow)?;
            let offset = latency_micros
                .checked_sub(pair[0].latency_micros)
                .ok_or(IntelligenceError::Overflow)?;
            let delta = pair[1]
                .net_return_bps
                .checked_sub(pair[0].net_return_bps)
                .ok_or(IntelligenceError::Overflow)?;
            let adj = i128::from(delta)
                .checked_mul(i128::from(offset))
                .and_then(|value| value.checked_div(i128::from(span)))
                .ok_or(IntelligenceError::Overflow)?;
            return i64::try_from(i128::from(pair[0].net_return_bps) + adj)
                .map_err(|_| IntelligenceError::Overflow);
        }
    }
    Err(IntelligenceError::Unsupported {
        what: "copyability_interpolation",
    })
}

fn fill_probability(
    bankroll: UsdAmount,
    depth: UsdAmount,
    participation_ppm: u32,
) -> Result<ProbabilityPpm, IntelligenceError> {
    if depth.raw() <= 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    let requested = bankroll
        .raw()
        .checked_mul(i128::from(participation_ppm))
        .and_then(|value| value.checked_div(1_000_000))
        .ok_or(IntelligenceError::Overflow)?;
    let ppm = 1_000_000_i128
        .checked_sub(
            requested
                .checked_mul(1_000_000)
                .and_then(|value| value.checked_div(depth.raw()))
                .ok_or(IntelligenceError::Overflow)?
                .min(1_000_000),
        )
        .ok_or(IntelligenceError::Overflow)?;
    ProbabilityPpm::from_ppm(u32::try_from(ppm.max(0)).map_err(|_| IntelligenceError::Overflow)?)
        .map_err(Into::into)
}

fn capacity_curve(inputs: &CopyabilityInputs) -> Result<CapacitySummary, IntelligenceError> {
    if inputs.impact_bps_per_participation_ppm <= 0 || inputs.cost_threshold_bps <= 0 {
        return Err(IntelligenceError::Malformed {
            what: "capacity",
            reason: "impact and cost threshold must be positive",
        });
    }
    let max_participation = inputs
        .cost_threshold_bps
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_div(inputs.impact_bps_per_participation_ppm))
        .ok_or(IntelligenceError::Overflow)?
        .clamp(0, 1_000_000);
    let maximum = inputs
        .executable_depth
        .raw()
        .checked_mul(i128::from(max_participation))
        .and_then(|value| value.checked_div(1_000_000))
        .ok_or(IntelligenceError::Overflow)?;
    let stressed = maximum
        .checked_div(2)
        .ok_or(IntelligenceError::DivisionByZero)?;
    Ok(CapacitySummary {
        maximum_notional: UsdAmount::from_raw(maximum, inputs.executable_depth.scale())?,
        cost_threshold_bps: BasisPoints::from_raw(i128::from(inputs.cost_threshold_bps), 0)?,
        stressed_maximum_notional: UsdAmount::from_raw(stressed, inputs.executable_depth.scale())?,
        book_as_of_block: BlockHeight::new(0),
        health: inputs.book_health.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
fn classify(
    p10: i64,
    p50: i64,
    p90: i64,
    impact: i64,
    cost_threshold: i64,
    fill: ProbabilityPpm,
    markouts: &[MarkoutHorizon],
    latency_micros: u64,
) -> CopyabilityClass {
    if p50 <= 0 || p90 <= 0 {
        return CopyabilityClass::NotCopyable;
    }
    if impact >= cost_threshold || fill.ppm() < 250_000 {
        return CopyabilityClass::CapacityLimited;
    }
    let later_negative = markouts
        .iter()
        .any(|point| point.latency_micros > latency_micros && point.net_return_bps <= 0);
    if later_negative || (p10 <= 0 && p50 > 0) || p90.saturating_sub(p10) >= 2 * p50.max(1) {
        return CopyabilityClass::LatencySensitive;
    }
    CopyabilityClass::Actionable
}
