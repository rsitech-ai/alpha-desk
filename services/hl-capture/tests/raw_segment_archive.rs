use std::{fs::OpenOptions, io::Write, sync::Arc};

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use domain_types::{ChainId, SourceId};
use hl_capture::spool::{DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolRotationPolicy};
use hl_capture::{BlockingRawSegmentArchive, RawSegmentArchive};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{RawObservationArchive, RawObservationRange};
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
        .archive_segment(&ChainId::new("mainnet").unwrap(), &closed, 1024)
        .await
        .unwrap();
    let second = archiver
        .archive_segment(&ChainId::new("mainnet").unwrap(), &closed, 1024)
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
        .archive_segment(&ChainId::new("mainnet").unwrap(), &closed, 1024)
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
        .archive_segment(&ChainId::new("mainnet").unwrap(), &sealed, 1024)
        .await
        .unwrap();
    drop(recovered);

    let reopened = spool(&root);
    assert_eq!(reopened.closed_segments(), std::slice::from_ref(&sealed));
    archiver
        .archive_segment(
            &ChainId::new("mainnet").unwrap(),
            &reopened.closed_segments()[0],
            1024,
        )
        .await
        .unwrap();
    assert_eq!(archive.inspect().unwrap().raw_observations(), 1);
}
