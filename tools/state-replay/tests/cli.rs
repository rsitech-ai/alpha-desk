use std::process::Command;

#[test]
fn cli_emits_stable_exit_codes_and_runs_the_fixture_evidence_path() {
    let binary = env!("CARGO_BIN_EXE_state-replay");

    let missing = Command::new(binary).output().expect("missing invocation");
    assert_eq!(missing.status.code(), Some(2));
    assert!(missing.stdout.is_empty());
    assert_eq!(
        String::from_utf8(missing.stderr).expect("UTF-8"),
        "usage: state-replay fixture-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay trade-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay order-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay market-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay account-e2e --output PATH --blocks N --checkpoint-after N --iterations N\n       state-replay archive-e2e --archive PATH --output PATH --chain ID --start-height N --end-height N --checkpoint-height N --iterations N\n"
    );

    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("evidence");
    let success = Command::new(binary)
        .args([
            "fixture-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "2",
        ])
        .output()
        .expect("successful invocation");
    assert_eq!(success.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(success.stdout).expect("UTF-8"),
        "PASS evidence_class=synthetic_fixture stage_2_qualified=false live_source_qualified=false\n"
    );
    assert!(success.stderr.is_empty());
    assert!(output.join("report.json").is_file());

    let repeated = Command::new(binary)
        .args([
            "fixture-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "1",
        ])
        .output()
        .expect("repeated invocation");
    assert_eq!(repeated.status.code(), Some(1));
    assert!(repeated.stdout.is_empty());
    assert_eq!(
        String::from_utf8(repeated.stderr).expect("UTF-8"),
        "ERROR state_replay.output_exists\n"
    );
}

#[test]
fn cli_runs_the_canonical_account_evidence_path_without_overclaiming() {
    let binary = env!("CARGO_BIN_EXE_state-replay");
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("account-evidence");

    let success = Command::new(binary)
        .args([
            "account-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "2",
        ])
        .output()
        .expect("successful invocation");

    assert_eq!(success.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(success.stdout).expect("UTF-8"),
        "PASS evidence_class=synthetic_canonical_account state_semantics=exact_observed_account_flows_relations_and_modes synthetic_account_flow_contract_proven=true source_qualification=synthetic_unassessed position_state_qualified=false episode_state_qualified=false liquidation_state_qualified=false settlement_state_qualified=false funding_attribution_qualified=false stage_1_qualified=false stage_2_qualified=false deployed_source_qualified=false live_source_qualified=false authoritative_opening_balance_qualified=false venue_balance_reconciliation_qualified=false twap_position_completeness_qualified=false backstop_cost_basis_qualified=false standard_margin_qualified=false unified_margin_qualified=false portfolio_margin_qualified=false liquidation_price_qualified=false book_state_qualified=false signal_state_qualified=false execution_qualified=false\n"
    );
    assert!(success.stderr.is_empty());
    assert!(output.join("report.json").is_file());
}

#[test]
fn cli_runs_the_canonical_trade_evidence_path_without_overclaiming() {
    let binary = env!("CARGO_BIN_EXE_state-replay");
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("trade-evidence");

    let success = Command::new(binary)
        .args([
            "trade-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "2",
        ])
        .output()
        .expect("successful invocation");

    assert_eq!(success.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(success.stdout).expect("UTF-8"),
        "PASS evidence_class=synthetic_canonical_trade state_semantics=canonical_trade_facts_and_exact_participant_anchors stage_1_qualified=false stage_2_qualified=false live_source_qualified=false\n"
    );
    assert!(success.stderr.is_empty());
    assert!(output.join("report.json").is_file());
}

#[test]
fn cli_runs_the_canonical_order_evidence_path_without_overclaiming() {
    let binary = env!("CARGO_BIN_EXE_state-replay");
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("order-evidence");

    let success = Command::new(binary)
        .args([
            "order-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "2",
        ])
        .output()
        .expect("successful invocation");

    assert_eq!(success.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(success.stdout).expect("UTF-8"),
        "PASS evidence_class=synthetic_canonical_order state_semantics=exact_order_lifecycle synthetic_order_contract_proven=true stage_1_qualified=false stage_2_qualified=false live_source_qualified=false deployed_source_qualified=false position_state_qualified=false margin_state_qualified=false execution_qualified=false\n"
    );
    assert!(success.stderr.is_empty());
    assert!(output.join("report.json").is_file());
}

#[test]
fn cli_runs_the_canonical_market_evidence_path_without_overclaiming() {
    let binary = env!("CARGO_BIN_EXE_state-replay");
    let temporary = tempfile::tempdir().expect("temporary root");
    let output = temporary.path().join("market-evidence");

    let success = Command::new(binary)
        .args([
            "market-e2e",
            "--output",
            output.to_str().expect("UTF-8 output"),
            "--blocks",
            "3",
            "--checkpoint-after",
            "1",
            "--iterations",
            "2",
        ])
        .output()
        .expect("successful invocation");

    assert_eq!(success.status.code(), Some(0));
    assert_eq!(
        String::from_utf8(success.stdout).expect("UTF-8"),
        "PASS evidence_class=synthetic_canonical_market state_semantics=exact_market_registry synthetic_market_contract_proven=true stage_1_qualified=false stage_2_qualified=false live_source_qualified=false deployed_source_qualified=false authoritative_metadata_qualified=false external_oracle_reconciliation_qualified=false account_state_qualified=false position_state_qualified=false margin_state_qualified=false book_state_qualified=false signal_state_qualified=false execution_qualified=false\n"
    );
    assert!(success.stderr.is_empty());
    assert!(output.join("report.json").is_file());
}
