use domain_types::{
    BlockHeight, ClosedInterval, CohortId, Decimal, Direction, EntityId, Horizon, KnownTime,
    ProbabilityPpm, ProtocolTime, UsdAmount,
};
use feature_core::HealthAssessment;
use serde::{Deserialize, Serialize};

use crate::{
    MarketError,
    cohort::{CohortDefinition, CohortMember, select_members},
    hash::digest,
    math::{COUNT_SCALE, USD_SCALE, ratio, require_matching_usd_scale},
    sentiment::{DimensionUnit, ScoredDimension},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RatioMeasure {
    IndependentEntityCount,
    GrossExposure,
    NewRiskFlow,
    HighConvictionFlow,
    LiquidationWeightedExposure,
    TakerOpeningFlow,
    SmartCrowdDivergence,
}

impl RatioMeasure {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::IndependentEntityCount => "independent_entity_count",
            Self::GrossExposure => "gross_exposure",
            Self::NewRiskFlow => "new_risk_flow",
            Self::HighConvictionFlow => "high_conviction_flow",
            Self::LiquidationWeightedExposure => "liquidation_weighted_exposure",
            Self::TakerOpeningFlow => "taker_opening_flow",
            Self::SmartCrowdDivergence => "smart_crowd_divergence",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RatioUnit {
    Count,
    Usd,
    ProbabilityPpm,
    Dimensionless,
}

impl RatioUnit {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Usd => "usd",
            Self::ProbabilityPpm => "probability_ppm",
            Self::Dimensionless => "dimensionless",
        }
    }

    #[must_use]
    pub const fn dimension_unit(self) -> DimensionUnit {
        match self {
            Self::Count => DimensionUnit::Count,
            Self::Usd => DimensionUnit::Usd,
            Self::ProbabilityPpm => DimensionUnit::ProbabilityPpm,
            Self::Dimensionless => DimensionUnit::Ratio,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatioScope {
    pub numerator_cohort_id: CohortId,
    pub denominator_cohort_id: CohortId,
    pub measure: RatioMeasure,
    pub unit: RatioUnit,
    pub horizon: Horizon,
    pub exclusions: Vec<String>,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub as_of_block: BlockHeight,
}

impl RatioScope {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        numerator_cohort_id: CohortId,
        denominator_cohort_id: CohortId,
        measure: RatioMeasure,
        unit: RatioUnit,
        horizon: Horizon,
        exclusions: Vec<String>,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        as_of_block: BlockHeight,
    ) -> Result<Self, MarketError> {
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(MarketError::Malformed {
                what: "ratio_scope",
                reason: "known_at precedes effective_at",
            });
        }
        Ok(Self {
            numerator_cohort_id,
            denominator_cohort_id,
            measure,
            unit,
            horizon,
            exclusions,
            effective_at,
            known_at,
            as_of_block,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionedMember {
    pub member: CohortMember,
    pub side: Direction,
    pub gross_exposure: UsdAmount,
    pub new_risk_flow: UsdAmount,
    pub high_conviction_flow: UsdAmount,
    pub liquidation_weighted_exposure: UsdAmount,
    pub taker_opening_flow: UsdAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RatioResult {
    pub scope: RatioScope,
    pub numerator: Decimal,
    pub denominator: Decimal,
    pub value: Decimal,
    pub effective_sample_size_milli: u64,
    pub confidence: ProbabilityPpm,
    pub health: HealthAssessment,
    pub provenance_hash: [u8; 32],
}

impl RatioResult {
    pub fn as_dimension(&self) -> Result<ScoredDimension, MarketError> {
        ScoredDimension::try_new(
            self.value,
            self.scope.unit.dimension_unit(),
            self.value,
            ClosedInterval::new(self.value, self.value)?,
            self.effective_sample_size_milli,
            self.health.clone(),
            Vec::new(),
        )
    }
}

pub fn compute_ratio(
    scope: RatioScope,
    numerator_definition: &CohortDefinition,
    denominator_definition: &CohortDefinition,
    universe: &[PositionedMember],
    health: HealthAssessment,
) -> Result<RatioResult, MarketError> {
    if numerator_definition.cohort_id != scope.numerator_cohort_id
        || denominator_definition.cohort_id != scope.denominator_cohort_id
    {
        return Err(MarketError::Malformed {
            what: "ratio",
            reason: "cohort definition does not match scope",
        });
    }
    if health.state == feature_core::HealthState::Red {
        return Err(MarketError::RedDataHealth { what: "ratio" });
    }
    let members: Vec<CohortMember> = universe.iter().map(|row| row.member.clone()).collect();
    let numerator_ids = selected_ids(
        numerator_definition,
        &members,
        scope.effective_at,
        scope.known_at,
        scope.as_of_block,
        &scope.exclusions,
    )?;
    let denominator_ids = selected_ids(
        denominator_definition,
        &members,
        scope.effective_at,
        scope.known_at,
        scope.as_of_block,
        &scope.exclusions,
    )?;
    let (numerator, numerator_ess) = measure_cohort(scope.measure, universe, &numerator_ids)?;
    let (denominator, denominator_ess) = measure_cohort(scope.measure, universe, &denominator_ids)?;
    if denominator.raw() == 0 {
        return Err(MarketError::EmptyDenominator);
    }
    let value = match scope.measure {
        RatioMeasure::SmartCrowdDivergence => numerator
            .checked_sub(denominator)
            .map_err(MarketError::from)?,
        RatioMeasure::IndependentEntityCount
        | RatioMeasure::GrossExposure
        | RatioMeasure::NewRiskFlow
        | RatioMeasure::HighConvictionFlow
        | RatioMeasure::LiquidationWeightedExposure
        | RatioMeasure::TakerOpeningFlow => ratio(numerator, denominator)?,
    };
    let ess = numerator_ess.saturating_add(denominator_ess) / 2;
    let confidence = confidence_from_ess(ess)?;
    let provenance_hash = digest(&[
        scope.numerator_cohort_id.as_str().as_bytes(),
        scope.denominator_cohort_id.as_str().as_bytes(),
        scope.measure.as_wire_name().as_bytes(),
        &scope.as_of_block.get().to_le_bytes(),
        &numerator.raw().to_le_bytes(),
        &denominator.raw().to_le_bytes(),
        numerator_definition.definition_hash.as_slice(),
        denominator_definition.definition_hash.as_slice(),
    ]);
    Ok(RatioResult {
        scope,
        numerator,
        denominator,
        value,
        effective_sample_size_milli: ess,
        confidence,
        health,
        provenance_hash,
    })
}

fn selected_ids(
    definition: &CohortDefinition,
    members: &[CohortMember],
    effective_at: ProtocolTime,
    known_at: KnownTime,
    as_of_block: BlockHeight,
    extra_exclusions: &[String],
) -> Result<Vec<EntityId>, MarketError> {
    Ok(
        select_members(definition, members, effective_at, known_at, as_of_block)?
            .into_iter()
            .filter(|member| {
                extra_exclusions
                    .iter()
                    .all(|exclusion| exclusion != member.entity_id.as_str())
            })
            .map(|member| member.entity_id.clone())
            .collect(),
    )
}

fn measure_cohort(
    measure: RatioMeasure,
    universe: &[PositionedMember],
    selected: &[EntityId],
) -> Result<(Decimal, u64), MarketError> {
    let mut ess = 0_u64;
    let mut acc = 0_i128;
    let mut scale = USD_SCALE;
    for row in universe {
        if !selected.iter().any(|id| id == &row.member.entity_id) {
            continue;
        }
        ess = ess
            .checked_add(u64::from(row.member.independence_weight.ppm()))
            .ok_or(MarketError::Overflow)?;
        match measure {
            RatioMeasure::IndependentEntityCount => {
                acc = acc
                    .checked_add(i128::from(row.member.independence_weight.ppm()))
                    .ok_or(MarketError::Overflow)?;
                scale = COUNT_SCALE;
            }
            RatioMeasure::GrossExposure => {
                require_matching_usd_scale(
                    row.gross_exposure,
                    UsdAmount::from_raw(0, row.gross_exposure.scale())?,
                )?;
                if row.gross_exposure.raw() < 0 {
                    return Err(MarketError::Malformed {
                        what: "gross_exposure",
                        reason: "must be non-negative",
                    });
                }
                acc = acc
                    .checked_add(row.gross_exposure.raw())
                    .ok_or(MarketError::Overflow)?;
                scale = row.gross_exposure.scale();
            }
            RatioMeasure::NewRiskFlow | RatioMeasure::SmartCrowdDivergence => {
                acc = acc
                    .checked_add(row.new_risk_flow.raw())
                    .ok_or(MarketError::Overflow)?;
                scale = row.new_risk_flow.scale();
            }
            RatioMeasure::HighConvictionFlow => {
                acc = acc
                    .checked_add(row.high_conviction_flow.raw())
                    .ok_or(MarketError::Overflow)?;
                scale = row.high_conviction_flow.scale();
            }
            RatioMeasure::LiquidationWeightedExposure => {
                acc = acc
                    .checked_add(row.liquidation_weighted_exposure.raw())
                    .ok_or(MarketError::Overflow)?;
                scale = row.liquidation_weighted_exposure.scale();
            }
            RatioMeasure::TakerOpeningFlow => {
                acc = acc
                    .checked_add(row.taker_opening_flow.raw())
                    .ok_or(MarketError::Overflow)?;
                scale = row.taker_opening_flow.scale();
            }
        }
    }
    if matches!(measure, RatioMeasure::IndependentEntityCount) {
        Decimal::from_raw(acc, COUNT_SCALE)
            .map_err(Into::into)
            .map(|value| (value, ess))
    } else {
        Decimal::from_raw(acc, scale)
            .map_err(Into::into)
            .map(|value| (value, ess))
    }
}

fn confidence_from_ess(ess_milli: u64) -> Result<ProbabilityPpm, MarketError> {
    let ppm = ess_milli.min(1_000_000);
    ProbabilityPpm::from_ppm(u32::try_from(ppm).map_err(|_| MarketError::Overflow)?)
        .map_err(Into::into)
}
