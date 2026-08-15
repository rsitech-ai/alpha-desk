use domain_types::{KnownTime, Price, ProtocolTime, Quantity};
use serde::{Deserialize, Serialize};

use crate::error::SimError;
use crate::math::{align_price, align_qty, qty_is_zero, qty_min, zero_qty};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookLevel {
    price: Price,
    quantity: Quantity,
}

impl BookLevel {
    pub fn new(price: Price, quantity: Quantity) -> Result<Self, SimError> {
        if qty_is_zero(quantity) || quantity.raw() < 0 {
            return Err(SimError::InvalidRequest {
                field: "book.level_quantity",
            });
        }
        Ok(Self {
            price: align_price(price)?,
            quantity: align_qty(quantity)?,
        })
    }

    #[must_use]
    pub const fn price(&self) -> Price {
        self.price
    }

    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookSnapshot {
    effective_at: ProtocolTime,
    known_at: KnownTime,
    bids: Vec<BookLevel>,
    asks: Vec<BookLevel>,
}

impl BookSnapshot {
    pub fn new(
        effective_at: ProtocolTime,
        known_at: KnownTime,
        mut bids: Vec<BookLevel>,
        mut asks: Vec<BookLevel>,
    ) -> Result<Self, SimError> {
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(SimError::InvalidRequest {
                field: "book.known_before_effective",
            });
        }
        bids.sort_by_key(|level| std::cmp::Reverse(level.price()));
        asks.sort_by_key(BookLevel::price);
        if bids.is_empty() || asks.is_empty() {
            return Err(SimError::UnmodeledCost {
                component: "two_sided_spread",
            });
        }
        if bids[0].price() >= asks[0].price() {
            return Err(SimError::InvalidRequest {
                field: "book.crossed",
            });
        }
        Ok(Self {
            effective_at,
            known_at,
            bids,
            asks,
        })
    }

    #[must_use]
    pub const fn effective_at(&self) -> ProtocolTime {
        self.effective_at
    }

    #[must_use]
    pub const fn known_at(&self) -> KnownTime {
        self.known_at
    }

    #[must_use]
    pub fn best_bid(&self) -> Price {
        self.bids[0].price()
    }

    #[must_use]
    pub fn best_ask(&self) -> Price {
        self.asks[0].price()
    }

    #[must_use]
    pub fn opposite_levels(&self, buy: bool) -> &[BookLevel] {
        if buy { &self.asks } else { &self.bids }
    }

    #[must_use]
    pub fn levels_for_rebuild(&self) -> (Vec<BookLevel>, Vec<BookLevel>) {
        (self.bids.clone(), self.asks.clone())
    }

    pub fn opposite_depth(&self, buy: bool) -> Result<Quantity, SimError> {
        let mut total = zero_qty()?;
        for level in self.opposite_levels(buy) {
            total = total.checked_add(level.quantity())?;
        }
        Ok(total)
    }

    pub fn take_liquidity(
        &self,
        buy: bool,
        requested: Quantity,
        limit_price: Option<Price>,
        fill_ppm: Option<u32>,
    ) -> Result<WalkFill, SimError> {
        let mut remaining = align_qty(requested)?;
        let mut filled = zero_qty()?;
        let mut notional = crate::math::zero_usd()?;
        let ppm = fill_ppm.unwrap_or(1_000_000);
        for level in self.opposite_levels(buy) {
            if qty_is_zero(remaining) {
                break;
            }
            if let Some(limit) = limit_price {
                let crosses = if buy {
                    level.price() <= limit
                } else {
                    level.price() >= limit
                };
                if !crosses {
                    break;
                }
            }
            let available = scale_qty(level.quantity(), ppm)?;
            let take = qty_min(remaining, available);
            if qty_is_zero(take) {
                continue;
            }
            let slice = crate::math::quote_notional(level.price(), take)?;
            notional = notional.checked_add(slice)?;
            filled = filled.checked_add(take)?;
            remaining = remaining.checked_sub(take)?;
        }
        Ok(WalkFill {
            filled_quantity: filled,
            remaining_quantity: remaining,
            quote_notional: notional,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkFill {
    pub filled_quantity: Quantity,
    pub remaining_quantity: Quantity,
    pub quote_notional: domain_types::UsdAmount,
}

impl WalkFill {
    pub fn vwap(&self) -> Result<Option<Price>, SimError> {
        if qty_is_zero(self.filled_quantity) {
            return Ok(None);
        }
        let price = domain_types::Decimal::from_raw(
            self.quote_notional.raw(),
            self.quote_notional.scale(),
        )?
        .checked_div(
            domain_types::Decimal::from_raw(
                self.filled_quantity.raw(),
                self.filled_quantity.scale(),
            )?,
            crate::math::PRICE_SCALE,
            domain_types::RoundingMode::TowardZero,
        )?;
        Ok(Some(Price::from_raw(price.raw(), price.scale())?))
    }
}

fn scale_qty(quantity: Quantity, ppm: u32) -> Result<Quantity, SimError> {
    if ppm == 0 {
        return zero_qty();
    }
    if ppm == 1_000_000 {
        return Ok(quantity);
    }
    let scaled = domain_types::Decimal::from_raw(quantity.raw(), quantity.scale())?.checked_mul(
        domain_types::Decimal::from_raw(i128::from(ppm), 6)?,
        crate::math::QTY_SCALE,
        domain_types::RoundingMode::TowardZero,
    )?;
    Ok(Quantity::from_raw(scaled.raw(), scaled.scale())?)
}

pub fn select_book(
    books: &[BookSnapshot],
    arrival: ProtocolTime,
    evaluation_known_at: KnownTime,
) -> Result<&BookSnapshot, SimError> {
    for book in books {
        if book.known_at() > evaluation_known_at {
            return Err(SimError::FutureData {
                field: "book.known_at",
            });
        }
    }
    books
        .iter()
        .filter(|book| {
            book.effective_at() <= arrival && book.known_at().unix_micros() <= arrival.unix_micros()
        })
        .max_by_key(|book| {
            (
                book.effective_at().unix_micros(),
                book.known_at().unix_micros(),
            )
        })
        .ok_or(SimError::MissingArrivalBook)
}
