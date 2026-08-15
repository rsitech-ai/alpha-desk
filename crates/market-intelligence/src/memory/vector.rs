use domain_types::{Horizon, KnownTime, MarketId, ProbabilityPpm, ProtocolTime};
use serde::{Deserialize, Serialize};

use crate::{MarketError, hash::digest};

pub const VECTOR_DIMENSION_COUNT: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VectorManifest {
    pub version: String,
    pub names: Vec<String>,
    pub manifest_hash: [u8; 32],
}

impl VectorManifest {
    pub fn try_new(version: impl Into<String>, names: Vec<String>) -> Result<Self, MarketError> {
        let version = version.into();
        if version.trim().is_empty() {
            return Err(MarketError::EmptyIdentifier {
                field: "vector_manifest.version",
            });
        }
        if names.len() != VECTOR_DIMENSION_COUNT {
            return Err(MarketError::Malformed {
                what: "vector_manifest",
                reason: "dimension count mismatch",
            });
        }
        if names.iter().any(|name| name.trim().is_empty()) {
            return Err(MarketError::EmptyIdentifier {
                field: "vector_manifest.names",
            });
        }
        let joined = names.join("\0");
        let manifest_hash = digest(&[version.as_bytes(), joined.as_bytes()]);
        Ok(Self {
            version,
            names,
            manifest_hash,
        })
    }

    #[must_use]
    pub fn v1() -> Self {
        Self::try_new(
            "market-memory-v1",
            vec![
                "directional_flow_z_milli".into(),
                "informedness_z_milli".into(),
                "crowding_z_milli".into(),
                "consensus_independence_z_milli".into(),
                "leverage_pressure_z_milli".into(),
                "liquidation_fragility_z_milli".into(),
                "liquidity_quality_z_milli".into(),
                "carry_pressure_z_milli".into(),
                "positioning_dispersion_z_milli".into(),
                "regime_quiet_range_milli".into(),
                "regime_volatile_range_milli".into(),
                "regime_orderly_uptrend_milli".into(),
                "regime_orderly_downtrend_milli".into(),
                "regime_leveraged_uptrend_milli".into(),
                "regime_leveraged_downtrend_milli".into(),
                "regime_liquidity_stress_milli".into(),
                "regime_post_liquidation_milli".into(),
                "funding_z_milli".into(),
                "oi_z_milli".into(),
                "cross_asset_stress_z_milli".into(),
            ],
        )
        .expect("static market-memory-v1 manifest")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub market_id: MarketId,
    pub episode_id: String,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub values_milli: [i64; VECTOR_DIMENSION_COUNT],
    pub outcome_bps: Option<i64>,
}

impl MemoryEntry {
    pub fn try_new(
        market_id: MarketId,
        episode_id: impl Into<String>,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        values_milli: [i64; VECTOR_DIMENSION_COUNT],
        outcome_bps: Option<i64>,
    ) -> Result<Self, MarketError> {
        let episode_id = episode_id.into();
        if episode_id.trim().is_empty() {
            return Err(MarketError::EmptyIdentifier {
                field: "episode_id",
            });
        }
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(MarketError::Malformed {
                what: "memory_entry",
                reason: "known_at precedes effective_at",
            });
        }
        Ok(Self {
            market_id,
            episode_id,
            effective_at,
            known_at,
            values_milli,
            outcome_bps,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryQuery {
    pub market_id: MarketId,
    pub episode_id: String,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub horizon: Horizon,
    pub values_milli: [i64; VECTOR_DIMENSION_COUNT],
    pub limit: usize,
    pub support_distance_milli: u128,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MemorySupport {
    InSupport,
    OutsideSupport { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalogueMatch {
    pub episode_id: String,
    pub distance_milli: u128,
    pub contributing_dimensions: Vec<usize>,
    pub outcome_bps: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AnalogueSet {
    pub matches: Vec<AnalogueMatch>,
    pub independent_episode_count: u32,
    pub support: MemorySupport,
    pub historical_support: ProbabilityPpm,
    pub manifest_hash: [u8; 32],
}

pub fn squared_distance(
    left: &[i64; VECTOR_DIMENSION_COUNT],
    right: &[i64; VECTOR_DIMENSION_COUNT],
) -> Result<u128, MarketError> {
    let mut acc = 0_u128;
    for (a, b) in left.iter().zip(right.iter()) {
        let delta = i128::from(*a)
            .checked_sub(i128::from(*b))
            .ok_or(MarketError::Overflow)?;
        let sq = delta.checked_mul(delta).ok_or(MarketError::Overflow)?;
        acc = acc
            .checked_add(u128::try_from(sq).map_err(|_| MarketError::Overflow)?)
            .ok_or(MarketError::Overflow)?;
    }
    Ok(acc)
}

pub fn contributing_dimensions(
    left: &[i64; VECTOR_DIMENSION_COUNT],
    right: &[i64; VECTOR_DIMENSION_COUNT],
) -> Result<Vec<usize>, MarketError> {
    let mut scored: Vec<(u128, usize)> = Vec::new();
    for (index, (a, b)) in left.iter().zip(right.iter()).enumerate() {
        let delta = i128::from(*a)
            .checked_sub(i128::from(*b))
            .ok_or(MarketError::Overflow)?;
        let sq = delta.checked_mul(delta).ok_or(MarketError::Overflow)?;
        scored.push((
            u128::try_from(sq).map_err(|_| MarketError::Overflow)?,
            index,
        ));
    }
    scored.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    Ok(scored.into_iter().take(3).map(|(_, index)| index).collect())
}
