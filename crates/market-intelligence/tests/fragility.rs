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

fn observed_snapshot() -> MarketFeatureSnapshot {
    market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
    )
}

fn snapshot_admitting(
    book: &SimulatedBook,
    accounts: &[SimulatedAccount],
) -> MarketFeatureSnapshot {
    let inventory = account_inventory_value(accounts);
    match book.observation {
        ObservationStatus::Observed => match book.health.state {
            HealthState::Green | HealthState::Amber => market_snapshot_with_health(
                FeatureValue::Decimal {
                    raw: book.executable_depth.raw(),
                    scale: u32::from(book.executable_depth.scale()),
                },
                FeatureValue::Boolean(true),
                inventory,
                book.health.clone(),
            ),
            HealthState::Red => {
                let missing = FeatureValue::Missing(MissingReason::RedDataHealth);
                let mut values = std::collections::BTreeMap::new();
                values.insert(market_feature_key("registry").unwrap(), missing.clone());
                values.insert(market_feature_key("book").unwrap(), missing.clone());
                values.insert(market_feature_key("fills").unwrap(), missing.clone());
                values.insert(market_feature_key("inventory").unwrap(), missing);
                MarketFeatureSnapshot::try_new(
                    MarketId::new("BTC").unwrap(),
                    Horizon::MINUTES_5,
                    FeatureSetVersion::new("market-v1").unwrap(),
                    ProtocolTime::from_unix_micros(1_000_000).unwrap(),
                    KnownTime::from_unix_micros(1_000_000).unwrap(),
                    BlockHeight::new(1),
                    values,
                    book.health.clone(),
                )
                .unwrap()
            }
        },
        ObservationStatus::Missing(reason) => market_snapshot_with_health(
            FeatureValue::Missing(reason),
            FeatureValue::Boolean(true),
            inventory,
            book.health.clone(),
        ),
    }
}

fn account_inventory_value(accounts: &[SimulatedAccount]) -> FeatureValue {
    if accounts.is_empty() {
        return FeatureValue::Decimal { raw: 0, scale: 8 };
    }
    let scale = u32::from(accounts[0].notional.scale());
    let raw: i128 = accounts.iter().map(|account| account.notional.raw()).sum();
    FeatureValue::Decimal { raw, scale }
}

fn fragility_from_caller_book(
    scenario: &FragilityScenario,
    accounts: &[SimulatedAccount],
    book: &SimulatedBook,
    shock_bps: i64,
) -> Result<market_intelligence::FragilityResult, MarketError> {
    let snapshot = snapshot_admitting(book, accounts);
    simulate_fragility(
        scenario,
        accounts,
        book,
        shock_bps,
        snapshot.require_observed_book_and_fills()?,
    )
}

fn simulate_bound_path(
    scenario: &FragilityScenario,
    accounts: &[SimulatedAccount],
    book: &SimulatedBook,
    shock_bps: i64,
    bound_sign: i32,
) -> Result<market_intelligence::ScenarioPathResult, MarketError> {
    let snapshot = snapshot_admitting(book, accounts);
    simulate_path(
        scenario,
        accounts,
        book,
        shock_bps,
        bound_sign,
        snapshot.require_observed_book_and_fills()?,
    )
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
    let first = simulate_bound_path(&scenario, &accounts, &book, -100, 0).unwrap();
    let second = simulate_bound_path(&scenario, &accounts, &book, -100, 0).unwrap();
    assert_eq!(first.waves.len(), 2);
    assert_eq!(first.waves[0].liquidated_accounts[0].as_str(), "a");
    assert_eq!(first.waves[1].liquidated_accounts[0].as_str(), "b");
    assert_eq!(
        first.terminal_price_change_bps,
        second.terminal_price_change_bps
    );
    assert_eq!(first.total_forced_notional, second.total_forced_notional);
    let result = fragility_from_caller_book(&scenario, &accounts, &book, -100).unwrap();
    assert_eq!(result.base.waves.len(), 2);
    assert_eq!(result.provenance_hash, {
        fragility_from_caller_book(&scenario, &accounts, &book, -100)
            .unwrap()
            .provenance_hash
    });
}

#[test]
fn no_cascade_when_shock_is_inside_buffers() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 400)];
    let path = simulate_bound_path(
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
    let red = simulate_bound_path(
        &scenario,
        &accounts,
        &book(20_000, HealthState::Red),
        -100,
        0,
    );
    assert!(matches!(
        red,
        Err(MarketError::MissingInput { name: "book" })
    ));
    let mut unsupported = account("a", Direction::Long, 100, 10);
    unsupported.margin_mode = SimulatedMarginMode::Unsupported;
    let path = simulate_bound_path(
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
    let result = fragility_from_caller_book(
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
    market_snapshot_with_health(
        book,
        fills,
        FeatureValue::Decimal {
            raw: 100 * 100_000_000,
            scale: 8,
        },
        health(HealthState::Amber),
    )
}

fn market_snapshot_with_health(
    book: FeatureValue,
    fills: FeatureValue,
    inventory: FeatureValue,
    health: HealthAssessment,
) -> MarketFeatureSnapshot {
    let mut values = std::collections::BTreeMap::new();
    values.insert(
        market_feature_key("registry").unwrap(),
        FeatureValue::Boolean(true),
    );
    values.insert(market_feature_key("book").unwrap(), book);
    values.insert(market_feature_key("fills").unwrap(), fills);
    values.insert(market_feature_key("inventory").unwrap(), inventory);
    MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        ProtocolTime::from_unix_micros(1_000_000).unwrap(),
        KnownTime::from_unix_micros(1_000_000).unwrap(),
        BlockHeight::new(1),
        values,
        health,
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
    let unrelated = observed_snapshot();
    let stolen = unrelated.require_observed_book_and_fills().unwrap();
    let error = simulate_path(&scenario, &accounts, &invented, -100, 0, stolen).unwrap_err();
    assert!(matches!(
        error,
        MarketError::Malformed {
            what: "book",
            reason: "missing book cannot carry executable depth",
        }
    ));

    let missing = SimulatedBook::missing(MissingReason::NotObserved).unwrap();
    assert!(matches!(
        simulate_bound_path(&scenario, &accounts, &missing, -100, 0),
        Err(MarketError::MissingInput { name: "book" })
    ));
    let stolen_empty = unrelated.require_observed_book_and_fills().unwrap();
    assert!(matches!(
        simulate_path(&scenario, &accounts, &missing, -100, 0, stolen_empty),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof is unrelated to missing constructed book",
        })
    ));
    assert!(matches!(
        simulate_fragility(&scenario, &accounts, &missing, -100, stolen_empty),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof is unrelated to missing constructed book",
        })
    ));
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

#[test]
fn constructed_observed_book_without_fills_cannot_produce_fragility_scores() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let book = SimulatedBook::observed(usd(20_000), health(HealthState::Green));
    let missing_fills = market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    assert!(matches!(
        missing_fills.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "fills" })
    ));
    assert!(matches!(
        simulate_fragility_from_snapshot(&missing_fills, &scenario, &accounts, -100),
        Err(MarketError::MissingInput { name: "fills" })
    ));
    let unrelated = observed_snapshot();
    let evidence = unrelated.require_observed_book_and_fills().unwrap();
    assert!(matches!(
        simulate_fragility(&scenario, &accounts, &book, -100, evidence),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof does not match simulated book health",
        })
    ));
    assert_eq!(book.observation, ObservationStatus::Observed);
}

#[test]
fn red_or_missing_constructed_book_cannot_admit_via_unrelated_proof() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let unrelated = observed_snapshot();
    let stolen = unrelated.require_observed_book_and_fills().unwrap();

    let red = book(20_000, HealthState::Red);
    assert!(matches!(
        simulate_path(&scenario, &accounts, &red, -100, 0, stolen),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof is unrelated to red constructed book",
        })
    ));
    assert!(matches!(
        simulate_fragility(&scenario, &accounts, &red, -100, stolen),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof is unrelated to red constructed book",
        })
    ));
    assert!(matches!(
        simulate_bound_path(&scenario, &accounts, &red, -100, 0),
        Err(MarketError::MissingInput { name: "book" })
    ));

    let missing = SimulatedBook::missing(MissingReason::RedDataHealth).unwrap();
    assert!(matches!(
        simulate_path(&scenario, &accounts, &missing, -100, 0, stolen),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof is unrelated to missing constructed book",
        })
    ));
}

#[test]
fn constructed_observed_book_without_fills_cannot_produce_path_scores() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let book = SimulatedBook::observed(usd(20_000), health(HealthState::Green));
    let missing_fills = market_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    assert!(matches!(
        missing_fills.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "fills" })
    ));
    let unrelated = observed_snapshot();
    let evidence = unrelated.require_observed_book_and_fills().unwrap();
    assert!(matches!(
        simulate_path(&scenario, &accounts, &book, -100, 0, evidence),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof does not match simulated book health",
        })
    ));
    let matching_health = health(HealthState::Green);
    let other_depth = market_snapshot_with_health(
        FeatureValue::Decimal {
            raw: 1_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Decimal {
            raw: 100 * 100_000_000,
            scale: 8,
        },
        matching_health.clone(),
    );
    let stolen_depth = other_depth.require_observed_book_and_fills().unwrap();
    let constructed = SimulatedBook::observed(usd(20_000), matching_health);
    assert!(matches!(
        simulate_path(&scenario, &accounts, &constructed, -100, 0, stolen_depth),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof does not match caller book depth",
        })
    ));
}

#[test]
fn constructed_accounts_with_invented_inventory_cannot_produce_path_scores() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let book = book(20_000, HealthState::Green);
    let missing_inventory = market_snapshot_with_health(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Missing(MissingReason::NotObserved),
        health(HealthState::Green),
    );
    assert!(matches!(
        missing_inventory.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    assert!(matches!(
        simulate_fragility_from_snapshot(&missing_inventory, &scenario, &accounts, -100),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    let mismatched = market_snapshot_with_health(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Decimal {
            raw: 100_000_000,
            scale: 8,
        },
        health(HealthState::Green),
    );
    let stolen_inventory = mismatched.require_observed_book_and_fills().unwrap();
    assert!(matches!(
        simulate_path(&scenario, &accounts, &book, -100, 0, stolen_inventory),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "observed inventory proof does not match caller inventory",
        })
    ));
}

#[test]
fn constructed_accounts_with_invented_inventory_cannot_produce_fragility_scores() {
    let scenario = scenario();
    let accounts = vec![account("a", Direction::Long, 100, 10)];
    let book = book(20_000, HealthState::Green);
    let missing_inventory = market_snapshot_with_health(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Missing(MissingReason::NotObserved),
        health(HealthState::Green),
    );
    assert!(matches!(
        simulate_fragility_from_snapshot(&missing_inventory, &scenario, &accounts, -100),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    let mismatched = market_snapshot_with_health(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        FeatureValue::Decimal {
            raw: 9_999 * 100_000_000,
            scale: 8,
        },
        health(HealthState::Green),
    );
    let stolen_inventory = mismatched.require_observed_book_and_fills().unwrap();
    assert!(matches!(
        simulate_fragility(&scenario, &accounts, &book, -100, stolen_inventory),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "observed inventory proof does not match caller inventory",
        })
    ));
}

#[test]
fn matching_inventory_with_unrelated_book_depth_cannot_produce_path_or_fragility_scores() {
    let scenario = scenario();
    let concentrated = vec![account("a", Direction::Long, 100, 10)];
    let split = vec![
        account("left", Direction::Long, 40, 10),
        account("right", Direction::Short, 60, 20),
    ];
    let constructed = book(20_000, HealthState::Green);
    let unrelated_depth = market_snapshot_with_health(
        FeatureValue::Decimal {
            raw: 1_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
        account_inventory_value(&concentrated),
        health(HealthState::Green),
    );
    let stolen_depth = unrelated_depth.require_observed_book_and_fills().unwrap();
    assert_eq!(
        account_inventory_value(&concentrated),
        account_inventory_value(&split)
    );
    assert!(matches!(
        simulate_path(
            &scenario,
            &concentrated,
            &constructed,
            -100,
            0,
            stolen_depth
        ),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof does not match caller book depth",
        })
    ));
    assert!(matches!(
        simulate_fragility(&scenario, &split, &constructed, -100, stolen_depth),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof does not match caller book depth",
        })
    ));
    let admitted = simulate_bound_path(&scenario, &split, &constructed, -100, 0).unwrap();
    assert_eq!(admitted.health.state, HealthState::Green);
}
