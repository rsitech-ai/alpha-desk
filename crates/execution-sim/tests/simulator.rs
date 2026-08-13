use domain_types::{
    BasisPoints, Direction, FeeRate, FundingRate, KnownTime, MarketId, Price, ProtocolTime,
    Quantity, SignalId, UsdAmount,
};
use execution_sim::{
    BookLevel, BookSnapshot, CostModel, ExitPolicy, FailureInjection, FeeSchedule, FundingSchedule,
    ImpactModel, LatencyAssumptions, LatencyModel, OrderPolicy, PortfolioLimits, SignalSnapshot,
    SimError, SimulationEvent, SimulationRequest, SlippageModel, run,
};

fn ts(micros: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(micros).unwrap()
}

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).unwrap()
}

fn px(value: &str) -> Price {
    Price::parse_at_scale(value, 8).unwrap()
}

fn qty(value: &str) -> Quantity {
    Quantity::parse_at_scale(value, 8).unwrap()
}

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, 8).unwrap()
}

fn fee(value: &str) -> FeeRate {
    value.parse().unwrap()
}

fn bps(value: &str) -> BasisPoints {
    value.parse().unwrap()
}

fn book(at: i64, bid: &str, bid_qty: &str, ask: &str, ask_qty: &str) -> BookSnapshot {
    BookSnapshot::new(
        ts(at),
        known(at),
        vec![BookLevel::new(px(bid), qty(bid_qty)).unwrap()],
        vec![BookLevel::new(px(ask), qty(ask_qty)).unwrap()],
    )
    .unwrap()
}

fn cost_model(
    taker: &str,
    slippage: &str,
    impact: &str,
    delay: u64,
    funding_interval: u64,
    funding_rate: &str,
) -> CostModel {
    CostModel::new(
        "synthetic-cost-v1",
        FeeSchedule::new(fee(taker), fee("0")).unwrap(),
        FundingSchedule::new(
            funding_interval,
            funding_rate.parse::<FundingRate>().unwrap(),
        )
        .unwrap(),
        SlippageModel::new(bps(slippage)).unwrap(),
        ImpactModel::new(bps(impact)).unwrap(),
        LatencyAssumptions::new(
            LatencyModel::Fixed {
                delay_micros: delay,
            },
            LatencyModel::Fixed { delay_micros: 0 },
            LatencyModel::Fixed { delay_micros: 0 },
            5_000_000,
        ),
    )
    .unwrap()
}

fn signal(size: &str) -> SignalSnapshot {
    SignalSnapshot::new(
        SignalId::new("sig-synthetic").unwrap(),
        MarketId::new("BTC").unwrap(),
        Direction::Long,
        ts(0),
        known(0),
        qty(size),
    )
    .unwrap()
}

fn request(
    books: Vec<BookSnapshot>,
    costs: CostModel,
    policy: OrderPolicy,
    hold: u64,
) -> SimulationRequest {
    SimulationRequest::new(
        known(10_000_000_000),
        signal("1"),
        books,
        costs,
        policy,
        ExitPolicy::time_hold(hold),
        PortfolioLimits::new(usd("1000000"), 1_000_000).unwrap(),
        FailureInjection::None,
        7,
    )
    .unwrap()
}

#[test]
fn market_round_trip_matches_hand_calculation() {
    let books = vec![
        book(0, "100", "10", "101", "10"),
        book(1_000_000, "100", "10", "101", "10"),
    ];
    let result = run(&request(
        books,
        cost_model("0.001", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        1_000_000,
    ))
    .unwrap();

    assert_eq!(result.entry_vwap().unwrap(), px("101"));
    assert_eq!(result.exit_vwap().unwrap(), px("100"));
    assert_eq!(result.entry_fees(), usd("0.101"));
    assert_eq!(result.exit_fees(), usd("0.1"));
    assert_eq!(result.funding(), usd("0"));
    assert_eq!(result.slippage(), usd("0"));
    assert_eq!(result.impact(), usd("0"));
    assert_eq!(result.net_pnl(), usd("-1.201"));
    assert!(result.spread_cost() > usd("0"));
    let hash = result.trace_hash();
    let again = run(&request(
        vec![
            book(0, "100", "10", "101", "10"),
            book(1_000_000, "100", "10", "101", "10"),
        ],
        cost_model("0.001", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        1_000_000,
    ))
    .unwrap();
    assert_eq!(again.trace_hash(), hash);
}

#[test]
fn higher_fees_do_not_improve_net_pnl() {
    let books = || {
        vec![
            book(0, "100", "10", "101", "10"),
            book(1_000_000, "100", "10", "101", "10"),
        ]
    };
    let cheap = run(&request(
        books(),
        cost_model("0.001", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        1_000_000,
    ))
    .unwrap();
    let expensive = run(&request(
        books(),
        cost_model("0.002", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        1_000_000,
    ))
    .unwrap();
    assert!(expensive.net_pnl() < cheap.net_pnl());
}

#[test]
fn higher_latency_does_not_improve_net_pnl_when_book_moves_against() {
    let fast_books = vec![
        book(0, "100", "10", "101", "10"),
        book(1_000_000, "100", "10", "101", "10"),
    ];
    let slow_books = vec![
        book(0, "100", "10", "101", "10"),
        book(250_000, "102", "10", "103", "10"),
        book(1_250_000, "100", "10", "101", "10"),
    ];
    let fast = run(&request(
        fast_books,
        cost_model("0.001", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        1_000_000,
    ))
    .unwrap();
    let slow = run(&request(
        slow_books,
        cost_model("0.001", "0", "0", 250_000, 3_600_000_000, "0"),
        OrderPolicy::market(),
        1_000_000,
    ))
    .unwrap();
    assert!(slow.net_pnl() < fast.net_pnl());
}

#[test]
fn market_partial_fill_records_missed_quantity() {
    let books = vec![
        book(0, "100", "10", "101", "0.4"),
        book(1_000_000, "100", "10", "101", "10"),
    ];
    let result = run(&request(
        books,
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        1_000_000,
    ))
    .unwrap();
    assert_eq!(result.filled_quantity(), qty("0.4"));
    assert_eq!(result.missed_quantity(), qty("0.6"));
    assert!(
        result
            .events()
            .iter()
            .any(|event| matches!(event, SimulationEvent::PartialFill { .. }))
    );
}

#[test]
fn ioc_non_crossing_limit_does_not_fill() {
    let books = vec![book(0, "100", "10", "101", "10")];
    let result = run(&request(
        books,
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::ioc_limit(px("100")),
        1_000_000,
    ))
    .unwrap();
    assert_eq!(result.filled_quantity(), qty("0"));
    assert_eq!(result.missed_quantity(), qty("1"));
    assert_eq!(result.net_pnl(), usd("0"));
}

#[test]
fn gtc_queue_sample_is_deterministic_for_a_fixed_seed() {
    let books = vec![book(0, "100", "10", "101", "1")];
    let policy = OrderPolicy::gtc(px("101"), 250_000, 250_000).unwrap();
    let first = run(&request(
        books.clone(),
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        policy,
        1_000_000,
    ))
    .unwrap();
    let second = run(&request(
        books,
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        policy,
        1_000_000,
    ))
    .unwrap();
    assert_eq!(first.trace_hash(), second.trace_hash());
    assert_eq!(first.filled_quantity(), qty("0.25"));
}

#[test]
fn alo_that_would_take_is_rejected() {
    let books = vec![book(0, "100", "10", "101", "10")];
    let error = run(&request(
        books,
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::alo(px("101")),
        1_000_000,
    ))
    .unwrap_err();
    assert_eq!(error.reason_code(), "execution_sim.order_rejected");
}

#[test]
fn future_book_is_refused() {
    let error = SimulationRequest::new(
        known(1_000),
        signal("1"),
        vec![book(2_000, "100", "10", "101", "10")],
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        ExitPolicy::time_hold(1_000),
        PortfolioLimits::new(usd("1000000"), 1_000_000).unwrap(),
        FailureInjection::None,
        1,
    )
    .unwrap_err();
    assert_eq!(
        error,
        SimError::FutureData {
            field: "book.known_at",
        }
    );
}

#[test]
fn future_signal_is_refused() {
    let late = SignalSnapshot::new(
        SignalId::new("sig-late").unwrap(),
        MarketId::new("BTC").unwrap(),
        Direction::Long,
        ts(5_000),
        known(5_000),
        qty("1"),
    )
    .unwrap();
    let error = SimulationRequest::new(
        known(1_000),
        late,
        vec![book(0, "100", "10", "101", "10")],
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        ExitPolicy::time_hold(1_000),
        PortfolioLimits::new(usd("1000000"), 1_000_000).unwrap(),
        FailureInjection::None,
        1,
    )
    .unwrap_err();
    assert_eq!(
        error,
        SimError::FutureData {
            field: "signal.known_at",
        }
    );
}

#[test]
fn missing_cost_model_version_fails_closed() {
    let error = CostModel::new(
        "  ",
        FeeSchedule::new(fee("0"), fee("0")).unwrap(),
        FundingSchedule::new(3_600_000_000, "0".parse().unwrap()).unwrap(),
        SlippageModel::new(bps("0")).unwrap(),
        ImpactModel::new(bps("0")).unwrap(),
        LatencyAssumptions::new(
            LatencyModel::Fixed { delay_micros: 0 },
            LatencyModel::Fixed { delay_micros: 0 },
            LatencyModel::Fixed { delay_micros: 0 },
            1,
        ),
    )
    .unwrap_err();
    assert_eq!(
        error,
        SimError::UnmodeledCost {
            component: "cost_model_version",
        }
    );
}

#[test]
fn zero_funding_interval_fails_closed() {
    let error = FundingSchedule::new(0, "0".parse().unwrap()).unwrap_err();
    assert_eq!(
        error,
        SimError::UnmodeledCost {
            component: "funding_interval",
        }
    );
}

#[test]
fn funding_is_applied_for_each_closed_interval() {
    let books = vec![
        book(0, "100", "10", "101", "10"),
        book(7_200_000_000, "100", "10", "101", "10"),
    ];
    let result = run(&request(
        books,
        cost_model("0", "0", "0", 0, 3_600_000_000, "0.0001"),
        OrderPolicy::market(),
        7_200_000_000,
    ))
    .unwrap();
    assert_eq!(result.funding(), usd("0.0202"));
    assert!(
        result
            .events()
            .iter()
            .any(|event| matches!(event, SimulationEvent::FundingApplied { .. }))
    );
}

#[test]
fn stale_book_fails_closed() {
    let books = vec![book(0, "100", "10", "101", "10")];
    let costs = CostModel::new(
        "synthetic-cost-v1",
        FeeSchedule::new(fee("0"), fee("0")).unwrap(),
        FundingSchedule::new(3_600_000_000, "0".parse().unwrap()).unwrap(),
        SlippageModel::new(bps("0")).unwrap(),
        ImpactModel::new(bps("0")).unwrap(),
        LatencyAssumptions::new(
            LatencyModel::Fixed {
                delay_micros: 1_000,
            },
            LatencyModel::Fixed { delay_micros: 0 },
            LatencyModel::Fixed { delay_micros: 0 },
            0,
        ),
    )
    .unwrap();
    let error = run(&request(books, costs, OrderPolicy::market(), 1_000)).unwrap_err();
    assert_eq!(error, SimError::StaleBook);
}

#[test]
fn injected_reject_does_not_place_a_live_order() {
    let error = SimulationRequest::new(
        known(10_000_000_000),
        signal("1"),
        vec![book(0, "100", "10", "101", "10")],
        cost_model("0", "0", "0", 0, 3_600_000_000, "0"),
        OrderPolicy::market(),
        ExitPolicy::time_hold(1_000),
        PortfolioLimits::new(usd("1000000"), 1_000_000).unwrap(),
        FailureInjection::RejectOrder,
        1,
    )
    .and_then(|request| run(&request))
    .unwrap_err();
    assert_eq!(
        error,
        SimError::OrderRejected {
            reason: "injected_reject",
        }
    );
}

#[test]
fn json_fixture_round_trip_is_executable() {
    let path = fixture("market-order-partial-fill.json");
    let encoded = std::fs::read(&path).unwrap();
    let request = SimulationRequest::from_json(&encoded).unwrap();
    let result = run(&request).unwrap();
    assert_eq!(result.filled_quantity(), qty("0.4"));
    assert_eq!(result.missed_quantity(), qty("0.6"));
}

#[test]
fn json_fixture_without_impact_fails_closed() {
    let encoded = std::fs::read(fixture("unmodeled-impact.json")).unwrap();
    let error = SimulationRequest::from_json(&encoded).unwrap_err();
    assert_eq!(error.reason_code(), "execution_sim.invalid_request");
}

fn fixture(name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/simulation")
        .join(name)
}
