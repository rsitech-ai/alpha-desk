use std::collections::BTreeMap;

use domain_types::{
    AccountId, BlockHeight, EvidenceId, FeatureSetVersion, KnownTime, ProtocolTime,
};
use feature_core::{
    EvidenceKind, EvidenceRef, FeatureError, FeatureKey, FeatureSnapshot, FeatureSubject,
    FeatureValue, HealthState, asof,
};

fn time(label: &str) -> ProtocolTime {
    let micros = match label {
        "t1" => 1_000_000,
        "t2" => 2_000_000,
        "t3" => 3_000_000,
        _ => panic!("unsupported fixture time {label}"),
    };
    ProtocolTime::from_unix_micros(micros).unwrap()
}

fn known(label: &str) -> KnownTime {
    let micros = match label {
        "t1" => 1_000_000,
        "t2" => 2_000_000,
        "t3" => 3_000_000,
        _ => panic!("unsupported fixture time {label}"),
    };
    KnownTime::from_unix_micros(micros).unwrap()
}

fn fixture_rows() -> Vec<FeatureSnapshot> {
    let key = FeatureKey::try_new("wallet", "twr", 1).unwrap();
    let mut values = BTreeMap::new();
    values.insert(key, FeatureValue::SignedInteger(15));
    vec![
        FeatureSnapshot::try_new(
            FeatureSubject::Account(AccountId::new("acct-a").unwrap()),
            FeatureSetVersion::new("wallet-v1").unwrap(),
            time("t1"),
            known("t3"),
            None,
            1,
            values,
            BlockHeight::new(10),
            HealthState::Green,
            None,
        )
        .unwrap(),
    ]
}

#[test]
fn asof_join_respects_effective_and_known_time() {
    let rows = fixture_rows();
    assert!(asof(&rows, time("t2"), known("t2")).is_none());
    assert_eq!(asof(&rows, time("t2"), known("t3")).unwrap().revision, 1);
}

#[test]
fn evidence_ref_rejects_empty_id_and_zero_hash() {
    let effective = time("t1");
    let known_at = known("t1");
    let zero = [0_u8; 32];
    let digest = [1_u8; 32];
    assert!(matches!(
        EvidenceRef::try_new(
            EvidenceKind::CanonicalEvent,
            EvidenceId::new("ev-1").unwrap(),
            zero,
            effective,
            known_at,
        ),
        Err(FeatureError::ZeroContentHash)
    ));
    assert!(
        EvidenceRef::try_new(
            EvidenceKind::StateSnapshot,
            EvidenceId::new("ev-1").unwrap(),
            digest,
            time("t3"),
            known("t1"),
        )
        .is_err()
    );
}

#[test]
fn red_health_fails_closed_unless_values_are_missing() {
    let key = FeatureKey::try_new("wallet", "twr", 1).unwrap();
    let mut values = BTreeMap::new();
    values.insert(key, FeatureValue::SignedInteger(1));
    let error = FeatureSnapshot::try_new(
        FeatureSubject::Account(AccountId::new("acct-a").unwrap()),
        FeatureSetVersion::new("wallet-v1").unwrap(),
        time("t1"),
        known("t1"),
        None,
        1,
        values,
        BlockHeight::new(1),
        HealthState::Red,
        None,
    )
    .unwrap_err();
    assert!(matches!(error, FeatureError::Malformed { .. }));
}

#[test]
fn superseded_row_is_hidden_after_cutoff() {
    let key = FeatureKey::try_new("wallet", "twr", 1).unwrap();
    let mut values = BTreeMap::new();
    values.insert(key.clone(), FeatureValue::SignedInteger(1));
    let first = FeatureSnapshot::try_new(
        FeatureSubject::Account(AccountId::new("acct-a").unwrap()),
        FeatureSetVersion::new("wallet-v1").unwrap(),
        time("t1"),
        known("t1"),
        Some(known("t3")),
        1,
        values.clone(),
        BlockHeight::new(1),
        HealthState::Green,
        None,
    )
    .unwrap();
    values.insert(key, FeatureValue::SignedInteger(2));
    let second = FeatureSnapshot::try_new(
        FeatureSubject::Account(AccountId::new("acct-a").unwrap()),
        FeatureSetVersion::new("wallet-v1").unwrap(),
        time("t1"),
        known("t3"),
        None,
        2,
        values,
        BlockHeight::new(2),
        HealthState::Green,
        None,
    )
    .unwrap();
    let rows = [first, second];
    assert_eq!(asof(&rows, time("t2"), known("t1")).unwrap().revision, 1);
    assert_eq!(asof(&rows, time("t2"), known("t3")).unwrap().revision, 2);
}
