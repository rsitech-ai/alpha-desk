use bytes::Bytes;
use domain_types::SourceId;
use hl_capture::spool::{
    DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolError, SpoolReader, SpoolRotationPolicy,
};
use hl_protocol::{
    ObservationClass, ParseWarning, ReceiveTimestamps, SourceCursor, SourceObservation,
};
use std::time::Duration;
use tempfile::TempDir;

fn config(root: &TempDir, source_version: &str) -> SourceSpoolConfig {
    SourceSpoolConfig::try_new(
        root.path().join("primary-node"),
        SourceId::new("primary-node").unwrap(),
        source_version,
        "spool-v1",
        [0x31; 32],
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
    )
    .unwrap()
}

#[test]
fn source_spool_rotates_with_a_hash_chained_manifest() {
    let root = TempDir::new().unwrap();
    let mut rotating = SourceSpool::open(
        SourceSpoolConfig::try_new(
            root.path().join("primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(1, Duration::from_secs(3600)).unwrap(),
        )
        .unwrap(),
        100,
    )
    .unwrap();

    let first = rotating.append(&observation(100), 101).unwrap();
    assert!(first.closed_segment().is_none());
    let second = rotating.append(&observation(101), 102).unwrap();
    let rotated = second.closed_segment().expect("rotated segment");

    assert_eq!(rotating.verified_segment_paths().len(), 2);
    assert_eq!(rotating.closed_segments(), std::slice::from_ref(rotated));
    assert!(rotated.segment_path().ends_with("segment-0000000001.hlsp"));
    assert_eq!(
        rotated.manifest().segment_blake3(),
        *blake3::hash(&std::fs::read(rotated.segment_path()).unwrap()).as_bytes()
    );
    assert!(
        root.path()
            .join("primary-node/segment-0000000001.hlsp.manifest")
            .is_file()
    );
    assert!(
        rotating
            .active_segment_path()
            .ends_with("segment-0000000002.hlsp")
    );
    let final_close = rotating.shutdown(103).unwrap().unwrap();
    assert!(final_close.manifest().previous_manifest_blake3().is_some());
}

fn observation(offset: u64) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-directory-epoch", offset).unwrap(),
        ReceiveTimestamps::new(1_785_240_000_000_000 + offset as i64, offset).unwrap(),
        "node-v1",
        Bytes::from(format!("payload-{offset}")),
        Vec::new(),
        1024,
    )
    .unwrap()
}

#[test]
fn source_spool_resumes_the_verified_tail_and_strict_cursor() {
    let root = TempDir::new().unwrap();
    let mut first = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 100).unwrap();
    first
        .append(&observation(100), 101)
        .unwrap()
        .durability_receipt()
        .unwrap();
    let segment = first.active_segment_path().to_owned();
    drop(first);

    let mut restarted = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 102).unwrap();
    assert_eq!(restarted.last_durable_cursor().unwrap().offset(), 100);
    assert_eq!(restarted.active_segment_path(), segment);
    let duplicate = restarted
        .append(&observation(100), 103)
        .expect_err("duplicate restart cursor");
    assert!(matches!(duplicate, SpoolError::CursorRegression));
    restarted
        .append(&observation(101), 104)
        .unwrap()
        .durability_receipt()
        .unwrap();
    drop(restarted);

    let records = SpoolReader::open(segment).unwrap().read_all().unwrap();
    assert_eq!(
        records
            .iter()
            .map(|record| record.cursor().offset())
            .collect::<Vec<_>>(),
        vec![100, 101]
    );
}

#[test]
fn recovered_nonempty_tail_can_be_sealed_and_is_reloaded_as_verified_closed_evidence() {
    let root = TempDir::new().unwrap();
    let mut first = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 100).unwrap();
    first.append(&observation(100), 101).unwrap();
    let segment = first.active_segment_path().to_owned();
    drop(first);

    let mut restarted = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 102).unwrap();
    assert!(restarted.closed_segments().is_empty());
    let sealed = restarted
        .seal_active(103)
        .unwrap()
        .expect("recovered records are sealed");
    assert_eq!(sealed.segment_path(), segment);
    assert_eq!(restarted.closed_segments(), std::slice::from_ref(&sealed));
    drop(restarted);

    let reopened = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 104).unwrap();
    assert_eq!(reopened.closed_segments(), std::slice::from_ref(&sealed));
    assert!(
        reopened
            .active_segment_path()
            .ends_with("segment-0000000002.hlsp")
    );
}

#[test]
fn source_spool_refuses_to_resume_under_a_different_source_contract() {
    let root = TempDir::new().unwrap();
    let mut first = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 100).unwrap();
    first.append(&observation(100), 101).unwrap();
    drop(first);

    let error = SourceSpool::open(config(&root, "hyperliquid-node-v2"), 102)
        .expect_err("source version mismatch");
    assert!(matches!(error, SpoolError::SourceMismatch));
}

#[test]
fn spool_v1_rejects_parse_warnings_instead_of_silently_losing_them() {
    let root = TempDir::new().unwrap();
    let mut source_spool = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 100).unwrap();
    let warned = SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-directory-epoch", 100).unwrap(),
        ReceiveTimestamps::new(100, 100).unwrap(),
        "node-v1",
        Bytes::from_static(b"payload"),
        vec![ParseWarning::new("test-warning", "must survive").unwrap()],
        1024,
    )
    .unwrap();

    let error = source_spool.append(&warned, 101).unwrap_err();

    assert!(matches!(error, SpoolError::UnsupportedWarnings));
    assert_eq!(error.reason_code(), "spool.unsupported_warnings");
    assert!(
        SpoolReader::open(source_spool.active_segment_path())
            .unwrap()
            .read_all()
            .unwrap()
            .is_empty()
    );
}
