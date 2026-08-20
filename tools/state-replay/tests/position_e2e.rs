use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};

use serde_json::{Value, json};
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
        blocks: 9,
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
        "fixture_oracle",
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
        "hyperliquid-alpha-desk-canonical-state@1.1.0"
    );
    assert_eq!(report["synthetic_position_contract_proven"], true);
    for field in FALSE_QUALIFICATIONS {
        assert_eq!(report[*field], false, "{field}");
    }
    assert_eq!(report["block_count"], 9);
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
    assert_eq!(segmented.len(), 8);
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
    assert_eq!(report["fixture_oracle"], literal_fixture_oracle());
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
        blocks: 8,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("eight suffix blocks are mandatory");
    assert_eq!(error.reason_code(), "state_replay.invalid_config");
    assert!(!incomplete.exists());

    let existing = temporary.path().join("existing");
    fs::create_dir(&existing).expect("existing output");
    let error = run_position_e2e(&PositionRunConfig {
        output: existing,
        blocks: 9,
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
        blocks: 9,
        checkpoint_after: 1,
        iterations: 2,
    })
    .expect_err("unsafe parent");
    assert_eq!(error.reason_code(), "state_replay.unsafe_output");
}

fn literal_fixture_oracle() -> Value {
    json!({
        "order_ids": {
            "opening_buyer": "position-open-buyer-order",
            "opening_seller": "position-open-seller-order",
            "reversal_buyer": "position-reversal-buyer-order",
            "reversal_seller": "position-reversal-seller-order",
            "recovery_buyer": "position-recovery-buyer-order",
            "recovery_seller": "position-recovery-seller-order"
        },
        "transaction_ids": {
            "opening_trade": "state-replay-position-open",
            "reversal_trade": "state-replay-position-reversal",
            "first_funding": "state-replay-position-first-funding",
            "liquidation_start": "state-replay-position-liquidation-start",
            "liquidation_fill": "state-replay-position-liquidation-fill",
            "backstop": "state-replay-position-backstop",
            "interrupted_funding": "state-replay-position-interrupted-funding",
            "settlement": "state-replay-position-settlement",
            "recovery_trade": "state-replay-position-recovery",
            "recovered_funding": "state-replay-position-recovered-funding"
        },
        "event_ids": {
            "opening_trade": "evt_0f54a9acbbdb126364303e7e93878130c42499955e67eb6cb640dc1643600a25",
            "reversal_trade": "evt_b8bc3c3f9ed9d85213efcb035eb745e49ce36bffc73a7789384426094aa511c5",
            "first_funding": "evt_c5d3fd8c057ee3c47c9010ce0f015c6391720925d84d0fc072c78a598164d238",
            "liquidation_start": "evt_a34e728e41e50caf47597029b2f039d8b5620b385ca020b7bae6c3bf19ccfd27",
            "liquidation_fill": "evt_ffc9f124e05781b31ed9ae7539e1436425f2f8a485ae640b6c4210907937e925",
            "backstop": "evt_36175305dbf66d7b0f68aa9ac983505aae70ae622054696705f624fcdde9fc14",
            "interrupted_funding": "evt_e61efc65a6c475598a13a275757b866b9a98ac39b71ae3ef58314e766842ac29",
            "settlement": "evt_8bcbf171818f1bb585b40d3ee1d356c6091d8cd23c797c013dd25fe8af9e5cd7",
            "recovery_trade": "evt_85d37b63e9690e3e78b6db8ccea250fef5e2ab3d4ae09b946d655a7bb5a4b59e",
            "recovered_funding": "evt_f0b7248da826eb6663d7fbf7873b1cec720e4b2393e8ae8bd98da7f6873c8219"
        },
        "episode_ids": {
            "opening_buyer": "pos_ep_2a8da82f97ac3c5f0382810be9a0bcf72968f884f430d5168d541ccea818666f",
            "opening_seller": "pos_ep_ecdd714df2737000b14bdc6b764dfb66bd5b5fc5c3b5b13fbc812e927e740acd",
            "reversal_buyer": "pos_ep_98bfaaf042280cdc79f91323388bc93d1f70ae86daee8da3aee656e9013d1a64",
            "reversal_seller": "pos_ep_36fad52e94c45dfc37a692d1fd935c049596273bda936d8adcdb95f48fe21e18",
            "liquidation_remainder": "pos_ep_887ca9925bd8847c9062b33f9953391cac58cdb5da7fee25d9b92d7bb7cee2a4",
            "recovery_buyer": "pos_ep_ed420bee0f852ff46725fcda495d1ee47b996e08a4561bf4f83e8c57a5e009af",
            "recovery_seller": "pos_ep_a265c5b07a93f39c1e454bec14da83a21bd07fb2fd36df9261e8320a2c11a4ce"
        },
        "state_keys": {
            "buyer_quantity_current": {"namespace": "position-quantity-current.v1", "key_hex": "000000000000001411111111111111111111111111111111111111110000000000000008706572703a425443"},
            "seller_quantity_current": {"namespace": "position-quantity-current.v1", "key_hex": "000000000000001422222222222222222222222222222222222222220000000000000008706572703a425443"},
            "recovery_buyer_effect": {"namespace": "position-effect-fact.v1", "key_hex": "00000000000000157472642d706f736974696f6e2d7265636f7665727900000000000000056275796572"},
            "recovery_seller_effect": {"namespace": "position-effect-fact.v1", "key_hex": "00000000000000157472642d706f736974696f6e2d7265636f76657279000000000000000673656c6c6572"},
            "buyer_unresolved_cause": {"namespace": "position-unresolved-cause-fact.v1", "key_hex": "000000000000001411111111111111111111111111111111111111110000000000000008706572703a42544300000000000000446576745f3336313735333035646266363664376230663638616139616339383335303561616537306165363232303534363936373035663632346663646465396663313400000000000000106c69712d706f736974696f6e2d653265"},
            "seller_unresolved_cause": {"namespace": "position-unresolved-cause-fact.v1", "key_hex": "000000000000001422222222222222222222222222222222222222220000000000000008706572703a42544300000000000000446576745f3336313735333035646266363664376230663638616139616339383335303561616537306165363232303534363936373035663632346663646465396663313400000000000000106c69712d706f736974696f6e2d653265"},
            "settlement_fact": {"namespace": "position-settlement-fact.v1", "key_hex": "00000000000000446576745f38626362663137313831386631626235383562343064336565316433353663363039316438636432336337393763303133646432356665386166396535636437000000000000001411111111111111111111111111111111111111110000000000000008706572703a425443"}
        },
        "notionals": {
            "opening_buyer_buy": "200",
            "opening_seller_sell": "200",
            "reversal_buyer_close_buy": "220",
            "reversal_buyer_open_buy": "110",
            "reversal_seller_close_sell": "220",
            "reversal_seller_open_sell": "110",
            "recovery_buyer_buy": "23.75",
            "recovery_seller_sell": "23.75"
        }
    })
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
