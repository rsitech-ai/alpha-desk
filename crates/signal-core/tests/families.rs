use std::collections::BTreeMap;

use domain_types::{
    AccountId, BasisPoints, BlockHeight, ClosedInterval, Decimal, Direction, EntityId,
    FeatureSetVersion, Horizon, KnownTime, MarketId, ProbabilityPpm, ProtocolTime, ScenarioId,
    UsdAmount,
};
use feature_core::{FeatureValue, HealthAssessment, HealthState, MissingReason};
use market_intelligence::{
    DimensionUnit, FragilityResult, FragilityScenario, MarketFeatureSnapshot, RegimeFeatureVector,
    RegimeModel, ScoredDimension, SimulatedAccount, SimulatedBook, SimulatedMarginMode,
    classify_regime, market_feature_key, simulate_fragility,
};
use signal_core::{
    FragilityAsymmetryEvaluator, SignalContext, SignalEvaluation, SignalEvaluator, SignalType,
    SmartCrowdDivergenceEvaluator, SmartFlowAccelerationEvaluator, suppress_missing_book_or_fills,
};

fn time(micros: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(micros).unwrap()
}

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).unwrap()
}

fn health(scope: &str, state: HealthState) -> HealthAssessment {
    HealthAssessment::try_new(scope, state, "synthetic").unwrap()
}

fn usd(dollars: i128) -> UsdAmount {
    UsdAmount::from_raw(dollars * 100_000_000, 8).unwrap()
}

fn snapshot(
    state: HealthState,
    values: BTreeMap<feature_core::FeatureKey, FeatureValue>,
) -> MarketFeatureSnapshot {
    MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        time(1_000_000),
        known(1_000_000),
        BlockHeight::new(20),
        values,
        health("market", state),
    )
    .unwrap()
}

fn green_values(
    pairs: &[(&str, FeatureValue)],
) -> BTreeMap<feature_core::FeatureKey, FeatureValue> {
    let mut values: BTreeMap<_, _> = pairs
        .iter()
        .map(|(name, value)| (market_feature_key(*name).unwrap(), value.clone()))
        .collect();
    values.insert(
        market_feature_key("book").unwrap(),
        FeatureValue::Boolean(true),
    );
    values.insert(
        market_feature_key("fills").unwrap(),
        FeatureValue::Boolean(true),
    );
    values
}

fn red_values(names: &[&str]) -> BTreeMap<feature_core::FeatureKey, FeatureValue> {
    names
        .iter()
        .map(|name| {
            (
                market_feature_key(*name).unwrap(),
                FeatureValue::Missing(MissingReason::RedDataHealth),
            )
        })
        .collect()
}

fn crowding() -> ScoredDimension {
    ScoredDimension::try_new(
        Decimal::from_raw(100_000, 0).unwrap(),
        DimensionUnit::ProbabilityPpm,
        Decimal::from_raw(100_000, 0).unwrap(),
        ClosedInterval::new(
            Decimal::from_raw(100_000, 0).unwrap(),
            Decimal::from_raw(100_000, 0).unwrap(),
        )
        .unwrap(),
        3_000_000,
        health("crowding", HealthState::Green),
        Vec::new(),
    )
    .unwrap()
}

fn regime() -> market_intelligence::RegimeAssessment {
    let model =
        RegimeModel::from_toml(include_str!("../../../config/models/market-regime-v1.toml"))
            .unwrap();
    let features =
        RegimeFeatureVector::try_new(800, 500, 800_000, 2, 200, 100_000, 50_000).unwrap();
    classify_regime(
        &model,
        Some(&features),
        None,
        time(1_000_000),
        known(1_000_000),
    )
    .unwrap()
}

fn fragility(red_book: bool, unsupported: bool) -> FragilityResult {
    let scenario = FragilityScenario::default_grid(ScenarioId::new("fam").unwrap());
    let mut account = SimulatedAccount {
        account_id: AccountId::new("a").unwrap(),
        market_id: MarketId::new("BTC").unwrap(),
        side: Direction::Long,
        notional: usd(100),
        distance_to_maintenance_bps: 80,
        margin_mode: SimulatedMarginMode::IsolatedExact,
        collateral_group: None,
    };
    if unsupported {
        account.margin_mode = SimulatedMarginMode::Unsupported;
    }
    let book = SimulatedBook::observed(
        usd(10),
        health(
            "book",
            if red_book {
                HealthState::Red
            } else {
                HealthState::Green
            },
        ),
    );
    simulate_fragility(&scenario, &[account], &book, -100).unwrap()
}

fn context(
    independent: u32,
    follower_dominated: bool,
    mm: bool,
    cost: i64,
    capacity: i128,
    red_book: bool,
    unsupported_margin: bool,
) -> SignalContext {
    let mut weights = BTreeMap::new();
    for index in 0..independent {
        weights.insert(
            EntityId::new(format!("e{index}")).unwrap(),
            ProbabilityPpm::ONE,
        );
    }
    SignalContext {
        wallet_intelligence: Vec::new(),
        independence_weights: weights,
        execution_cost_bps: BasisPoints::from_raw(i128::from(cost), 0).unwrap(),
        executable_capacity: usd(capacity),
        regime: regime(),
        crowding: crowding(),
        fragility: fragility(red_book, unsupported_margin),
        historical_support: ProbabilityPpm::from_ppm(800_000).unwrap(),
        required_health: health("required", HealthState::Green),
        book_health: health(
            "book",
            if red_book {
                HealthState::Red
            } else {
                HealthState::Green
            },
        ),
        originator_ids: vec![EntityId::new("e0").unwrap()],
        smart_intent_explained_by_mm: mm,
        follower_dominated,
    }
}

fn flow_eval() -> SmartFlowAccelerationEvaluator {
    SmartFlowAccelerationEvaluator::from_toml(include_str!(
        "../../../config/signals/v1/independent-smart-flow.toml"
    ))
    .unwrap()
}

#[test]
fn smart_flow_trigger_just_below_and_suppression() {
    let evaluator = flow_eval();
    let trigger = snapshot(
        HealthState::Green,
        green_values(&[
            (
                "smart_flow_acceleration_milli",
                FeatureValue::SignedInteger(400),
            ),
            ("historical_markout_bps", FeatureValue::SignedInteger(40)),
        ]),
    );
    match evaluator
        .evaluate(&trigger, &context(4, false, false, 5, 100, false, false))
        .unwrap()
    {
        SignalEvaluation::Candidate(signal) => {
            assert_eq!(
                signal.signal_type,
                SignalType::IndependentSmartFlowAcceleration
            );
        }
        other => panic!("expected candidate, got {other:?}"),
    }
    let below = snapshot(
        HealthState::Green,
        green_values(&[
            (
                "smart_flow_acceleration_milli",
                FeatureValue::SignedInteger(10),
            ),
            ("historical_markout_bps", FeatureValue::SignedInteger(40)),
        ]),
    );
    assert!(matches!(
        evaluator
            .evaluate(&below, &context(4, false, false, 5, 100, false, false))
            .unwrap(),
        SignalEvaluation::NoSignal { .. }
    ));
    let red = snapshot(
        HealthState::Red,
        red_values(&["smart_flow_acceleration_milli", "historical_markout_bps"]),
    );
    assert!(matches!(
        evaluator
            .evaluate(&red, &context(4, false, false, 5, 100, false, false))
            .unwrap(),
        SignalEvaluation::Suppressed { .. }
    ));
    assert!(matches!(
        evaluator
            .evaluate(&trigger, &context(1, false, false, 5, 100, false, false))
            .unwrap(),
        SignalEvaluation::NoSignal { .. }
    ));
    assert!(matches!(
        evaluator
            .evaluate(&trigger, &context(4, true, false, 5, 100, false, false))
            .unwrap(),
        SignalEvaluation::NoSignal { .. }
    ));
    assert!(matches!(
        evaluator
            .evaluate(&trigger, &context(4, false, false, 40, 100, false, false))
            .unwrap(),
        SignalEvaluation::NoSignal { .. }
    ));
}

#[test]
fn crowd_divergence_requires_opposite_flow_and_rejects_mm_explanation() {
    let evaluator = SmartCrowdDivergenceEvaluator::from_toml(include_str!(
        "../../../config/signals/v1/smart-crowd-divergence.toml"
    ))
    .unwrap();
    let snap = snapshot(
        HealthState::Green,
        green_values(&[
            ("smart_flow_usd_milli", FeatureValue::SignedInteger(50)),
            ("crowd_flow_usd_milli", FeatureValue::SignedInteger(-40)),
        ]),
    );
    assert!(matches!(
        evaluator
            .evaluate(&snap, &context(4, false, false, 5, 100, false, false))
            .unwrap(),
        SignalEvaluation::Candidate(_)
    ));
    assert!(matches!(
        evaluator
            .evaluate(&snap, &context(4, false, true, 5, 100, false, false))
            .unwrap(),
        SignalEvaluation::NoSignal { .. }
    ));
}

#[test]
fn fragility_family_suppresses_on_red_or_unsupported_margin() {
    let evaluator = FragilityAsymmetryEvaluator::from_toml(include_str!(
        "../../../config/signals/v1/liquidation-fragility-asymmetry.toml"
    ))
    .unwrap();
    let snap = snapshot(
        HealthState::Green,
        green_values(&[("placeholder", FeatureValue::SignedInteger(1))]),
    );
    assert!(matches!(
        evaluator
            .evaluate(&snap, &context(1, false, false, 5, 1, true, false))
            .unwrap(),
        SignalEvaluation::Suppressed { .. }
    ));
    assert!(matches!(
        evaluator
            .evaluate(&snap, &context(1, false, false, 5, 1, false, true))
            .unwrap(),
        SignalEvaluation::Suppressed { .. }
    ));
}

#[test]
fn only_three_v1_types_are_live_capable() {
    assert!(SignalType::IndependentSmartFlowAcceleration.can_enter_live());
    assert!(SignalType::SmartCrowdDivergence.can_enter_live());
    assert!(SignalType::LiquidationFragilityAsymmetry.can_enter_live());
    assert!(
        !SignalType::research_only("trapped-cohort")
            .unwrap()
            .can_enter_live()
    );
}

#[test]
fn missing_book_or_fills_suppresses_live_capable_families() {
    let trigger = snapshot(
        HealthState::Green,
        green_values(&[
            (
                "smart_flow_acceleration_milli",
                FeatureValue::SignedInteger(400),
            ),
            ("historical_markout_bps", FeatureValue::SignedInteger(40)),
            ("smart_flow_usd_milli", FeatureValue::SignedInteger(50)),
            ("crowd_flow_usd_milli", FeatureValue::SignedInteger(-40)),
            ("placeholder", FeatureValue::SignedInteger(1)),
        ]),
    );
    let mut missing_book = trigger.clone();
    missing_book.values.insert(
        market_feature_key("book").unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    let mut missing_fills = trigger.clone();
    missing_fills.values.insert(
        market_feature_key("fills").unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    missing_book.provenance_hash = missing_book.compute_provenance_hash();
    missing_fills.provenance_hash = missing_fills.compute_provenance_hash();

    let flow = flow_eval();
    let crowd = SmartCrowdDivergenceEvaluator::from_toml(include_str!(
        "../../../config/signals/v1/smart-crowd-divergence.toml"
    ))
    .unwrap();
    let fragility = FragilityAsymmetryEvaluator::from_toml(include_str!(
        "../../../config/signals/v1/liquidation-fragility-asymmetry.toml"
    ))
    .unwrap();
    let ctx = context(4, false, false, 5, 100, false, false);
    for snapshot in [&missing_book, &missing_fills] {
        assert!(suppress_missing_book_or_fills(snapshot).is_some());
        for evaluation in [
            flow.evaluate(snapshot, &ctx).unwrap(),
            crowd.evaluate(snapshot, &ctx).unwrap(),
            fragility.evaluate(snapshot, &ctx).unwrap(),
        ] {
            match evaluation {
                SignalEvaluation::Suppressed { reasons, .. } => {
                    assert!(
                        reasons
                            .iter()
                            .any(|reason| reason == "missing_book_or_fills")
                    );
                }
                other => panic!("missing book/fills must not emit {other:?}"),
            }
        }
    }
}
