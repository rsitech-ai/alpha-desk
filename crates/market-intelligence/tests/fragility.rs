use domain_types::{
    AccountId, BlockHeight, Direction, FeatureSetVersion, Horizon, KnownTime, MarketId,
    ProbabilityPpm, ProtocolTime, ScenarioId, UsdAmount,
};
use feature_core::{FeatureValue, HealthAssessment, HealthState, MissingReason};
use market_intelligence::{
    FragilityScenario, MarketError, MarketFeatureSnapshot, ObservationStatus, SimulatedAccount,
    SimulatedBook, SimulatedMarginMode, market_feature_key, simulate_fragility,
    simulate_fragility_from_snapshot, simulate_path,
};

fn usd(dollars: i128) -> UsdAmount {
    UsdAmount::from_raw(dollars * 100_000_000, 8).unwrap()
}

fn health(state: HealthState) -> HealthAssessment {
    HealthAssessment::try_new("book", state, "synthetic").unwrap()
}

fn account(id: &str, side: Direction, dollars: i128, distance: i64) -> SimulatedAccount {
    SimulatedAccount {
        account_id: AccountId::new(id).unwrap(),
        market_id: MarketId::new("BTC").unwrap(),
        side,
        notional: usd(dollars),
        distance_to_maintenance_bps: distance,
        margin_mode: SimulatedMarginMode::IsolatedExact,
        collateral_group: None,
    }
}

fn book(depth: i128, state: HealthState) -> SimulatedBook {
    SimulatedBook::observed(usd(depth), health(state))
}

fn scenario() -> FragilityScenario {
    FragilityScenario::from_toml(include_str!("../../../config/features/fragility-v1.toml"))
        .unwrap()
}

#[test]
fn long_cascade_is_deterministic_and_stops_after_second_wave() {
    let scenario = scenario();
    let accounts = vec![
        account("a", Direction::Long, 100, 100),
        account("b", Direction::Long, 50, 150),
        account("c", Direction::Long, 20, 400),
    ];
    let book = book(20_000, HealthState::Green);
    let first = simulate_path(&scenario, &accounts, &book, -100, 0).unwrap();
    let second = simulate_path(&scenario, &accounts, &book, -100, 0).unwrap();
    assert_eq!(first.waves.len(), 2);
    assert_eq!(first.waves[0].liquidated_accounts[0].as_str(), "a");
    assert_eq!(first.waves[1].liquidated_accounts[0].as_str(), "b");
    assert_eq!(
        first.terminal_price_change_bps,
        second.terminal_price_change_bps
    );
    assert_eq!(first.total_forced_notional, second.total_forced_notional);
    let result = simulate_fragility(&scenario, &accounts, &book, -100).unwrap();
    assert_eq!(result.base.waves.len(), 2);
    assert_eq!(result.provenance_hash, {
        simulate_fragility(&scenario, &accounts, &book, -100)
            .unwrap()
            .provenance_hash
    });
}

#[test]
fn no_cascade_when_shock_is_inside_buffers() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 400)];
    let path = simulate_path(
        &scenario,
        &accounts,
        &book(20_000, HealthState::Green),
        -25,
        0,
    )
    .unwrap();
    assert!(path.waves.is_empty());
}

#[test]
fn red_book_and_unsupported_margin_fail_closed() {
    let scenario = FragilityScenario::default_grid(ScenarioId::new("x").unwrap());
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let red = simulate_path(
        &scenario,
        &accounts,
        &book(20_000, HealthState::Red),
        -100,
        0,
    )
    .unwrap();
    assert_eq!(red.health.state, HealthState::Red);
    let mut unsupported = account("a", Direction::Long, 100, 10);
    unsupported.margin_mode = SimulatedMarginMode::Unsupported;
    let path = simulate_path(
        &scenario,
        &[unsupported],
        &book(20_000, HealthState::Green),
        -100,
        0,
    )
    .unwrap();
    assert_eq!(path.health.state, HealthState::Red);
}

#[test]
fn portfolio_uncertainty_separates_low_and_high_paths() {
    let scenario = scenario();
    let mut account = account("a", Direction::Long, 100, 80);
    account.margin_mode = SimulatedMarginMode::PortfolioUncertain { band_bps: 40 };
    let result = simulate_fragility(
        &scenario,
        &[account],
        &book(20_000, HealthState::Green),
        -100,
    )
    .unwrap();
    assert!(result.low.total_forced_notional.raw() <= result.high.total_forced_notional.raw());
    let _ = ProbabilityPpm::ONE;
}

fn market_snapshot(book: FeatureValue, fills: FeatureValue) -> MarketFeatureSnapshot {
    let mut values = std::collections::BTreeMap::new();
    values.insert(
        market_feature_key("registry").unwrap(),
        FeatureValue::Boolean(true),
    );
    values.insert(market_feature_key("book").unwrap(), book);
    values.insert(market_feature_key("fills").unwrap(), fills);
    MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        ProtocolTime::from_unix_micros(1_000_000).unwrap(),
        KnownTime::from_unix_micros(1_000_000).unwrap(),
        BlockHeight::new(1),
        values,
        health(HealthState::Amber),
    )
    .unwrap()
}

#[test]
fn missing_book_does_not_invent_depth_or_emit_waves() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let invented = SimulatedBook {
        executable_depth: usd(20_000),
        health: health(HealthState::Amber),
        observation: ObservationStatus::Missing(MissingReason::NotObserved),
    };
    let error = simulate_path(&scenario, &accounts, &invented, -100, 0).unwrap_err();
    assert!(matches!(
        error,
        MarketError::Malformed {
            what: "book",
            reason: "missing book cannot carry executable depth",
        }
    ));

    let missing = SimulatedBook::missing(MissingReason::NotObserved).unwrap();
    let path = simulate_path(&scenario, &accounts, &missing, -100, 0).unwrap();
    assert!(path.waves.is_empty());
    assert_eq!(path.total_forced_notional.raw(), 0);
    assert_eq!(path.health.state, HealthState::Red);
    let result = simulate_fragility(&scenario, &accounts, &missing, -100).unwrap();
    assert!(result.missing_inputs.iter().any(|item| item == "book"));
    assert_eq!(result.confidence, ProbabilityPpm::ZERO);
    assert!(result.base.waves.is_empty());
}

#[test]
fn snapshot_without_book_or_fills_refuses_fragility() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let missing_both = market_snapshot(
        FeatureValue::Missing(MissingReason::NotObserved),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    assert!(matches!(
        simulate_fragility_from_snapshot(&missing_both, &scenario, &accounts, -100),
        Err(MarketError::MissingInput { name: "book" })
    ));
    let missing_fills = market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    assert!(matches!(
        simulate_fragility_from_snapshot(&missing_fills, &scenario, &accounts, -100),
        Err(MarketError::MissingInput { name: "fills" })
    ));
}
