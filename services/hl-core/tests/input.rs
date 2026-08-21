use hl_core::{
    BLOCK_COMMITTED_SUBJECT, BLOCK_PROVISIONAL_SUBJECT, CanonicalSubject, CoreInputSubject,
    HEALTH_SOURCE_SUBJECT, SNAPSHOT_ACCOUNT_SUBJECT, SNAPSHOT_ECOSYSTEM_SUBJECT,
    SNAPSHOT_MARKET_SUBJECT,
};

#[test]
fn committed_consumer_rejects_snapshot_and_provisional_subjects() {
    for subject in [
        BLOCK_PROVISIONAL_SUBJECT,
        SNAPSHOT_ACCOUNT_SUBJECT,
        SNAPSHOT_MARKET_SUBJECT,
        SNAPSHOT_ECOSYSTEM_SUBJECT,
    ] {
        let error = CanonicalSubject::parse(subject).expect_err(subject);
        assert_eq!(error.reason_code(), "core.jetstream_provisional");
    }
}

#[test]
fn core_input_accepts_ws_lanes_without_committed_watermark() {
    let snapshots = [
        (SNAPSHOT_ACCOUNT_SUBJECT, CoreInputSubject::SnapshotAccount),
        (SNAPSHOT_MARKET_SUBJECT, CoreInputSubject::SnapshotMarket),
        (
            SNAPSHOT_ECOSYSTEM_SUBJECT,
            CoreInputSubject::SnapshotEcosystem,
        ),
        (HEALTH_SOURCE_SUBJECT, CoreInputSubject::HealthSource),
        (
            BLOCK_PROVISIONAL_SUBJECT,
            CoreInputSubject::BlockProvisional,
        ),
    ];
    for (wire, expected) in snapshots {
        let parsed = CoreInputSubject::parse(wire).expect(wire);
        assert_eq!(parsed, expected);
        assert_eq!(parsed.as_str(), wire);
        assert!(!parsed.can_advance_committed_watermark());
    }
    let committed = CoreInputSubject::parse(BLOCK_COMMITTED_SUBJECT).expect("committed");
    assert!(committed.can_advance_committed_watermark());
    assert!(CoreInputSubject::SnapshotAccount.is_provisional_lane());
    assert!(!CoreInputSubject::HealthSource.is_provisional_lane());
}

#[test]
fn committed_block_subject_still_parses() {
    assert_eq!(
        CanonicalSubject::parse(BLOCK_COMMITTED_SUBJECT).unwrap(),
        CanonicalSubject::BlockCommitted
    );
}
