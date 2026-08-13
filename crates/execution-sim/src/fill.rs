use domain_types::{BasisPoints, Decimal, FeeRate, Price, Quantity, RoundingMode, UsdAmount};
use serde::Serialize;

use crate::book::{BookSnapshot, WalkFill};
use crate::error::SimError;
use crate::impact::{ImpactModel, SlippageModel};
use crate::math::{
    abs_usd_diff, apply_bps, apply_fee, mid_price, qty_is_zero, quote_notional, zero_usd,
};

/// Provenance of a simulated fill. Venue fills are not representable as a
/// successful simulation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FillClass {
    Synthetic,
    Venue,
}

impl FillClass {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Synthetic => "synthetic",
            Self::Venue => "venue",
        }
    }

    pub const fn admit_simulated(self) -> Result<(), SimError> {
        match self {
            Self::Synthetic => Ok(()),
            Self::Venue => Err(SimError::VenueFillForbidden),
        }
    }

    pub const fn venue() -> Result<Self, SimError> {
        Err(SimError::VenueFillForbidden)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CostedFill {
    pub filled_quantity: Quantity,
    pub remaining_quantity: Quantity,
    pub vwap: Option<Price>,
    pub quote_notional: UsdAmount,
    pub fee: UsdAmount,
    pub slippage: UsdAmount,
    pub impact: UsdAmount,
    pub spread_cost: UsdAmount,
    pub is_taker: bool,
    pub fill_class: FillClass,
    pub venue_fill: bool,
}

impl CostedFill {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn synthetic(
        filled_quantity: Quantity,
        remaining_quantity: Quantity,
        vwap: Option<Price>,
        quote_notional: UsdAmount,
        fee: UsdAmount,
        slippage: UsdAmount,
        impact: UsdAmount,
        spread_cost: UsdAmount,
        is_taker: bool,
    ) -> Result<Self, SimError> {
        let fill = Self {
            filled_quantity,
            remaining_quantity,
            vwap,
            quote_notional,
            fee,
            slippage,
            impact,
            spread_cost,
            is_taker,
            fill_class: FillClass::Synthetic,
            venue_fill: false,
        };
        fill.admit_synthetic()?;
        Ok(fill)
    }

    pub(crate) fn admit_synthetic(&self) -> Result<(), SimError> {
        self.fill_class.admit_simulated()?;
        if self.venue_fill {
            return Err(SimError::VenueFillForbidden);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FillCostParams {
    pub is_taker: bool,
    pub taker_fee_rate: FeeRate,
    pub maker_fee_rate: FeeRate,
    pub slippage: SlippageModel,
    pub impact: ImpactModel,
}

pub fn costed_walk(
    book: &BookSnapshot,
    buy: bool,
    requested: Quantity,
    limit_price: Option<Price>,
    fill_ppm: Option<u32>,
    costs: FillCostParams,
) -> Result<CostedFill, SimError> {
    let walk = book.take_liquidity(buy, requested, limit_price, fill_ppm)?;
    apply_costs(book, buy, walk, costs)
}

fn apply_costs(
    book: &BookSnapshot,
    buy: bool,
    walk: WalkFill,
    costs: FillCostParams,
) -> Result<CostedFill, SimError> {
    let vwap = walk.vwap()?;
    if qty_is_zero(walk.filled_quantity) {
        return CostedFill::synthetic(
            walk.filled_quantity,
            walk.remaining_quantity,
            vwap,
            walk.quote_notional,
            zero_usd()?,
            zero_usd()?,
            zero_usd()?,
            zero_usd()?,
            costs.is_taker,
        );
    }
    let fee_rate = if costs.is_taker {
        costs.taker_fee_rate
    } else {
        costs.maker_fee_rate
    };
    let fee = apply_fee(walk.quote_notional, fee_rate)?;
    let slippage_cost = apply_bps(walk.quote_notional, costs.slippage.extra_bps())?;
    let depth = book.opposite_depth(buy)?;
    if qty_is_zero(depth) {
        return Err(SimError::UnmodeledCost {
            component: "impact_depth",
        });
    }
    let participation =
        Decimal::from_raw(walk.filled_quantity.raw(), walk.filled_quantity.scale())?.checked_div(
            Decimal::from_raw(depth.raw(), depth.scale())?,
            8,
            RoundingMode::TowardZero,
        )?;
    let scaled_bps = Decimal::from_raw(
        costs.impact.bps_at_full_depth().raw(),
        costs.impact.bps_at_full_depth().scale(),
    )?
    .checked_mul(
        participation,
        costs.impact.bps_at_full_depth().scale(),
        RoundingMode::TowardZero,
    )?;
    let bps = BasisPoints::from_raw(scaled_bps.raw(), scaled_bps.scale())?;
    let impact_cost = apply_bps(walk.quote_notional, bps)?;
    let mid = mid_price(book.best_bid(), book.best_ask())?;
    let mid_notional = quote_notional(mid, walk.filled_quantity)?;
    let spread_cost = abs_usd_diff(walk.quote_notional, mid_notional)?;
    CostedFill::synthetic(
        walk.filled_quantity,
        walk.remaining_quantity,
        vwap,
        walk.quote_notional,
        fee,
        slippage_cost,
        impact_cost,
        spread_cost,
        costs.is_taker,
    )
}
