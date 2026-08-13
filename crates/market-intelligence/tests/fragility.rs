use domain_types::{AccountId, Direction, MarketId, ProbabilityPpm, ScenarioId, UsdAmount};
use feature_core::{HealthAssessment, HealthState};
use market_intelligence::{
    FragilityScenario, SimulatedAccount, SimulatedBook, SimulatedMarginMode, simulate_fragility,
    simulate_path,
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
    SimulatedBook {
        executable_depth: usd(depth),
        health: health(state),
    }
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
