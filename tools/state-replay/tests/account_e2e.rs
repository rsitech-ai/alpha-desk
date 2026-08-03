use std::{fs, os::unix::fs::PermissionsExt};

use serde_json::Value;
use state_replay::{AccountRunConfig, run_account_e2e};

#[test]
fn account_e2e_proves_exact_synthetic_account_flows_relations_modes_and_boundaries() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("account-evidence");
    let evidence = run_account_e2e(&AccountRunConfig {
        output: output.clone(),
        blocks: 5,
        checkpoint_after: 2,
        iterations: 3,
    })
    .expect("account evidence");

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
        "hyperliquid-alpha-desk/state-replay-account-e2e-report/v1"
    );
    assert_eq!(report["evidence_class"], "synthetic_canonical_account");
    assert_eq!(
        report["state_semantics"],
        "exact_observed_account_flows_relations_and_modes"
    );
    assert_eq!(report["source_qualification"], "synthetic_unassessed");
    assert_eq!(
        report["reducer_version"],
        "hyperliquid-alpha-desk-canonical-state@1.0.0"
    );
    assert_eq!(report["synthetic_account_flow_contract_proven"], true);
    for field in [
        "position_state_qualified",
        "episode_state_qualified",
        "liquidation_state_qualified",
        "settlement_state_qualified",
        "funding_attribution_qualified",
        "stage_1_qualified",
        "stage_2_qualified",
        "deployed_source_qualified",
        "live_source_qualified",
        "authoritative_opening_balance_qualified",
        "venue_balance_reconciliation_qualified",
        "twap_position_completeness_qualified",
        "backstop_cost_basis_qualified",
        "standard_margin_qualified",
        "unified_margin_qualified",
        "portfolio_margin_qualified",
        "liquidation_price_qualified",
        "book_state_qualified",
        "signal_state_qualified",
        "execution_qualified",
    ] {
        assert_eq!(report[field], false, "{field}");
    }
    assert_eq!(report["block_count"], 5);
    assert_eq!(report["checkpoint_after"], 2);
    assert_eq!(report["iterations_completed"], 3);
    assert_eq!(
        report["expected_final_state_hash"],
        report["resumed_final_state_hash"]
    );
    assert!(
        report["deterministic_replay_receipt_hash"]
            .as_str()
            .is_some()
    );
    assert!(report["resume_receipt_hash"].as_str().is_some());
    assert_eq!(report["account_fact_count"], 15);
    assert_eq!(report["account_quantity_flow_current_count"], 10);
    assert_eq!(report["account_quote_flow_current_count"], 4);
    assert_eq!(report["vault_principal_flow_current_count"], 1);
    assert_eq!(report["vault_share_flow_current_count"], 1);
    assert_eq!(report["subaccount_master_current_count"], 1);
    assert_eq!(report["account_vault_relation_current_count"], 1);
    assert_eq!(report["account_mode_current_count"], 1);
    assert_eq!(report["margin_mode_current_count"], 1);
    assert_eq!(report["leverage_current_count"], 1);
    assert!(report.get("debit_credit_symmetry_proven").is_none());
    assert_atomic_rejection(
        &report["missing_asset_prerequisite"],
        "ledger.reducer_failed",
        Some("account_state.asset_prerequisite_missing"),
    );
    assert_atomic_rejection(
        &report["missing_market_prerequisite"],
        "ledger.reducer_failed",
        Some("account_state.market_prerequisite_missing"),
    );
    assert_atomic_rejection(
        &report["cross_component_late_invalid"],
        "ledger.reducer_failed",
        Some("trade_state.trade_id_collision"),
    );
    assert_atomic_rejection(
        &report["unsupported_schema"],
        "ledger.unsupported_event",
        None,
    );
    assert!(output.join("archive").is_dir());
    assert!(output.join("checkpoints").is_dir());
    assert_private_tree(&output);
}

#[test]
fn account_e2e_refuses_invalid_or_existing_output() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let invalid = temporary.path().join("invalid");
    let error = run_account_e2e(&AccountRunConfig {
        output: invalid.clone(),
        blocks: 2,
        checkpoint_after: 1,
        iterations: 1,
    })
    .expect_err("invalid bounds");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!invalid.exists());

    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).expect("existing output");
    let error = run_account_e2e(&AccountRunConfig {
        output: existing,
        blocks: 3,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("existing output");
    assert_eq!(error.reason_code(), "state_replay.output_exists");

    let traversal = temporary.path().join("safe").join("..").join("escape");
    let error = run_account_e2e(&AccountRunConfig {
        output: traversal,
        blocks: 3,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("traversal must fail");
    assert_eq!(error.reason_code(), "state_replay.unsafe_output");

    let file = temporary.path().join("file-parent");
    fs::write(&file, b"not a directory").expect("file parent");
    let error = run_account_e2e(&AccountRunConfig {
        output: file.join("evidence"),
        blocks: 3,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("non-directory parent must fail");
    assert_eq!(error.reason_code(), "state_replay.unsafe_output");

    let target = temporary.path().join("target");
    fs::create_dir(&target).expect("target");
    let link = temporary.path().join("link");
    std::os::unix::fs::symlink(&target, &link).expect("symlink");
    let error = run_account_e2e(&AccountRunConfig {
        output: link.join("evidence"),
        blocks: 3,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("symlink parent must fail");
    assert_eq!(error.reason_code(), "state_replay.unsafe_output");
}

fn assert_atomic_rejection(
    report: &Value,
    source_reason_code: &str,
    reducer_reason_code: Option<&str>,
) {
    assert_eq!(report["reason_code"], "replay.block_quarantined");
    assert_eq!(report["source_reason_code"], source_reason_code);
    match reducer_reason_code {
        Some(reason) => assert_eq!(report["reducer_reason_code"], reason),
        None => assert!(report["reducer_reason_code"].is_null()),
    }
    assert_eq!(report["applied_block_count"], 0);
    assert_eq!(report["state_hash_before"], report["state_hash_after"]);
}

fn assert_private_tree(root: &std::path::Path) {
    for entry in walk(root) {
        let metadata = fs::symlink_metadata(&entry).expect("metadata");
        assert!(
            !metadata.file_type().is_symlink(),
            "symlink: {}",
            entry.display()
        );
        let expected = if metadata.is_dir() { 0o700 } else { 0o600 };
        assert_eq!(
            metadata.permissions().mode() & 0o777,
            expected,
            "{}",
            entry.display()
        );
    }
}

fn walk(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut entries = vec![root.to_path_buf()];
    let mut index = 0;
    while index < entries.len() {
        if fs::symlink_metadata(&entries[index])
            .expect("metadata")
            .is_dir()
        {
            for child in fs::read_dir(&entries[index]).expect("directory") {
                entries.push(child.expect("entry").path());
            }
        }
        index += 1;
    }
    entries
}
