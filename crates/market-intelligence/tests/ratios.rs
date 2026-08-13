use domain_types::{
    BlockHeight, CohortId, Direction, EntityId, Horizon, KnownTime, ProbabilityPpm, ProtocolTime,
    UsdAmount,
};
use feature_core::{HealthAssessment, HealthState};
use market_intelligence::{
    CohortDefinition, CohortMember, CohortPredicate, PositionedMember, RatioMeasure, RatioScope,
    RatioUnit, compute_ratio,
};
use wallet_intelligence::StyleClass;

fn usd(dollars: i128) -> UsdAmount {
    UsdAmount::from_raw(dollars.checked_mul(100_000_000).unwrap(), 8).unwrap()
}

fn time(micros: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(micros).unwrap()
}

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).unwrap()
}

fn ppm(value: u32) -> ProbabilityPpm {
    ProbabilityPpm::from_ppm(value).unwrap()
}

fn health() -> HealthAssessment {
    HealthAssessment::try_new("ratio", HealthState::Green, "synthetic").unwrap()
}

fn member(
    id: &str,
    style: StyleClass,
    known_at: KnownTime,
    effective_at: ProtocolTime,
) -> CohortMember {
    CohortMember {
        entity_id: EntityId::new(id).unwrap(),
        independence_weight: ProbabilityPpm::ONE,
        skill_probability: Some(ProbabilityPpm::ONE),
        style: Some((style, ProbabilityPpm::ONE)),
        intent: None,
        equity_percentile: None,
        leverage: None,
        regime: None,
        known_at,
        effective_at,
    }
}

fn positioned(
    id: &str,
    style: StyleClass,
    side: Direction,
    exposure: UsdAmount,
    known_at: KnownTime,
    effective_at: ProtocolTime,
) -> PositionedMember {
    let zero = UsdAmount::from_raw(0, exposure.scale()).unwrap();
    PositionedMember {
        member: member(id, style, known_at, effective_at),
        side,
        gross_exposure: exposure,
        new_risk_flow: zero,
        high_conviction_flow: zero,
        liquidation_weighted_exposure: zero,
        taker_opening_flow: zero,
    }
}

fn longs_and_whale() -> (CohortDefinition, CohortDefinition, Vec<PositionedMember>) {
    let longs = CohortDefinition::try_new(
        CohortId::new("net-long").unwrap(),
        1,
        CohortPredicate::StyleProbabilityAtLeast {
            style: StyleClass::DirectionalDiscretionary,
            value: ppm(500_000),
        },
        Vec::new(),
    )
    .unwrap();
    let shorts = CohortDefinition::try_new(
        CohortId::new("net-short").unwrap(),
        1,
        CohortPredicate::StyleProbabilityAtLeast {
            style: StyleClass::SwingTrading,
            value: ppm(500_000),
        },
        Vec::new(),
    )
    .unwrap();
    let mut universe = vec![positioned(
        "whale-short",
        StyleClass::SwingTrading,
        Direction::Short,
        usd(100_000_000),
        known(2_000_000),
        time(1_000_000),
    )];
    for index in 0..5_000 {
        universe.push(positioned(
            &format!("long-{index}"),
            StyleClass::DirectionalDiscretionary,
            Direction::Long,
            usd(1),
            known(2_000_000),
            time(1_000_000),
        ));
    }
    (longs, shorts, universe)
}

#[test]
fn entity_count_ratio_differs_from_exposure_ratio_and_exposes_denominator() {
    let (longs, shorts, universe) = longs_and_whale();
    let count_scope = RatioScope::try_new(
        longs.cohort_id.clone(),
        shorts.cohort_id.clone(),
        RatioMeasure::IndependentEntityCount,
        RatioUnit::Count,
        Horizon::MINUTES_5,
        Vec::new(),
        time(1_000_000),
        known(2_000_000),
        BlockHeight::new(10),
    )
    .unwrap();
    let exposure_scope = RatioScope::try_new(
        longs.cohort_id.clone(),
        shorts.cohort_id.clone(),
        RatioMeasure::GrossExposure,
        RatioUnit::Usd,
        Horizon::MINUTES_5,
        Vec::new(),
        time(1_000_000),
        known(2_000_000),
        BlockHeight::new(10),
    )
    .unwrap();
    let count = compute_ratio(count_scope, &longs, &shorts, &universe, health()).unwrap();
    let exposure = compute_ratio(exposure_scope, &longs, &shorts, &universe, health()).unwrap();
    assert_eq!(count.numerator.raw(), 5_000_000_000);
    assert_eq!(count.denominator.raw(), 1_000_000);
    assert_eq!(exposure.numerator.raw(), usd(5_000).raw());
    assert_eq!(exposure.denominator.raw(), usd(100_000_000).raw());
    assert_ne!(count.value, exposure.value);
    assert!(exposure.denominator.raw() > 0);
}

#[test]
fn empty_denominator_fails_closed() {
    let longs = CohortDefinition::try_new(
        CohortId::new("longs").unwrap(),
        1,
        CohortPredicate::SkillProbabilityAtLeast(ProbabilityPpm::ONE),
        Vec::new(),
    )
    .unwrap();
    let empty = CohortDefinition::try_new(
        CohortId::new("empty").unwrap(),
        1,
        CohortPredicate::SkillProbabilityAtLeast(ProbabilityPpm::ONE),
        vec!["only".to_owned()],
    )
    .unwrap();
    let universe = vec![positioned(
        "only",
        StyleClass::DirectionalDiscretionary,
        Direction::Long,
        usd(10),
        known(1_000_000),
        time(1_000_000),
    )];
    let scope = RatioScope::try_new(
        longs.cohort_id.clone(),
        empty.cohort_id.clone(),
        RatioMeasure::GrossExposure,
        RatioUnit::Usd,
        Horizon::MINUTES_5,
        Vec::new(),
        time(1_000_000),
        known(1_000_000),
        BlockHeight::new(1),
    )
    .unwrap();
    let error = compute_ratio(scope, &longs, &empty, &universe, health()).unwrap_err();
    assert!(matches!(
        error,
        market_intelligence::MarketError::EmptyDenominator
    ));
}

#[test]
fn as_of_membership_does_not_leak_future_knowledge() {
    let longs = CohortDefinition::try_new(
        CohortId::new("late").unwrap(),
        1,
        CohortPredicate::StyleProbabilityAtLeast {
            style: StyleClass::DirectionalDiscretionary,
            value: ppm(1),
        },
        Vec::new(),
    )
    .unwrap();
    let shorts = CohortDefinition::try_new(
        CohortId::new("base").unwrap(),
        1,
        CohortPredicate::StyleProbabilityAtLeast {
            style: StyleClass::SwingTrading,
            value: ppm(1),
        },
        Vec::new(),
    )
    .unwrap();
    let universe = vec![
        positioned(
            "base",
            StyleClass::SwingTrading,
            Direction::Short,
            usd(10),
            known(1_000_000),
            time(1_000_000),
        ),
        positioned(
            "late",
            StyleClass::DirectionalDiscretionary,
            Direction::Long,
            usd(10),
            known(3_000_000),
            time(1_000_000),
        ),
    ];
    let scope = RatioScope::try_new(
        longs.cohort_id.clone(),
        shorts.cohort_id.clone(),
        RatioMeasure::IndependentEntityCount,
        RatioUnit::Count,
        Horizon::MINUTES_5,
        Vec::new(),
        time(2_000_000),
        known(2_000_000),
        BlockHeight::new(2),
    )
    .unwrap();
    let result = compute_ratio(scope, &longs, &shorts, &universe, health()).unwrap();
    assert_eq!(result.numerator.raw(), 0);
}

#[test]
fn crate_source_has_no_venue_gross_long_short_ratio() {
    let lib = include_str!("../src/lib.rs");
    let ratio = include_str!("../src/ratio.rs");
    assert!(!lib.contains("venue_gross_long_short_ratio"));
    assert!(!ratio.contains("venue_gross_long_short_ratio"));
}

#[test]
fn red_health_fails_closed() {
    let (longs, shorts, universe) = longs_and_whale();
    let scope = RatioScope::try_new(
        longs.cohort_id.clone(),
        shorts.cohort_id.clone(),
        RatioMeasure::GrossExposure,
        RatioUnit::Usd,
        Horizon::MINUTES_5,
        Vec::new(),
        time(1_000_000),
        known(2_000_000),
        BlockHeight::new(10),
    )
    .unwrap();
    let red = HealthAssessment::try_new("ratio", HealthState::Red, "book_red").unwrap();
    assert!(matches!(
        compute_ratio(scope, &longs, &shorts, &universe, red),
        Err(market_intelligence::MarketError::RedDataHealth { .. })
    ));
}

#[test]
fn cohort_hashes_are_stable() {
    let first = CohortDefinition::try_new(
        CohortId::new("smart").unwrap(),
        1,
        CohortPredicate::SkillProbabilityAtLeast(ppm(800_000)),
        Vec::new(),
    )
    .unwrap();
    let second = CohortDefinition::try_new(
        CohortId::new("smart").unwrap(),
        1,
        CohortPredicate::SkillProbabilityAtLeast(ppm(800_000)),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(first.definition_hash, second.definition_hash);
}
