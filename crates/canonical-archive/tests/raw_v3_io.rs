use canonical_archive::{ArchiveConfig, RawArchiveCheckpoint, RawV3Archive};
use domain_types::{ChainId, KnownTime, SourceId};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    ArchiveError, LocalRecordSequence, LocalRecordSequenceRange, RawArchiveCapacityBudgets,
    RawArchiveCapacityRejection, RawArchiveCheckpointEntriesV2, RawArchiveCheckpointEntryV2,
    RawArchiveWorkloadEnvelope, RawObservationArchive, RawObservationBatch, RawObservationRange,
};

fn observation(offset: u64, payload: &[u8]) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("node-fills").unwrap(),
        "capture-v1",
        ObservationClass::AuxiliaryLedger,
        SourceCursor::new("epoch-1", offset).unwrap(),
        ReceiveTimestamps::new(1_722_000_000_000_000, offset).unwrap(),
        "raw-parser-v1",
        bytes::Bytes::copy_from_slice(payload),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn batch(first_sequence: u64, offsets: &[u64], payload: &[u8]) -> RawObservationBatch {
    let observations = offsets
        .iter()
        .map(|offset| observation(*offset, payload))
        .collect();
    RawObservationBatch::try_new_byte_offsets(
        ChainId::new("mainnet").unwrap(),
        observations,
        [0x11; 32],
        [0x22; 32],
        LocalRecordSequence::try_new(first_sequence).unwrap(),
    )
    .unwrap()
}

fn generous_capacity() -> (RawArchiveWorkloadEnvelope, RawArchiveCapacityBudgets) {
    (
        RawArchiveWorkloadEnvelope::try_new(
            100,
            1,
            1_000,
            3_600,
            1_024,
            1_000,
            64 * 1024 * 1024,
            64,
        )
        .unwrap(),
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true).unwrap(),
    )
}

fn open_archive(path: &std::path::Path) -> RawV3Archive {
    let (workload, budgets) = generous_capacity();
    RawV3Archive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            "raw-v3-io-test",
            KnownTime::from_unix_micros(1_722_000_000_000_000).unwrap(),
        )
        .unwrap(),
        workload,
        budgets,
    )
    .unwrap()
}

#[test]
fn append_then_read_replays_exact_rows_and_binds_previous_root() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let first = archive
        .append_batch(&batch(1, &[10, 11], b"ab"))
        .expect("first append");
    let second = archive
        .append_batch(&batch(3, &[20], b"cd"))
        .expect("second append");

    assert_eq!(first.local_sequence_range().unwrap().start().get(), 1);
    assert_eq!(first.local_sequence_range().unwrap().end().get(), 2);
    assert_eq!(second.local_sequence_range().unwrap().start().get(), 3);
    assert_ne!(first.manifest_sha256(), second.manifest_sha256());

    let replayed = archive
        .read_observations_by_sequence(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(3).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(replayed.len(), 3);
    assert_eq!(replayed[0].observation().payload().as_ref(), b"ab");
    assert_eq!(replayed[2].observation().payload().as_ref(), b"cd");
    assert_eq!(replayed[0].local_sequence().get(), 1);
    assert_eq!(replayed[2].local_sequence().get(), 3);

    let verified = archive
        .verify_raw_manifest_at_sequence(
            &first.manifest_id().clone(),
            first.local_sequence_range().unwrap(),
        )
        .unwrap();
    assert_eq!(verified.manifest_sha256(), first.manifest_sha256());

    let retry = archive
        .append_batch(&batch(1, &[10, 11], b"ab"))
        .expect("idempotent retry");
    assert_eq!(retry.manifest_sha256(), first.manifest_sha256());
}

#[test]
fn capacity_admission_rejects_insufficient_budget_and_runtime_limits() {
    let temporary = tempfile::tempdir().unwrap();
    let workload = RawArchiveWorkloadEnvelope::try_new(
        100,
        100,
        1_000,
        3_600,
        1_024,
        100,
        64 * 1024 * 1024,
        64,
    )
    .unwrap();
    let tiny = RawArchiveCapacityBudgets::try_new(1, u64::MAX, u64::MAX, u64::MAX, true).unwrap();
    let error = RawV3Archive::open(
        temporary.path(),
        ArchiveConfig::deterministic_fixture(
            "raw-v3-capacity",
            KnownTime::from_unix_micros(1_000).unwrap(),
        )
        .unwrap(),
        workload,
        tiny,
    )
    .expect_err("startup must fail closed");
    assert!(matches!(
        error,
        ArchiveError::Capacity(RawArchiveCapacityRejection::RawDataBudget)
    ));

    let runtime_workload = RawArchiveWorkloadEnvelope::try_new(1, 1, 1_000, 1, 4, 1, 1, 1).unwrap();
    let generous =
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true).unwrap();
    let archive = RawV3Archive::open(
        temporary.path(),
        ArchiveConfig::deterministic_fixture(
            "raw-v3-runtime",
            KnownTime::from_unix_micros(1_722_000_000_000_000).unwrap(),
        )
        .unwrap(),
        runtime_workload,
        generous,
    )
    .unwrap();
    let oversized = archive
        .append_batch(&batch(1, &[10], b"abcde"))
        .expect_err("record larger than admitted encoded size");
    assert!(matches!(
        oversized,
        ArchiveError::Capacity(RawArchiveCapacityRejection::RuntimeLimitExceeded)
    ));

    let first = archive
        .append_batch(&batch(1, &[10], b"ab"))
        .expect("admitted first commit");
    assert_eq!(first.local_sequence_range().unwrap().len(), 1);
    let second = archive
        .append_batch(&batch(2, &[11], b"cd"))
        .expect_err("uncompacted commit budget");
    assert!(matches!(
        second,
        ArchiveError::Capacity(RawArchiveCapacityRejection::RuntimeLimitExceeded)
    ));
}

#[test]
fn cursor_epoch_lookup_and_offset_read_use_authenticated_sequence_truth() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[40, 41], b"xy")).unwrap();
    assert!(
        archive
            .contains_raw_cursor_epoch(&chain, &source, "epoch-1")
            .unwrap()
    );
    assert!(
        !archive
            .contains_raw_cursor_epoch(&chain, &source, "epoch-missing")
            .unwrap()
    );
    let rows = archive
        .read_observations(
            &chain,
            &source,
            RawObservationRange::try_new("epoch-1", 40, 41).unwrap(),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].cursor().offset(), 40);
}

fn dataset_dir(root: &std::path::Path) -> std::path::PathBuf {
    root.join("chain=mainnet")
        .join("dataset=raw_source_observations_byte_v3")
        .join("source=node-fills")
}

#[test]
fn pack_index_rotates_journal_generation_and_keeps_replay() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10, 11], b"ab")).unwrap();
    archive.append_batch(&batch(3, &[20], b"cd")).unwrap();
    let dataset = dataset_dir(temporary.path());
    let generation_one = std::fs::read(dataset.join("journals/generation-1.log")).unwrap();
    let packed_root = archive.pack_index(&chain, &source).expect("pack index");
    let generation_one_after = std::fs::read(dataset.join("journals/generation-1.log")).unwrap();
    assert_eq!(generation_one, generation_one_after);
    assert!(dataset.join("journals/generation-2.log").is_file());
    let packs: Vec<_> = std::fs::read_dir(dataset.join("index-packs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("pack"))
        .collect();
    assert_eq!(packs.len(), 1);
    archive.append_batch(&batch(4, &[30], b"ef")).unwrap();
    let replayed = archive
        .read_observations_by_sequence(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(4).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(replayed.len(), 4);
    assert_eq!(replayed[0].observation().payload().as_ref(), b"ab");
    assert_eq!(replayed[3].observation().payload().as_ref(), b"ef");
    let packed_again = archive.pack_index(&chain, &source).expect("second pack");
    assert_ne!(packed_root, packed_again);
    assert!(dataset.join("journals/generation-3.log").is_file());
    let generation_one_final = std::fs::read(dataset.join("journals/generation-1.log")).unwrap();
    assert_eq!(generation_one, generation_one_final);
}

#[test]
fn pack_index_is_noop_when_the_active_journal_has_one_record() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    let first_hash = archive.pack_index(&chain, &source).unwrap();
    let second_hash = archive.pack_index(&chain, &source).unwrap();
    assert_eq!(first_hash, second_hash);
    assert!(
        !dataset_dir(temporary.path())
            .join("journals/generation-2.log")
            .exists()
    );
}

#[test]
fn pack_logical_range_replays_exact_rows_and_keeps_receipts() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let first = archive.append_batch(&batch(1, &[10, 11], b"ab")).unwrap();
    let second = archive.append_batch(&batch(3, &[20], b"cd")).unwrap();
    let packed_root = archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(3).unwrap(),
            )
            .unwrap(),
        )
        .expect("pack logical range");
    let manifests = temporary.path().join("_manifests/raw-byte-v3/packs");
    let pack_manifests: Vec<_> = std::fs::read_dir(&manifests)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(pack_manifests.len(), 1);
    let replayed = archive
        .read_observations_by_sequence(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(3).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(replayed.len(), 3);
    assert_eq!(replayed[0].observation().payload().as_ref(), b"ab");
    assert_eq!(replayed[2].observation().payload().as_ref(), b"cd");
    let verified = archive
        .verify_raw_manifest_at_sequence(
            &first.manifest_id().clone(),
            first.local_sequence_range().unwrap(),
        )
        .unwrap();
    assert_eq!(verified.manifest_sha256(), first.manifest_sha256());
    let verified_second = archive
        .verify_raw_manifest_at_sequence(
            &second.manifest_id().clone(),
            second.local_sequence_range().unwrap(),
        )
        .unwrap();
    assert_eq!(verified_second.manifest_sha256(), second.manifest_sha256());
    let retry = archive
        .append_batch(&batch(1, &[10, 11], b"ab"))
        .expect("idempotent retry after packing");
    assert_eq!(retry.manifest_sha256(), first.manifest_sha256());
    let packed_again = archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(3).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_eq!(packed_root, packed_again);
    archive.append_batch(&batch(4, &[30], b"ef")).unwrap();
    let mixed = archive
        .read_observations_by_sequence(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(4).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(mixed.len(), 4);
    assert_eq!(mixed[3].observation().payload().as_ref(), b"ef");
    let after_tail = archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(3).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert_ne!(after_tail, packed_root);
    assert_eq!(
        std::fs::read_dir(&manifests).unwrap().count(),
        1,
        "repacking an already packed span must not publish another pack"
    );
}

#[test]
fn packed_object_mutation_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let dataset = dataset_dir(temporary.path());
    let mut parquet = None;
    fn find_pack_parquet(dir: &std::path::Path, found: &mut Option<std::path::PathBuf>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                find_pack_parquet(&path, found);
            } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet")
                && path
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|name| name.to_str())
                    == Some("packs")
            {
                *found = Some(path);
            }
        }
    }
    find_pack_parquet(&dataset, &mut parquet);
    let parquet = parquet.expect("packed parquet");
    let mut bytes = std::fs::read(&parquet).unwrap();
    bytes[0] ^= 1;
    std::fs::write(&parquet, bytes).unwrap();
    let error = match archive.read_observations_by_sequence(
        &chain,
        &source,
        LocalRecordSequenceRange::try_new(
            LocalRecordSequence::try_new(1).unwrap(),
            LocalRecordSequence::try_new(2).unwrap(),
        )
        .unwrap(),
    ) {
        Err(error) => error,
        Ok(_) => panic!("mutated pack must fail closed"),
    };
    assert!(matches!(
        error,
        ArchiveError::CorruptObject(_) | ArchiveError::ManifestVerification(_)
    ));
}

#[test]
fn sequence_read_holds_an_exact_root_lease() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    let iterator = archive
        .read_observations_by_sequence(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(1).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let leases: Vec<_> = std::fs::read_dir(dataset_dir(temporary.path()).join("leases"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(leases.len(), 1);
    assert!(
        leases[0]
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("root-")
    );
    assert_eq!(
        leases[0].extension().and_then(|ext| ext.to_str()),
        Some("lease")
    );
    let replayed = iterator.collect::<Result<Vec<_>, _>>().unwrap();
    assert_eq!(replayed.len(), 1);
    assert!(leases[0].is_file());
}

const FIXTURE_TIME_MICROS: i64 = 1_722_000_000_000_000;

fn open_retention_archive(path: &std::path::Path, micros: i64) -> RawV3Archive {
    let workload =
        RawArchiveWorkloadEnvelope::try_new(100, 1, 1_000, 1, 1_024, 1_000, 64 * 1024 * 1024, 64)
            .unwrap();
    let budgets =
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true).unwrap();
    RawV3Archive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            "raw-v3-io-test",
            KnownTime::from_unix_micros(micros).unwrap(),
        )
        .unwrap(),
        workload,
        budgets,
    )
    .unwrap()
}

fn collect_parquet(dir: &std::path::Path, found: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect_parquet(&path, found);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet") {
            found.push(path);
        }
    }
}

fn parent_named(path: &std::path::Path, name: &str) -> bool {
    path.parent()
        .and_then(|parent| parent.file_name())
        .and_then(|value| value.to_str())
        == Some(name)
}

#[test]
fn checkpoint_v1_stays_current_until_atomic_switch_to_verified_v2() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let first = archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    let second = archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();

    let v1 = archive
        .publish_checkpoint_v1(
            &chain,
            &source,
            &[first.manifest_id().clone(), second.manifest_id().clone()],
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    assert!(archive.load_checkpoint(&chain, &source).unwrap().is_none());
    archive
        .switch_checkpoint_current(&chain, &source, None, v1)
        .unwrap();
    match archive.load_checkpoint(&chain, &source).unwrap() {
        Some(RawArchiveCheckpoint::V1(loaded)) => assert_eq!(loaded.sha256(), v1),
        other => panic!("expected V1 current, got {other:?}"),
    }

    let v2_entries = RawArchiveCheckpointEntriesV2::try_new(vec![
        RawArchiveCheckpointEntryV2::new(
            first.manifest_id().clone(),
            first.manifest_sha256(),
            first.local_sequence_range().unwrap(),
        ),
        RawArchiveCheckpointEntryV2::new(
            second.manifest_id().clone(),
            second.manifest_sha256(),
            second.local_sequence_range().unwrap(),
        ),
    ])
    .unwrap();
    let v2 = archive
        .publish_checkpoint_v2(&chain, &source, v2_entries)
        .unwrap();
    match archive.load_checkpoint(&chain, &source).unwrap() {
        Some(RawArchiveCheckpoint::V1(loaded)) => assert_eq!(loaded.sha256(), v1),
        other => panic!("V1 must stay current until switch, got {other:?}"),
    }
    assert!(
        archive
            .switch_checkpoint_current(&chain, &source, Some([0x11; 32]), v2)
            .is_err()
    );
    archive
        .switch_checkpoint_current(&chain, &source, Some(v1), v2)
        .unwrap();
    match archive.load_checkpoint(&chain, &source).unwrap() {
        Some(RawArchiveCheckpoint::V2(loaded)) => {
            assert_eq!(loaded.sha256(), v2);
            for entry in loaded.entries().entries() {
                archive
                    .verify_raw_manifest_at_sequence(
                        entry.manifest_id(),
                        entry.local_sequence_range(),
                    )
                    .unwrap();
            }
        }
        other => panic!("expected V2 current, got {other:?}"),
    }
}

#[test]
fn packed_object_gc_unlinks_through_a_crash_recoverable_journal() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS);
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let first = archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    let backup = [0xAB; 32];
    assert!(
        archive
            .plan_packed_object_gc(&chain, &source, backup)
            .unwrap()
            .is_empty()
    );
    drop(archive);

    let too_early = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS);
    assert!(
        too_early
            .plan_packed_object_gc(&chain, &source, backup)
            .unwrap()
            .is_empty()
    );
    drop(too_early);

    let later = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS + 1_000_000);
    assert!(matches!(
        later.plan_packed_object_gc(&chain, &source, [0; 32]),
        Err(ArchiveError::InvalidInput(_))
    ));
    let plan = later
        .plan_packed_object_gc(&chain, &source, backup)
        .unwrap();
    assert!(!plan.is_empty());
    assert!(
        later
            .execute_packed_object_gc(&chain, &source, plan.digest(), [0xCD; 32])
            .is_err()
    );
    let receipt = later
        .execute_packed_object_gc(&chain, &source, plan.digest(), backup)
        .unwrap();
    assert_eq!(receipt.plan_digest(), plan.digest());
    assert!(receipt.unlinked_files() > 0);
    let replayed = later
        .execute_packed_object_gc(&chain, &source, plan.digest(), backup)
        .unwrap();
    assert_eq!(replayed.unlinked_files(), receipt.unlinked_files());

    let mut parquet = Vec::new();
    collect_parquet(temporary.path(), &mut parquet);
    assert!(parquet.iter().any(|path| parent_named(path, "packs")));
    assert!(parquet.iter().all(|path| {
        !path
            .components()
            .any(|component| component.as_os_str() == "objects")
    }));
    let rows = later
        .read_observations_by_sequence(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(rows.len(), 2);

    let dataset = dataset_dir(temporary.path());
    let remaining_roots: Vec<_> = std::fs::read_dir(dataset.join("roots"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(remaining_roots.len(), 1);
    let remaining_leases: Vec<_> = std::fs::read_dir(dataset.join("leases"))
        .map(|entries| {
            entries
                .map(|entry| entry.unwrap().path())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    for lease in &remaining_leases {
        let name = lease.file_name().unwrap().to_str().unwrap();
        let hex = name
            .strip_prefix("root-")
            .and_then(|value| value.strip_suffix(".lease"))
            .unwrap();
        assert!(
            dataset
                .join("roots")
                .join(format!("root-{hex}.json"))
                .is_file(),
            "unlocked lease survived after its root was unreferenced"
        );
    }
    let manifests: Vec<_> =
        std::fs::read_dir(temporary.path().join("_manifests").join("raw-byte-v3"))
            .map(|entries| {
                entries
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
    assert!(
        manifests.is_empty(),
        "packed logical manifests must be eligible after digest-authenticated packing"
    );
    let verified = later.verify_raw_manifest(first.manifest_id()).unwrap();
    assert_eq!(verified.manifest_sha256(), first.manifest_sha256());
    later
        .verify_raw_manifest_at_sequence(first.manifest_id(), first.local_sequence_range().unwrap())
        .unwrap();
}

#[test]
fn packed_object_gc_fails_closed_while_an_old_root_lease_is_held() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS);
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    let iterator = archive
        .read_observations_by_sequence(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    drop(archive);
    let later = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS + 1_000_000);
    assert!(matches!(
        later.plan_packed_object_gc(&chain, &source, [0xAB; 32]),
        Err(ArchiveError::WriterBusy)
    ));
    drop(iterator);
    let plan = later
        .plan_packed_object_gc(&chain, &source, [0xAB; 32])
        .unwrap();
    assert!(!plan.is_empty());
}

#[test]
fn receipt_hint_rebuild_is_non_authoritative_and_reauthenticates() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let first = archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    archive.rebuild_receipt_hints(&chain, &source).unwrap();
    let (first_seq, last_seq) = archive
        .lookup_receipt_hint(&chain, &source, first.manifest_id())
        .unwrap();
    assert_eq!(first_seq, 1);
    assert_eq!(last_seq, 1);
    let missing =
        domain_types::ManifestId::new(format!("archive-manifest-v1-{}", "11".repeat(32))).unwrap();
    assert!(matches!(
        archive.lookup_receipt_hint(&chain, &source, &missing),
        Err(ArchiveError::ReceiptIndexRebuildRequired)
    ));
}

#[test]
fn gc_fails_closed_when_an_eligible_object_vanishes_before_unlink() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS);
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    drop(archive);
    let later = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS + 1_000_000);
    let plan = later
        .plan_packed_object_gc(&chain, &source, [0xAB; 32])
        .unwrap();
    let (path, _, _) = plan.files().next().expect("eligible file");
    std::fs::remove_file(temporary.path().join(path)).unwrap();
    assert!(matches!(
        later.execute_packed_object_gc(&chain, &source, plan.digest(), [0xAB; 32]),
        Err(ArchiveError::ManifestVerification(_))
    ));
}

#[test]
fn pack_index_embeds_receipt_hint_pages() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_archive(temporary.path());
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let first = archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive.pack_index(&chain, &source).unwrap();
    let (first_seq, last_seq) = archive
        .lookup_receipt_hint(&chain, &source, first.manifest_id())
        .unwrap();
    assert_eq!(first_seq, 1);
    assert_eq!(last_seq, 1);
    let hints = dataset_dir(temporary.path()).join("hints");
    let standalone_pages: Vec<_> = std::fs::read_dir(&hints)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .filter(|name| {
            name.to_str()
                .is_some_and(|value| value.starts_with("hint-"))
        })
        .collect();
    assert!(
        standalone_pages.is_empty(),
        "pack_index must embed hint pages in the index pack"
    );
    let packs: Vec<_> = std::fs::read_dir(dataset_dir(temporary.path()).join("index-packs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("pack"))
        .collect();
    assert_eq!(packs.len(), 1);
}

#[test]
fn gc_resumes_when_a_journaled_planned_file_is_already_gone() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS);
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    drop(archive);
    let later = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS + 1_000_000);
    let plan = later
        .plan_packed_object_gc(&chain, &source, [0xAB; 32])
        .unwrap();
    let (path, object_sha256, byte_len) = plan
        .files()
        .find(|(path, _, _)| !path.contains("/leases/"))
        .map(|(path, hash, len)| (path.to_owned(), hash.to_owned(), len))
        .expect("eligible file");
    let journal = dataset_dir(temporary.path())
        .join("gc")
        .join(format!("deletion-{}.log", encode_sha256(plan.digest())));
    std::fs::write(
        &journal,
        format!(
            "{{\"kind\":\"planned\",\"relative_path\":\"{path}\",\"object_sha256\":\"{object_sha256}\",\"byte_len\":{byte_len}}}\n"
        ),
    )
    .unwrap();
    std::fs::remove_file(temporary.path().join(&path)).unwrap();
    let receipt = later
        .execute_packed_object_gc(&chain, &source, plan.digest(), [0xAB; 32])
        .unwrap();
    assert_eq!(receipt.unlinked_files(), plan.files().count() as u64);
}

#[test]
fn gc_reclaims_exclusive_leases_after_their_roots_are_gone() {
    let temporary = tempfile::tempdir().unwrap();
    let archive = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS);
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    drop(archive);
    let later = open_retention_archive(temporary.path(), FIXTURE_TIME_MICROS + 1_000_000);
    let backup = [0xAB; 32];
    let plan = later
        .plan_packed_object_gc(&chain, &source, backup)
        .unwrap();
    let old_root = plan
        .files()
        .map(|(path, _, _)| path.to_owned())
        .find(|path| path.contains("/roots/root-") && path.ends_with(".json"))
        .expect("old root is eligible");
    later
        .execute_packed_object_gc(&chain, &source, plan.digest(), backup)
        .unwrap();
    assert!(!temporary.path().join(&old_root).exists());
    let name = std::path::Path::new(&old_root)
        .file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_prefix("root-"))
        .and_then(|value| value.strip_suffix(".json"))
        .unwrap();
    let leftover = dataset_dir(temporary.path())
        .join("leases")
        .join(format!("root-{name}.lease"));
    std::fs::create_dir_all(leftover.parent().unwrap()).unwrap();
    std::fs::write(&leftover, b"stale-exclusive-lease").unwrap();
    let leftover_plan = later
        .plan_packed_object_gc(&chain, &source, backup)
        .unwrap();
    assert!(
        leftover_plan
            .files()
            .any(|(path, _, _)| path.ends_with(&format!("leases/root-{name}.lease")))
    );
    later
        .execute_packed_object_gc(&chain, &source, leftover_plan.digest(), backup)
        .unwrap();
    assert!(!leftover.exists());
}

fn encode_sha256(hash: [u8; 32]) -> String {
    hash.iter().map(|byte| format!("{byte:02x}")).collect()
}
