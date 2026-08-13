use domain_types::{AccountId, BasisPoints};
use serde::{Deserialize, Serialize};

use crate::GraphError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MakerTakerRole {
    Maker,
    Taker,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterpartyTrade {
    pub maker: AccountId,
    pub taker: AccountId,
    pub maker_markout_bps: i64,
    pub market_return_bps: i64,
    pub inventory_transferred: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CounterpartySummary {
    pub a_versus_b_markout_bps: BasisPoints,
    pub passive_adverse_selection_bps: BasisPoints,
    pub inventory_transfer_count: u32,
    pub sample_size: u32,
}

pub fn summarize_counterparty(
    trades: &[CounterpartyTrade],
    account_a: &AccountId,
    account_b: &AccountId,
) -> Result<CounterpartySummary, GraphError> {
    if trades.is_empty() {
        return Err(GraphError::Malformed {
            what: "counterparty",
            reason: "empty trades",
        });
    }
    let mut residual = 0_i128;
    let mut adverse = 0_i128;
    let mut count = 0_i128;
    let mut inventory = 0_u32;
    for trade in trades {
        let involves = (trade.maker == *account_a && trade.taker == *account_b)
            || (trade.maker == *account_b && trade.taker == *account_a);
        if !involves {
            continue;
        }
        count += 1;
        let signed = if trade.taker == *account_a {
            trade.maker_markout_bps
        } else {
            -trade.maker_markout_bps
        };
        residual += i128::from(signed) - i128::from(trade.market_return_bps);
        if trade.maker == *account_a {
            adverse += i128::from(-trade.maker_markout_bps);
        }
        if trade.inventory_transferred {
            inventory += 1;
        }
    }
    if count == 0 {
        return Err(GraphError::Malformed {
            what: "counterparty",
            reason: "pair not observed",
        });
    }
    Ok(CounterpartySummary {
        a_versus_b_markout_bps: BasisPoints::from_raw(residual / count, 0)?,
        passive_adverse_selection_bps: BasisPoints::from_raw(adverse / count, 0)?,
        inventory_transfer_count: inventory,
        sample_size: u32::try_from(count).map_err(|_| GraphError::Overflow)?,
    })
}
