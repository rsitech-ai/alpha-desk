use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use serde_json::Value;
use state_replay::{MarketRunConfig, run_market_e2e};

#[test]
fn market_e2e_proves_exact_registry_repeat_resume_and_fail_closed_boundaries() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("market-evidence");
    let evidence =
        run_market_e2e(&MarketRunConfig::new(&output, 5, 2, 3)).expect("market evidence");

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
        "hyperliquid-alpha-desk/state-replay-market-e2e-report/v1"
    );
    assert_eq!(report["evidence_class"], "synthetic_canonical_market");
    assert_eq!(report["state_semantics"], "exact_market_registry");
    assert_eq!(report["source_qualification"], "synthetic_unassessed");
    assert_eq!(
        report["reducer_set_version"],
        "hyperliquid-alpha-desk-canonical-market@1.0.0"
    );
    assert_eq!(report["synthetic_market_contract_proven"], true);
    for field in [
        "stage_1_qualified",
        "stage_2_qualified",
        "live_source_qualified",
        "deployed_source_qualified",
        "authoritative_metadata_qualified",
        "external_oracle_reconciliation_qualified",
        "account_state_qualified",
        "position_state_qualified",
        "margin_state_qualified",
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
    assert_eq!(
        report["expected_final_state_hash"],
        report["unresolved_final_state_hash"]
    );
    assert_eq!(report["metadata_transition_height"], 1_000_004);
    assert_eq!(report["market_fact_count"], 29);
    assert_eq!(report["dex_current_count"], 1);
    assert_eq!(report["asset_context_current_count"], 2);
    assert_eq!(report["market_current_count"], 1);
    assert_eq!(report["market_metadata_version_count"], 2);
    assert_eq!(report["outcome_current_count"], 1);
    assert_eq!(report["active_market_count"], 1);
    assert_eq!(report["halted_market_count"], 0);
    assert_eq!(report["exact_current_metadata_count"], 0);
    assert_eq!(report["unresolved_current_metadata_count"], 1);
    assert_eq!(report["exact_metadata_version_count"], 1);
    assert_eq!(report["unresolved_metadata_version_count"], 1);
    assert_eq!(report["resolved_outcome_count"], 1);
    assert_eq!(report["unresolved_outcome_count"], 0);
    assert_eq!(report["sample_market"]["market_id"], "perp:BTC");
    assert_eq!(report["sample_market"]["dex_id"], "hyperliquid");
    assert_eq!(report["sample_market"]["base_asset_id"], "BTC");
    assert_eq!(report["sample_market"]["quote_asset_id"], "USDC");
    assert_eq!(report["sample_market"]["status"], "active");
    assert_eq!(report["sample_market"]["metadata_resolution"], "unresolved");
    assert_eq!(
        report["sample_market"]["metadata_version"],
        "metadata@1.0.1"
    );
    for field in [
        "tick_size",
        "lot_size",
        "price_scale",
        "quantity_scale",
        "open_interest_cap",
        "margin_table_hash",
        "oracle_price",
        "oracle_source",
        "oracle_effective_at_micros",
        "funding_rate",
        "funding_effective_at_micros",
    ] {
        assert!(report["sample_market"][field].is_null(), "{field}");
    }
    assert_eq!(report["sample_market"]["outcome_id"], "BTC-ABOVE-60000");
    assert_eq!(report["sample_market"]["outcome_resolution"], "resolved");
    assert_eq!(report["sample_market"]["settlement_value"], "1.000000");

    let metadata = &report["hash_only_metadata"];
    assert_eq!(metadata["prior_version"], "creation@1.0.0");
    assert_eq!(metadata["next_version"], "metadata@1.0.1");
    assert_eq!(metadata["prior_effective_until_block"], 1_000_003);
    assert_eq!(metadata["next_effective_from_block"], 1_000_004);
    assert_eq!(metadata["next_resolution"], "unresolved");
    for field in [
        "tick_size",
        "lot_size",
        "price_scale",
        "quantity_scale",
        "open_interest_cap",
        "margin_table_hash",
        "oracle_price",
        "oracle_source",
        "oracle_effective_at_micros",
        "funding_rate",
        "funding_effective_at_micros",
    ] {
        assert!(metadata[field].is_null(), "{field}");
    }
    assert_atomic_rejection(
        &metadata["suppressed_value_update"],
        "ledger.reducer_failed",
        Some("market_state.metadata_unresolved"),
    );
    assert_atomic_rejection(
        &report["malformed_transition"],
        "ledger.reducer_failed",
        Some("market_state.invalid_status_transition"),
    );
    assert_atomic_rejection(
        &report["unsupported_schema"],
        "ledger.unsupported_event",
        None,
    );
    assert_private_tree(&output);
}

#[test]
fn market_e2e_rejects_invalid_bounds_and_existing_output() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let invalid_output = temporary.path().join("invalid");
    let error = run_market_e2e(&MarketRunConfig::new(&invalid_output, 1, 1, 1))
        .expect_err("invalid bounds");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!invalid_output.exists());

    let excessive_work = temporary.path().join("excessive");
    let error = run_market_e2e(&MarketRunConfig::new(&excessive_work, 100_000, 1, 1_000))
        .expect_err("excessive aggregate work");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!excessive_work.exists());

    let one_replay = temporary.path().join("one-replay");
    let error = run_market_e2e(&MarketRunConfig::new(&one_replay, 3, 1, 1))
        .expect_err("at least two independent replays");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!one_replay.exists());

    let unsafe_output = temporary.path().join("parent").join("..").join("unsafe");
    let error =
        run_market_e2e(&MarketRunConfig::new(&unsafe_output, 3, 1, 2)).expect_err("unsafe output");
    assert_eq!(error.reason_code(), "state_replay.unsafe_output");

    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).expect("existing");
    let error =
        run_market_e2e(&MarketRunConfig::new(&existing, 3, 1, 2)).expect_err("existing output");
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

fn assert_private_tree(path: &Path) {
    let metadata = fs::symlink_metadata(path).expect("evidence metadata");
    assert!(!metadata.file_type().is_symlink());
    if metadata.is_dir() {
        assert_eq!(metadata.permissions().mode() & 0o777, 0o700, "{path:?}");
        for entry in fs::read_dir(path).expect("evidence directory") {
            assert_private_tree(&entry.expect("evidence entry").path());
        }
    } else {
        assert!(metadata.is_file(), "{path:?}");
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600, "{path:?}");
    }
}
