use std::collections::BTreeMap;

use canonical_events::{CanonicalEventEnvelope, EventPayload};
use domain_types::{BasisPoints, Decimal, OrderId, Price, Quantity, RoundingMode, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::{ActionSide, IntelligenceError};

struct InForceLimit {
    side: ActionSide,
    limit_price: Price,
}

fn order_event_malformed(reason: &'static str) -> IntelligenceError {
    IntelligenceError::Malformed {
        what: "order_event",
        reason,
    }
}

fn require_positive_price(price: Price) -> Result<(), IntelligenceError> {
    if price.raw() <= 0 {
        return Err(IntelligenceError::Malformed {
            what: "observed_fill",
            reason: "prices must be positive",
        });
    }
    Ok(())
}

fn fill_notional(price: Price, quantity: Quantity) -> Result<UsdAmount, IntelligenceError> {
    if quantity.raw() <= 0 {
        return Err(IntelligenceError::Malformed {
            what: "order_event",
            reason: "quantity must be positive",
        });
    }
    let output_scale = price
        .scale()
        .checked_add(quantity.scale())
        .ok_or(IntelligenceError::Overflow)?;
    let product = Decimal::from_raw(price.raw(), price.scale())?.checked_mul(
        Decimal::from_raw(quantity.raw(), quantity.scale())?,
        output_scale,
        RoundingMode::TowardZero,
    )?;
    UsdAmount::from_raw(product.raw(), product.scale()).map_err(Into::into)
}

fn canonical_order_key(event: &CanonicalEventEnvelope) -> (u64, u32, u32) {
    (
        event.block_height().get(),
        event.transaction_index(),
        event.canonical_event_index(),
    )
}

fn admit_event_order(
    previous_time: &mut Option<i64>,
    previous_key: &mut Option<(u64, u32, u32)>,
    event: &CanonicalEventEnvelope,
) -> Result<(), IntelligenceError> {
    let time = event.block_time().unix_micros();
    if previous_time.is_some_and(|previous| time < previous) {
        return Err(order_event_malformed("inverted times"));
    }
    let key = canonical_order_key(event);
    if previous_key.is_some_and(|previous| key <= previous) {
        return Err(order_event_malformed("unknown event order"));
    }
    *previous_time = Some(time);
    *previous_key = Some(key);
    Ok(())
}

fn join_observed_fill(
    fills: &mut Vec<ObservedFill>,
    in_force: &BTreeMap<OrderId, InForceLimit>,
    order_id: &OrderId,
    fill_price: Price,
    fill_quantity: Quantity,
) -> Result<(), IntelligenceError> {
    require_positive_price(fill_price)?;
    if fill_quantity.raw() <= 0 {
        return Err(order_event_malformed("quantity must be positive"));
    }
    let Some(order) = in_force.get(order_id) else {
        return Ok(());
    };
    fills.push(ObservedFill::try_new(
        fill_price,
        Some(order.limit_price),
        order.side,
        fill_notional(fill_price, fill_quantity)?,
    )?);
    Ok(())
}

/// Join canonical observed fill prices to the limit in force at each fill.
///
/// In-force limits come from `OrderAccepted` / `OrderModified` that precede the
/// fill in canonical event order. A later `OrderModified` does not rewrite an
/// earlier fill. Missing in-force limits skip that fill (withhold); this path
/// never invents a mid or mark.
pub fn observed_fills_from_order_events(
    events: &[CanonicalEventEnvelope],
) -> Result<Vec<ObservedFill>, IntelligenceError> {
    let mut previous_time = None;
    let mut previous_key = None;
    let mut in_force = BTreeMap::new();
    let mut fills = Vec::new();
    for event in events {
        admit_event_order(&mut previous_time, &mut previous_key, event)?;
        match event.payload() {
            EventPayload::OrderAccepted(accepted) => {
                require_positive_price(accepted.limit_price)?;
                if in_force.contains_key(&accepted.order_id) {
                    return Err(order_event_malformed("unknown event order"));
                }
                in_force.insert(
                    accepted.order_id.clone(),
                    InForceLimit {
                        side: ActionSide::from(accepted.side),
                        limit_price: accepted.limit_price,
                    },
                );
            }
            EventPayload::OrderModified(modified) => {
                require_positive_price(modified.previous_price)?;
                require_positive_price(modified.new_price)?;
                let Some(order) = in_force.get_mut(&modified.order_id) else {
                    return Err(order_event_malformed("unknown event order"));
                };
                if order.limit_price != modified.previous_price {
                    return Err(order_event_malformed("unknown event order"));
                }
                order.limit_price = modified.new_price;
            }
            EventPayload::OrderPartiallyFilled(fill) => {
                join_observed_fill(
                    &mut fills,
                    &in_force,
                    &fill.order_id,
                    fill.fill_price,
                    fill.fill_quantity,
                )?;
            }
            EventPayload::OrderFilled(fill) => {
                join_observed_fill(
                    &mut fills,
                    &in_force,
                    &fill.order_id,
                    fill.fill_price,
                    fill.fill_quantity,
                )?;
                in_force.remove(&fill.order_id);
            }
            EventPayload::OrderCancelled(cancelled) => {
                in_force.remove(&cancelled.order_id);
            }
            EventPayload::OrderRested(_)
            | EventPayload::OrderRejected(_)
            | EventPayload::TriggerOrderActivated(_)
            | EventPayload::TwapStarted(_)
            | EventPayload::TwapSliceFilled(_)
            | EventPayload::TwapCompleted(_)
            | EventPayload::TradeMatched(_)
            | EventPayload::DepositCredited(_)
            | EventPayload::WithdrawalDebited(_)
            | EventPayload::SpotTransfer(_)
            | EventPayload::PerpTransfer(_)
            | EventPayload::SubaccountTransfer(_)
            | EventPayload::VaultDeposit(_)
            | EventPayload::VaultWithdrawal(_)
            | EventPayload::FeeCharged(_)
            | EventPayload::BuilderFeeCharged(_)
            | EventPayload::FundingPaid(_)
            | EventPayload::FundingReceived(_)
            | EventPayload::ReferralReward(_)
            | EventPayload::AccountModeChanged(_)
            | EventPayload::MarginModeChanged(_)
            | EventPayload::LeverageChanged(_)
            | EventPayload::LiquidationStarted(_)
            | EventPayload::LiquidationFill(_)
            | EventPayload::BackstopLiquidation(_)
            | EventPayload::PositionSettled(_)
            | EventPayload::MarketHalted(_)
            | EventPayload::MarketResumed(_)
            | EventPayload::OpenInterestCapChanged(_)
            | EventPayload::MarginTableChanged(_)
            | EventPayload::MarketCreated(_)
            | EventPayload::MarketMetadataChanged(_)
            | EventPayload::OracleUpdated(_)
            | EventPayload::FundingRateUpdated(_)
            | EventPayload::AssetContextUpdated(_)
            | EventPayload::DexCreated(_)
            | EventPayload::OutcomeCreated(_)
            | EventPayload::OutcomeResolved(_) => {}
        }
    }
    Ok(fills)
}

/// Join canonical fills to in-force limits, then score with [`slippage_from_fills`].
pub fn slippage_from_order_events(
    events: &[CanonicalEventEnvelope],
) -> Result<Option<SlippageSummary>, IntelligenceError> {
    let fills = observed_fills_from_order_events(events)?;
    slippage_from_fills(&fills)
}

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
