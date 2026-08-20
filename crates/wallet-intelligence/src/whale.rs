use domain_types::{MarginRatio, ProbabilityPpm, RoundingMode, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::IntelligenceError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhaleInputs {
    pub equity: UsdAmount,
    pub cohort_equities: Vec<UsdAmount>,
    pub position_notional: UsdAmount,
    pub market_open_interest: UsdAmount,
    pub delta_notional: UsdAmount,
    pub rolling_market_volume: UsdAmount,
    pub executable_depth_25bps: UsdAmount,
    pub account_equity: UsdAmount,
    pub equity_floor: UsdAmount,
    pub vulnerable_notional: UsdAmount,
    pub depth_to_liquidation: UsdAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WhaleComponents {
    pub capital_percentile: ProbabilityPpm,
    pub position_oi_share: MarginRatio,
    pub flow_volume_share: MarginRatio,
    pub impact_depth_ratio_25bps: MarginRatio,
    pub account_commitment: MarginRatio,
    pub forced_flow_potential: MarginRatio,
    pub influence_score: Option<ProbabilityPpm>,
    pub skill_probability: Option<ProbabilityPpm>,
    pub fragility_score: Option<ProbabilityPpm>,
}

impl WhaleComponents {
    pub fn try_from_inputs(
        inputs: &WhaleInputs,
        influence_score: Option<ProbabilityPpm>,
        skill_probability: Option<ProbabilityPpm>,
        fragility_score: Option<ProbabilityPpm>,
    ) -> Result<Self, IntelligenceError> {
        Ok(Self {
            capital_percentile: capital_percentile(inputs.equity, &inputs.cohort_equities)?,
            position_oi_share: share(inputs.position_notional, inputs.market_open_interest)?,
            flow_volume_share: share(inputs.delta_notional, inputs.rolling_market_volume)?,
            impact_depth_ratio_25bps: share(inputs.delta_notional, inputs.executable_depth_25bps)?,
            account_commitment: share(
                inputs.position_notional,
                max_equity(inputs.account_equity, inputs.equity_floor)?,
            )?,
            forced_flow_potential: share(inputs.vulnerable_notional, inputs.depth_to_liquidation)?,
            influence_score,
            skill_probability,
            fragility_score,
        })
    }
}

fn share(numerator: UsdAmount, denominator: UsdAmount) -> Result<MarginRatio, IntelligenceError> {
    if denominator.raw() == 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    if numerator.scale() != denominator.scale() {
        return Err(IntelligenceError::ScaleMismatch);
    }
    let abs_num = numerator
        .raw()
        .checked_abs()
        .ok_or(IntelligenceError::Overflow)?;
    let ratio = domain_types::Decimal::from_raw(abs_num, numerator.scale())?.checked_div(
        domain_types::Decimal::from_raw(denominator.raw(), denominator.scale())?,
        numerator.scale(),
        RoundingMode::NearestTiesToEven,
    )?;
    MarginRatio::from_raw(ratio.raw(), ratio.scale()).map_err(Into::into)
}

fn capital_percentile(
    equity: UsdAmount,
    cohort: &[UsdAmount],
) -> Result<ProbabilityPpm, IntelligenceError> {
    if cohort.is_empty() {
        return Err(IntelligenceError::InsufficientHistory {
            what: "capital_percentile",
        });
    }
    if cohort.iter().any(|value| value.scale() != equity.scale()) {
        return Err(IntelligenceError::ScaleMismatch);
    }
    let below = cohort.iter().filter(|value| **value <= equity).count();
    let ppm = u128::try_from(below)
        .ok()
        .and_then(|count| count.checked_mul(1_000_000))
        .and_then(|value| value.checked_div(u128::try_from(cohort.len()).ok()?))
        .ok_or(IntelligenceError::Overflow)?;
    ProbabilityPpm::from_ppm(u32::try_from(ppm).map_err(|_| IntelligenceError::Overflow)?)
        .map_err(Into::into)
}

fn max_equity(equity: UsdAmount, floor: UsdAmount) -> Result<UsdAmount, IntelligenceError> {
    if equity.scale() != floor.scale() {
        return Err(IntelligenceError::ScaleMismatch);
    }
    if equity.raw() < 0 || floor.raw() < 0 {
        return Err(IntelligenceError::Malformed {
            what: "whale",
            reason: "equity and floor must be non-negative",
        });
    }
    Ok(if equity >= floor { equity } else { floor })
}
