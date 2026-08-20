use domain_types::{EntityId, ProbabilityPpm, UsdAmount};
use domain_types::{Horizon, KnownTime, MarketId, ProtocolTime};
use market_intelligence::CrossAssetInputs;
use market_intelligence::{
    ExactVectorIndex, MemoryEntry, MemoryQuery, MemorySupport, VECTOR_DIMENSION_COUNT, VectorIndex,
    VectorManifest, cross_asset_features,
};

fn zeros() -> [i64; VECTOR_DIMENSION_COUNT] {
    [0; VECTOR_DIMENSION_COUNT]
}

fn entry(
    episode: &str,
    effective: i64,
    known: i64,
    values: [i64; VECTOR_DIMENSION_COUNT],
) -> MemoryEntry {
    MemoryEntry::try_new(
        MarketId::new("BTC").unwrap(),
        episode,
        ProtocolTime::from_unix_micros(effective).unwrap(),
        KnownTime::from_unix_micros(known).unwrap(),
        values,
        Some(12),
    )
    .unwrap()
}

#[test]
fn query_cannot_match_current_episode_or_future_window() {
    let mut index = ExactVectorIndex::new(VectorManifest::v1());
    index
        .insert(entry("current", 8_000_000, 8_000_000, zeros()))
        .unwrap();
    index
        .insert(entry("future", 12_000_000, 12_000_000, zeros()))
        .unwrap();
    index
        .insert(entry("past", 1_000_000, 1_000_000, zeros()))
        .unwrap();
    let result = index
        .query(&MemoryQuery {
            market_id: MarketId::new("BTC").unwrap(),
            episode_id: "current".to_owned(),
            effective_at: ProtocolTime::from_unix_micros(10_000_000).unwrap(),
            known_at: KnownTime::from_unix_micros(10_000_000).unwrap(),
            horizon: Horizon::SECONDS_5,
            values_milli: zeros(),
            limit: 5,
            support_distance_milli: 10_000,
        })
        .unwrap();
    assert_eq!(result.matches.len(), 1);
    assert_eq!(result.matches[0].episode_id, "past");
}

#[test]
fn adjacent_snapshots_count_as_one_independent_episode() {
    let mut index = ExactVectorIndex::new(VectorManifest::v1());
    for offset in 0..10 {
        let mut values = zeros();
        values[0] = i64::from(offset);
        index
            .insert(entry(
                "episode-a",
                1_000_000 + i64::from(offset) * 1_000,
                1_000_000 + i64::from(offset) * 1_000,
                values,
            ))
            .unwrap();
    }
    let result = index
        .query(&MemoryQuery {
            market_id: MarketId::new("BTC").unwrap(),
            episode_id: "query".to_owned(),
            effective_at: ProtocolTime::from_unix_micros(20_000_000).unwrap(),
            known_at: KnownTime::from_unix_micros(20_000_000).unwrap(),
            horizon: Horizon::SECOND_1,
            values_milli: zeros(),
            limit: 10,
            support_distance_milli: u128::MAX,
        })
        .unwrap();
    assert_eq!(result.independent_episode_count, 1);
    assert_eq!(result.matches.len(), 1);
}

#[test]
fn outside_support_is_labeled() {
    let mut index = ExactVectorIndex::new(VectorManifest::v1());
    let mut far = zeros();
    far[0] = 50_000;
    index
        .insert(entry("past", 1_000_000, 1_000_000, far))
        .unwrap();
    let result = index
        .query(&MemoryQuery {
            market_id: MarketId::new("BTC").unwrap(),
            episode_id: "query".to_owned(),
            effective_at: ProtocolTime::from_unix_micros(10_000_000).unwrap(),
            known_at: KnownTime::from_unix_micros(10_000_000).unwrap(),
            horizon: Horizon::SECOND_1,
            values_milli: zeros(),
            limit: 3,
            support_distance_milli: 10,
        })
        .unwrap();
    assert!(matches!(
        result.support,
        MemorySupport::OutsideSupport { .. }
    ));
}

#[test]
fn cross_asset_net_cannot_exceed_gross() {
    let error = cross_asset_features(&CrossAssetInputs {
        entity_id: EntityId::new("e").unwrap(),
        from_market: MarketId::new("BTC").unwrap(),
        to_market: MarketId::new("ETH").unwrap(),
        rotated_notional: UsdAmount::from_raw(1, 8).unwrap(),
        simultaneous_deleveraging: true,
        lead_lag_micros: 1_000,
        shared_collateral: true,
        beta_neutral_notional: UsdAmount::from_raw(1, 8).unwrap(),
        correlation_stress_ppm: ProbabilityPpm::from_ppm(100_000).unwrap(),
        gross_risk: UsdAmount::from_raw(10, 8).unwrap(),
        net_risk: UsdAmount::from_raw(40, 8).unwrap(),
    })
    .unwrap_err();
    assert!(matches!(
        error,
        market_intelligence::MarketError::Malformed { .. }
    ));
}
