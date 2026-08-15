use domain_types::{Quantity, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::error::SimError;
use crate::math::{qty_is_zero, qty_min};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioLimits {
    bankroll: UsdAmount,
    participation_limit_ppm: u32,
}

impl PortfolioLimits {
    pub fn new(bankroll: UsdAmount, participation_limit_ppm: u32) -> Result<Self, SimError> {
        if bankroll.raw() <= 0 {
            return Err(SimError::UnmodeledCost {
                component: "bankroll",
            });
        }
        if participation_limit_ppm == 0 || participation_limit_ppm > 1_000_000 {
            return Err(SimError::UnmodeledCost {
                component: "participation_limit",
            });
        }
        Ok(Self {
            bankroll,
            participation_limit_ppm,
        })
    }

    pub fn cap_quantity(
        self,
        requested: Quantity,
        opposite_depth: Quantity,
    ) -> Result<Quantity, SimError> {
        if qty_is_zero(opposite_depth) {
            return Err(SimError::OrderRejected {
                reason: "empty_book",
            });
        }
        let allowed =
            domain_types::Decimal::from_raw(opposite_depth.raw(), opposite_depth.scale())?
                .checked_mul(
                    domain_types::Decimal::from_raw(i128::from(self.participation_limit_ppm), 6)?,
                    opposite_depth.scale(),
                    domain_types::RoundingMode::TowardZero,
                )?;
        let allowed = Quantity::from_raw(allowed.raw(), allowed.scale())?;
        let capped = qty_min(requested, allowed);
        if qty_is_zero(capped) {
            return Err(SimError::OrderRejected {
                reason: "participation_limit",
            });
        }
        Ok(capped)
    }

    pub fn admit_notional(self, notional: UsdAmount) -> Result<(), SimError> {
        if notional > self.bankroll {
            return Err(SimError::OrderRejected { reason: "bankroll" });
        }
        Ok(())
    }
}
