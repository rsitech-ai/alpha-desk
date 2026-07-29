use std::time::Duration;

use bytes::Bytes;
use domain_types::SourceId;
use hl_capture::spool::{DurabilityPolicy, SourceSpool, SourceSpoolConfig, SpoolRotationPolicy};
use hl_capture::{BacklogError, BacklogRead, SpoolBacklog};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};

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
