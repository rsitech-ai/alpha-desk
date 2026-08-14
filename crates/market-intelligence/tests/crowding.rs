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
    let components = crowding_from_caller_marks(&positions, usd(50)).unwrap();
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
    let left = crowding_from_caller_marks(&dispersed, usd(100)).unwrap();
    let right = crowding_from_caller_marks(&clustered, usd(100)).unwrap();
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

fn crowding_from_caller_marks(
    positions: &[CrowdingPosition],
    remaining_capacity: UsdAmount,
) -> Result<market_intelligence::CrowdingComponents, MarketError> {
    let mut snapshot = observed_snapshot();
    snapshot.values.insert(
        market_feature_key("inventory").unwrap(),
        mark_inventory_value(positions),
    );
    snapshot.provenance_hash = snapshot.compute_provenance_hash();
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
fn missing_book_or_fills_refuses_crowding_without_inventing_marks() {
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
        crowding_components(&positions, usd(50), stolen_inventory),
        Err(MarketError::Malformed {
            what: "inventory",
            reason: "observed inventory proof does not match caller inventory",
        })
    ));
}

#[test]
fn boolean_book_cannot_mint_crowding_proof() {
    let positions = vec![invented_mark_position()];
    let boolean_book = crowding_snapshot(FeatureValue::Boolean(true), FeatureValue::Boolean(true));
    assert!(matches!(
        boolean_book.require_observed_book_and_fills(),
        Err(MarketError::Malformed {
            what: "observation",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        crowding_components_from_snapshot(&boolean_book, &positions, usd(50)),
        Err(MarketError::Malformed {
            what: "observation",
            reason: "boolean cannot mint decimal depth",
        })
    ));
}
