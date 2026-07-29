use std::{fs, os::unix::fs::PermissionsExt};

use serde_json::Value;
use state_replay::{OrderRunConfig, run_order_e2e};

#[test]
fn order_e2e_proves_exact_lifecycle_repeat_resume_and_atomic_rejection() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("order-evidence");
    let evidence = run_order_e2e(&OrderRunConfig::new(&output, 5, 2, 3)).expect("order evidence");

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
        "hyperliquid-alpha-desk/state-replay-order-e2e-report/v1"
    );
    assert_eq!(report["evidence_class"], "synthetic_canonical_order");
    assert_eq!(report["state_semantics"], "exact_order_lifecycle");
    assert_eq!(report["source_qualification"], "synthetic_unassessed");
    assert_eq!(
        report["reducer_set_version"],
        "hyperliquid-alpha-desk-canonical-order@1.0.0"
    );
    assert_eq!(report["synthetic_order_contract_proven"], true);
    assert_eq!(report["stage_1_qualified"], false);
    assert_eq!(report["stage_2_qualified"], false);
    assert_eq!(report["live_source_qualified"], false);
    assert_eq!(report["deployed_source_qualified"], false);
    assert_eq!(report["position_state_qualified"], false);
    assert_eq!(report["margin_state_qualified"], false);
    assert_eq!(report["execution_qualified"], false);
    assert_eq!(report["block_count"], 5);
    assert_eq!(report["checkpoint_after"], 2);
    assert_eq!(report["iterations_completed"], 3);
    assert_eq!(
        report["expected_final_state_hash"],
        report["resumed_final_state_hash"]
    );
    assert_eq!(report["order_fact_count"], 21);
    assert_eq!(report["order_current_count"], 5);
    assert_eq!(report["order_transition_count"], 21);
    assert_eq!(report["filled_order_count"], 3);
    assert_eq!(report["cancelled_order_count"], 2);
    assert_eq!(report["rejection_fact_count"], 2);
    assert_eq!(
        report["sample_order"]["order_id"],
        "state-replay-order-1000004"
    );
    assert_eq!(report["sample_order"]["lifecycle"], "filled");
    assert_eq!(report["sample_order"]["market_id"], "perp:BTC");
    assert_eq!(report["sample_order"]["side"], "buy");
    assert_eq!(report["sample_order"]["accepted_quantity"], "1.25000000");
    assert_eq!(report["sample_order"]["filled_quantity"], "1.25000000");
    assert_eq!(report["sample_order"]["remaining_quantity"], "0.00000000");

    assert_atomic_rejection(
        &report["malformed_order"],
        "ledger.reducer_failed",
        Some("order_state.overfill"),
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
fn order_e2e_rejects_invalid_bounds_and_existing_output() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let invalid_output = temporary.path().join("invalid");
    let error =
        run_order_e2e(&OrderRunConfig::new(&invalid_output, 1, 1, 1)).expect_err("invalid bounds");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!invalid_output.exists());

    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).expect("existing");
    let error =
        run_order_e2e(&OrderRunConfig::new(&existing, 3, 1, 1)).expect_err("existing output");
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
