use std::{fs::OpenOptions, io::Write, sync::Arc};

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use domain_types::{ChainId, SourceId};
use hl_capture::spool::{DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolRotationPolicy};
use hl_capture::{
    BlockingRawSegmentArchive, RawSegmentArchive, RawSegmentArchiveConfig,
    RawSegmentArchiveVerification, RawSpoolArchiveEvidence,
};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    CursorPolicy, LocalRecordSequence, LocalRecordSequenceRange, RawObservationArchive,
    RawObservationRange,
};
use tempfile::TempDir;

fn observation(offset: u64, wall_micros: i64) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-directory-epoch", offset).unwrap(),
        ReceiveTimestamps::new(wall_micros, offset).unwrap(),
        "node-v1",
        Bytes::from(format!("payload-{offset}")),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn spool(root: &TempDir) -> SourceSpool {
    SourceSpool::open(
        SourceSpoolConfig::try_new(
            root.path().join("spool/primary-node"),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, std::time::Duration::from_secs(3600)).unwrap(),
        )
        .unwrap(),
        100,
    )
    .unwrap()
}

fn byte_observation(offset: u64, wall_micros: i64) -> SourceObservation {
    byte_observation_with_schema(offset, wall_micros, "node-v1")
}

fn byte_observation_with_schema(
    offset: u64,
    wall_micros: i64,
    parser_schema_version: &str,
) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("node-fills").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::AuxiliaryLedger,
        SourceCursor::new("node-fills-epoch", offset).unwrap(),
        ReceiveTimestamps::new(wall_micros, offset).unwrap(),
        parser_schema_version,
        Bytes::from(format!("fill-{offset}")),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn byte_spool(root: &TempDir) -> SourceSpool {
    SourceSpool::open(
        SourceSpoolConfig::try_new_with_cursor_policy(
            root.path().join("spool/node-fills"),
            SourceId::new("node-fills").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x43; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, std::time::Duration::from_secs(3600)).unwrap(),
            CursorPolicy::MonotonicByteOffset,
        )
        .unwrap(),
        100,
    )
    .unwrap()
}

fn archive_config(max_batch_records: usize) -> RawSegmentArchiveConfig {
    RawSegmentArchiveConfig::try_new(1024, max_batch_records, 4096).unwrap()
}

#[tokio::test]
async fn verified_closed_segment_is_archived_byte_exactly_and_idempotently() {
    let root = TempDir::new().unwrap();
    let archive = Arc::new(
        LocalParquetArchive::open(
            root.path().join("archive"),
            ArchiveConfig::deterministic_fixture(
                "raw-segment-test",
                domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
    let archiver = BlockingRawSegmentArchive::new(raw_port);
    let mut source_spool = spool(&root);
    source_spool
        .append(&observation(100, 3_599_999_999), 101)
        .unwrap();
    source_spool
        .append(&observation(101, 3_600_000_000), 102)
        .unwrap();
    let closed = source_spool.shutdown(103).unwrap().unwrap();

    let first = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(1024),
        )
        .await
        .unwrap();
    let second = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(1024),
        )
        .await
        .unwrap();

    assert_eq!(first.observation_count(), 2);
    assert_eq!(first.batch_count(), 2);
    assert_eq!(first, second);
    let inspection = archive.inspect().unwrap();
    assert_eq!(inspection.raw_observations(), 2);
    let replayed = archive
        .read_observations(
            &ChainId::new("mainnet").unwrap(),
            &SourceId::new("primary-node").unwrap(),
            RawObservationRange::try_new("node-directory-epoch", 100, 101).unwrap(),
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(replayed.len(), 2);
    assert_eq!(replayed[0].payload(), &Bytes::from_static(b"payload-100"));
    assert_eq!(replayed[1].payload(), &Bytes::from_static(b"payload-101"));
}

#[tokio::test]
async fn segment_mutation_after_close_is_rejected_before_raw_publication() {
    let root = TempDir::new().unwrap();
    let archive = Arc::new(
        LocalParquetArchive::open(
            root.path().join("archive"),
            ArchiveConfig::deterministic_fixture(
                "raw-segment-test",
                domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
    let archiver = BlockingRawSegmentArchive::new(raw_port);
    let mut source_spool = spool(&root);
    source_spool.append(&observation(100, 1_000), 101).unwrap();
    let closed = source_spool.shutdown(102).unwrap().unwrap();
    OpenOptions::new()
        .append(true)
        .open(closed.segment_path())
        .unwrap()
        .write_all(b"mutation")
        .unwrap();

    let error = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(1024),
        )
        .await
        .unwrap_err();

    assert_eq!(error.reason_code(), "spool.segment_size_mismatch");
    assert_eq!(archive.inspect().unwrap().raw_observations(), 0);
}

#[tokio::test]
async fn recovered_tail_is_sealed_archived_and_safe_to_replay_idempotently() {
    let root = TempDir::new().unwrap();
    let archive = Arc::new(
        LocalParquetArchive::open(
            root.path().join("archive"),
            ArchiveConfig::deterministic_fixture(
                "raw-segment-test",
                domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
    let archiver = BlockingRawSegmentArchive::new(raw_port);
    let mut crashed = spool(&root);
    crashed.append(&observation(100, 1_000), 101).unwrap();
    drop(crashed);

    let mut recovered = spool(&root);
    let sealed = recovered.seal_active(102).unwrap().unwrap();
    archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &sealed,
            archive_config(1024),
        )
        .await
        .unwrap();
    drop(recovered);

    let reopened = spool(&root);
    assert_eq!(reopened.closed_segments(), std::slice::from_ref(&sealed));
    archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &reopened.closed_segments()[0],
            archive_config(1024),
        )
        .await
        .unwrap();
    assert_eq!(archive.inspect().unwrap().raw_observations(), 1);
}

#[tokio::test]
async fn same_hour_segment_is_streamed_into_bounded_record_batches() {
    let root = TempDir::new().unwrap();
    let archive = Arc::new(
        LocalParquetArchive::open(
            root.path().join("archive"),
            ArchiveConfig::deterministic_fixture(
                "raw-segment-test",
                domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
    let archiver = BlockingRawSegmentArchive::new(raw_port);
    let mut source_spool = spool(&root);
    for offset in 100..105 {
        source_spool
            .append(&observation(offset, 1_000), i64::try_from(offset).unwrap())
            .unwrap();
    }
    let closed = source_spool.shutdown(200).unwrap().unwrap();

    let summary = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(2),
        )
        .await
        .unwrap();

    assert_eq!(summary.observation_count(), 5);
    assert_eq!(summary.batch_count(), 3);
    assert_eq!(archive.inspect().unwrap().raw_observations(), 5);
}

#[tokio::test]
async fn sparse_byte_offset_segment_archives_and_replays_by_local_sequence() {
    let root = TempDir::new().unwrap();
    let archive = Arc::new(
        LocalParquetArchive::open(
            root.path().join("archive"),
            ArchiveConfig::deterministic_fixture(
                "raw-segment-byte-test",
                domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
    let archiver = BlockingRawSegmentArchive::new(raw_port);
    let mut source_spool = byte_spool(&root);
    for offset in [19, 47, 85] {
        source_spool
            .append(
                &byte_observation(offset, 1_000),
                i64::try_from(offset).unwrap(),
            )
            .unwrap();
    }
    let closed = source_spool.shutdown(200).unwrap().unwrap();

    let summary = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(2),
        )
        .await
        .unwrap();

    assert_eq!(summary.observation_count(), 3);
    assert_eq!(summary.batch_count(), 2);
    let sequence_range = LocalRecordSequenceRange::try_new(
        LocalRecordSequence::try_new(1).unwrap(),
        LocalRecordSequence::try_new(3).unwrap(),
    )
    .unwrap();
    let replayed = archive
        .read_observations_by_sequence(
            &ChainId::new("mainnet").unwrap(),
            &SourceId::new("node-fills").unwrap(),
            sequence_range,
        )
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        replayed
            .iter()
            .map(|item| (
                item.local_sequence().get(),
                item.observation().cursor().offset(),
            ))
            .collect::<Vec<_>>(),
        vec![(1, 19), (2, 47), (3, 85)]
    );

    let first_local_sequence = closed.manifest().first_local_sequence().unwrap();
    let last_local_sequence = closed.manifest().last_local_sequence().unwrap();
    let spool_evidence = RawSpoolArchiveEvidence::try_new(
        closed.manifest_hash(),
        closed.manifest().segment_blake3(),
        first_local_sequence,
        closed.manifest().max_cursor().clone(),
        last_local_sequence,
        closed.manifest().record_count(),
    )
    .unwrap();
    let verification = RawSegmentArchiveVerification::new(
        ChainId::new("mainnet").unwrap(),
        SourceId::new("node-fills").unwrap(),
        spool_evidence.clone(),
        summary.manifest_ids().to_vec(),
    );
    archiver
        .verify_archived_segment(&verification)
        .await
        .unwrap();
    let incomplete_verification = RawSegmentArchiveVerification::new(
        ChainId::new("mainnet").unwrap(),
        SourceId::new("node-fills").unwrap(),
        spool_evidence,
        summary.manifest_ids()[1..].to_vec(),
    );
    let error = archiver
        .verify_archived_segment(&incomplete_verification)
        .await
        .unwrap_err();
    assert_eq!(
        error.reason_code(),
        "capture_raw_archive.verification_mismatch"
    );
}

#[tokio::test]
async fn mixed_parser_dispositions_split_batches_without_breaking_local_sequence() {
    let root = TempDir::new().unwrap();
    let archive = Arc::new(
        LocalParquetArchive::open(
            root.path().join("archive"),
            ArchiveConfig::deterministic_fixture(
                "raw-segment-mixed-parser-test",
                domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
            )
            .unwrap(),
        )
        .unwrap(),
    );
    let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
    let archiver = BlockingRawSegmentArchive::new(raw_port);
    let mut source_spool = byte_spool(&root);
    for observation in [
        byte_observation_with_schema(19, 1_000, "node-v1"),
        byte_observation_with_schema(47, 1_001, "quarantine-v1:source.schema_drift"),
        byte_observation_with_schema(85, 1_002, "node-v1"),
    ] {
        let durable_at = observation.received().wall_micros();
        source_spool.append(&observation, durable_at).unwrap();
    }
    let closed = source_spool.shutdown(200).unwrap().unwrap();

    let summary = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(1024),
        )
        .await
        .unwrap();

    assert_eq!(summary.observation_count(), 3);
    assert_eq!(summary.batch_count(), 3);
    let replayed = archive
        .read_observations_by_sequence(
            &ChainId::new("mainnet").unwrap(),
            &SourceId::new("node-fills").unwrap(),
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
    assert_eq!(replayed[0].observation().parser_schema_version(), "node-v1");
    assert_eq!(
        replayed[1].observation().parser_schema_version(),
        "quarantine-v1:source.schema_drift"
    );
    assert_eq!(replayed[2].observation().parser_schema_version(), "node-v1");
    assert_eq!(
        replayed
            .iter()
            .map(|item| item.local_sequence().get())
            .collect::<Vec<_>>(),
        vec![1, 2, 3]
    );
}

fn generous_v3_capacity() -> (
    storage_ports::RawArchiveWorkloadEnvelope,
    storage_ports::RawArchiveCapacityBudgets,
) {
    (
        storage_ports::RawArchiveWorkloadEnvelope::try_new(
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
        storage_ports::RawArchiveCapacityBudgets::try_new(
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            true,
        )
        .unwrap(),
    )
}

fn open_v3(path: &std::path::Path) -> std::sync::Arc<canonical_archive::RawV3Archive> {
    let (workload, budgets) = generous_v3_capacity();
    std::sync::Arc::new(
        canonical_archive::RawV3Archive::open(
            path,
            ArchiveConfig::deterministic_fixture(
                "raw-segment-v3-test",
                domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
            )
            .unwrap(),
            workload,
            budgets,
        )
        .unwrap(),
    )
}

#[tokio::test]
async fn v3_byte_offset_segment_archives_and_open_scrubs() {
    let root = TempDir::new().unwrap();
    let archive_path = root.path().join("archive");
    let archive = open_v3(&archive_path);
    let archiver = BlockingRawSegmentArchive::from_v3(archive.clone());
    let mut source_spool = byte_spool(&root);
    for offset in [19, 47, 85] {
        source_spool
            .append(
                &byte_observation(offset, 1_000),
                i64::try_from(offset).unwrap(),
            )
            .unwrap();
    }
    let closed = source_spool.shutdown(200).unwrap().unwrap();

    let summary = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(2),
        )
        .await
        .unwrap();

    assert_eq!(summary.observation_count(), 3);
    assert_eq!(summary.batch_count(), 2);
    let entries = summary.checkpoint_entries().unwrap().unwrap();
    assert_eq!(entries.entries().len(), 2);

    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    match archive.load_checkpoint(&chain, &source).unwrap() {
        Some(canonical_archive::RawArchiveCheckpoint::V2(loaded)) => {
            assert_eq!(loaded.entries(), &entries);
        }
        other => panic!("expected archive-side V2 CURRENT, got {other:?}"),
    }
    assert_eq!(
        archiver
            .load_checkpoint_entries(&chain, &source)
            .await
            .unwrap()
            .as_ref(),
        Some(&entries)
    );
    let scrub = archive.scrub(&chain, &source).unwrap();
    assert_eq!(scrub.logical_manifest_count(), 2);
    let spool_evidence = RawSpoolArchiveEvidence::try_new(
        closed.manifest_hash(),
        closed.manifest().segment_blake3(),
        closed.manifest().first_local_sequence().unwrap(),
        closed.manifest().max_cursor().clone(),
        closed.manifest().last_local_sequence().unwrap(),
        closed.manifest().record_count(),
    )
    .unwrap();
    archiver
        .verify_archived_segment(
            &RawSegmentArchiveVerification::new(
                chain.clone(),
                source.clone(),
                spool_evidence,
                summary.manifest_ids().to_vec(),
            )
            .with_checkpoint_entries(entries),
        )
        .await
        .unwrap();
    drop(archive);

    let reopened = open_v3(&archive_path);
    let inspections = reopened.inspect_sources().unwrap();
    assert_eq!(inspections.len(), 1);
    assert_eq!(inspections[0].source_id().as_str(), "node-fills");
    assert_eq!(inspections[0].scrub().logical_manifest_count(), 2);
}

#[tokio::test]
async fn v3_archive_failure_before_publication_leaves_zero_receipts() {
    let root = TempDir::new().unwrap();
    let archive = open_v3(&root.path().join("archive"));
    let archiver = BlockingRawSegmentArchive::from_v3(archive.clone());
    let mut source_spool = byte_spool(&root);
    source_spool
        .append(&byte_observation(19, 1_000), 1_000)
        .unwrap();
    let closed = source_spool.shutdown(200).unwrap().unwrap();
    OpenOptions::new()
        .append(true)
        .open(closed.segment_path())
        .unwrap()
        .write_all(b"mutation")
        .unwrap();

    let error = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(1024),
        )
        .await
        .unwrap_err();

    assert_eq!(error.reason_code(), "spool.segment_size_mismatch");
    assert!(archive.inspect_sources().unwrap().is_empty());
    assert!(
        archive
            .load_checkpoint(
                &ChainId::new("mainnet").unwrap(),
                &SourceId::new("node-fills").unwrap(),
            )
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn v3_checkpoint_current_failure_is_not_a_successful_archive() {
    let root = TempDir::new().unwrap();
    let data = open_v3(&root.path().join("archive"));
    let checkpoint_store = open_v3(&root.path().join("checkpoint-only"));
    let archiver = BlockingRawSegmentArchive::with_v3(
        Arc::clone(&data) as Arc<dyn RawObservationArchive>,
        checkpoint_store.clone(),
    );
    let mut source_spool = byte_spool(&root);
    source_spool
        .append(&byte_observation(19, 1_000), 1_000)
        .unwrap();
    let closed = source_spool.shutdown(200).unwrap().unwrap();

    let error = archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(1024),
        )
        .await
        .unwrap_err();

    assert_eq!(error.reason_code(), "archive.range_unavailable");
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    assert!(data.load_checkpoint(&chain, &source).unwrap().is_none());
    assert!(
        checkpoint_store
            .load_checkpoint(&chain, &source)
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn v3_verify_requires_archive_side_checkpoint_current() {
    let root = TempDir::new().unwrap();
    let archive = open_v3(&root.path().join("archive"));
    let append_only =
        BlockingRawSegmentArchive::new(Arc::clone(&archive) as Arc<dyn RawObservationArchive>);
    let mut source_spool = byte_spool(&root);
    source_spool
        .append(&byte_observation(19, 1_000), 1_000)
        .unwrap();
    let closed = source_spool.shutdown(200).unwrap().unwrap();
    let summary = append_only
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &closed,
            archive_config(1024),
        )
        .await
        .unwrap();
    let entries = summary.checkpoint_entries().unwrap().unwrap();
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    assert!(archive.load_checkpoint(&chain, &source).unwrap().is_none());
    let error = BlockingRawSegmentArchive::from_v3(archive.clone())
        .load_checkpoint_entries(&chain, &source)
        .await
        .unwrap_err();
    assert_eq!(
        error.reason_code(),
        "capture_raw_archive.verification_mismatch"
    );

    let spool_evidence = RawSpoolArchiveEvidence::try_new(
        closed.manifest_hash(),
        closed.manifest().segment_blake3(),
        closed.manifest().first_local_sequence().unwrap(),
        closed.manifest().max_cursor().clone(),
        closed.manifest().last_local_sequence().unwrap(),
        closed.manifest().record_count(),
    )
    .unwrap();
    let verification = RawSegmentArchiveVerification::new(
        chain.clone(),
        source.clone(),
        spool_evidence,
        summary.manifest_ids().to_vec(),
    )
    .with_checkpoint_entries(entries);
    let error = BlockingRawSegmentArchive::from_v3(archive.clone())
        .verify_archived_segment(&verification)
        .await
        .unwrap_err();
    assert_eq!(
        error.reason_code(),
        "capture_raw_archive.verification_mismatch"
    );
}

#[tokio::test]
async fn last_local_sequence_check_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let root = TempDir::new().unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "raw-segment-last-local-seq-test",
                    domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let archiver = BlockingRawSegmentArchive::new(raw_port);
        let mut source_spool = match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => spool(&root),
            CursorPolicy::MonotonicByteOffset => byte_spool(&root),
        };
        match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => {
                source_spool.append(&observation(100, 1_000), 101).unwrap();
                source_spool.append(&observation(101, 1_001), 102).unwrap();
            }
            CursorPolicy::MonotonicByteOffset => {
                source_spool
                    .append(&byte_observation(19, 1_000), 101)
                    .unwrap();
                source_spool
                    .append(&byte_observation(47, 1_001), 102)
                    .unwrap();
            }
        }
        let closed = source_spool.shutdown(200).unwrap().unwrap();

        let summary = archiver
            .archive_segment(
                &ChainId::new("mainnet").unwrap(),
                &closed,
                archive_config(1024),
            )
            .await
            .expect(
                "last-local-sequence check still follows today's rule for this constructible cursor policy",
            );

        assert_eq!(summary.observation_count(), 2);
        match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => {
                assert!(closed.manifest().last_local_sequence().is_none());
            }
            CursorPolicy::MonotonicByteOffset => {
                assert_eq!(closed.manifest().last_local_sequence().unwrap().get(), 2);
            }
        }
        assert_eq!(archive.inspect().unwrap().raw_observations(), 2);
    }
}

#[tokio::test]
async fn verify_archived_segment_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let root = TempDir::new().unwrap();
        let archive = Arc::new(
            LocalParquetArchive::open(
                root.path().join("archive"),
                ArchiveConfig::deterministic_fixture(
                    "raw-segment-verify-policy-test",
                    domain_types::KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let raw_port: Arc<dyn RawObservationArchive> = archive.clone();
        let archiver = BlockingRawSegmentArchive::new(raw_port);
        let mut source_spool = match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => spool(&root),
            CursorPolicy::MonotonicByteOffset => byte_spool(&root),
        };
        match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => {
                source_spool.append(&observation(100, 1_000), 101).unwrap();
                source_spool.append(&observation(101, 1_001), 102).unwrap();
            }
            CursorPolicy::MonotonicByteOffset => {
                source_spool
                    .append(&byte_observation(19, 1_000), 101)
                    .unwrap();
                source_spool
                    .append(&byte_observation(47, 1_001), 102)
                    .unwrap();
            }
        }
        let closed = source_spool.shutdown(200).unwrap().unwrap();
        let summary = archiver
            .archive_segment(
                &ChainId::new("mainnet").unwrap(),
                &closed,
                archive_config(1024),
            )
            .await
            .unwrap();

        let first_local_sequence = match cursor_policy {
            CursorPolicy::MonotonicByteOffset => closed.manifest().first_local_sequence().unwrap(),
            CursorPolicy::ContiguousNativeOffset => LocalRecordSequence::try_new(1).unwrap(),
        };
        let last_local_sequence = match cursor_policy {
            CursorPolicy::MonotonicByteOffset => closed.manifest().last_local_sequence().unwrap(),
            CursorPolicy::ContiguousNativeOffset => LocalRecordSequence::try_new(2).unwrap(),
        };
        let source_id = match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => SourceId::new("primary-node").unwrap(),
            CursorPolicy::MonotonicByteOffset => SourceId::new("node-fills").unwrap(),
        };
        let spool_evidence = RawSpoolArchiveEvidence::try_new(
            closed.manifest_hash(),
            closed.manifest().segment_blake3(),
            first_local_sequence,
            closed.manifest().max_cursor().clone(),
            last_local_sequence,
            closed.manifest().record_count(),
        )
        .unwrap();
        let verification = RawSegmentArchiveVerification::new(
            ChainId::new("mainnet").unwrap(),
            source_id,
            spool_evidence,
            summary.manifest_ids().to_vec(),
        );
        let result = archiver.verify_archived_segment(&verification).await;
        match cursor_policy {
            CursorPolicy::MonotonicByteOffset => {
                result.expect("verify_archived_segment still admits MonotonicByteOffset");
            }
            CursorPolicy::ContiguousNativeOffset => {
                let error = result
                    .expect_err("verify_archived_segment still mismatches ContiguousNativeOffset");
                assert_eq!(
                    error.reason_code(),
                    "capture_raw_archive.verification_mismatch"
                );
            }
        }
    }
}
