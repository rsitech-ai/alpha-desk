use domain_types::{EntityId, MarketId, ProbabilityPpm, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::MarketError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossAssetInputs {
    pub entity_id: EntityId,
    pub from_market: MarketId,
    pub to_market: MarketId,
    pub rotated_notional: UsdAmount,
    pub simultaneous_deleveraging: bool,
    pub lead_lag_micros: i64,
    pub shared_collateral: bool,
    pub beta_neutral_notional: UsdAmount,
    pub correlation_stress_ppm: ProbabilityPpm,
    pub gross_risk: UsdAmount,
    pub net_risk: UsdAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossAssetFeatures {
    pub rotation_notional: UsdAmount,
    pub simultaneous_deleveraging: bool,
    pub lead_lag_micros: i64,
    pub shared_collateral_contagion: bool,
    pub beta_neutral_notional: UsdAmount,
    pub correlation_stress: ProbabilityPpm,
    pub gross_risk: UsdAmount,
    pub net_risk: UsdAmount,
    pub net_to_gross_ppm: ProbabilityPpm,
}

pub fn cross_asset_features(input: &CrossAssetInputs) -> Result<CrossAssetFeatures, MarketError> {
    if input.gross_risk.raw() < 0 || input.net_risk.raw().abs() > input.gross_risk.raw() {
        return Err(MarketError::Malformed {
            what: "cross_asset",
            reason: "gross risk must dominate net risk",
        });
    }
    if input.gross_risk.scale() != input.net_risk.scale() {
        return Err(MarketError::ScaleMismatch);
    }
    let net_to_gross = if input.gross_risk.raw() == 0 {
        ProbabilityPpm::ZERO
    } else {
        let ppm = input
            .net_risk
            .raw()
            .abs()
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_div(input.gross_risk.raw()))
            .ok_or(MarketError::Overflow)?;
        ProbabilityPpm::from_ppm(
            u32::try_from(ppm.min(1_000_000)).map_err(|_| MarketError::Overflow)?,
        )?
    };
    Ok(CrossAssetFeatures {
        rotation_notional: input.rotated_notional,
        simultaneous_deleveraging: input.simultaneous_deleveraging,
        lead_lag_micros: input.lead_lag_micros,
        shared_collateral_contagion: input.shared_collateral,
        beta_neutral_notional: input.beta_neutral_notional,
        correlation_stress: input.correlation_stress_ppm,
        gross_risk: input.gross_risk,
        net_risk: input.net_risk,
        net_to_gross_ppm: net_to_gross,
    })
}
