use domain_types::{Direction, EntityId, MarketId};
use serde::{Deserialize, Serialize};

use crate::{SignalError, signal::SignalType};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndependenceClass {
    Independent,
    FollowerSaturated,
}

impl IndependenceClass {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Independent => "independent",
            Self::FollowerSaturated => "follower_saturated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DedupKey {
    pub market_id: MarketId,
    pub family: String,
    pub originator_hash: [u8; 32],
    pub direction: Direction,
    pub independence_class: IndependenceClass,
}

pub fn originator_hash(originators: &[EntityId]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    let mut ordered: Vec<_> = originators.iter().map(EntityId::as_str).collect();
    ordered.sort_unstable();
    ordered.dedup();
    for id in ordered {
        hasher.update(id.as_bytes());
        hasher.update(&[0]);
    }
    *hasher.finalize().as_bytes()
}

pub fn dedup_key(
    market_id: MarketId,
    signal_type: &SignalType,
    originators: &[EntityId],
    direction: Direction,
    independence_class: IndependenceClass,
) -> DedupKey {
    DedupKey {
        market_id,
        family: signal_type.as_wire_name().to_owned(),
        originator_hash: originator_hash(originators),
        direction,
        independence_class,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialChange {
    pub net_edge_delta_bps: i64,
    pub confidence_delta_ppm: i64,
    pub crowding_delta_ppm: i64,
}

impl MaterialChange {
    pub fn from_toml(text: &str) -> Result<Self, SignalError> {
        toml::from_str(text).map_err(|_| SignalError::ContractViolation("material change toml"))
    }

    #[must_use]
    pub fn is_material(&self, thresholds: &Self) -> bool {
        self.net_edge_delta_bps.abs() >= thresholds.net_edge_delta_bps
            || self.confidence_delta_ppm.abs() >= thresholds.confidence_delta_ppm
            || self.crowding_delta_ppm.abs() >= thresholds.crowding_delta_ppm
    }
}
