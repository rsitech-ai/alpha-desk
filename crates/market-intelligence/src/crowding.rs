use domain_types::{ClosedInterval, Decimal, EntityId, ProbabilityPpm, UsdAmount};
use feature_core::HealthAssessment;
use serde::{Deserialize, Serialize};

use crate::{
    MarketError,
    math::{COUNT_SCALE, USD_SCALE},
    sentiment::{DimensionUnit, MarketFeatureSnapshot, ObservedBookAndFills, ScoredDimension},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrowdingPosition {
    pub entity_id: EntityId,
    pub independence_weight: ProbabilityPpm,
    pub is_follower: bool,
    pub post_originator: bool,
    pub exposure: UsdAmount,
    pub entry_bps_from_mark: i64,
    pub funding_percentile: ProbabilityPpm,
    pub leverage_milli: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrowdingComponents {
    pub independent_entity_count: ScoredDimension,
    pub exposure_concentration: ScoredDimension,
    pub follower_saturation: ScoredDimension,
    pub post_originator_flow_share: ScoredDimension,
    pub entry_clustering: ScoredDimension,
    pub funding_percentile: ScoredDimension,
    pub leverage_concentration: ScoredDimension,
    pub capacity_consumed: ScoredDimension,
}

pub fn crowding_components_from_snapshot(
    snapshot: &MarketFeatureSnapshot,
    positions: &[CrowdingPosition],
    remaining_capacity: UsdAmount,
) -> Result<CrowdingComponents, MarketError> {
    crowding_components(
        positions,
        remaining_capacity,
        snapshot.require_observed_book_and_fills()?,
    )
}

/// Crowding components from caller marks. Marks are admitted only with
/// [`ObservedBookAndFills`] issued after book and fills are observed.
pub fn crowding_components(
    positions: &[CrowdingPosition],
    remaining_capacity: UsdAmount,
    evidence: ObservedBookAndFills<'_>,
) -> Result<CrowdingComponents, MarketError> {
    let health = evidence.health();
    if positions.is_empty() {
        return Err(MarketError::InsufficientHistory { what: "crowding" });
    }
    if remaining_capacity.raw() < 0 {
        return Err(MarketError::Malformed {
            what: "crowding",
            reason: "capacity must be non-negative",
        });
    }
    let mut independent = 0_u64;
    let mut follower = 0_u64;
    let mut post = 0_u64;
    let mut exposure = 0_i128;
    let mut post_exposure = 0_i128;
    let mut leverage_acc = 0_u128;
    let mut funding_acc = 0_u128;
    let mut unique_bins = std::collections::BTreeSet::new();
    let scale = positions[0].exposure.scale();
    for position in positions {
        if position.exposure.scale() != scale {
            return Err(MarketError::ScaleMismatch);
        }
        if position.exposure.raw() < 0 {
            return Err(MarketError::Malformed {
                what: "crowding",
                reason: "exposure must be non-negative",
            });
        }
        independent = independent
            .checked_add(u64::from(position.independence_weight.ppm()))
            .ok_or(MarketError::Overflow)?;
        if position.is_follower {
            follower = follower
                .checked_add(u64::from(position.independence_weight.ppm()))
                .ok_or(MarketError::Overflow)?;
        }
        if position.post_originator {
            post = post
                .checked_add(u64::from(position.independence_weight.ppm()))
                .ok_or(MarketError::Overflow)?;
            post_exposure = post_exposure
                .checked_add(position.exposure.raw())
                .ok_or(MarketError::Overflow)?;
        }
        exposure = exposure
            .checked_add(position.exposure.raw())
            .ok_or(MarketError::Overflow)?;
        leverage_acc = leverage_acc
            .checked_add(u128::from(position.leverage_milli))
            .ok_or(MarketError::Overflow)?;
        funding_acc = funding_acc
            .checked_add(u128::from(position.funding_percentile.ppm()))
            .ok_or(MarketError::Overflow)?;
        unique_bins.insert(position.entry_bps_from_mark.div_euclid(25));
    }
    let count = u128::try_from(positions.len()).map_err(|_| MarketError::Overflow)?;
    let hhi = herfindahl(positions)?;
    let saturation = if independent == 0 {
        0
    } else {
        follower
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_div(independent))
            .ok_or(MarketError::Overflow)?
    };
    let post_share = if exposure == 0 {
        0
    } else {
        u64::try_from(
            post_exposure
                .checked_mul(1_000_000)
                .and_then(|value| value.checked_div(exposure))
                .ok_or(MarketError::Overflow)?,
        )
        .map_err(|_| MarketError::Overflow)?
    };
    let clustering = 1_000_000_u64
        .checked_div(u64::try_from(unique_bins.len().max(1)).map_err(|_| MarketError::Overflow)?)
        .ok_or(MarketError::Overflow)?;
    let funding = u64::try_from(funding_acc / count).map_err(|_| MarketError::Overflow)?;
    let leverage = u64::try_from(leverage_acc / count).map_err(|_| MarketError::Overflow)?;
    let consumed = if remaining_capacity.raw() == 0 && exposure == 0 {
        0
    } else {
        let total = exposure
            .checked_add(remaining_capacity.raw())
            .ok_or(MarketError::Overflow)?;
        if total == 0 {
            0
        } else {
            u64::try_from(
                exposure
                    .checked_mul(1_000_000)
                    .and_then(|value| value.checked_div(total))
                    .ok_or(MarketError::Overflow)?,
            )
            .map_err(|_| MarketError::Overflow)?
        }
    };
    Ok(CrowdingComponents {
        independent_entity_count: ppm_dimension(
            Decimal::from_raw(i128::from(independent), COUNT_SCALE)?,
            DimensionUnit::Count,
            independent,
            health,
        )?,
        exposure_concentration: ppm_dimension(
            Decimal::from_raw(i128::from(hhi), 0)?,
            DimensionUnit::ProbabilityPpm,
            independent,
            health,
        )?,
        follower_saturation: ppm_dimension(
            Decimal::from_raw(i128::from(saturation), 0)?,
            DimensionUnit::ProbabilityPpm,
            independent,
            health,
        )?,
        post_originator_flow_share: ppm_dimension(
            Decimal::from_raw(i128::from(post_share), 0)?,
            DimensionUnit::ProbabilityPpm,
            post,
            health,
        )?,
        entry_clustering: ppm_dimension(
            Decimal::from_raw(i128::from(clustering), 0)?,
            DimensionUnit::ProbabilityPpm,
            independent,
            health,
        )?,
        funding_percentile: ppm_dimension(
            Decimal::from_raw(i128::from(funding), 0)?,
            DimensionUnit::ProbabilityPpm,
            independent,
            health,
        )?,
        leverage_concentration: ppm_dimension(
            Decimal::from_raw(i128::from(leverage.min(1_000_000)), 0)?,
            DimensionUnit::ProbabilityPpm,
            independent,
            health,
        )?,
        capacity_consumed: ppm_dimension(
            Decimal::from_raw(i128::from(consumed), 0)?,
            DimensionUnit::ProbabilityPpm,
            independent,
            health,
        )?,
    })
}

fn herfindahl(positions: &[CrowdingPosition]) -> Result<u32, MarketError> {
    let total: i128 = positions
        .iter()
        .map(|position| position.exposure.raw())
        .sum();
    if total == 0 {
        return Ok(0);
    }
    let mut acc = 0_i128;
    for position in positions {
        let share = position
            .exposure
            .raw()
            .checked_mul(1_000)
            .and_then(|value| value.checked_div(total))
            .ok_or(MarketError::Overflow)?;
        acc = acc
            .checked_add(share.checked_mul(share).ok_or(MarketError::Overflow)?)
            .ok_or(MarketError::Overflow)?;
    }
    u32::try_from(acc.min(1_000_000)).map_err(|_| MarketError::Overflow)
}

fn ppm_dimension(
    raw: Decimal,
    unit: DimensionUnit,
    ess: u64,
    health: &HealthAssessment,
) -> Result<ScoredDimension, MarketError> {
    let _ = USD_SCALE;
    ScoredDimension::try_new(
        raw,
        unit,
        raw,
        ClosedInterval::new(raw, raw)?,
        ess,
        health.clone(),
        Vec::new(),
    )
}
