use domain_types::{AccountId, ProbabilityPpm, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::{MarketError, fragility::SimulatedAccount};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LiquidationWave {
    pub iteration: u32,
    pub liquidated_accounts: Vec<AccountId>,
    pub forced_notional: UsdAmount,
    pub estimated_impact_bps: i64,
}

pub fn liquidate_accounts(
    accounts: &[SimulatedAccount],
) -> Result<(Vec<SimulatedAccount>, Vec<SimulatedAccount>), MarketError> {
    let mut liquidated = Vec::new();
    let mut survivors = Vec::new();
    for account in accounts {
        if account.notional.raw() < 0 {
            return Err(MarketError::Malformed {
                what: "liquidation",
                reason: "notional must be non-negative",
            });
        }
        if account.distance_to_maintenance_bps <= 0 {
            liquidated.push(account.clone());
        } else {
            survivors.push(account.clone());
        }
    }
    liquidated.sort_by(|left, right| left.account_id.as_str().cmp(right.account_id.as_str()));
    Ok((liquidated, survivors))
}

pub fn forced_impact_bps(
    forced: UsdAmount,
    depth: UsdAmount,
    participation: ProbabilityPpm,
    stress: ProbabilityPpm,
) -> Result<i64, MarketError> {
    if depth.raw() <= 0 {
        return Err(MarketError::EmptyDenominator);
    }
    if forced.scale() != depth.scale() {
        return Err(MarketError::ScaleMismatch);
    }
    let numerator = forced
        .raw()
        .checked_mul(i128::from(participation.ppm()))
        .and_then(|value| value.checked_mul(i128::from(stress.ppm())))
        .and_then(|value| value.checked_mul(10_000))
        .ok_or(MarketError::Overflow)?;
    let denominator = depth
        .raw()
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_mul(1_000_000))
        .ok_or(MarketError::Overflow)?;
    i64::try_from(numerator / denominator).map_err(|_| MarketError::Overflow)
}
