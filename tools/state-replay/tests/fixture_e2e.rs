use std::{fs, os::unix::fs::PermissionsExt};

use serde_json::Value;
use state_replay::{FixtureRunConfig, run_fixture_e2e};

#[test]
fn fixture_e2e_proves_repeat_resume_and_poison_boundaries() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("evidence");
    let evidence =
        run_fixture_e2e(&FixtureRunConfig::new(&output, 5, 2, 3)).expect("fixture evidence");

    assert_eq!(
        evidence.report_path,
        output
            .canonicalize()
            .expect("canonical output")
            .join("report.json")
    );
    let report: Value =
        serde_json::from_slice(&fs::read(&evidence.report_path).expect("report")).expect("JSON");
    assert_eq!(
        report["schema_version"],
        "hyperliquid-alpha-desk/state-replay-e2e-report/v1"
    );
    assert_eq!(report["evidence_class"], "synthetic_fixture");
    assert_eq!(report["stage_2_qualified"], false);
    assert_eq!(report["live_source_qualified"], false);
    assert_eq!(report["block_count"], 5);
    assert_eq!(report["checkpoint_after"], 2);
    assert_eq!(report["iterations_completed"], 3);
    assert_eq!(
        report["expected_final_state_hash"],
        report["resumed_final_state_hash"]
    );
    assert_eq!(report["poison"]["reason_code"], "replay.block_quarantined");
    assert_eq!(
        report["poison"]["source_reason_code"],
        "ledger.unsupported_event"
    );
    assert_eq!(report["poison"]["applied_block_count"], 0);
    assert_eq!(
        report["poison"]["state_hash_before"],
        report["poison"]["state_hash_after"]
    );
    assert!(output.join("archive").is_dir());
    assert!(output.join("checkpoints").is_dir());
    assert_eq!(
        fs::metadata(&evidence.report_path)
            .expect("report metadata")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(output.canonicalize().expect("canonical output"))
            .expect("output metadata")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
}

#[test]
fn invalid_bounds_and_existing_output_fail_closed() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let invalid_output = temporary.path().join("invalid");
    let error = run_fixture_e2e(&FixtureRunConfig::new(&invalid_output, 1, 1, 1))
        .expect_err("invalid bounds");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!invalid_output.exists());

    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).expect("existing");
    let error =
        run_fixture_e2e(&FixtureRunConfig::new(&existing, 3, 1, 1)).expect_err("existing output");
    assert_eq!(error.reason_code(), "state_replay.output_exists");
}
