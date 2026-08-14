use bytes::Bytes;
use domain_types::SourceId;
use hl_capture::spool::{
    DurabilityPolicy, SegmentHeaderV1, SourceSpool, SourceSpoolAppendDisposition,
    SourceSpoolConfig, SpoolError, SpoolReader, SpoolRotationPolicy, SpoolWriter, inspect_spool,
};
use hl_protocol::{
    ObservationClass, ParseWarning, ReceiveTimestamps, SourceCursor, SourceObservation,
};
use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;
use storage_ports::CursorPolicy;
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

fn byte_config(root: &TempDir) -> SourceSpoolConfig {
    SourceSpoolConfig::try_new_with_cursor_policy(
        root.path().join("primary-node"),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        "spool-v1",
        [0x31; 32],
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
        CursorPolicy::MonotonicByteOffset,
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
    observation_with("node-directory-epoch", offset, format!("payload-{offset}"))
}

fn observation_with(epoch: &str, offset: u64, payload: impl Into<Bytes>) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new(epoch, offset).unwrap(),
        ReceiveTimestamps::new(1_785_240_000_000_000 + offset as i64, offset).unwrap(),
        "node-v1",
        payload.into(),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn observation_with_class(class: ObservationClass, offset: u64) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        class,
        SourceCursor::new("node-directory-epoch", offset).unwrap(),
        ReceiveTimestamps::new(1_785_240_000_000_000 + offset as i64, offset).unwrap(),
        "node-v1",
        Bytes::from(format!("payload-{offset}")),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn byte_observation(offset: u64) -> SourceObservation {
    byte_observation_with("node-directory-epoch", offset, format!("payload-{offset}"))
}

fn byte_observation_with(epoch: &str, offset: u64, payload: impl Into<Bytes>) -> SourceObservation {
    byte_observation_with_warnings(epoch, offset, payload, Vec::new())
}

fn byte_observation_with_warnings(
    epoch: &str,
    offset: u64,
    payload: impl Into<Bytes>,
    warnings: Vec<ParseWarning>,
) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::AuxiliaryLedger,
        SourceCursor::new(epoch, offset).unwrap(),
        ReceiveTimestamps::new(1_785_240_000_000_000 + offset as i64, offset).unwrap(),
        "node-v1",
        payload.into(),
        warnings,
        1024,
    )
    .unwrap()
}

#[test]
fn byte_offsets_receive_physical_local_sequences_and_resume_after_restart() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();

    for (offset, expected_sequence) in [(17, 1), (49, 2), (121, 3)] {
        let appended = spool
            .append(&byte_observation(offset), 100 + expected_sequence)
            .unwrap();
        assert_eq!(appended.local_sequence().get(), expected_sequence as u64);
        assert_eq!(
            appended.disposition(),
            SourceSpoolAppendDisposition::Appended
        );
    }
    assert_eq!(spool.last_local_sequence().unwrap().get(), 3);
    drop(spool);

    let mut restarted = SourceSpool::open(byte_config(&root), 200).unwrap();
    assert_eq!(restarted.last_local_sequence().unwrap().get(), 3);
    assert_eq!(
        restarted
            .append(&byte_observation(233), 201)
            .unwrap()
            .local_sequence()
            .get(),
        4
    );
}

#[test]
fn byte_offset_closed_manifest_binds_policy_and_local_sequence_span() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();

    for (offset, durable_at) in [(17, 101), (49, 102), (121, 103)] {
        spool.append(&byte_observation(offset), durable_at).unwrap();
    }

    let closed = spool.seal_active(104).unwrap().unwrap();
    assert_eq!(
        hex::encode(closed.manifest_hash()),
        "062121f89955077deef42a50152b21057457c17a9cc260a0a3e3084a896d3b30"
    );
    assert_eq!(closed.manifest().schema_version(), "hl-spool-manifest-v2");
    assert_eq!(
        closed.manifest().cursor_policy(),
        Some(CursorPolicy::MonotonicByteOffset)
    );
    assert_eq!(closed.manifest().first_local_sequence().unwrap().get(), 1);
    assert_eq!(closed.manifest().last_local_sequence().unwrap().get(), 3);

    let encoded: serde_json::Value =
        serde_json::from_slice(&std::fs::read(closed.manifest_path()).unwrap()).unwrap();
    assert_eq!(encoded["schema_version"], "hl-spool-manifest-v2");
    assert_eq!(encoded["cursor_policy"], "monotonic-byte-offset");
    assert_eq!(encoded["first_local_sequence"], 1);
    assert_eq!(encoded["last_local_sequence"], 3);

    drop(spool);
    let reopened = SourceSpool::open(byte_config(&root), 200).unwrap();
    let verified = &reopened.closed_segments()[0];
    assert_eq!(verified.manifest_hash(), closed.manifest_hash());
    assert_eq!(verified.manifest().first_local_sequence().unwrap().get(), 1);
    assert_eq!(verified.manifest().last_local_sequence().unwrap().get(), 3);
    assert_eq!(reopened.last_local_sequence().unwrap().get(), 3);
}

#[test]
fn byte_offset_closed_manifest_rejects_a_tampered_sequence_span() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool.append(&byte_observation(17), 101).unwrap();
    spool.append(&byte_observation(49), 102).unwrap();
    let closed = spool.seal_active(103).unwrap().unwrap();

    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(closed.manifest_path()).unwrap()).unwrap();
    value["last_local_sequence"] = serde_json::json!(3);
    let mut tampered = serde_json::to_vec_pretty(&value).unwrap();
    tampered.push(b'\n');
    std::fs::write(closed.manifest_path(), tampered).unwrap();

    assert!(matches!(
        hl_capture::spool::CloseReceipt::load(closed.segment_path()),
        Err(SpoolError::InvalidManifest)
    ));
}

#[test]
fn pre_m2d_byte_segment_with_a_v1_manifest_migrates_to_v2_without_renumbering() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool.append(&byte_observation(17), 101).unwrap();
    let legacy = spool.seal_active(102).unwrap().unwrap();
    let legacy_path = legacy.manifest_path().to_owned();
    drop(spool);

    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&legacy_path).unwrap()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.insert(
        "schema_version".to_owned(),
        serde_json::json!("hl-spool-manifest-v1"),
    );
    object.remove("cursor_policy");
    object.remove("first_local_sequence");
    object.remove("last_local_sequence");
    let mut encoded = serde_json::to_vec_pretty(&value).unwrap();
    encoded.push(b'\n');
    std::fs::write(&legacy_path, &encoded).unwrap();

    let inspection = inspect_spool(root.path().join("primary-node")).unwrap();
    assert_eq!(inspection.records(), 1);
    let legacy_hash = *blake3::hash(&encoded).as_bytes();
    let mut reopened = SourceSpool::open(byte_config(&root), 200).unwrap();
    assert_eq!(reopened.last_local_sequence().unwrap().get(), 1);
    assert_eq!(
        reopened.closed_segments()[0].manifest().cursor_policy(),
        None
    );
    reopened.append(&byte_observation(49), 201).unwrap();
    let migrated = reopened.shutdown(202).unwrap().unwrap();
    assert_eq!(
        migrated.manifest().cursor_policy(),
        Some(CursorPolicy::MonotonicByteOffset)
    );
    assert_eq!(migrated.manifest().first_local_sequence().unwrap().get(), 2);
    assert_eq!(migrated.manifest().last_local_sequence().unwrap().get(), 2);
    assert_eq!(
        migrated.manifest().previous_manifest_blake3(),
        Some(legacy_hash)
    );
    let migrated_inspection = inspect_spool(root.path().join("primary-node")).unwrap();
    assert_eq!(migrated_inspection.closed_segments(), 2);
    assert_eq!(migrated_inspection.records(), 2);
}

#[test]
fn inspection_rejects_gap_and_overlap_between_v2_sequence_spans() {
    for shifted_sequence in [1_u64, 3] {
        let root = TempDir::new().unwrap();
        let rotation = SpoolRotationPolicy::try_new(1, Duration::from_secs(3600)).unwrap();
        let config = SourceSpoolConfig::try_new_with_cursor_policy(
            root.path().join("primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            DurabilityPolicy::FsyncEveryRecord,
            rotation,
            CursorPolicy::MonotonicByteOffset,
        )
        .unwrap();
        let mut spool = SourceSpool::open(config, 100).unwrap();
        spool.append(&byte_observation(17), 101).unwrap();
        spool.append(&byte_observation(49), 102).unwrap();
        let second = spool.shutdown(103).unwrap().unwrap();

        let mut value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(second.manifest_path()).unwrap()).unwrap();
        value["first_local_sequence"] = serde_json::json!(shifted_sequence);
        value["last_local_sequence"] = serde_json::json!(shifted_sequence);
        let mut encoded = serde_json::to_vec_pretty(&value).unwrap();
        encoded.push(b'\n');
        std::fs::write(second.manifest_path(), encoded).unwrap();

        assert!(matches!(
            inspect_spool(root.path().join("primary-node")),
            Err(SpoolError::ManifestChainBroken)
        ));
    }
}

#[test]
fn byte_offset_duplicate_is_idempotent_but_conflicting_content_fails_closed() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    let original = byte_observation_with("node-directory-epoch", 17, Bytes::from_static(b"same"));
    assert_eq!(
        spool.append(&original, 101).unwrap().local_sequence().get(),
        1
    );

    let duplicate = spool.append(&original, 102).unwrap();
    assert_eq!(duplicate.local_sequence().get(), 1);
    assert_eq!(
        duplicate.disposition(),
        SourceSpoolAppendDisposition::Duplicate
    );
    assert!(duplicate.durability_receipt().is_none());
    assert!(duplicate.closed_segment().is_none());
    assert_eq!(
        SpoolReader::open(spool.active_segment_path())
            .unwrap()
            .read_all()
            .unwrap()
            .len(),
        1
    );
    drop(spool);

    let mut restarted = SourceSpool::open(byte_config(&root), 200).unwrap();
    assert_eq!(
        restarted
            .append(&original, 201)
            .unwrap()
            .local_sequence()
            .get(),
        1
    );
    let conflicting =
        byte_observation_with("node-directory-epoch", 17, Bytes::from_static(b"different"));
    let error = restarted
        .append(&conflicting, 202)
        .expect_err("same cursor with different bytes must fail");
    assert!(matches!(error, SpoolError::CursorConflict));
    assert_eq!(error.reason_code(), "spool.cursor_conflict");
    assert_eq!(restarted.last_local_sequence().unwrap().get(), 1);
}

#[test]
fn every_retained_byte_cursor_is_idempotent_after_restart_and_old_conflicts_fail_closed() {
    let root = TempDir::new().unwrap();
    let retained = [
        byte_observation_with("node-directory-epoch", 17, "first"),
        byte_observation_with("node-directory-epoch", 49, "second"),
        byte_observation_with("node-directory-epoch", 121, "third"),
    ];
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    for (index, observation) in retained.iter().enumerate() {
        spool
            .append(observation, 101 + i64::try_from(index).unwrap())
            .unwrap();
    }
    drop(spool);

    let mut restarted = SourceSpool::open(byte_config(&root), 200).unwrap();
    for (index, observation) in retained.iter().enumerate() {
        let duplicate = restarted
            .append(observation, 201 + i64::try_from(index).unwrap())
            .unwrap();
        assert_eq!(
            duplicate.local_sequence().get(),
            u64::try_from(index + 1).unwrap()
        );
        assert_eq!(
            duplicate.disposition(),
            SourceSpoolAppendDisposition::Duplicate
        );
    }
    let conflict = byte_observation_with("node-directory-epoch", 17, "changed");
    assert!(matches!(
        restarted.append(&conflict, 204),
        Err(SpoolError::CursorConflict)
    ));
    assert_eq!(restarted.last_local_sequence().unwrap().get(), 3);
    assert_eq!(
        SpoolReader::open(restarted.active_segment_path())
            .unwrap()
            .read_all()
            .unwrap()
            .len(),
        3
    );
}

#[test]
fn warned_byte_cursor_duplicate_is_rejected_instead_of_losing_warning_evidence() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool
        .append(
            &byte_observation_with("node-directory-epoch", 17, "same"),
            101,
        )
        .unwrap();
    let warned = byte_observation_with_warnings(
        "node-directory-epoch",
        17,
        "same",
        vec![ParseWarning::new("test-warning", "must survive").unwrap()],
    );

    assert!(matches!(
        spool.append(&warned, 102),
        Err(SpoolError::UnsupportedWarnings)
    ));
    assert_eq!(spool.last_local_sequence().unwrap().get(), 1);
}

#[test]
fn byte_offset_epoch_change_rotates_before_append_and_keeps_sequence() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool
        .append(&byte_observation_with("epoch-a", 49, "first"), 101)
        .unwrap();

    let second = spool
        .append(&byte_observation_with("epoch-b", 7, "second"), 102)
        .unwrap();

    assert_eq!(second.local_sequence().get(), 2);
    let first_close = second.closed_segment().expect("epoch rotation");
    assert_eq!(
        first_close.manifest().cursor_policy(),
        Some(CursorPolicy::MonotonicByteOffset)
    );
    assert_eq!(
        first_close.manifest().first_local_sequence().unwrap().get(),
        1
    );
    assert_eq!(
        first_close.manifest().last_local_sequence().unwrap().get(),
        1
    );
    assert_eq!(spool.verified_segment_paths().len(), 2);
    let first_records = SpoolReader::open(&spool.verified_segment_paths()[0])
        .unwrap()
        .read_all()
        .unwrap();
    let second_records = SpoolReader::open(&spool.verified_segment_paths()[1])
        .unwrap()
        .read_all()
        .unwrap();
    assert_eq!(first_records[0].cursor().epoch(), "epoch-a");
    assert_eq!(second_records[0].cursor().epoch(), "epoch-b");
    drop(spool);

    let mut reopened = SourceSpool::open(byte_config(&root), 200).unwrap();
    assert_eq!(reopened.last_local_sequence().unwrap().get(), 2);
    assert_eq!(reopened.last_durable_cursor().unwrap().epoch(), "epoch-b");
    assert_eq!(reopened.last_durable_cursor().unwrap().offset(), 7);
    assert_eq!(reopened.cursor_policy(), CursorPolicy::MonotonicByteOffset);
    let old_epoch_duplicate = reopened
        .append(&byte_observation_with("epoch-a", 49, "first"), 201)
        .unwrap();
    assert_eq!(
        old_epoch_duplicate.disposition(),
        SourceSpoolAppendDisposition::Duplicate
    );
    assert_eq!(old_epoch_duplicate.local_sequence().get(), 1);
    assert!(matches!(
        reopened.append(&byte_observation_with("epoch-a", 100, "reused"), 202),
        Err(SpoolError::CursorRegression)
    ));
}

#[test]
fn byte_offset_policy_is_bound_into_a_distinct_bounded_spool_identity() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool.append(&byte_observation(17), 101).unwrap();
    let header = SpoolReader::open(spool.active_segment_path()).unwrap();
    assert_ne!(header.header().schema_version(), "spool-v1");
    assert!(header.header().schema_version().len() <= 256);
    assert_eq!(
        header.header().cursor_policy(),
        CursorPolicy::MonotonicByteOffset
    );
    assert_eq!(
        &std::fs::read(spool.active_segment_path()).unwrap()[..8],
        b"HLSPV002"
    );
    let persisted_policy_identity = header.header().schema_version().to_owned();
    drop(spool);

    let wrong_policy = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 102)
        .expect_err("cursor policy mismatch must fail closed");
    assert!(matches!(wrong_policy, SpoolError::SourceMismatch));

    let alias_config = SourceSpoolConfig::try_new(
        root.path().join("primary-node"),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        persisted_policy_identity.clone(),
        [0x31; 32],
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
    )
    .expect("legacy constructors must retain every previously accepted schema identity");
    assert!(matches!(
        SourceSpool::open(alias_config, 103),
        Err(SpoolError::CursorPolicyMismatch)
    ));

    let legacy_root = TempDir::new().unwrap();
    let legacy_alias = SourceSpoolConfig::try_new(
        legacy_root.path().join("legacy"),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        persisted_policy_identity.clone(),
        [0x31; 32],
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
    )
    .expect("reserved-prefix schemas remain valid under the legacy constructor");
    let legacy = SourceSpool::open(legacy_alias, 104).unwrap();
    assert_eq!(
        SpoolReader::open(legacy.active_segment_path())
            .unwrap()
            .header()
            .schema_version(),
        persisted_policy_identity
    );
    assert!(!legacy_root.path().join("legacy/.cursor-policy-v1").exists());
}

#[test]
fn byte_offset_policy_requires_per_record_fsync() {
    let root = TempDir::new().unwrap();
    let error = SourceSpoolConfig::try_new_with_cursor_policy(
        root.path().join("primary-node"),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        "spool-v1",
        [0x31; 32],
        DurabilityPolicy::FsyncEvery {
            max_records: 2,
            max_delay: Duration::from_secs(1),
        },
        SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
        CursorPolicy::MonotonicByteOffset,
    )
    .expect_err("byte-offset acknowledgements require immediate durability");

    assert!(matches!(error, SpoolError::InvalidDurabilityPolicy));
    assert!(!root.path().join("primary-node").exists());
}

#[test]
fn spool_config_fsync_gate_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let batched = DurabilityPolicy::FsyncEvery {
            max_records: 2,
            max_delay: Duration::from_secs(1),
        };
        let root = TempDir::new().unwrap();
        let batched_result = SourceSpoolConfig::try_new_with_cursor_policy(
            root.path().join("primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            batched,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            cursor_policy,
        );
        match cursor_policy {
            CursorPolicy::MonotonicByteOffset => {
                let error = batched_result
                    .expect_err("byte-offset acknowledgements require immediate durability");
                assert!(matches!(error, SpoolError::InvalidDurabilityPolicy));
                assert_eq!(error.reason_code(), "spool.invalid_durability_policy");
                assert!(!root.path().join("primary-node").exists());
            }
            CursorPolicy::ContiguousNativeOffset => {
                batched_result.expect(
                    "contiguous native offset still admits bounded FsyncEvery at construction",
                );
            }
        }

        let admitted = TempDir::new().unwrap();
        SourceSpoolConfig::try_new_with_cursor_policy(
            admitted.path().join("primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            cursor_policy,
        )
        .expect("fsync-every-record remains admitted for every current cursor policy");
    }
}

#[test]
fn byte_offset_durability_covers_every_constructible_writer_policy() {
    for policy in [
        DurabilityPolicy::FsyncEveryRecord,
        DurabilityPolicy::FsyncEvery {
            max_records: 2,
            max_delay: Duration::from_secs(1),
        },
    ] {
        let root = TempDir::new().unwrap();
        let result = SourceSpoolConfig::try_new_with_cursor_policy(
            root.path().join("primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            policy,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            CursorPolicy::MonotonicByteOffset,
        );
        match policy {
            DurabilityPolicy::FsyncEveryRecord => {
                result
                    .expect("fsync-every-record remains admitted for byte-offset acknowledgements");
            }
            DurabilityPolicy::FsyncEvery {
                max_records: _,
                max_delay: _,
            } => {
                let error = result.expect_err("non-admitted byte-offset durability");
                assert!(matches!(error, SpoolError::InvalidDurabilityPolicy));
                assert_eq!(error.reason_code(), "spool.invalid_durability_policy");
                assert!(!root.path().join("primary-node").exists());
            }
        }
    }
}

#[test]
fn byte_offset_duplicate_index_is_bounded_by_segments_not_record_count() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    for offset in 1..=100 {
        spool
            .append(&byte_observation(offset), 100 + offset as i64)
            .unwrap();
    }
    assert_eq!(spool.retained_segment_count(), 1);
    drop(spool);

    let reopened = SourceSpool::open(byte_config(&root), 300).unwrap();
    assert_eq!(reopened.last_local_sequence().unwrap().get(), 100);
    assert_eq!(reopened.retained_segment_count(), 1);
}

#[test]
fn byte_offset_preflight_failures_do_not_rotate_or_consume_sequence() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool
        .append(&byte_observation_with("epoch-a", 17, "first"), 101)
        .unwrap();
    let active = spool.active_segment_path().to_owned();
    let before = std::fs::read(&active).unwrap();
    let warned = byte_observation_with_warnings(
        "epoch-b",
        7,
        "second",
        vec![ParseWarning::new("test-warning", "must survive").unwrap()],
    );

    assert!(matches!(
        spool.append(&warned, 102),
        Err(SpoolError::UnsupportedWarnings)
    ));
    assert!(matches!(
        spool.append(&byte_observation_with("epoch-b", 7, "second"), -1),
        Err(SpoolError::InvalidTimestamp)
    ));
    assert_eq!(spool.active_segment_path(), active);
    assert_eq!(
        spool.verified_segment_paths(),
        std::slice::from_ref(&active)
    );
    assert!(spool.closed_segments().is_empty());
    assert_eq!(spool.last_local_sequence().unwrap().get(), 1);
    assert_eq!(std::fs::read(&active).unwrap(), before);

    let retried = spool
        .append(&byte_observation_with("epoch-b", 7, "second"), 103)
        .unwrap();
    assert_eq!(retried.local_sequence().get(), 2);
    assert!(retried.closed_segment().is_some());
}

#[test]
fn legacy_source_spool_bytes_match_the_v1_writer_and_create_no_policy_sidecar() {
    let source_root = TempDir::new().unwrap();
    let writer_root = TempDir::new().unwrap();
    let schema = "hl-spool-policy-v1:legacy-schema-is-still-valid";
    let observation = observation_with("node-directory-epoch", 17, "legacy-payload");
    let source_config = SourceSpoolConfig::try_new(
        source_root.path().join("legacy"),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        schema,
        [0x31; 32],
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
    )
    .unwrap();
    let mut source_spool = SourceSpool::open(source_config, 100).unwrap();
    source_spool.append(&observation, 101).unwrap();

    let header = SegmentHeaderV1::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        schema,
        1,
        100,
        [0x31; 32],
    )
    .unwrap();
    let mut writer = SpoolWriter::create(
        writer_root.path(),
        header,
        DurabilityPolicy::FsyncEveryRecord,
    )
    .unwrap();
    writer.append(&observation, 101).unwrap();

    assert_eq!(
        &std::fs::read(source_spool.active_segment_path()).unwrap()[..8],
        b"HLSPV001"
    );
    let source_close = source_spool.shutdown(102).unwrap().unwrap();
    let writer_close = writer.close(102, None).unwrap();
    let source_segment = std::fs::read(source_close.segment_path()).unwrap();
    let source_manifest = std::fs::read(source_close.manifest_path()).unwrap();
    assert_eq!(
        source_segment,
        std::fs::read(writer_close.segment_path()).unwrap()
    );
    assert_eq!(
        source_manifest,
        std::fs::read(writer_close.manifest_path()).unwrap()
    );
    assert_eq!(
        blake3::hash(&source_segment).to_hex().as_str(),
        "d30b056b57cd9f17f4ae7842d88ffb5120ffcbb7bc557542364a2131afd46719"
    );
    assert_eq!(
        blake3::hash(&source_manifest).to_hex().as_str(),
        "6317669073ba0da34eeadc5617611aa6f72c7e06bc32de8624c86d24fc0b643d"
    );
    assert!(!source_root.path().join("legacy/.cursor-policy-v1").exists());
}

#[test]
fn legacy_batched_rotation_retains_the_pre_m2_cursor_behavior() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(
        SourceSpoolConfig::try_new(
            root.path().join("legacy"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            DurabilityPolicy::FsyncEvery {
                max_records: 2,
                max_delay: Duration::from_secs(3600),
            },
            SpoolRotationPolicy::try_new(1, Duration::from_secs(3600)).unwrap(),
        )
        .unwrap(),
        100,
    )
    .unwrap();
    let first = spool.append(&observation(17), 101).unwrap();
    assert!(first.durability_receipt().is_none());

    let duplicate_after_rotation = spool.append(&observation(17), 102).unwrap();
    assert!(duplicate_after_rotation.closed_segment().is_some());
    assert_eq!(duplicate_after_rotation.local_sequence().get(), 2);
    assert_eq!(
        duplicate_after_rotation.disposition(),
        SourceSpoolAppendDisposition::Appended
    );
    assert_eq!(spool.verified_segment_paths().len(), 2);
}

#[test]
fn byte_offset_policy_rejects_block_height_observations() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();

    let error = spool
        .append(&observation(17), 101)
        .expect_err("block heights are not byte offsets");

    assert!(matches!(error, SpoolError::CursorPolicyMismatch));
    assert_eq!(error.reason_code(), "spool.cursor_policy_mismatch");
    assert!(spool.last_local_sequence().is_none());
}

#[test]
fn append_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let root = TempDir::new().unwrap();
        let mut spool = SourceSpool::open(
            SourceSpoolConfig::try_new_with_cursor_policy(
                root.path().join("primary-node"),
                SourceId::new("primary-node").unwrap(),
                "hyperliquid-node-v1",
                "spool-v1",
                [0x31; 32],
                DurabilityPolicy::FsyncEveryRecord,
                SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
                cursor_policy,
            )
            .unwrap(),
            100,
        )
        .unwrap();

        match cursor_policy {
            CursorPolicy::MonotonicByteOffset => {
                let appended = spool
                    .append(&byte_observation(17), 101)
                    .expect("byte-offset append remains admitted");
                assert_eq!(appended.local_sequence().get(), 1);
                assert_eq!(
                    appended.disposition(),
                    SourceSpoolAppendDisposition::Appended
                );

                let error = spool
                    .append(&observation(17), 102)
                    .expect_err("block heights are not byte offsets");
                assert!(matches!(error, SpoolError::CursorPolicyMismatch));
                assert_eq!(error.reason_code(), "spool.cursor_policy_mismatch");
            }
            CursorPolicy::ContiguousNativeOffset => {
                let appended = spool
                    .append(&observation(100), 101)
                    .expect("contiguous native offset still uses the legacy append path");
                assert_eq!(appended.local_sequence().get(), 1);
                assert_eq!(
                    appended.disposition(),
                    SourceSpoolAppendDisposition::Appended
                );
            }
        }
    }
}

#[test]
fn observation_policy_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let root = TempDir::new().unwrap();
        let config = SourceSpoolConfig::try_new_with_cursor_policy(
            root.path().join("primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            cursor_policy,
        )
        .unwrap();
        let mut spool = SourceSpool::open(config.clone(), 100).unwrap();

        match cursor_policy {
            CursorPolicy::MonotonicByteOffset => {
                for class in [
                    ObservationClass::CommittedBlock,
                    ObservationClass::HistoricalBlock,
                ] {
                    let error = spool
                        .append(&observation_with_class(class, 17), 101)
                        .expect_err("block heights are not byte offsets");
                    assert!(matches!(error, SpoolError::CursorPolicyMismatch));
                    assert_eq!(error.reason_code(), "spool.cursor_policy_mismatch");
                }
                assert!(spool.last_local_sequence().is_none());

                let appended = spool
                    .append(&byte_observation(17), 101)
                    .expect("non-block observations remain admitted under byte-offset policy");
                assert_eq!(appended.local_sequence().get(), 1);
                assert_eq!(
                    appended.disposition(),
                    SourceSpoolAppendDisposition::Appended
                );
            }
            CursorPolicy::ContiguousNativeOffset => {
                let committed = spool
                    .append(
                        &observation_with_class(ObservationClass::CommittedBlock, 100),
                        101,
                    )
                    .expect("contiguous native offset still admits committed block observations");
                assert_eq!(committed.local_sequence().get(), 1);
                assert_eq!(
                    committed.disposition(),
                    SourceSpoolAppendDisposition::Appended
                );

                let historical = spool
                    .append(
                        &observation_with_class(ObservationClass::HistoricalBlock, 101),
                        102,
                    )
                    .expect("contiguous native offset still admits historical block observations");
                assert_eq!(historical.local_sequence().get(), 2);
                assert_eq!(
                    historical.disposition(),
                    SourceSpoolAppendDisposition::Appended
                );
            }
        }

        drop(spool);
        SourceSpool::open(config, 200).expect(
            "recovery still applies today's observation-policy admission for this cursor policy",
        );
    }
}

#[test]
fn persisted_schema_identity_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let root = TempDir::new().unwrap();
        let config = SourceSpoolConfig::try_new_with_cursor_policy(
            root.path().join("primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x31; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            cursor_policy,
        )
        .unwrap();
        let spool = SourceSpool::open(config, 100).unwrap();
        let schema_version = SpoolReader::open(spool.active_segment_path())
            .unwrap()
            .header()
            .schema_version()
            .to_owned();

        match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => {
                assert_eq!(schema_version, "spool-v1");
            }
            CursorPolicy::MonotonicByteOffset => {
                assert_eq!(
                    schema_version,
                    format!(
                        "hl-spool-policy-v1:monotonic-byte-offset:{}",
                        blake3::hash(b"spool-v1").to_hex()
                    )
                );
            }
        }
    }
}

#[test]
fn local_sequence_chain_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let root = TempDir::new().unwrap();
        let mut spool = SourceSpool::open(
            SourceSpoolConfig::try_new_with_cursor_policy(
                root.path().join("primary-node"),
                SourceId::new("primary-node").unwrap(),
                "hyperliquid-node-v1",
                "spool-v1",
                [0x31; 32],
                DurabilityPolicy::FsyncEveryRecord,
                SpoolRotationPolicy::try_new(1, Duration::from_secs(3600)).unwrap(),
                cursor_policy,
            )
            .unwrap(),
            100,
        )
        .unwrap();

        match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => {
                spool.append(&observation(100), 101).unwrap();
                let rotated = spool.append(&observation(101), 102).unwrap();
                let first = rotated
                    .closed_segment()
                    .expect("contiguous rotation still closes the first segment");
                assert!(first.manifest().first_local_sequence().is_none());
                assert!(first.manifest().last_local_sequence().is_none());
                let last = spool
                    .shutdown(103)
                    .unwrap()
                    .expect("contiguous shutdown still closes the second segment");
                assert!(last.manifest().first_local_sequence().is_none());
                assert!(last.manifest().last_local_sequence().is_none());
            }
            CursorPolicy::MonotonicByteOffset => {
                spool.append(&byte_observation(17), 101).unwrap();
                let rotated = spool.append(&byte_observation(49), 102).unwrap();
                let first = rotated
                    .closed_segment()
                    .expect("byte-offset rotation still closes the first segment");
                assert_eq!(first.manifest().first_local_sequence().unwrap().get(), 1);
                assert_eq!(first.manifest().last_local_sequence().unwrap().get(), 1);
                let last = spool
                    .shutdown(103)
                    .unwrap()
                    .expect("byte-offset shutdown still closes the second segment");
                assert_eq!(last.manifest().first_local_sequence().unwrap().get(), 2);
                assert_eq!(last.manifest().last_local_sequence().unwrap().get(), 2);
            }
        }

        let inspection = inspect_spool(root.path().join("primary-node")).expect(
            "closed local-sequence chain still verifies for this constructible cursor policy",
        );
        assert_eq!(inspection.closed_segments(), 2);
        assert_eq!(inspection.open_segments(), 0);
        assert_eq!(inspection.records(), 2);
    }
}

#[test]
fn byte_offset_policy_identity_rejects_an_overlong_base_schema() {
    let root = TempDir::new().unwrap();
    let error = SourceSpoolConfig::try_new_with_cursor_policy(
        root.path().join("primary-node"),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        "x".repeat(257),
        [0x31; 32],
        DurabilityPolicy::FsyncEveryRecord,
        SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
        CursorPolicy::MonotonicByteOffset,
    )
    .expect_err("persisted policy identity must stay bounded");

    assert!(matches!(error, SpoolError::InvalidHeader));
}

#[test]
fn byte_offset_regression_fails_without_consuming_a_sequence() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool.append(&byte_observation(49), 101).unwrap();

    let error = spool
        .append(&byte_observation(17), 102)
        .expect_err("byte offset regression");
    assert!(matches!(error, SpoolError::CursorRegression));
    assert_eq!(spool.last_local_sequence().unwrap().get(), 1);
}

#[test]
fn incomplete_tail_recovery_does_not_consume_a_local_sequence() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(byte_config(&root), 100).unwrap();
    spool.append(&byte_observation(17), 101).unwrap();
    spool.append(&byte_observation(49), 102).unwrap();
    let segment = spool.active_segment_path().to_owned();
    drop(spool);

    OpenOptions::new()
        .append(true)
        .open(&segment)
        .unwrap()
        .write_all(&[0x08, 0x00])
        .unwrap();

    let mut recovered = SourceSpool::open(byte_config(&root), 200).unwrap();
    assert_eq!(recovered.last_local_sequence().unwrap().get(), 2);
    assert_eq!(
        recovered
            .append(&byte_observation(121), 201)
            .unwrap()
            .local_sequence()
            .get(),
        3
    );
}

#[test]
fn legacy_policy_keeps_its_schema_and_rejects_duplicate_and_epoch_change() {
    let root = TempDir::new().unwrap();
    let mut spool = SourceSpool::open(config(&root, "hyperliquid-node-v1"), 100).unwrap();
    spool.append(&observation(17), 101).unwrap();
    assert_eq!(
        SpoolReader::open(spool.active_segment_path())
            .unwrap()
            .header()
            .schema_version(),
        "spool-v1"
    );

    assert!(matches!(
        spool.append(&observation(17), 102),
        Err(SpoolError::CursorRegression)
    ));
    assert!(matches!(
        spool.append(&observation_with("next-epoch", 1, "next"), 103),
        Err(SpoolError::CursorRegression)
    ));
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
