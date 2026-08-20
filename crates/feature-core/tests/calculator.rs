use std::collections::BTreeMap;

use domain_types::{AccountId, BlockHeight, FeatureSetVersion, KnownTime, ProtocolTime};
use feature_core::{
    FeatureCalculator, FeatureContext, FeatureDelta, FeatureError, FeatureKey, FeatureSubject,
    FeatureValue, HealthState, MissingReason, PitSnapshotCalculator, asof, require_asof,
};

fn time(micros: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(micros).unwrap()
}

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).unwrap()
}

fn reconstructed_delta() -> FeatureDelta {
    let mut values = BTreeMap::new();
    values.insert(
        FeatureKey::try_new("wallet", "reconstructed", 1).unwrap(),
        FeatureValue::Boolean(true),
    );
    values.insert(
        FeatureKey::try_new("wallet", "equity_usd", 1).unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    values.insert(
        FeatureKey::try_new("wallet", "fills", 1).unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    FeatureDelta::try_new(
        FeatureSubject::Account(AccountId::new("acct-a").unwrap()),
        values,
    )
    .unwrap()
}

#[test]
fn empty_delta_fails_closed() {
    let error = FeatureDelta::try_new(
        FeatureSubject::Account(AccountId::new("acct-a").unwrap()),
        BTreeMap::new(),
    )
    .unwrap_err();
    assert!(matches!(error, FeatureError::Malformed { .. }));
}

#[test]
fn calculator_appends_pit_snapshots_without_inventing_fills() {
    let mut calculator = PitSnapshotCalculator::new();
    let ctx = FeatureContext::try_new(
        FeatureSetVersion::new("synthetic-replay-v1").unwrap(),
        time(1_000_000),
        known(1_000_000),
        BlockHeight::new(1),
        HealthState::Amber,
    )
    .unwrap();
    let emitted = calculator
        .on_delta(&reconstructed_delta(), &ctx, None)
        .unwrap();
    assert_eq!(emitted.len(), 1);
    assert_eq!(emitted[0].revision, 1);
    assert_eq!(emitted[0].data_health, HealthState::Amber);
    assert!(matches!(
        emitted[0]
            .values
            .get(&FeatureKey::try_new("wallet", "fills", 1).unwrap()),
        Some(FeatureValue::Missing(MissingReason::NotObserved))
    ));

    let later = FeatureContext::try_new(
        FeatureSetVersion::new("synthetic-replay-v1").unwrap(),
        time(2_000_000),
        known(2_000_000),
        BlockHeight::new(2),
        HealthState::Amber,
    )
    .unwrap();
    calculator
        .on_delta(&reconstructed_delta(), &later, None)
        .unwrap();
    assert_eq!(
        asof(calculator.snapshots(), time(1_000_000), known(1_000_000))
            .unwrap()
            .revision,
        1
    );
    assert_eq!(
        require_asof(calculator.snapshots(), time(2_000_000), known(2_000_000))
            .unwrap()
            .revision,
        2
    );
}
