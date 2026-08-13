use canonical_archive::{ArchiveConfig, RawV3Archive};
use domain_types::{ChainId, KnownTime, SourceId};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    ArchiveError, LocalRecordSequence, LocalRecordSequenceRange, RawArchiveCapacityBudgets,
    RawArchiveCapacityRejection, RawArchiveWorkloadEnvelope, RawObservationArchive,
    RawObservationBatch, RawObservationRange,
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
