use std::{fs, os::unix::fs::PermissionsExt};

use serde_json::Value;
use state_replay::{TradeRunConfig, run_trade_e2e};

#[test]
fn trade_e2e_proves_exact_state_repeat_resume_and_atomic_rejection() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("trade-evidence");
    let evidence = run_trade_e2e(&TradeRunConfig::new(&output, 5, 2, 3)).expect("trade evidence");

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
        "hyperliquid-alpha-desk/state-replay-trade-e2e-report/v1"
    );
    assert_eq!(report["evidence_class"], "synthetic_canonical_trade");
    assert_eq!(report["state_semantics"], "canonical_trade_facts");
    assert_eq!(report["source_qualification"], "synthetic_unassessed");
    assert_eq!(
        report["reducer_set_version"],
        "hyperliquid-alpha-desk-canonical-trade@1.0.0"
    );
    assert_eq!(report["stage_1_qualified"], false);
    assert_eq!(report["stage_2_qualified"], false);
    assert_eq!(report["live_source_qualified"], false);
    assert_eq!(report["account_state_qualified"], false);
    assert_eq!(report["order_state_qualified"], false);
    assert_eq!(report["position_state_qualified"], false);
    assert_eq!(report["block_count"], 5);
    assert_eq!(report["checkpoint_after"], 2);
    assert_eq!(report["iterations_completed"], 3);
    assert_eq!(
        report["expected_final_state_hash"],
        report["resumed_final_state_hash"]
    );
    assert_eq!(report["trade_record_count"], 5);
    assert_eq!(report["participant_record_count"], 10);
    assert_eq!(report["reconciliation_record_count"], 5);
    assert_eq!(report["passed_reconciliation_count"], 5);
    assert_eq!(
        report["sample_reconciliation"]["trade_id"],
        "state-replay-trade-1000004"
    );
    assert_eq!(report["sample_reconciliation"]["status"], "passed");
    assert_eq!(report["sample_reconciliation"]["quantity"], "0.01000000");
    assert_eq!(report["sample_reconciliation"]["participant_count"], 2);
    assert_eq!(report["sample_reconciliation"]["block_height"], 1_000_004);
    assert_eq!(
        report["sample_reconciliation"]["evidence_blake3"]
            .as_str()
            .expect("evidence hash")
            .len(),
        64
    );

    assert_atomic_rejection(
        &report["malformed_trade"],
        "ledger.reducer_failed",
        Some("trade_state.invalid_trade_id"),
    );
    assert_atomic_rejection(
        &report["unsupported_schema"],
        "ledger.unsupported_event",
        None,
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
fn trade_e2e_rejects_invalid_bounds_and_existing_output() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let invalid_output = temporary.path().join("invalid");
    let error =
        run_trade_e2e(&TradeRunConfig::new(&invalid_output, 1, 1, 1)).expect_err("invalid bounds");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!invalid_output.exists());

    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).expect("existing");
    let error =
        run_trade_e2e(&TradeRunConfig::new(&existing, 3, 1, 1)).expect_err("existing output");
    assert_eq!(error.reason_code(), "state_replay.output_exists");
}

fn assert_atomic_rejection(
    report: &Value,
    source_reason_code: &str,
    reducer_reason_code: Option<&str>,
) {
    assert_eq!(report["reason_code"], "replay.block_quarantined");
    assert_eq!(report["source_reason_code"], source_reason_code);
    match reducer_reason_code {
        Some(reason_code) => assert_eq!(report["reducer_reason_code"], reason_code),
        None => assert!(report["reducer_reason_code"].is_null()),
    }
    assert_eq!(report["applied_block_count"], 0);
    assert_eq!(report["state_hash_before"], report["state_hash_after"]);
}
