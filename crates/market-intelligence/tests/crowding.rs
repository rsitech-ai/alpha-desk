use domain_types::{
    BlockHeight, Direction, EntityId, FeatureSetVersion, Horizon, KnownTime, MarketId,
    ProbabilityPpm, ProtocolTime, UsdAmount,
};
use feature_core::{FeatureValue, HealthAssessment, HealthState, MissingReason};
use market_intelligence::{
    CrowdingPosition, MarketError, MarketFeatureSnapshot, PainObservation, PainState,
    PainThresholds, classify_pain, crowding_components, crowding_components_from_snapshot,
    entry_histogram, market_feature_key,
};

fn usd(dollars: i128) -> UsdAmount {
    UsdAmount::from_raw(dollars * 100_000_000, 8).unwrap()
}

fn ppm(value: u32) -> ProbabilityPpm {
    ProbabilityPpm::from_ppm(value).unwrap()
}

fn health() -> HealthAssessment {
    HealthAssessment::try_new("crowding", HealthState::Green, "synthetic").unwrap()
}

#[test]
fn originator_plus_followers_raises_saturation_without_inflating_independent_count() {
    let mut positions = vec![CrowdingPosition {
        entity_id: EntityId::new("originator").unwrap(),
        independence_weight: ProbabilityPpm::ONE,
        is_follower: false,
        post_originator: false,
        exposure: usd(100),
        entry_bps_from_mark: 0,
        funding_percentile: ppm(500_000),
        leverage_milli: 200_000,
    }];
    for index in 0..100 {
        positions.push(CrowdingPosition {
            entity_id: EntityId::new(format!("f{index}")).unwrap(),
            independence_weight: ppm(10_000),
            is_follower: true,
            post_originator: true,
            exposure: usd(1),
            entry_bps_from_mark: 5,
            funding_percentile: ppm(500_000),
            leverage_milli: 200_000,
        });
    }
    let components = crowding_from_caller_marks(&positions, observed_book_depth()).unwrap();
    assert_eq!(
        components.independent_entity_count.raw_value.raw(),
        2_000_000
    );
    assert!(components.follower_saturation.raw_value.raw() > 400_000);
    assert!(components.capacity_consumed.raw_value.raw() > 0);
}

#[test]
fn dispersed_entries_cluster_less_than_tight_cohort() {
    let dispersed: Vec<_> = (0..10)
        .map(|index| CrowdingPosition {
            entity_id: EntityId::new(format!("d{index}")).unwrap(),
            independence_weight: ProbabilityPpm::ONE,
            is_follower: false,
            post_originator: false,
            exposure: usd(10),
            entry_bps_from_mark: i64::from(index) * 80,
            funding_percentile: ppm(400_000),
            leverage_milli: 100_000,
        })
        .collect();
    let clustered: Vec<_> = (0..10)
        .map(|index| CrowdingPosition {
            entity_id: EntityId::new(format!("c{index}")).unwrap(),
            independence_weight: ProbabilityPpm::ONE,
            is_follower: false,
            post_originator: false,
            exposure: usd(10),
            entry_bps_from_mark: 2,
            funding_percentile: ppm(400_000),
            leverage_milli: 100_000,
        })
        .collect();
    let left = crowding_from_caller_marks(&dispersed, observed_book_depth()).unwrap();
    let right = crowding_from_caller_marks(&clustered, observed_book_depth()).unwrap();
    assert!(left.entry_clustering.raw_value.raw() < right.entry_clustering.raw_value.raw());
}

#[test]
fn histogram_mass_equals_scoped_position_mass() {
    let entries = vec![(10, usd(4)), (40, usd(6)), (90, usd(5))];
    let histogram = entry_histogram(&entries, 25).unwrap();
    assert_eq!(histogram.total_mass.raw(), usd(15).raw());
    let reconstructed: i128 = histogram.bins.iter().map(|bin| bin.mass.raw()).sum();
    assert_eq!(reconstructed, usd(15).raw());
}

#[test]
fn unknown_margin_stays_unknown() {
    let thresholds =
        PainThresholds::from_toml(include_str!("../../../config/features/pain-v1.toml")).unwrap();
    let state = classify_pain(
        PainObservation {
            side: Direction::Long,
            pnl_bps: Some(-80),
            liquidation_distance_bps: Some(10),
            age_micros: 10,
            margin_known: false,
        },
        &thresholds,
    );
    assert_eq!(state, PainState::Unknown);
}

fn crowding_snapshot(book: FeatureValue, fills: FeatureValue) -> MarketFeatureSnapshot {
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
        health(),
    )
    .unwrap()
}

fn invented_mark_position() -> CrowdingPosition {
    CrowdingPosition {
        entity_id: EntityId::new("invented").unwrap(),
        independence_weight: ProbabilityPpm::ONE,
        is_follower: false,
        post_originator: false,
        exposure: usd(100),
        entry_bps_from_mark: 12,
        funding_percentile: ppm(500_000),
        leverage_milli: 200_000,
    }
}

fn observed_snapshot() -> MarketFeatureSnapshot {
    crowding_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
    )
}

fn observed_book_depth() -> UsdAmount {
    usd(20_000)
}

fn crowding_from_caller_marks_snapshot(positions: &[CrowdingPosition]) -> MarketFeatureSnapshot {
    let mut snapshot = observed_snapshot();
    snapshot.values.insert(
        market_feature_key("inventory").unwrap(),
        mark_inventory_value(positions),
    );
    snapshot.provenance_hash = snapshot.compute_provenance_hash();
    snapshot
}

fn crowding_from_caller_marks(
    positions: &[CrowdingPosition],
    remaining_capacity: UsdAmount,
) -> Result<market_intelligence::CrowdingComponents, MarketError> {
    let snapshot = crowding_from_caller_marks_snapshot(positions);
    crowding_components(
        positions,
        remaining_capacity,
        snapshot.require_observed_book_and_fills()?,
    )
}

fn mark_inventory_value(positions: &[CrowdingPosition]) -> FeatureValue {
    let scale = u32::from(positions[0].exposure.scale());
    let raw: i128 = positions
        .iter()
        .map(|position| position.exposure.raw())
        .sum();
    FeatureValue::Decimal { raw, scale }
}

#[test]
fn caller_supplied_marks_with_not_observed_book_or_fills_cannot_produce_crowding_components() {
    let positions = vec![invented_mark_position()];
    let missing_book = crowding_snapshot(
        FeatureValue::Missing(MissingReason::NotObserved),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    assert!(matches!(
        missing_book.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "book" })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&missing_book, &positions, usd(50)),
        Err(MarketError::MissingInput { name: "book" })
    ));
    let missing_fills = crowding_snapshot(
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
        crowding_components_from_snapshot(&missing_fills, &positions, usd(50)),
        Err(MarketError::MissingInput { name: "fills" })
    ));
}

#[test]
fn missing_inventory_refuses_crowding_without_inventing_marks() {
    let positions = vec![invented_mark_position()];
    let observed = observed_snapshot();
    assert!(matches!(
        observed.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&observed, &positions, usd(50)),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
}

#[test]
fn constructed_accounts_with_invented_inventory_cannot_produce_crowding_scores() {
    let positions = vec![invented_mark_position()];
    let missing_inventory = crowding_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
    );
    assert!(matches!(
        missing_inventory.require_observed_book_and_fills(),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&missing_inventory, &positions, usd(50)),
        Err(MarketError::MissingInput { name: "inventory" })
    ));
    let mut mismatched = missing_inventory.clone();
    mismatched.values.insert(
        market_feature_key("inventory").unwrap(),
        FeatureValue::Decimal {
            raw: 100_000_000,
            scale: 8,
        },
    );
    mismatched.provenance_hash = mismatched.compute_provenance_hash();
    let stolen_inventory = mismatched.require_observed_book_and_fills().unwrap();
    assert!(matches!(
        crowding_components(&positions, observed_book_depth(), stolen_inventory),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "observed inventory proof does not match caller inventory",
        })
    ));
}

#[test]
fn matching_inventory_with_unrelated_book_depth_cannot_produce_crowding_scores() {
    let concentrated = vec![invented_mark_position()];
    let split = vec![
        CrowdingPosition {
            entity_id: EntityId::new("left").unwrap(),
            independence_weight: ProbabilityPpm::ONE,
            is_follower: false,
            post_originator: false,
            exposure: usd(40),
            entry_bps_from_mark: 4,
            funding_percentile: ppm(500_000),
            leverage_milli: 200_000,
        },
        CrowdingPosition {
            entity_id: EntityId::new("right").unwrap(),
            independence_weight: ProbabilityPpm::ONE,
            is_follower: true,
            post_originator: true,
            exposure: usd(60),
            entry_bps_from_mark: 40,
            funding_percentile: ppm(200_000),
            leverage_milli: 80_000,
        },
    ];
    let snapshot = crowding_from_caller_marks_snapshot(&concentrated);
    let evidence = snapshot.require_observed_book_and_fills().unwrap();
    assert_eq!(
        mark_inventory_value(&concentrated),
        mark_inventory_value(&split)
    );
    assert!(matches!(
        crowding_components(&concentrated, usd(50), evidence),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof does not match caller book depth",
        })
    ));
    assert!(matches!(
        crowding_components(&split, usd(50), evidence),
        Err(MarketError::Malformed {
            what: "book",
            reason: "observed book proof does not match caller book depth",
        })
    ));
    let admitted = crowding_components(&split, observed_book_depth(), evidence).unwrap();
    assert!(admitted.capacity_consumed.raw_value.raw() > 0);
}

#[test]
fn boolean_book_cannot_mint_crowding_proof() {
    let positions = vec![invented_mark_position()];
    let boolean_book = crowding_snapshot(FeatureValue::Boolean(true), FeatureValue::Boolean(true));
    assert!(matches!(
        boolean_book.require_observed_book_and_fills(),
        Err(MarketError::Malformed {
            what: "book",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&boolean_book, &positions, usd(50)),
        Err(MarketError::Malformed {
            what: "book",
            reason: "boolean cannot mint decimal depth",
        })
    ));
}

#[test]
fn boolean_inventory_cannot_mint_crowding_proof() {
    let positions = vec![invented_mark_position()];
    let mut boolean_inventory = observed_snapshot();
    boolean_inventory.values.insert(
        market_feature_key("inventory").unwrap(),
        FeatureValue::Boolean(true),
    );
    boolean_inventory.provenance_hash = boolean_inventory.compute_provenance_hash();
    assert!(matches!(
        boolean_inventory.require_observed_book_and_fills(),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&boolean_inventory, &positions, usd(50)),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
}

#[test]
fn empty_marks_with_observed_proof_cannot_look_like_empty_observation() {
    let mut snapshot = crowding_snapshot(
        FeatureValue::Decimal {
            raw: 20_000 * 100_000_000,
            scale: 8,
        },
        FeatureValue::Boolean(true),
    );
    snapshot.values.insert(
        market_feature_key("inventory").unwrap(),
        FeatureValue::Decimal { raw: 0, scale: 8 },
    );
    snapshot.provenance_hash = snapshot.compute_provenance_hash();
    let evidence = snapshot.require_observed_book_and_fills().unwrap();
    assert!(matches!(
        crowding_components(&[], usd(50), evidence),
        Err(MarketError::InsufficientHistory { what: "crowding" })
    ));
}
