use serde::{Deserialize, Serialize};

use crate::MarketError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegimeFeatureVector {
    pub trend_milli: i64,
    pub realized_vol_milli: i64,
    pub liquidity_quality_ppm: u32,
    pub funding_bps: i64,
    pub oi_change_milli: i64,
    pub correlation_stress_ppm: u32,
    pub liquidation_intensity_ppm: u32,
}

impl RegimeFeatureVector {
    pub fn try_new(
        trend_milli: i64,
        realized_vol_milli: i64,
        liquidity_quality_ppm: u32,
        funding_bps: i64,
        oi_change_milli: i64,
        correlation_stress_ppm: u32,
        liquidation_intensity_ppm: u32,
    ) -> Result<Self, MarketError> {
        for ppm in [
            liquidity_quality_ppm,
            correlation_stress_ppm,
            liquidation_intensity_ppm,
        ] {
            if ppm > 1_000_000 {
                return Err(MarketError::OutOfRange);
            }
        }
        if realized_vol_milli < 0 {
            return Err(MarketError::Malformed {
                what: "regime_features",
                reason: "realized volatility cannot be negative",
            });
        }
        Ok(Self {
            trend_milli,
            realized_vol_milli,
            liquidity_quality_ppm,
            funding_bps,
            oi_change_milli,
            correlation_stress_ppm,
            liquidation_intensity_ppm,
        })
    }
}
