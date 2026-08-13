use domain_types::{
    BasisPoints, Decimal, FeeRate, FundingRate, Price, Quantity, RoundingMode, UsdAmount,
};

use crate::error::SimError;

pub(crate) const MONEY_SCALE: u8 = 8;
pub(crate) const PRICE_SCALE: u8 = 8;
pub(crate) const QTY_SCALE: u8 = 8;

pub(crate) fn align_price(price: Price) -> Result<Price, SimError> {
    Ok(price.rescale(PRICE_SCALE, RoundingMode::TowardZero)?)
}

pub(crate) fn align_qty(quantity: Quantity) -> Result<Quantity, SimError> {
    Ok(quantity.rescale(QTY_SCALE, RoundingMode::TowardZero)?)
}

pub(crate) fn zero_usd() -> Result<UsdAmount, SimError> {
    Ok(UsdAmount::from_raw(0, MONEY_SCALE)?)
}

pub(crate) fn zero_qty() -> Result<Quantity, SimError> {
    Ok(Quantity::from_raw(0, QTY_SCALE)?)
}

pub(crate) fn quote_notional(price: Price, quantity: Quantity) -> Result<UsdAmount, SimError> {
    let product = decimal(price.raw(), price.scale())?.checked_mul(
        decimal(quantity.raw(), quantity.scale())?,
        MONEY_SCALE,
        RoundingMode::TowardZero,
    )?;
    Ok(UsdAmount::from_raw(product.raw(), product.scale())?)
}

pub(crate) fn apply_fee(notional: UsdAmount, rate: FeeRate) -> Result<UsdAmount, SimError> {
    mul_rate(notional, rate.raw(), rate.scale())
}

pub(crate) fn apply_funding(notional: UsdAmount, rate: FundingRate) -> Result<UsdAmount, SimError> {
    mul_rate(notional, rate.raw(), rate.scale())
}

pub(crate) fn apply_bps(notional: UsdAmount, bps: BasisPoints) -> Result<UsdAmount, SimError> {
    let ten_thousand = decimal(10_000, 0)?;
    let bps_decimal = decimal(bps.raw(), bps.scale())?;
    let product = decimal(notional.raw(), notional.scale())?.checked_mul(
        bps_decimal,
        MONEY_SCALE + 4,
        RoundingMode::TowardZero,
    )?;
    let divided = product.checked_div(ten_thousand, MONEY_SCALE, RoundingMode::TowardZero)?;
    Ok(UsdAmount::from_raw(divided.raw(), divided.scale())?)
}

pub(crate) fn mid_price(bid: Price, ask: Price) -> Result<Price, SimError> {
    let sum = decimal(bid.raw(), bid.scale())?.checked_add(decimal(ask.raw(), ask.scale())?)?;
    let two = decimal(2, 0)?;
    let mid = sum.checked_div(two, PRICE_SCALE, RoundingMode::TowardZero)?;
    Ok(Price::from_raw(mid.raw(), mid.scale())?)
}

pub(crate) fn abs_usd_diff(left: UsdAmount, right: UsdAmount) -> Result<UsdAmount, SimError> {
    if left >= right {
        Ok(left.checked_sub(right)?)
    } else {
        Ok(right.checked_sub(left)?)
    }
}

pub(crate) fn qty_min(left: Quantity, right: Quantity) -> Quantity {
    if left <= right { left } else { right }
}

pub(crate) fn qty_is_zero(quantity: Quantity) -> bool {
    quantity.raw() == 0
}

fn mul_rate(notional: UsdAmount, rate_raw: i128, rate_scale: u8) -> Result<UsdAmount, SimError> {
    let product = decimal(notional.raw(), notional.scale())?.checked_mul(
        decimal(rate_raw, rate_scale)?,
        MONEY_SCALE,
        RoundingMode::TowardZero,
    )?;
    Ok(UsdAmount::from_raw(product.raw(), product.scale())?)
}

fn decimal(raw: i128, scale: u8) -> Result<Decimal, SimError> {
    Ok(Decimal::from_raw(raw, scale)?)
}
