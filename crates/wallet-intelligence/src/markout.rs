use domain_types::{
    BasisPoints, EventId, Horizon, KnownTime, MarketId, OrderSide, Price, ProtocolTime, UsdAmount,
};
use serde::{Deserialize, Serialize};

use crate::IntelligenceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiquidityRole {
    Maker,
    Taker,
}

impl LiquidityRole {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Maker => "maker",
            Self::Taker => "taker",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionSide {
    Buy,
    Sell,
}

impl ActionSide {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }

    #[must_use]
    pub const fn markout_sign(self) -> i128 {
        match self {
            Self::Buy => 1,
            Self::Sell => -1,
        }
    }
}

impl From<OrderSide> for ActionSide {
    fn from(side: OrderSide) -> Self {
        match side {
            OrderSide::Buy => Self::Buy,
            OrderSide::Sell => Self::Sell,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MarkoutKind {
    Entry,
    Exit,
}

impl MarkoutKind {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Entry => "entry",
            Self::Exit => "exit",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkoutPoint {
    pub action_id: EventId,
    pub market_id: MarketId,
    pub kind: MarkoutKind,
    pub side: ActionSide,
    pub role: LiquidityRole,
    pub entry_at: ProtocolTime,
    pub entry_price: Price,
    pub horizon: Horizon,
    pub price_at_horizon: Price,
    pub price_known_at: KnownTime,
    pub fee: UsdAmount,
    pub funding: UsdAmount,
    pub notional: UsdAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkoutResult {
    pub action_id: EventId,
    pub market_id: MarketId,
    pub kind: MarkoutKind,
    pub horizon: Horizon,
    pub net_markout_bps: BasisPoints,
    pub complete: bool,
    pub role: LiquidityRole,
    pub side: ActionSide,
}

impl MarkoutPoint {
    pub fn evaluate(&self, known_at: KnownTime) -> Result<MarkoutResult, IntelligenceError> {
        let horizon_end = self
            .entry_at
            .unix_micros()
            .checked_add(
                i64::try_from(self.horizon.as_micros()).map_err(|_| IntelligenceError::Overflow)?,
            )
            .ok_or(IntelligenceError::Overflow)?;
        if self.price_known_at.unix_micros() < horizon_end {
            return Err(IntelligenceError::Malformed {
                what: "markout",
                reason: "price known before horizon elapsed",
            });
        }
        if known_at.unix_micros() < self.price_known_at.unix_micros() {
            return Ok(MarkoutResult {
                action_id: self.action_id.clone(),
                market_id: self.market_id.clone(),
                kind: self.kind,
                horizon: self.horizon,
                net_markout_bps: BasisPoints::from_raw(0, 2)?,
                complete: false,
                role: self.role,
                side: self.side,
            });
        }
        if self.entry_price.raw() == 0 {
            return Err(IntelligenceError::DivisionByZero);
        }
        if self.entry_price.scale() != self.price_at_horizon.scale() {
            return Err(IntelligenceError::ScaleMismatch);
        }
        let delta = self
            .price_at_horizon
            .checked_sub(self.entry_price)?
            .raw()
            .checked_mul(self.side.markout_sign())
            .ok_or(IntelligenceError::Overflow)?;
        let gross_bps = delta
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_div(self.entry_price.raw()))
            .ok_or(IntelligenceError::Overflow)?;
        let cost_bps = cost_to_bps(self.fee, self.funding, self.notional)?;
        let net = gross_bps
            .checked_sub(cost_bps)
            .ok_or(IntelligenceError::Overflow)?;
        Ok(MarkoutResult {
            action_id: self.action_id.clone(),
            market_id: self.market_id.clone(),
            kind: self.kind,
            horizon: self.horizon,
            net_markout_bps: BasisPoints::from_raw(net, 2)?,
            complete: true,
            role: self.role,
            side: self.side,
        })
    }
}

pub fn evaluate_markouts(
    points: &[MarkoutPoint],
    known_at: KnownTime,
) -> Result<Option<Vec<MarkoutResult>>, IntelligenceError> {
    if points.is_empty() {
        return Ok(None);
    }
    let mut results = Vec::with_capacity(points.len());
    for point in points {
        results.push(point.evaluate(known_at)?);
    }
    Ok(Some(results))
}

fn cost_to_bps(
    fee: UsdAmount,
    funding: UsdAmount,
    notional: UsdAmount,
) -> Result<i128, IntelligenceError> {
    if notional.raw() == 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    if fee.scale() != notional.scale() || funding.scale() != notional.scale() {
        return Err(IntelligenceError::ScaleMismatch);
    }
    let cost = fee
        .raw()
        .checked_add(funding.raw())
        .ok_or(IntelligenceError::Overflow)?;
    cost.checked_mul(1_000_000)
        .and_then(|value| value.checked_div(notional.raw()))
        .ok_or(IntelligenceError::Overflow)
}
