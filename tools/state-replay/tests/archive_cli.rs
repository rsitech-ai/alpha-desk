use std::{collections::BTreeMap, process::Command};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, KnownTime, ProtocolTime, SourceId};
use storage_ports::CanonicalArchive;

#[test]
fn archive_cli_emits_explicit_unqualified_operator_evidence() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let archive_root = temporary.path().join("archive");
    let archive = LocalParquetArchive::open(
        &archive_root,
        ArchiveConfig::deterministic_fixture(
            "archive-cli-test",
            KnownTime::from_unix_micros(1_000).expect("time"),
        )
        .expect("config"),
    )
    .expect("archive");
    for height in 800..=802 {
        archive.append_block(&block(height)).expect("append");
    }
    let output = temporary.path().join("evidence");

    let result = Command::new(env!("CARGO_BIN_EXE_state-replay"))
        .args([
            "archive-e2e",
            "--archive",
            archive_root.to_str().expect("archive UTF-8"),
            "--output",
            output.to_str().expect("output UTF-8"),
            "--chain",
            "mainnet",
            "--start-height",
            "800",
            "--end-height",
            "802",
            "--checkpoint-height",
            "801",
            "--iterations",
            "2",
        ])
        .output()
        .expect("archive CLI");

    assert_eq!(result.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(result.stdout).expect("stdout UTF-8"),
        "PASS evidence_class=operator_archive state_semantics=watermark_only stage_2_qualified=false live_source_qualified=false\n"
    );
    assert!(result.stderr.is_empty());
    assert!(output.join("report.json").is_file());
}

fn block(height: u64) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("archive-cli-test").expect("source"),
            *blake3::hash(&height.to_be_bytes()).as_bytes(),
        )]),
    )
    .expect("block")
}
