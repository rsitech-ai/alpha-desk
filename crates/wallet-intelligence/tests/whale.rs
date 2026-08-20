use domain_types::{ProbabilityPpm, UsdAmount};
use wallet_intelligence::{DEFAULT_USD_SCALE, IntelligenceError, WhaleComponents, WhaleInputs};

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, DEFAULT_USD_SCALE).unwrap()
}

#[test]
fn whale_components_are_visible_and_fail_closed_on_zero_denominators() {
    let inputs = WhaleInputs {
        equity: usd("1000"),
        cohort_equities: vec![usd("100"), usd("500"), usd("1000"), usd("2000")],
        position_notional: usd("50"),
        market_open_interest: usd("200"),
        delta_notional: usd("10"),
        rolling_market_volume: usd("100"),
        executable_depth_25bps: usd("40"),
        account_equity: usd("1000"),
        equity_floor: usd("1"),
        vulnerable_notional: usd("20"),
        depth_to_liquidation: usd("80"),
    };
    let components = WhaleComponents::try_from_inputs(&inputs, None, None, None).unwrap();
    assert_eq!(components.capital_percentile.ppm(), 750_000);
    assert_eq!(components.position_oi_share.raw(), 25_000_000);
    assert!(components.influence_score.is_none());
    assert!(components.skill_probability.is_none());
    let mut bad = inputs.clone();
    bad.market_open_interest = usd("0");
    assert!(matches!(
        WhaleComponents::try_from_inputs(&bad, None, None, None),
        Err(IntelligenceError::DivisionByZero)
    ));
}

#[test]
fn optional_scores_do_not_create_a_hidden_canonical_blend() {
    let inputs = WhaleInputs {
        equity: usd("10"),
        cohort_equities: vec![usd("10")],
        position_notional: usd("1"),
        market_open_interest: usd("10"),
        delta_notional: usd("1"),
        rolling_market_volume: usd("10"),
        executable_depth_25bps: usd("10"),
        account_equity: usd("10"),
        equity_floor: usd("1"),
        vulnerable_notional: usd("1"),
        depth_to_liquidation: usd("10"),
    };
    let skill = ProbabilityPpm::from_ppm(900_000).unwrap();
    let components = WhaleComponents::try_from_inputs(&inputs, None, Some(skill), None).unwrap();
    assert_eq!(components.skill_probability, Some(skill));
    assert!(components.influence_score.is_none());
    assert!(components.fragility_score.is_none());
}
