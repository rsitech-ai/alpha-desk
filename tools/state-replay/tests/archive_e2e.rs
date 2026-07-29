use std::{collections::BTreeMap, fs};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, KnownTime, ProtocolTime, SourceId};
use serde_json::Value;
use state_replay::{ArchiveRunConfig, run_archive_e2e};
use storage_ports::CanonicalArchive;

#[test]
fn operator_archive_replay_proves_repeat_and_checkpoint_resume_without_mutating_archive() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let archive_root = temporary.path().join("archive");
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::deterministic_fixture(
            "operator-archive-test",
            KnownTime::from_unix_micros(1_000).expect("time"),
        )
        .expect("config"),
    )
    .expect("archive");
    for height in 700..=704 {
        archive.append_block(&block(height)).expect("append");
    }
    let before = archive.inspect().expect("inspection");
    let output = temporary.path().join("evidence");

    let evidence = run_archive_e2e(&ArchiveRunConfig::new(
        &archive_root,
        &output,
        "mainnet",
        700,
        704,
        702,
        3,
    ))
    .expect("archive evidence");

    let report: Value =
        serde_json::from_slice(&fs::read(&evidence.report_path).expect("report")).expect("JSON");
    assert_eq!(
        report["schema_version"],
        "hyperliquid-alpha-desk/state-replay-archive-e2e-report/v1"
    );
    assert_eq!(report["evidence_class"], "operator_archive");
    assert_eq!(report["state_semantics"], "watermark_only");
    assert_eq!(report["source_qualification"], "unassessed");
    assert_eq!(report["stage_2_qualified"], false);
    assert_eq!(report["live_source_qualified"], false);
    assert_eq!(report["start_height"], 700);
    assert_eq!(report["end_height"], 704);
    assert_eq!(report["checkpoint_height"], 702);
    assert_eq!(report["iterations_completed"], 3);
    assert_eq!(report["manifests"].as_array().expect("manifests").len(), 5);
    assert_eq!(
        report["expected_final_state_hash"],
        report["resumed_final_state_hash"]
    );
    assert_eq!(archive.inspect().expect("after inspection"), before);
}

#[test]
fn archive_replay_rejects_missing_archives_and_non_boundary_checkpoints_before_output() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("evidence");
    let missing = temporary.path().join("missing");
    let error = run_archive_e2e(&ArchiveRunConfig::new(
        &missing, &output, "mainnet", 1, 3, 2, 1,
    ))
    .expect_err("missing archive");
    assert_eq!(error.reason_code(), "state_replay.invalid_archive");
    assert!(!output.exists());

    let archive_root = temporary.path().join("archive");
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::deterministic_fixture(
            "operator-archive-test",
            KnownTime::from_unix_micros(1_000).expect("time"),
        )
        .expect("config"),
    )
    .expect("archive");
    for height in 10..=12 {
        archive.append_block(&block(height)).expect("append");
    }
    let error = run_archive_e2e(&ArchiveRunConfig::new(
        &archive_root,
        &output,
        "mainnet",
        10,
        12,
        12,
        1,
    ))
    .expect_err("checkpoint must precede end");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!output.exists());
}

fn block(height: u64) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("operator-archive-test").expect("source"),
            *blake3::hash(&height.to_be_bytes()).as_bytes(),
        )]),
    )
    .expect("block")
}
