use std::collections::BTreeMap;

use domain_types::{
    BlockHeight, ClosedInterval, Decimal, FeatureSetVersion, Horizon, KnownTime, MarketId,
    ProbabilityPpm, ProtocolTime,
};
use feature_core::{
    EvidenceRef, FeatureKey, FeatureValue, HealthAssessment, HealthState, MissingReason,
};
use serde::{Deserialize, Serialize};

use crate::{MarketError, hash::digest, math::RATIO_SCALE, regime::RegimeAssessment};

/// Presence of a required market input. Missing values must not be treated as zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObservationStatus {
    Observed,
    Missing(MissingReason),
}

impl ObservationStatus {
    #[must_use]
    pub fn from_feature(value: Option<&FeatureValue>) -> Self {
        match value {
            Some(FeatureValue::Missing(reason)) => Self::Missing(*reason),
            None => Self::Missing(MissingReason::NotObserved),
            Some(FeatureValue::Decimal { .. })
            | Some(FeatureValue::SignedInteger(_))
            | Some(FeatureValue::UnsignedInteger(_))
            | Some(FeatureValue::ProbabilityPpm(_))
            | Some(FeatureValue::Category(_))
            | Some(FeatureValue::Boolean(_)) => Self::Observed,
        }
    }

    #[must_use]
    pub const fn is_observed(self) -> bool {
        matches!(self, Self::Observed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DimensionUnit {
    Usd,
    BasisPoints,
    ProbabilityPpm,
    Count,
    Ratio,
    StandardizedScore,
}

impl DimensionUnit {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Usd => "usd",
            Self::BasisPoints => "basis_points",
            Self::ProbabilityPpm => "probability_ppm",
            Self::Count => "count",
            Self::Ratio => "ratio",
            Self::StandardizedScore => "standardized_score",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoredDimension {
    pub raw_value: Decimal,
    pub raw_unit: DimensionUnit,
    pub normalized_value: Decimal,
    pub interval: ClosedInterval<Decimal>,
    pub effective_sample_size_milli: u64,
    pub health: HealthAssessment,
    pub feature_refs: Vec<EvidenceRef>,
}

impl ScoredDimension {
    pub fn try_new(
        raw_value: Decimal,
        raw_unit: DimensionUnit,
        normalized_value: Decimal,
        interval: ClosedInterval<Decimal>,
        effective_sample_size_milli: u64,
        health: HealthAssessment,
        feature_refs: Vec<EvidenceRef>,
    ) -> Result<Self, MarketError> {
        if health.scope.trim().is_empty() {
            return Err(MarketError::EmptyIdentifier {
                field: "health.scope",
            });
        }
        validate_unit(raw_unit, raw_value)?;
        if normalized_value < interval.lower || normalized_value > interval.upper {
            return Err(MarketError::Malformed {
                what: "scored_dimension",
                reason: "normalized value outside interval",
            });
        }
        let analytic = raw_value.to_analytic_float();
        if !analytic.value.is_finite() {
            return Err(MarketError::Malformed {
                what: "scored_dimension",
                reason: "non-finite analytical conversion",
            });
        }
        Ok(Self {
            raw_value,
            raw_unit,
            normalized_value,
            interval,
            effective_sample_size_milli,
            health,
            feature_refs,
        })
    }
}

fn validate_unit(unit: DimensionUnit, value: Decimal) -> Result<(), MarketError> {
    match unit {
        DimensionUnit::Usd | DimensionUnit::Ratio | DimensionUnit::StandardizedScore => Ok(()),
        DimensionUnit::Count if value.raw() < 0 => Err(MarketError::Malformed {
            what: "scored_dimension",
            reason: "count must be non-negative",
        }),
        DimensionUnit::Count => Ok(()),
        DimensionUnit::BasisPoints => Ok(()),
        DimensionUnit::ProbabilityPpm => {
            if value.scale() != 0 || value.raw() < 0 || value.raw() > 1_000_000 {
                Err(MarketError::Malformed {
                    what: "scored_dimension",
                    reason: "probability ppm must be an integer in 0..=1000000",
                })
            } else {
                Ok(())
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketFeatureSnapshot {
    pub market_id: MarketId,
    pub horizon: Horizon,
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub input_watermark: BlockHeight,
    pub values: BTreeMap<FeatureKey, FeatureValue>,
    pub health: HealthAssessment,
    pub provenance_hash: [u8; 32],
}

impl MarketFeatureSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        market_id: MarketId,
        horizon: Horizon,
        feature_set_version: FeatureSetVersion,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        input_watermark: BlockHeight,
        values: BTreeMap<FeatureKey, FeatureValue>,
        health: HealthAssessment,
    ) -> Result<Self, MarketError> {
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(MarketError::Malformed {
                what: "market_feature_snapshot",
                reason: "known_at precedes effective_at",
            });
        }
        if values.is_empty() {
            return Err(MarketError::Malformed {
                what: "market_feature_snapshot",
                reason: "empty values",
            });
        }
        if health.state == HealthState::Red
            && !values
                .values()
                .all(|value| matches!(value, FeatureValue::Missing(_)))
        {
            return Err(MarketError::Malformed {
                what: "market_feature_snapshot",
                reason: "red data health must emit missing values",
            });
        }
        let mut snapshot = Self {
            market_id,
            horizon,
            feature_set_version,
            effective_at,
            known_at,
            input_watermark,
            values,
            health,
            provenance_hash: [0_u8; 32],
        };
        snapshot.provenance_hash = snapshot.compute_provenance_hash();
        Ok(snapshot)
    }

    #[must_use]
    pub fn compute_provenance_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"alpha-desk/market-intelligence/v1");
        hasher.update(self.market_id.as_str().as_bytes());
        hasher.update(&self.horizon.as_micros().to_le_bytes());
        hasher.update(self.feature_set_version.as_str().as_bytes());
        hasher.update(&self.effective_at.unix_micros().to_le_bytes());
        hasher.update(&self.known_at.unix_micros().to_le_bytes());
        hasher.update(&self.input_watermark.get().to_le_bytes());
        hasher.update(self.health.state.as_wire_name().as_bytes());
        hasher.update(self.health.scope.as_bytes());
        hasher.update(self.health.reason_code.as_bytes());
        for (key, value) in &self.values {
            hasher.update(key.namespace.as_bytes());
            hasher.update(&[0]);
            hasher.update(key.name.as_bytes());
            hasher.update(&[0]);
            hasher.update(&key.version.to_le_bytes());
            let encoded = encode_feature_value(value);
            hasher.update(&encoded);
        }
        *hasher.finalize().as_bytes()
    }

    pub fn require_signed(&self, key: &FeatureKey) -> Result<i64, MarketError> {
        match self.values.get(key) {
            Some(FeatureValue::SignedInteger(value)) => Ok(*value),
            Some(FeatureValue::Missing(_)) | None => Err(MarketError::MissingInput {
                name: "snapshot_signed",
            }),
            Some(_) => Err(MarketError::Malformed {
                what: "market_feature_snapshot",
                reason: "unexpected feature value kind",
            }),
        }
    }

    pub fn observation(&self, name: &'static str) -> Result<ObservationStatus, MarketError> {
        let key = market_feature_key(name)?;
        Ok(ObservationStatus::from_feature(self.values.get(&key)))
    }

    pub fn require_observed_book_and_fills(&self) -> Result<(), MarketError> {
        match self.observation("book")? {
            ObservationStatus::Observed => {}
            ObservationStatus::Missing(_) => {
                return Err(MarketError::MissingInput { name: "book" });
            }
        }
        match self.observation("fills")? {
            ObservationStatus::Observed => {}
            ObservationStatus::Missing(_) => {
                return Err(MarketError::MissingInput { name: "fills" });
            }
        }
        Ok(())
    }
}

fn encode_feature_value(value: &FeatureValue) -> Vec<u8> {
    match value {
        FeatureValue::Decimal { raw, scale } => {
            let mut out = vec![0];
            out.extend_from_slice(&raw.to_le_bytes());
            out.extend_from_slice(&scale.to_le_bytes());
            out
        }
        FeatureValue::SignedInteger(raw) => {
            let mut out = vec![1];
            out.extend_from_slice(&raw.to_le_bytes());
            out
        }
        FeatureValue::UnsignedInteger(raw) => {
            let mut out = vec![2];
            out.extend_from_slice(&raw.to_le_bytes());
            out
        }
        FeatureValue::ProbabilityPpm(probability) => {
            let mut out = vec![3];
            out.extend_from_slice(&probability.ppm().to_le_bytes());
            out
        }
        FeatureValue::Category(category) => {
            let mut out = vec![4];
            out.extend_from_slice(category.as_bytes());
            out
        }
        FeatureValue::Boolean(flag) => vec![5, u8::from(*flag)],
        FeatureValue::Missing(reason) => {
            let mut out = vec![6];
            out.extend_from_slice(reason.as_wire_name().as_bytes());
            out
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketSentimentVector {
    pub market_id: MarketId,
    pub horizon: Horizon,
    pub directional_flow: ScoredDimension,
    pub informedness: ScoredDimension,
    pub crowding: ScoredDimension,
    pub consensus_independence: ScoredDimension,
    pub leverage_pressure: ScoredDimension,
    pub liquidation_fragility: ScoredDimension,
    pub liquidity_quality: ScoredDimension,
    pub carry_pressure: ScoredDimension,
    pub positioning_dispersion: ScoredDimension,
    pub regime: RegimeAssessment,
    pub confidence: ProbabilityPpm,
    pub data_freshness: ProbabilityPpm,
    pub as_of_block: BlockHeight,
    pub provenance_hash: [u8; 32],
}

impl MarketSentimentVector {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        market_id: MarketId,
        horizon: Horizon,
        directional_flow: ScoredDimension,
        informedness: ScoredDimension,
        crowding: ScoredDimension,
        consensus_independence: ScoredDimension,
        leverage_pressure: ScoredDimension,
        liquidation_fragility: ScoredDimension,
        liquidity_quality: ScoredDimension,
        carry_pressure: ScoredDimension,
        positioning_dispersion: ScoredDimension,
        regime: RegimeAssessment,
        confidence: ProbabilityPpm,
        data_freshness: ProbabilityPpm,
        as_of_block: BlockHeight,
    ) -> Result<Self, MarketError> {
        let mut vector = Self {
            market_id,
            horizon,
            directional_flow,
            informedness,
            crowding,
            consensus_independence,
            leverage_pressure,
            liquidation_fragility,
            liquidity_quality,
            carry_pressure,
            positioning_dispersion,
            regime,
            confidence,
            data_freshness,
            as_of_block,
            provenance_hash: [0_u8; 32],
        };
        vector.provenance_hash = vector.compute_provenance_hash();
        Ok(vector)
    }

    #[must_use]
    pub fn compute_provenance_hash(&self) -> [u8; 32] {
        digest(&[
            self.market_id.as_str().as_bytes(),
            &self.horizon.as_micros().to_le_bytes(),
            &self.as_of_block.get().to_le_bytes(),
            &self.confidence.ppm().to_le_bytes(),
            &self.data_freshness.ppm().to_le_bytes(),
            self.regime.model_version.as_str().as_bytes(),
            &self.directional_flow.raw_value.raw().to_le_bytes(),
            &self.informedness.raw_value.raw().to_le_bytes(),
            &self.crowding.raw_value.raw().to_le_bytes(),
            &self.consensus_independence.raw_value.raw().to_le_bytes(),
            &self.leverage_pressure.raw_value.raw().to_le_bytes(),
            &self.liquidation_fragility.raw_value.raw().to_le_bytes(),
            &self.liquidity_quality.raw_value.raw().to_le_bytes(),
            &self.carry_pressure.raw_value.raw().to_le_bytes(),
            &self.positioning_dispersion.raw_value.raw().to_le_bytes(),
        ])
    }
}

pub fn market_feature_key(name: impl Into<String>) -> Result<FeatureKey, MarketError> {
    FeatureKey::try_new("market", name, 1).map_err(Into::into)
}

pub fn missing_dimension(
    unit: DimensionUnit,
    health: HealthAssessment,
) -> Result<ScoredDimension, MarketError> {
    let zero = Decimal::from_raw(0, RATIO_SCALE)?;
    ScoredDimension::try_new(
        zero,
        unit,
        zero,
        ClosedInterval::new(zero, zero)?,
        0,
        health,
        Vec::new(),
    )
}
