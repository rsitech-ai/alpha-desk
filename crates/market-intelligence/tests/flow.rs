use domain_types::{BasisPoints, EntityId, ProbabilityPpm, UsdAmount};
use market_intelligence::{
    LiquidityNormalizer, RiskFlowKind, SmartFlowContribution, accumulate_smart_flow,
    informed_taker_aggression,
};

fn usd(dollars: i128) -> UsdAmount {
    UsdAmount::from_raw(dollars * 100_000_000, 8).unwrap()
}

fn ppm(value: u32) -> ProbabilityPpm {
    ProbabilityPpm::from_ppm(value).unwrap()
}

fn contribution(
    id: &str,
    kind: RiskFlowKind,
    dollars: i128,
    independence: u32,
    freshness: u32,
) -> SmartFlowContribution {
    SmartFlowContribution::try_new(
        EntityId::new(id).unwrap(),
        kind,
        usd(dollars),
        ProbabilityPpm::ONE,
        BasisPoints::from_raw(50, 0).unwrap(),
        ProbabilityPpm::ONE,
        ProbabilityPpm::ONE,
        ppm(independence),
        ProbabilityPpm::ONE,
        ppm(freshness),
        ProbabilityPpm::ONE,
    )
    .unwrap()
}

fn normalizer() -> LiquidityNormalizer {
    LiquidityNormalizer::from_toml(include_str!("../../../config/features/market-flow-v1.toml"))
        .unwrap()
}

#[test]
fn opening_long_is_positive_new_risk_and_static_is_zero() {
    let open = contribution("a", RiskFlowKind::OpenLong, 100, 1_000_000, 1_000_000);
    assert!(open.signed_new_risk_usd().unwrap().raw() > 0);
    let close_short = contribution("b", RiskFlowKind::CloseShort, 100, 1_000_000, 1_000_000);
    assert_eq!(close_short.signed_new_risk_usd().unwrap().raw(), 0);
    assert!(close_short.close_risk_usd().unwrap().raw() > 0);
    let reduce = contribution("c", RiskFlowKind::ReduceLong, 100, 1_000_000, 1_000_000);
    assert!(reduce.signed_new_risk_usd().unwrap().raw() < 0);
    let static_pos = contribution("d", RiskFlowKind::Static, 100, 1_000_000, 1_000_000);
    assert_eq!(static_pos.signed_new_risk_usd().unwrap().raw(), 0);
}

#[test]
fn twenty_followers_contribute_one_independent_vote() {
    let mut contributions = vec![contribution(
        "originator",
        RiskFlowKind::OpenLong,
        100,
        1_000_000,
        1_000_000,
    )];
    for index in 0..20 {
        contributions.push(contribution(
            &format!("f{index}"),
            RiskFlowKind::OpenLong,
            10,
            50_000,
            1_000_000,
        ));
    }
    let aggregate = accumulate_smart_flow(&contributions, &normalizer(), &[]).unwrap();
    assert_eq!(aggregate.independent_votes_milli, 2_000_000);
}

#[test]
fn splitting_one_entity_across_linked_accounts_preserves_weighted_flow() {
    let single = vec![contribution(
        "one",
        RiskFlowKind::OpenLong,
        100,
        1_000_000,
        1_000_000,
    )];
    let split = vec![
        contribution("one-a", RiskFlowKind::OpenLong, 40, 400_000, 1_000_000),
        contribution("one-b", RiskFlowKind::OpenLong, 60, 600_000, 1_000_000),
    ];
    let left = accumulate_smart_flow(&single, &normalizer(), &[]).unwrap();
    let right = accumulate_smart_flow(&split, &normalizer(), &[]).unwrap();
    assert_eq!(left.independent_votes_milli, right.independent_votes_milli);
}

#[test]
fn worse_freshness_never_increases_weighted_flow() {
    let fresh = accumulate_smart_flow(
        &[contribution(
            "a",
            RiskFlowKind::OpenLong,
            100,
            1_000_000,
            1_000_000,
        )],
        &normalizer(),
        &[],
    )
    .unwrap();
    let stale = accumulate_smart_flow(
        &[contribution(
            "a",
            RiskFlowKind::OpenLong,
            100,
            1_000_000,
            100_000,
        )],
        &normalizer(),
        &[],
    )
    .unwrap();
    assert!(stale.raw_usd.raw().abs() <= fresh.raw_usd.raw().abs());
}

#[test]
fn aggression_does_not_conflate_open_long_and_close_short() {
    let totals = informed_taker_aggression(&[
        contribution("a", RiskFlowKind::OpenLong, 80, 1_000_000, 1_000_000),
        contribution("b", RiskFlowKind::CloseShort, 80, 1_000_000, 1_000_000),
    ])
    .unwrap();
    assert!(totals.open_long.raw() > 0);
    assert!(totals.close_short.raw() > 0);
    assert_eq!(totals.open_short.raw(), 0);
}
