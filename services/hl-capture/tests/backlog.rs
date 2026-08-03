use std::fs::OpenOptions;
use std::io::Write;
use std::time::Duration;

use bytes::Bytes;
use domain_types::SourceId;
use hl_capture::spool::{DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolRotationPolicy};
use hl_capture::{
    BacklogError, BacklogRead, ByteOffsetSpoolBacklog, SequencedBacklogRead, SpoolBacklog,
};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{CursorPolicy, LocalRecordSequence};

fn observation(offset: u64) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-directory-epoch", offset).unwrap(),
        ReceiveTimestamps::new(i64::try_from(offset).unwrap(), offset).unwrap(),
        "node-v1",
        Bytes::from(format!("payload-{offset}")),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn byte_observation(epoch: &str, offset: u64) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::AuxiliaryLedger,
        SourceCursor::new(epoch, offset).unwrap(),
        ReceiveTimestamps::new(i64::try_from(offset).unwrap(), offset).unwrap(),
        "node-v1",
        Bytes::from(format!("payload-{offset}")),
        Vec::new(),
        1024,
    )
    .unwrap()
}

#[test]
fn backlog_skips_durable_prefix_and_advances_only_after_exact_acknowledgement() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("spool/primary-node");
    let mut spool = SourceSpool::open(
        SourceSpoolConfig::try_new(
            directory.clone(),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
        )
        .unwrap(),
        100,
    )
    .unwrap();
    for offset in 100..105 {
        spool
            .append(&observation(offset), i64::try_from(offset).unwrap())
            .unwrap();
    }
    spool.shutdown(200).unwrap();

    let mut backlog = SpoolBacklog::open(
        directory,
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        102,
        1024,
    )
    .unwrap();
    let first = match backlog.next_observation().unwrap() {
        BacklogRead::Observation(observation) => observation,
        other => panic!("expected observation, got {other:?}"),
    };
    assert_eq!(first.cursor().offset(), 102);
    assert_eq!(backlog.next_expected_offset(), 102);
    assert!(matches!(
        backlog.next_observation().unwrap_err(),
        BacklogError::PendingAcknowledgement
    ));
    assert!(matches!(
        backlog.acknowledge(103).unwrap_err(),
        BacklogError::AcknowledgementMismatch
    ));
    backlog.acknowledge(102).unwrap();
    assert_eq!(backlog.next_expected_offset(), 103);

    for expected in 103..105 {
        let observation = match backlog.next_observation().unwrap() {
            BacklogRead::Observation(observation) => observation,
            other => panic!("expected observation, got {other:?}"),
        };
        assert_eq!(observation.cursor().offset(), expected);
        backlog.acknowledge(expected).unwrap();
    }
    assert!(matches!(
        backlog.next_observation().unwrap(),
        BacklogRead::CaughtUp {
            next_expected_offset: 105
        }
    ));
}

#[test]
fn byte_offset_backlog_advances_by_physical_sequence_not_native_offset() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("spool/primary-node");
    let mut spool = SourceSpool::open(
        SourceSpoolConfig::try_new_with_cursor_policy(
            directory.clone(),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(1, Duration::from_secs(3600)).unwrap(),
            CursorPolicy::MonotonicByteOffset,
        )
        .unwrap(),
        100,
    )
    .unwrap();
    for offset in [17, 49, 121] {
        spool
            .append(
                &byte_observation("node-directory-epoch", offset),
                i64::try_from(offset).unwrap(),
            )
            .unwrap();
    }
    spool.shutdown(200).unwrap();

    let sequence_two = LocalRecordSequence::try_new(2).unwrap();
    let mut backlog = ByteOffsetSpoolBacklog::open(
        directory.clone(),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        sequence_two,
        1024,
    )
    .unwrap();
    let first = match backlog.next_observation().unwrap() {
        SequencedBacklogRead::Observation(observation) => observation,
        other => panic!("expected sequenced observation, got {other:?}"),
    };
    assert_eq!(first.local_sequence(), sequence_two);
    assert_eq!(first.observation().cursor().offset(), 49);
    assert_eq!(backlog.next_expected_sequence(), sequence_two);
    assert!(matches!(
        backlog.next_observation(),
        Err(BacklogError::PendingAcknowledgement)
    ));
    assert!(matches!(
        backlog.acknowledge(LocalRecordSequence::try_new(3).unwrap()),
        Err(BacklogError::AcknowledgementMismatch)
    ));
    backlog.acknowledge(sequence_two).unwrap();

    let third = match backlog.next_observation().unwrap() {
        SequencedBacklogRead::Observation(observation) => observation,
        other => panic!("expected sequenced observation, got {other:?}"),
    };
    assert_eq!(third.local_sequence().get(), 3);
    assert_eq!(third.observation().cursor().offset(), 121);
    backlog.acknowledge(third.local_sequence()).unwrap();
    assert!(matches!(
        backlog.next_observation().unwrap(),
        SequencedBacklogRead::CaughtUp {
            next_expected_sequence
        } if next_expected_sequence.get() == 4
    ));

    let mut restarted = ByteOffsetSpoolBacklog::open(
        directory,
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        LocalRecordSequence::try_new(3).unwrap(),
        1024,
    )
    .unwrap();
    let replay = match restarted.next_observation().unwrap() {
        SequencedBacklogRead::Observation(observation) => observation,
        other => panic!("expected replay observation, got {other:?}"),
    };
    assert_eq!(replay.local_sequence().get(), 3);
    assert_eq!(replay.observation().cursor().offset(), 121);

    let mut ahead = ByteOffsetSpoolBacklog::open(
        root.path().join("spool/primary-node"),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        LocalRecordSequence::try_new(5).unwrap(),
        1024,
    )
    .unwrap();
    assert!(matches!(
        ahead.next_observation(),
        Err(BacklogError::SequenceGap {
            expected: 5,
            observed: 4
        })
    ));
}

#[test]
fn byte_offset_backlog_preserves_epoch_boundaries_and_rejects_legacy_segments() {
    let root = tempfile::tempdir().unwrap();
    let byte_directory = root.path().join("spool/byte");
    let mut byte_spool = SourceSpool::open(
        SourceSpoolConfig::try_new_with_cursor_policy(
            byte_directory.clone(),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            CursorPolicy::MonotonicByteOffset,
        )
        .unwrap(),
        100,
    )
    .unwrap();
    byte_spool
        .append(&byte_observation("epoch-a", 49), 101)
        .unwrap();
    byte_spool
        .append(&byte_observation("epoch-b", 7), 102)
        .unwrap();
    byte_spool.shutdown(103).unwrap();

    let mut backlog = ByteOffsetSpoolBacklog::open(
        byte_directory.clone(),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        LocalRecordSequence::try_new(1).unwrap(),
        1024,
    )
    .unwrap();
    let first = match backlog.next_observation().unwrap() {
        SequencedBacklogRead::Observation(observation) => observation,
        other => panic!("expected first epoch, got {other:?}"),
    };
    assert_eq!(first.observation().cursor().epoch(), "epoch-a");
    backlog.acknowledge(first.local_sequence()).unwrap();
    let second = match backlog.next_observation().unwrap() {
        SequencedBacklogRead::Observation(observation) => observation,
        other => panic!("expected second epoch, got {other:?}"),
    };
    assert_eq!(second.local_sequence().get(), 2);
    assert_eq!(second.observation().cursor().epoch(), "epoch-b");

    let mut wrong_policy = SpoolBacklog::open(
        byte_directory,
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        49,
        1024,
    )
    .unwrap();
    assert!(matches!(
        wrong_policy.next_observation(),
        Err(BacklogError::CursorPolicyMismatch)
    ));

    let legacy_directory = root.path().join("spool/legacy");
    let legacy = SourceSpool::open(
        SourceSpoolConfig::try_new(
            legacy_directory.clone(),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
        )
        .unwrap(),
        100,
    )
    .unwrap();
    drop(legacy);
    assert!(matches!(
        ByteOffsetSpoolBacklog::open(
            legacy_directory,
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            LocalRecordSequence::try_new(1).unwrap(),
            1024,
        ),
        Err(BacklogError::CursorPolicyMismatch)
    ));
}

#[test]
fn byte_offset_reconstruction_failure_retries_the_same_physical_record() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("spool/primary-node");
    let mut spool = SourceSpool::open(
        SourceSpoolConfig::try_new_with_cursor_policy(
            directory.clone(),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            CursorPolicy::MonotonicByteOffset,
        )
        .unwrap(),
        100,
    )
    .unwrap();
    spool.append(&byte_observation("epoch-a", 17), 101).unwrap();
    spool.append(&byte_observation("epoch-a", 49), 102).unwrap();
    spool.shutdown(103).unwrap();

    let mut backlog = ByteOffsetSpoolBacklog::open(
        directory,
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        LocalRecordSequence::try_new(1).unwrap(),
        1,
    )
    .unwrap();
    for _ in 0..3 {
        assert!(matches!(
            backlog.next_observation(),
            Err(BacklogError::Observation)
        ));
        assert_eq!(backlog.next_expected_sequence().get(), 1);
    }
}

#[test]
fn byte_offset_segment_open_failure_does_not_advance_on_retry() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("spool/primary-node");
    let mut spool = SourceSpool::open(
        SourceSpoolConfig::try_new_with_cursor_policy(
            directory.clone(),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            CursorPolicy::MonotonicByteOffset,
        )
        .unwrap(),
        100,
    )
    .unwrap();
    spool.append(&byte_observation("epoch-a", 17), 101).unwrap();
    let segment = spool.active_segment_path().to_owned();
    drop(spool);
    let mut backlog = ByteOffsetSpoolBacklog::open(
        directory,
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        LocalRecordSequence::try_new(1).unwrap(),
        1024,
    )
    .unwrap();
    let moved = segment.with_extension("hlsp.away");
    std::fs::rename(&segment, &moved).unwrap();

    for _ in 0..2 {
        assert!(matches!(
            backlog.next_observation(),
            Err(BacklogError::Spool(
                hl_capture::spool::SpoolError::Io { .. }
            ))
        ));
        assert_eq!(backlog.next_expected_sequence().get(), 1);
    }
}

#[test]
fn byte_offset_closed_verification_failure_cannot_be_retried_past() {
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join("spool/primary-node");
    let mut spool = SourceSpool::open(
        SourceSpoolConfig::try_new_with_cursor_policy(
            directory.clone(),
            SourceId::new("primary-node").unwrap(),
            "hyperliquid-node-v1",
            "spool-v1",
            [0x42; 32],
            DurabilityPolicy::FsyncEveryRecord,
            SpoolRotationPolicy::try_new(u64::MAX, Duration::from_secs(3600)).unwrap(),
            CursorPolicy::MonotonicByteOffset,
        )
        .unwrap(),
        100,
    )
    .unwrap();
    spool.append(&byte_observation("epoch-a", 17), 101).unwrap();
    let close = spool.shutdown(102).unwrap().unwrap();
    let mut backlog = ByteOffsetSpoolBacklog::open(
        directory,
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        LocalRecordSequence::try_new(1).unwrap(),
        1024,
    )
    .unwrap();
    let first = match backlog.next_observation().unwrap() {
        SequencedBacklogRead::Observation(observation) => observation,
        other => panic!("expected observation, got {other:?}"),
    };
    backlog.acknowledge(first.local_sequence()).unwrap();
    OpenOptions::new()
        .append(true)
        .open(close.manifest_path())
        .unwrap()
        .write_all(b"\n")
        .unwrap();

    for _ in 0..2 {
        assert!(matches!(
            backlog.next_observation(),
            Err(BacklogError::Spool(
                hl_capture::spool::SpoolError::ManifestContentMismatch
            ))
        ));
        assert_eq!(backlog.next_expected_sequence().get(), 2);
    }
}
