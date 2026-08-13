use domain_types::{
    BlockHeight, FeeScheduleId, LatencyDistribution, MarketId, OrderId, OrderSide, Price,
    ProbabilityPpm, Quantity,
};
use orderbook::{
    BookHealth, ExecutionError, ExecutionRequest, OrderBook, RestingOrder, quote_execution,
};

#[test]
fn healthy_book_quotes_exact_visible_vwap_and_red_book_is_refused() {
    let market = MarketId::new("perp:BTC").unwrap();
    let mut book = OrderBook::awaiting_snapshot(market.clone(), BlockHeight::new(1));
    book.apply_snapshot(
        1,
        BlockHeight::new(20),
        vec![
            RestingOrder {
                order_id: OrderId::new("ask-1").unwrap(),
                side: OrderSide::Sell,
                price: price("100"),
                remaining: qty("1"),
                sequence: 1,
            },
            RestingOrder {
                order_id: OrderId::new("ask-2").unwrap(),
                side: OrderSide::Sell,
                price: price("102"),
                remaining: qty("1"),
                sequence: 2,
            },
        ],
    );
    let latency = LatencyDistribution::new(1, 2, 3, 4).unwrap();
    let estimate = quote_execution(
        &book,
        &ExecutionRequest {
            market_id: market.clone(),
            side: OrderSide::Buy,
            quantity: qty("2"),
            max_participation: ProbabilityPpm::ONE,
            fee_schedule_id: FeeScheduleId::new("default").unwrap(),
            exit_stress_multiplier: ProbabilityPpm::ONE,
        },
        &latency,
    )
    .unwrap();
    assert_eq!(estimate.expected_fill_quantity, qty("2"));
    assert_eq!(estimate.p50_vwap, price("101"));
    assert_eq!(estimate.fill_probability, ProbabilityPpm::ONE);
    assert_eq!(estimate.time_to_fill, latency);

    let too_large = quote_execution(
        &book,
        &ExecutionRequest {
            market_id: market.clone(),
            side: OrderSide::Buy,
            quantity: qty("3"),
            max_participation: ProbabilityPpm::ONE,
            fee_schedule_id: FeeScheduleId::new("default").unwrap(),
            exit_stress_multiplier: ProbabilityPpm::ONE,
        },
        &latency,
    );
    assert_eq!(too_large, Err(ExecutionError::InsufficientLiquidity));

    book.apply_diff(
        3,
        BlockHeight::new(21),
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
    let red = quote_execution(
        &book,
        &ExecutionRequest {
            market_id: market,
            side: OrderSide::Buy,
            quantity: qty("1"),
            max_participation: ProbabilityPpm::ONE,
            fee_schedule_id: FeeScheduleId::new("default").unwrap(),
            exit_stress_multiplier: ProbabilityPpm::ONE,
        },
        &latency,
    );
    assert_eq!(red, Err(ExecutionError::BookNotHealthy));
}

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 0).unwrap()
}

fn qty(value: &str) -> Quantity {
    Quantity::parse_at_scale(value, 0).unwrap()
}
