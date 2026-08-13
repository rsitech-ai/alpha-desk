use std::collections::BTreeMap;

use domain_types::{
    BasisPoints, BlockHeight, ExactQuoteNotional, FeeScheduleId, LatencyDistribution, MarketId,
    OrderSide, Price, ProbabilityPpm, Quantity, UsdAmount,
};

use crate::book::{BookHealth, OrderBook};

const UNMODELED_FEE_SCHEDULE: &str = "none";

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
    #[error("execution assumption is unsupported: {0}")]
    UnsupportedAssumption(&'static str),
    #[error("execution metric is not exact")]
    InexactMetric,
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
    if request.max_participation != ProbabilityPpm::ONE {
        return Err(ExecutionError::UnsupportedAssumption(
            "partial participation is unmodeled",
        ));
    }
    if request.exit_stress_multiplier != ProbabilityPpm::ONE {
        return Err(ExecutionError::UnsupportedAssumption(
            "stressed exit cost is unmodeled",
        ));
    }
    if request.fee_schedule_id.as_str() != UNMODELED_FEE_SCHEDULE {
        return Err(ExecutionError::UnsupportedAssumption(
            "fee schedules are unmodeled",
        ));
    }
    if !latency_is_point_estimate(latency) {
        return Err(ExecutionError::UnsupportedAssumption(
            "latency-dependent fill times are unmodeled",
        ));
    }

    let (best_bid, best_ask) = match (book.best_bid(), book.best_ask()) {
        (Some(bid), Some(ask)) => (bid.price, ask.price),
        _ => {
            return Err(ExecutionError::UnsupportedAssumption(
                "spread and impact require a two-sided book",
            ));
        }
    };

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
    let spread_bps = exact_spread_bps(best_bid, best_ask)?;
    let impact_bps = exact_impact_bps(vwap, best_bid, best_ask)?;
    Ok(ExecutionEstimate {
        fill_probability: ProbabilityPpm::ONE,
        expected_fill_quantity: filled,
        p10_vwap: vwap,
        p50_vwap: vwap,
        p90_vwap: vwap,
        spread_bps,
        impact_bps,
        queue_uncertainty: ProbabilityPpm::ZERO,
        time_to_fill: *latency,
        normal_exit_cost_bps: spread_bps,
        stressed_exit_cost_bps: spread_bps,
        capacity_by_cost: BTreeMap::new(),
        as_of_block: book.as_of_block(),
    })
}

fn latency_is_point_estimate(latency: &LatencyDistribution) -> bool {
    latency.p10_micros == latency.p50_micros
        && latency.p50_micros == latency.p90_micros
        && latency.p90_micros == latency.p99_micros
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
        return Err(ExecutionError::InexactMetric);
    }
    let vwap_scale = notional
        .scale()
        .checked_sub(filled.scale())
        .ok_or(ExecutionError::InexactMetric)?;
    let raw = i128::try_from(&quotient).map_err(|_| ExecutionError::InexactMetric)?;
    Price::from_raw(raw, vwap_scale).map_err(|_| ExecutionError::InexactMetric)
}

fn exact_spread_bps(bid: Price, ask: Price) -> Result<BasisPoints, ExecutionError> {
    if bid.scale() != ask.scale() {
        return Err(ExecutionError::InexactMetric);
    }
    let diff = ask
        .checked_sub(bid)
        .map_err(|_| ExecutionError::InexactMetric)?;
    let sum = ask
        .checked_add(bid)
        .map_err(|_| ExecutionError::InexactMetric)?;
    ratio_to_bps(diff.raw(), sum.raw(), 20_000)
}

fn exact_impact_bps(vwap: Price, bid: Price, ask: Price) -> Result<BasisPoints, ExecutionError> {
    if vwap.scale() != bid.scale() || vwap.scale() != ask.scale() {
        return Err(ExecutionError::InexactMetric);
    }
    let twice_vwap = vwap
        .raw()
        .checked_mul(2)
        .ok_or(ExecutionError::InexactMetric)?;
    let sum = ask
        .raw()
        .checked_add(bid.raw())
        .ok_or(ExecutionError::InexactMetric)?;
    let numer = twice_vwap
        .checked_sub(sum)
        .ok_or(ExecutionError::InexactMetric)?;
    ratio_to_bps(numer, sum, 10_000)
}

fn ratio_to_bps(
    numerator: i128,
    denominator: i128,
    times: i128,
) -> Result<BasisPoints, ExecutionError> {
    if denominator == 0 {
        return Err(ExecutionError::InexactMetric);
    }
    let product = num_bigint::BigInt::from(numerator) * times;
    let den = num_bigint::BigInt::from(denominator);
    let quotient = &product / &den;
    let remainder = &product - &quotient * &den;
    if remainder != num_bigint::BigInt::from(0) {
        return Err(ExecutionError::InexactMetric);
    }
    let raw = i128::try_from(&quotient).map_err(|_| ExecutionError::InexactMetric)?;
    BasisPoints::from_raw(raw, 0).map_err(|_| ExecutionError::InexactMetric)
}
