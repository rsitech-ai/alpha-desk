use domain_types::{Direction, EntityId, ProbabilityPpm, UsdAmount};
use feature_core::{HealthAssessment, HealthState};
use market_intelligence::{
    CrowdingPosition, PainObservation, PainState, PainThresholds, classify_pain,
    crowding_components, entry_histogram,
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
    let components = crowding_components(&positions, usd(50), &health()).unwrap();
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
    let left = crowding_components(&dispersed, usd(100), &health()).unwrap();
    let right = crowding_components(&clustered, usd(100), &health()).unwrap();
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
