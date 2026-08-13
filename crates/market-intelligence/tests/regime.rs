use domain_types::{KnownTime, ProtocolTime};
use market_intelligence::{RegimeFeatureVector, RegimeModel, RegimeName, classify_regime};
use serde::Deserialize;
use wallet_intelligence::ApplicabilitySupport;

fn model() -> RegimeModel {
    RegimeModel::from_toml(include_str!("../../../config/models/market-regime-v1.toml")).unwrap()
}

fn time(label: i64) -> (ProtocolTime, KnownTime) {
    (
        ProtocolTime::from_unix_micros(label).unwrap(),
        KnownTime::from_unix_micros(label).unwrap(),
    )
}

#[derive(Deserialize)]
struct Fixture {
    trend_milli: i64,
    realized_vol_milli: i64,
    liquidity_quality_ppm: u32,
    funding_bps: i64,
    oi_change_milli: i64,
    correlation_stress_ppm: u32,
    liquidation_intensity_ppm: u32,
    expected_top: Option<String>,
}

fn load(name: &str) -> Fixture {
    let path = format!("../../../fixtures/models/market-regime-v1/{name}.json");
    let text = match name {
        "uptrend" => include_str!("../../../fixtures/models/market-regime-v1/uptrend.json"),
        "downtrend" => include_str!("../../../fixtures/models/market-regime-v1/downtrend.json"),
        "range" => include_str!("../../../fixtures/models/market-regime-v1/range.json"),
        "high-volatility" => {
            include_str!("../../../fixtures/models/market-regime-v1/high-volatility.json")
        }
        "liquidity-stress" => {
            include_str!("../../../fixtures/models/market-regime-v1/liquidity-stress.json")
        }
        "ambiguous" => include_str!("../../../fixtures/models/market-regime-v1/ambiguous.json"),
        _ => panic!("unknown fixture {name} at {path}"),
    };
    serde_json::from_str(text).unwrap()
}

fn features(fixture: &Fixture) -> RegimeFeatureVector {
    RegimeFeatureVector::try_new(
        fixture.trend_milli,
        fixture.realized_vol_milli,
        fixture.liquidity_quality_ppm,
        fixture.funding_bps,
        fixture.oi_change_milli,
        fixture.correlation_stress_ppm,
        fixture.liquidation_intensity_ppm,
    )
    .unwrap()
}

#[test]
fn named_regimes_match_fixtures_and_probabilities_sum() {
    let model = model();
    let (effective, known) = time(1_000_000);
    for name in [
        "uptrend",
        "downtrend",
        "range",
        "high-volatility",
        "liquidity-stress",
    ] {
        let fixture = load(name);
        let assessment =
            classify_regime(&model, Some(&features(&fixture)), None, effective, known).unwrap();
        let sum: u64 = assessment
            .probabilities
            .values()
            .map(|value| u64::from(value.ppm()))
            .sum();
        assert_eq!(sum, 1_000_000);
        if let Some(expected) = fixture.expected_top {
            assert_eq!(
                assessment.dominant().unwrap().as_wire_name(),
                expected.as_str()
            );
        }
    }
}

#[test]
fn future_observations_do_not_rewrite_historical_regime() {
    let model = model();
    let first_features = features(&load("range"));
    let later_features = features(&load("uptrend"));
    let historical = classify_regime(
        &model,
        Some(&first_features),
        None,
        ProtocolTime::from_unix_micros(1_000_000).unwrap(),
        KnownTime::from_unix_micros(1_000_000).unwrap(),
    )
    .unwrap();
    let _later = classify_regime(
        &model,
        Some(&later_features),
        Some(&historical),
        ProtocolTime::from_unix_micros(10_000_000).unwrap(),
        KnownTime::from_unix_micros(10_000_000).unwrap(),
    )
    .unwrap();
    let replay = classify_regime(
        &model,
        Some(&first_features),
        None,
        ProtocolTime::from_unix_micros(1_000_000).unwrap(),
        KnownTime::from_unix_micros(1_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(historical.probabilities, replay.probabilities);
    assert_eq!(historical.dominant().unwrap(), RegimeName::QuietRange);
}

#[test]
fn missing_inputs_are_unsupported_not_optimistic() {
    let model = model();
    let (effective, known) = time(1_000_000);
    let assessment = classify_regime(&model, None, None, effective, known).unwrap();
    assert_eq!(assessment.support, ApplicabilitySupport::Unsupported);
}
