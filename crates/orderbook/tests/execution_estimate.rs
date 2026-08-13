use std::collections::BTreeMap;

use domain_types::{
    BasisPoints, BlockHeight, FeeScheduleId, LatencyDistribution, MarketId, OrderId, OrderSide,
    Price, ProbabilityPpm, Quantity, UsdAmount,
};
use orderbook::{
    BookHealth, ExecutionError, ExecutionRequest, FEE_SCHEDULE_NONE, FEE_SCHEDULE_TAKER_100BPS_V1,
    OrderBook, RestingOrder, quote_execution,
};

#[test]
fn healthy_two_sided_book_quotes_exact_vwap_spread_and_impact() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut book = OrderBook::awaiting_snapshot(market.clone(), BlockHeight::new(1));
    book.apply_snapshot(
        1,
        BlockHeight::new(20),
        vec![
            RestingOrder {
                order_id: OrderId::new("bid-1").unwrap(),
                side: OrderSide::Buy,
                price: price("99"),
                remaining: qty("10"),
                sequence: 1,
            },
            RestingOrder {
                order_id: OrderId::new("ask-1").unwrap(),
                side: OrderSide::Sell,
                price: price("101"),
                remaining: qty("2"),
                sequence: 2,
            },
        ],
    );
    let latency = point_latency();
    let estimate = quote_execution(&book, &buy_request(&market, "2"), &latency).unwrap();
    assert_eq!(estimate.expected_fill_quantity, qty("2"));
    assert_eq!(estimate.p50_vwap, price("101"));
    assert_eq!(estimate.spread_bps, bps(200));
    assert_eq!(estimate.impact_bps, bps(100));
    assert_eq!(estimate.normal_exit_cost_bps, bps(200));
    assert_eq!(estimate.stressed_exit_cost_bps, bps(200));
    assert_eq!(estimate.p10_vwap, estimate.p50_vwap);
    assert_eq!(estimate.p90_vwap, estimate.p50_vwap);
    assert_eq!(
        estimate.capacity_by_cost,
        BTreeMap::from([(bps(100), usd("202"))])
    );
    assert_eq!(estimate.fill_probability, ProbabilityPpm::ONE);
    assert_eq!(estimate.time_to_fill, latency);

    let too_large = quote_execution(&book, &buy_request(&market, "3"), &latency);
    assert_eq!(too_large, Err(ExecutionError::InsufficientLiquidity));
}

#[test]
fn two_level_visible_vwap_is_exact_when_spread_divides() {
    let market = MarketId::new("perp:ETH").unwrap();
    let mut book = OrderBook::awaiting_snapshot(market.clone(), BlockHeight::new(1));
    book.apply_snapshot(
        1,
        BlockHeight::new(20),
        vec![
            RestingOrder {
                order_id: OrderId::new("bid-1").unwrap(),
                side: OrderSide::Buy,
                price: price("60"),
                remaining: qty("10"),
                sequence: 1,
            },
            RestingOrder {
                order_id: OrderId::new("ask-1").unwrap(),
                side: OrderSide::Sell,
                price: price("100"),
                remaining: qty("1"),
                sequence: 2,
            },
            RestingOrder {
                order_id: OrderId::new("ask-2").unwrap(),
                side: OrderSide::Sell,
                price: price("102"),
                remaining: qty("1"),
                sequence: 3,
            },
        ],
    );
    let estimate = quote_execution(&book, &buy_request(&market, "2"), &point_latency()).unwrap();
    assert_eq!(estimate.p50_vwap, price("101"));
    assert_eq!(estimate.spread_bps, bps(5_000));
    assert_eq!(estimate.impact_bps, bps(2_625));
    assert_eq!(
        estimate.capacity_by_cost,
        BTreeMap::from([(bps(2_500), usd("100")), (bps(2_625), usd("202"))])
    );
}

#[test]
fn exact_fee_participation_and_stress_are_applied_without_inventing_vwap_bands() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut book = OrderBook::awaiting_snapshot(market.clone(), BlockHeight::new(1));
    book.apply_snapshot(
        1,
        BlockHeight::new(20),
        vec![
            RestingOrder {
                order_id: OrderId::new("bid-1").unwrap(),
                side: OrderSide::Buy,
                price: price("99"),
                remaining: qty("10"),
                sequence: 1,
            },
            RestingOrder {
                order_id: OrderId::new("ask-1").unwrap(),
                side: OrderSide::Sell,
                price: price("101"),
                remaining: qty("2"),
                sequence: 2,
            },
        ],
    );

    let mut fee = buy_request(&market, "2");
    fee.fee_schedule_id = FeeScheduleId::new(FEE_SCHEDULE_TAKER_100BPS_V1).unwrap();
    let with_fee = quote_execution(&book, &fee, &point_latency()).unwrap();
    assert_eq!(with_fee.p50_vwap, price("101"));
    assert_eq!(with_fee.normal_exit_cost_bps, bps(300));
    assert_eq!(with_fee.stressed_exit_cost_bps, bps(300));

    let mut half = buy_request(&market, "2");
    half.max_participation = ProbabilityPpm::from_ppm(500_000).unwrap();
    let half_fill = quote_execution(&book, &half, &point_latency()).unwrap();
    assert_eq!(half_fill.expected_fill_quantity, qty("1"));
    assert_eq!(
        half_fill.capacity_by_cost,
        BTreeMap::from([(bps(100), usd("101"))])
    );

    let mut stressed = buy_request(&market, "2");
    stressed.exit_stress_multiplier = ProbabilityPpm::from_ppm(500_000).unwrap();
    let stressed_quote = quote_execution(&book, &stressed, &point_latency()).unwrap();
    assert_eq!(stressed_quote.normal_exit_cost_bps, bps(200));
    assert_eq!(stressed_quote.stressed_exit_cost_bps, bps(100));
}

#[test]
fn unmodeled_assumptions_and_inexact_metrics_are_refused() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut book = OrderBook::awaiting_snapshot(market.clone(), BlockHeight::new(1));
    book.apply_snapshot(
        1,
        BlockHeight::new(20),
        vec![RestingOrder {
            order_id: OrderId::new("ask-1").unwrap(),
            side: OrderSide::Sell,
            price: price("100"),
            remaining: qty("1"),
            sequence: 1,
        }],
    );
    let one_sided = quote_execution(&book, &buy_request(&market, "1"), &point_latency());
    assert_eq!(
        one_sided,
        Err(ExecutionError::UnsupportedAssumption(
            "spread and impact require a two-sided book",
        ))
    );

    book.apply_snapshot(
        1,
        BlockHeight::new(21),
        vec![
            RestingOrder {
                order_id: OrderId::new("bid-1").unwrap(),
                side: OrderSide::Buy,
                price: price("99"),
                remaining: qty("1"),
                sequence: 1,
            },
            RestingOrder {
                order_id: OrderId::new("ask-1").unwrap(),
                side: OrderSide::Sell,
                price: price("100"),
                remaining: qty("1"),
                sequence: 2,
            },
        ],
    );
    let inexact = quote_execution(&book, &buy_request(&market, "1"), &point_latency());
    assert_eq!(inexact, Err(ExecutionError::InexactMetric));

    book.apply_snapshot(
        1,
        BlockHeight::new(22),
        vec![
            RestingOrder {
                order_id: OrderId::new("bid-1").unwrap(),
                side: OrderSide::Buy,
                price: price("99"),
                remaining: qty("1"),
                sequence: 1,
            },
            RestingOrder {
                order_id: OrderId::new("ask-1").unwrap(),
                side: OrderSide::Sell,
                price: price("101"),
                remaining: qty("1"),
                sequence: 2,
            },
        ],
    );
    let mut fee = buy_request(&market, "1");
    fee.fee_schedule_id = FeeScheduleId::new("default").unwrap();
    assert_eq!(
        quote_execution(&book, &fee, &point_latency()),
        Err(ExecutionError::UnsupportedAssumption(
            "unknown fee schedule"
        ))
    );

    let mut participation = buy_request(&market, "1");
    participation.max_participation = ProbabilityPpm::from_ppm(500_000).unwrap();
    assert_eq!(
        quote_execution(&book, &participation, &point_latency()),
        Err(ExecutionError::InexactMetric)
    );

    let distributed = LatencyDistribution::new(1, 2, 3, 4).unwrap();
    assert_eq!(
        quote_execution(&book, &buy_request(&market, "1"), &distributed),
        Err(ExecutionError::UnsupportedAssumption(
            "latency-dependent fill times are unmodeled",
        ))
    );

    book.apply_diff(
        2,
        BlockHeight::new(23),
        orderbook::BookDiff::Add {
            order: RestingOrder {
                order_id: OrderId::new("ask-1").unwrap(),
                side: OrderSide::Sell,
                price: price("103"),
                remaining: qty("1"),
                sequence: 3,
            },
        },
    );
    assert!(matches!(book.health(), BookHealth::Red { .. }));
    assert_eq!(
        quote_execution(&book, &buy_request(&market, "1"), &point_latency()),
        Err(ExecutionError::BookNotHealthy)
    );
}

fn buy_request(market: &MarketId, quantity: &str) -> ExecutionRequest {
    ExecutionRequest {
        market_id: market.clone(),
        side: OrderSide::Buy,
        quantity: qty(quantity),
        max_participation: ProbabilityPpm::ONE,
        fee_schedule_id: FeeScheduleId::new(FEE_SCHEDULE_NONE).unwrap(),
        exit_stress_multiplier: ProbabilityPpm::ONE,
    }
}

fn point_latency() -> LatencyDistribution {
    LatencyDistribution::new(1, 1, 1, 1).unwrap()
}

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 0).unwrap()
}

fn qty(value: &str) -> Quantity {
    Quantity::parse_at_scale(value, 0).unwrap()
}

fn bps(raw: i128) -> BasisPoints {
    BasisPoints::from_raw(raw, 0).unwrap()
}

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, 0).unwrap()
}
