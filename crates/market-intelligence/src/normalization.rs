use domain_types::{ProbabilityPpm, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::{MarketError, math::require_matching_usd_scale};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidityNormalizer {
    pub version: String,
    pub volume_weight: ProbabilityPpm,
    pub oi_weight: ProbabilityPpm,
    pub depth_weight: ProbabilityPpm,
    pub recent_volume: UsdAmount,
    pub open_interest: UsdAmount,
    pub executable_depth: UsdAmount,
    pub min_volume: UsdAmount,
    pub min_open_interest: UsdAmount,
    pub min_depth: UsdAmount,
}

impl LiquidityNormalizer {
    pub fn from_toml(text: &str) -> Result<Self, MarketError> {
        let raw: RawNormalizer = toml::from_str(text).map_err(|_| MarketError::Malformed {
            what: "liquidity_normalizer",
            reason: "toml parse failed",
        })?;
        Self::try_from_raw(raw)
    }

    fn try_from_raw(raw: RawNormalizer) -> Result<Self, MarketError> {
        if raw.version.trim().is_empty() {
            return Err(MarketError::EmptyIdentifier {
                field: "normalizer.version",
            });
        }
        let volume_weight = ProbabilityPpm::from_ppm(raw.volume_weight_ppm)?;
        let oi_weight = ProbabilityPpm::from_ppm(raw.oi_weight_ppm)?;
        let depth_weight = ProbabilityPpm::from_ppm(raw.depth_weight_ppm)?;
        let weight_sum = u64::from(volume_weight.ppm())
            + u64::from(oi_weight.ppm())
            + u64::from(depth_weight.ppm());
        if weight_sum != 1_000_000 {
            return Err(MarketError::Malformed {
                what: "liquidity_normalizer",
                reason: "weights must sum to 1000000 ppm",
            });
        }
        let recent_volume = UsdAmount::from_raw(raw.recent_volume_raw, raw.usd_scale)?;
        let open_interest = UsdAmount::from_raw(raw.open_interest_raw, raw.usd_scale)?;
        let executable_depth = UsdAmount::from_raw(raw.executable_depth_raw, raw.usd_scale)?;
        let min_volume = UsdAmount::from_raw(raw.min_volume_raw, raw.usd_scale)?;
        let min_open_interest = UsdAmount::from_raw(raw.min_open_interest_raw, raw.usd_scale)?;
        let min_depth = UsdAmount::from_raw(raw.min_depth_raw, raw.usd_scale)?;
        Ok(Self {
            version: raw.version,
            volume_weight,
            oi_weight,
            depth_weight,
            recent_volume,
            open_interest,
            executable_depth,
            min_volume,
            min_open_interest,
            min_depth,
        })
    }

    pub fn denominator(&self) -> Result<UsdAmount, MarketError> {
        require_matching_usd_scale(self.recent_volume, self.open_interest)?;
        require_matching_usd_scale(self.recent_volume, self.executable_depth)?;
        let volume = self.recent_volume.raw().max(self.min_volume.raw());
        let oi = self.open_interest.raw().max(self.min_open_interest.raw());
        let depth = self.executable_depth.raw().max(self.min_depth.raw());
        let weighted = volume
            .checked_mul(i128::from(self.volume_weight.ppm()))
            .and_then(|value| value.checked_add(oi.checked_mul(i128::from(self.oi_weight.ppm()))?))
            .and_then(|value| {
                value.checked_add(depth.checked_mul(i128::from(self.depth_weight.ppm()))?)
            })
            .and_then(|value| value.checked_div(1_000_000))
            .ok_or(MarketError::Overflow)?;
        if weighted <= 0 {
            return Err(MarketError::EmptyDenominator);
        }
        UsdAmount::from_raw(weighted, self.recent_volume.scale()).map_err(Into::into)
    }

    pub fn normalize(&self, raw: UsdAmount) -> Result<UsdAmount, MarketError> {
        require_matching_usd_scale(raw, self.recent_volume)?;
        let denom = self.denominator()?;
        let scaled = raw
            .raw()
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_div(denom.raw()))
            .ok_or(MarketError::Overflow)?;
        UsdAmount::from_raw(scaled, raw.scale()).map_err(Into::into)
    }
}

#[derive(Debug, Deserialize)]
struct RawNormalizer {
    version: String,
    volume_weight_ppm: u32,
    oi_weight_ppm: u32,
    depth_weight_ppm: u32,
    usd_scale: u8,
    recent_volume_raw: i128,
    open_interest_raw: i128,
    executable_depth_raw: i128,
    min_volume_raw: i128,
    min_open_interest_raw: i128,
    min_depth_raw: i128,
}
