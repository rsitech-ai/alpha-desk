use std::collections::BTreeMap;

use domain_types::{
    BasisPoints, BlockHeight, ExactQuoteNotional, FeeScheduleId, LatencyDistribution, MarketId,
    OrderSide, Price, ProbabilityPpm, Quantity, UsdAmount,
};

use crate::book::{BookHealth, OrderBook};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionRequest {
    pub market_id: MarketId,
    pub side: OrderSide,
    pub quantity: Quantity,
    pub max_participation: ProbabilityPpm,
    pub fee_schedule_id: FeeScheduleId,
    pub exit_stress_multiplier: ProbabilityPpm,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionEstimate {
    pub fill_probability: ProbabilityPpm,
    pub expected_fill_quantity: Quantity,
    pub p10_vwap: Price,
    pub p50_vwap: Price,
    pub p90_vwap: Price,
    pub spread_bps: BasisPoints,
    pub impact_bps: BasisPoints,
    pub queue_uncertainty: ProbabilityPpm,
    pub time_to_fill: LatencyDistribution,
    pub normal_exit_cost_bps: BasisPoints,
    pub stressed_exit_cost_bps: BasisPoints,
    pub capacity_by_cost: BTreeMap<BasisPoints, UsdAmount>,
    pub as_of_block: BlockHeight,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecutionError {
    #[error("order book is not healthy")]
    BookNotHealthy,
    #[error("requested market does not match the book")]
    MarketMismatch,
    #[error("requested quantity is not positive")]
    InvalidQuantity,
    #[error("insufficient visible liquidity")]
    InsufficientLiquidity,
}

pub fn quote_execution(
    book: &OrderBook,
    request: &ExecutionRequest,
    latency: &LatencyDistribution,
) -> Result<ExecutionEstimate, ExecutionError> {
    if !matches!(book.health(), BookHealth::Healthy) {
        return Err(ExecutionError::BookNotHealthy);
    }
    if request.market_id != *book.market_id() {
        return Err(ExecutionError::MarketMismatch);
    }
    if request.quantity.raw() <= 0 {
        return Err(ExecutionError::InvalidQuantity);
    }

    let _ = &request.fee_schedule_id;
    let _ = request.max_participation;
    let _ = request.exit_stress_multiplier;

    let mut remaining = request.quantity;
    let mut filled = Quantity::from_raw(0, request.quantity.scale())
        .map_err(|_| ExecutionError::InvalidQuantity)?;
    let mut notional: Option<ExactQuoteNotional> = None;
    let opposite = match request.side {
        OrderSide::Buy => book.l2_asks(),
        OrderSide::Sell => book.l2_bids(),
    };
    for level in opposite {
        if remaining.raw() == 0 {
            break;
        }
        let take = if remaining.raw() <= level.quantity.raw() {
            remaining
        } else {
            level.quantity
        };
        let level_notional = ExactQuoteNotional::checked_product(level.price, take)
            .map_err(|_| ExecutionError::InsufficientLiquidity)?;
        notional = Some(match notional {
            Some(existing) => existing
                .checked_add(&level_notional)
                .map_err(|_| ExecutionError::InsufficientLiquidity)?,
            None => level_notional,
        });
        filled = filled
            .checked_add(take)
            .map_err(|_| ExecutionError::InsufficientLiquidity)?;
        remaining = remaining
            .checked_sub(take)
            .map_err(|_| ExecutionError::InsufficientLiquidity)?;
    }
    if remaining.raw() != 0 {
        return Err(ExecutionError::InsufficientLiquidity);
    }
    let vwap = exact_vwap(notional.as_ref(), filled)?;
    let zero_bps = BasisPoints::from_raw(0, 0).map_err(|_| ExecutionError::InvalidQuantity)?;
    Ok(ExecutionEstimate {
        fill_probability: ProbabilityPpm::ONE,
        expected_fill_quantity: filled,
        p10_vwap: vwap,
        p50_vwap: vwap,
        p90_vwap: vwap,
        spread_bps: zero_bps,
        impact_bps: zero_bps,
        queue_uncertainty: ProbabilityPpm::ZERO,
        time_to_fill: *latency,
        normal_exit_cost_bps: zero_bps,
        stressed_exit_cost_bps: zero_bps,
        capacity_by_cost: BTreeMap::new(),
        as_of_block: book.as_of_block(),
    })
}

fn exact_vwap(
    notional: Option<&ExactQuoteNotional>,
    filled: Quantity,
) -> Result<Price, ExecutionError> {
    let notional = notional.ok_or(ExecutionError::InsufficientLiquidity)?;
    if filled.raw() == 0 {
        return Err(ExecutionError::InvalidQuantity);
    }
    let numerator = notional.coefficient().clone();
    let denominator = num_bigint::BigInt::from(filled.raw());
    let (quotient, remainder) = (
        &numerator / &denominator,
        &numerator - (&numerator / &denominator) * &denominator,
    );
    if remainder != num_bigint::BigInt::from(0) {
        return Err(ExecutionError::InsufficientLiquidity);
    }
    let vwap_scale = notional
        .scale()
        .checked_sub(filled.scale())
        .ok_or(ExecutionError::InsufficientLiquidity)?;
    let raw = i128::try_from(&quotient).map_err(|_| ExecutionError::InsufficientLiquidity)?;
    Price::from_raw(raw, vwap_scale).map_err(|_| ExecutionError::InsufficientLiquidity)
}
