use crate::{
    MarketError,
    fragility::{SimulatedAccount, SimulatedMarginMode},
};

pub fn path_variant(
    mut account: SimulatedAccount,
    bound_sign: i32,
) -> Result<SimulatedAccount, MarketError> {
    match account.margin_mode {
        SimulatedMarginMode::IsolatedExact => Ok(account),
        SimulatedMarginMode::Unsupported => Ok(account),
        SimulatedMarginMode::PortfolioUncertain { band_bps } => {
            let band = i64::from(band_bps);
            let delta = match bound_sign {
                -1 => band,
                0 => 0,
                1 => -band,
                _ => {
                    return Err(MarketError::Malformed {
                        what: "fragility_bounds",
                        reason: "bound sign must be -1, 0, or 1",
                    });
                }
            };
            account.distance_to_maintenance_bps = account
                .distance_to_maintenance_bps
                .checked_add(delta)
                .ok_or(MarketError::Overflow)?;
            Ok(account)
        }
    }
}
