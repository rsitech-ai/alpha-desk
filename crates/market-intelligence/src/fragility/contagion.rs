use crate::{MarketError, fragility::SimulatedAccount};

pub fn apply_collateral_contagion(
    mut accounts: Vec<SimulatedAccount>,
    extra_bps: i64,
) -> Result<Vec<SimulatedAccount>, MarketError> {
    if extra_bps < 0 {
        return Err(MarketError::Malformed {
            what: "contagion",
            reason: "extra distance reduction must be non-negative",
        });
    }
    let groups: std::collections::BTreeSet<String> = accounts
        .iter()
        .filter_map(|account| account.collateral_group.clone())
        .collect();
    if groups.is_empty() {
        return Ok(accounts);
    }
    for account in &mut accounts {
        if account.collateral_group.is_some() {
            account.distance_to_maintenance_bps = account
                .distance_to_maintenance_bps
                .checked_sub(extra_bps)
                .ok_or(MarketError::Overflow)?;
        }
    }
    Ok(accounts)
}
