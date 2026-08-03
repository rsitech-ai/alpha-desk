use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};

use serde_json::Value;
use state_replay::{PositionRunConfig, run_position_e2e};

const FALSE_QUALIFICATIONS: &[&str] = &[
    "stage_1_qualified",
    "stage_2_qualified",
    "deployed_source_qualified",
    "live_source_qualified",
    "authoritative_opening_position_qualified",
    "authoritative_opening_balance_qualified",
    "venue_position_reconciliation_qualified",
    "protocol_entry_price_parity_qualified",
    "source_closed_pnl_completeness_qualified",
    "execution_fee_attribution_qualified",
    "twap_position_completeness_qualified",
    "backstop_cost_basis_qualified",
    "standard_margin_qualified",
    "unified_margin_qualified",
    "portfolio_margin_qualified",
    "liquidation_price_qualified",
    "book_state_qualified",
    "signal_state_qualified",
    "execution_qualified",
    "live_product_qualified",
];

#[test]
fn position_e2e_proves_only_the_frozen_synthetic_position_contract() {
    let temporary = private_temporary_root();
    let output = temporary.path().join("position-evidence");
    let evidence = run_position_e2e(&PositionRunConfig {
        output: output.clone(),
        blocks: 8,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect("position evidence");

    assert_eq!(
        evidence.report_path,
        output
            .canonicalize()
            .expect("canonical output")
            .join("report.json")
    );
    let report_bytes = fs::read(&evidence.report_path).expect("report");
    let report: Value = serde_json::from_slice(&report_bytes).expect("JSON");
    let ordered_fields = [
        "schema_version",
        "evidence_class",
        "state_semantics",
        "source_qualification",
        "reducer_version",
        "synthetic_position_contract_proven",
        "stage_1_qualified",
        "stage_2_qualified",
        "deployed_source_qualified",
        "live_source_qualified",
        "authoritative_opening_position_qualified",
        "authoritative_opening_balance_qualified",
        "venue_position_reconciliation_qualified",
        "protocol_entry_price_parity_qualified",
        "source_closed_pnl_completeness_qualified",
        "execution_fee_attribution_qualified",
        "twap_position_completeness_qualified",
        "backstop_cost_basis_qualified",
        "standard_margin_qualified",
        "unified_margin_qualified",
        "portfolio_margin_qualified",
        "liquidation_price_qualified",
        "book_state_qualified",
        "signal_state_qualified",
        "execution_qualified",
        "live_product_qualified",
        "block_count",
        "checkpoint_after",
        "iterations_completed",
        "expected_final_state_hash",
        "resumed_final_state_hash",
        "checkpoint_state_hash_before_publish",
        "checkpoint_state_hash_after_load",
        "deterministic_full_replay_receipt_hash",
        "segmented_resume_receipt_hashes",
        "checkpoint_id",
        "replay_elapsed_micros",
        "namespace_counts",
        "duplicate_trade_identity",
        "start_position_mismatch",
        "unsupported_schema",
    ];
    let report_text = std::str::from_utf8(&report_bytes).expect("UTF-8 report");
    let mut prior = None;
    for field in ordered_fields {
        let offset = report_text
            .find(&format!("\"{field}\""))
            .unwrap_or_else(|| panic!("missing report field {field}"));
        assert!(prior.is_none_or(|previous| previous < offset), "{field}");
        prior = Some(offset);
    }
    assert_eq!(
        report.as_object().expect("report object").len(),
        ordered_fields.len()
    );
    assert_eq!(
        report["schema_version"],
        "hyperliquid-alpha-desk/state-replay-position-e2e-report/v1"
    );
    assert_eq!(report["evidence_class"], "synthetic_canonical_position");
    assert_eq!(
        report["state_semantics"],
        "exact_trade_anchored_quantity_and_analytical_episode_flows"
    );
    assert_eq!(report["source_qualification"], "synthetic_unassessed");
    assert_eq!(
        report["reducer_version"],
        "hyperliquid-alpha-desk-canonical-state@1.0.0"
    );
    assert_eq!(report["synthetic_position_contract_proven"], true);
    for field in FALSE_QUALIFICATIONS {
        assert_eq!(report[*field], false, "{field}");
    }
    assert_eq!(report["block_count"], 8);
    assert_eq!(report["checkpoint_after"], 1);
    assert_eq!(report["iterations_completed"], 2);
    assert_eq!(
        report["expected_final_state_hash"],
        report["resumed_final_state_hash"]
    );
    assert_eq!(
        report["checkpoint_state_hash_before_publish"],
        report["checkpoint_state_hash_after_load"]
    );
    assert!(
        report["deterministic_full_replay_receipt_hash"]
            .as_str()
            .is_some()
    );
    let segmented = report["segmented_resume_receipt_hashes"]
        .as_array()
        .expect("segmented receipts");
    assert_eq!(segmented.len(), 7);
    assert!(segmented.iter().all(Value::is_string));
    assert!(
        segmented
            .iter()
            .all(|hash| hash != &report["deterministic_full_replay_receipt_hash"])
    );
    let expected_counts = BTreeMap::from([
        ("account-fact.v1", 3),
        ("account-quote-flow-current.v1", 1),
        ("asset-context-current.v1", 2),
        ("backstop-liquidation-fact.v1", 1),
        ("dex-current.v1", 1),
        ("liquidation-current.v1", 1),
        ("liquidation-fill-fact.v1", 1),
        ("liquidation-market-flow-current.v1", 1),
        ("liquidation-start-fact.v1", 1),
        ("market-current.v1", 1),
        ("market-fact.v1", 4),
        ("market-metadata-version.v1", 1),
        ("order-current.v1", 6),
        ("order-fact.v1", 6),
        ("order-transition.v1", 6),
        ("position-effect-fact.v1", 6),
        ("position-episode-current.v1", 2),
        ("position-episode-effect-fact.v1", 14),
        ("position-episode.v1", 7),
        ("position-quantity-current.v1", 2),
        ("position-settlement-fact.v1", 1),
        ("position-unresolved-cause-fact.v1", 2),
        ("reconciliation.v1", 3),
        ("trade-participant.v1", 6),
        ("trade-participant.v2", 6),
        ("trade-reconciliation.v2", 3),
        ("trade.v1", 3),
        ("trade.v2", 3),
    ]);
    assert_eq!(
        serde_json::from_value::<BTreeMap<String, usize>>(report["namespace_counts"].clone())
            .expect("namespace counts"),
        expected_counts
            .into_iter()
            .map(|(name, count)| (name.to_owned(), count))
            .collect()
    );
    assert_atomic_rejection(
        &report["duplicate_trade_identity"],
        "ledger.reducer_failed",
        Some("trade_state.trade_id_collision"),
    );
    assert_atomic_rejection(
        &report["start_position_mismatch"],
        "ledger.reducer_failed",
        Some("position_state.start_position_mismatch"),
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
fn position_e2e_refuses_incomplete_suffix_and_existing_or_unsafe_output() {
    let temporary = private_temporary_root();
    let incomplete = temporary.path().join("incomplete");
    let error = run_position_e2e(&PositionRunConfig {
        output: incomplete.clone(),
        blocks: 7,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("seven suffix blocks are mandatory");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!incomplete.exists());

    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).expect("existing output");
    let error = run_position_e2e(&PositionRunConfig {
        output: existing,
        blocks: 8,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("existing output");
    assert_eq!(error.reason_code(), "state_replay.output_exists");

    let public_parent = temporary.path().join("public-parent");
    fs::create_dir(&public_parent).expect("public parent");
    fs::set_permissions(&public_parent, fs::Permissions::from_mode(0o777))
        .expect("public permissions");
    let error = run_position_e2e(&PositionRunConfig {
        output: public_parent.join("evidence"),
        blocks: 8,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("unsafe parent");
    assert_eq!(error.reason_code(), "state_replay.unsafe_output");
}

fn private_temporary_root() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private temporary parent");
    temporary
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
    let mut entries = vec![root.to_path_buf()];
    let mut index = 0;
    while index < entries.len() {
        let metadata = fs::symlink_metadata(&entries[index]).expect("metadata");
        assert!(!metadata.file_type().is_symlink());
        let expected = if metadata.is_dir() { 0o700 } else { 0o600 };
        assert_eq!(metadata.permissions().mode() & 0o777, expected);
        if metadata.is_dir() {
            for child in fs::read_dir(&entries[index]).expect("directory") {
                entries.push(child.expect("entry").path());
            }
        }
        index += 1;
    }
}
