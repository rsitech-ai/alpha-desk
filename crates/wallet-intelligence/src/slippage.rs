use domain_types::{BasisPoints, Price, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::{ActionSide, IntelligenceError};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObservedFill {
    pub fill_price: Price,
    pub observed_reference_price: Option<Price>,
    pub side: ActionSide,
    pub notional: UsdAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SlippageSummary {
    pub observed_fill_count: u64,
    pub withheld_missing_reference_count: u64,
    pub notional_weighted_slippage_bps: BasisPoints,
    pub signed_slippage: UsdAmount,
}

impl ObservedFill {
    pub fn try_new(
        fill_price: Price,
        observed_reference_price: Option<Price>,
        side: ActionSide,
        notional: UsdAmount,
    ) -> Result<Self, IntelligenceError> {
        let fill = Self {
            fill_price,
            observed_reference_price,
            side,
            notional,
        };
        fill.validate()?;
        Ok(fill)
    }

    fn validate(&self) -> Result<(), IntelligenceError> {
        if self.fill_price.raw() <= 0 {
            return Err(IntelligenceError::Malformed {
                what: "observed_fill",
                reason: "prices must be positive",
            });
        }
        if let Some(reference) = self.observed_reference_price {
            if reference.raw() <= 0 {
                return Err(IntelligenceError::Malformed {
                    what: "observed_fill",
                    reason: "prices must be positive",
                });
            }
            if reference.scale() != self.fill_price.scale() {
                return Err(IntelligenceError::ScaleMismatch);
            }
        }
        if self.notional.raw() <= 0 {
            return Err(IntelligenceError::Malformed {
                what: "observed_fill",
                reason: "notional must be positive",
            });
        }
        Ok(())
    }
}

pub fn slippage_from_fills(
    fills: &[ObservedFill],
) -> Result<Option<SlippageSummary>, IntelligenceError> {
    if fills.is_empty() {
        return Ok(None);
    }
    let mut observed = 0_u64;
    let mut withheld = 0_u64;
    let mut total_notional = 0_i128;
    let mut signed_slippage = 0_i128;
    let mut usd_scale = None;
    for fill in fills {
        fill.validate()?;
        if usd_scale.is_some_and(|scale| scale != fill.notional.scale()) {
            return Err(IntelligenceError::ScaleMismatch);
        }
        usd_scale = Some(fill.notional.scale());
        let Some(reference) = fill.observed_reference_price else {
            withheld = withheld.checked_add(1).ok_or(IntelligenceError::Overflow)?;
            continue;
        };
        observed = observed.checked_add(1).ok_or(IntelligenceError::Overflow)?;
        let signed_delta = fill
            .fill_price
            .checked_sub(reference)?
            .raw()
            .checked_mul(fill.side.markout_sign())
            .ok_or(IntelligenceError::Overflow)?;
        let fill_slippage = fill
            .notional
            .raw()
            .checked_mul(signed_delta)
            .and_then(|value| value.checked_div(reference.raw()))
            .ok_or(IntelligenceError::Overflow)?;
        signed_slippage = signed_slippage
            .checked_add(fill_slippage)
            .ok_or(IntelligenceError::Overflow)?;
        total_notional = total_notional
            .checked_add(fill.notional.raw())
            .ok_or(IntelligenceError::Overflow)?;
    }
    if observed == 0 {
        return Ok(None);
    }
    if total_notional == 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    let scale = usd_scale.ok_or(IntelligenceError::Overflow)?;
    let bps = signed_slippage
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_div(total_notional))
        .ok_or(IntelligenceError::Overflow)?;
    Ok(Some(SlippageSummary {
        observed_fill_count: observed,
        withheld_missing_reference_count: withheld,
        notional_weighted_slippage_bps: BasisPoints::from_raw(bps, 2)?,
        signed_slippage: UsdAmount::from_raw(signed_slippage, scale)?,
    }))
}
