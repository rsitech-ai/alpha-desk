use hyperliquid_capabilities::{
    parse_manifest, parse_request_cost_base_weight, rest_info_base_weight, validate_manifest,
    Status, REST_INFO_WEIGHT_2,
};

fn valid_manifest() -> String {
    r#"
schema_version = 1

[[capability]]
id = "official.info.all_mids"
source = "official"
network = ["mainnet", "testnet"]
transport = "rest_info"
identifier = "allMids"
domain = "market_data"
source_role = "reconciliation"
request_cost = "base:2"
pagination = "none"
parser = "planned"
fixture_set = "none"
retention = "raw_indefinite"
freshness_target_ms = 1000
owner = "hl-capture"
state_target = "reference_snapshot"
status = "planned"
limitations = "REST /info adapter is not on this tree"
"#
    .to_owned()
}

fn assert_error_contains(source: &str, needle: &str) {
    let manifest = parse_manifest(source).expect("fixture must parse");
    let errors = validate_manifest(&manifest).expect_err("fixture must fail validation");
    assert!(
        errors.iter().any(|error| error.contains(needle)),
        "expected {needle:?} in {errors:?}"
    );
}

#[test]
fn duplicate_capability_ids_fail() {
    let source = format!(
        "{}\n{}",
        valid_manifest(),
        r#"
[[capability]]
id = "official.info.all_mids"
source = "official"
network = ["mainnet", "testnet"]
transport = "websocket"
identifier = "allMids"
domain = "market_data"
source_role = "reconciliation"
request_cost = "base:2"
pagination = "none"
parser = "planned"
fixture_set = "none"
retention = "raw_indefinite"
freshness_target_ms = 1000
owner = "hl-capture"
state_target = "reference_snapshot"
status = "planned"
limitations = "duplicate id"
"#
    );
    assert_error_contains(&source, "duplicate capability id: official.info.all_mids");
}

#[test]
fn missing_owner_fails() {
    let source = valid_manifest().replace("owner = \"hl-capture\"", "owner = \"\"");
    assert_error_contains(&source, "missing owner: official.info.all_mids");
}

#[test]
fn missing_parser_fails() {
    let source = valid_manifest().replace("parser = \"planned\"", "parser = \"\"");
    assert_error_contains(&source, "missing parser: official.info.all_mids");
}

#[test]
fn missing_fixture_set_fails_for_implemented_status() {
    let source = valid_manifest()
        .replace(
            "status = \"planned\"",
            "status = \"implemented_unqualified\"",
        )
        .replace(
            "parser = \"planned\"",
            "parser = \"hl_protocol::node::v1::parse_node_record\"",
        );
    assert_error_contains(&source, "missing fixture set: official.info.all_mids");
}

#[test]
fn missing_fixture_set_fails_for_qualified_live() {
    let source = valid_manifest().replace("status = \"planned\"", "status = \"qualified_live\"");
    assert_error_contains(&source, "missing fixture set: official.info.all_mids");
}

#[test]
fn unsupported_network_requires_a_reason() {
    let source = valid_manifest().replace(
        "network = [\"mainnet\", \"testnet\"]",
        "network = [\"testnet\"]",
    );
    assert_error_contains(
        &source,
        "unsupported network requires a reason: official.info.all_mids (mainnet)",
    );
}

#[test]
fn unsupported_status_limitations_do_not_satisfy_per_network_reason() {
    for status in ["unsupported", "unsupported_by_network"] {
        let source = valid_manifest()
            .replace(
                "network = [\"mainnet\", \"testnet\"]",
                "network = [\"testnet\"]",
            )
            .replace("status = \"planned\"", &format!("status = \"{status}\""));
        assert_error_contains(
            &source,
            "unsupported network requires a reason: official.info.all_mids (mainnet)",
        );
    }
}

#[test]
fn state_affecting_capabilities_cannot_be_opaque_continue() {
    for target in [
        "committed_state",
        "canonical_event",
        "reconciled_snapshot",
        "l4_book",
    ] {
        let source = valid_manifest()
            .replace("parser = \"planned\"", "parser = \"opaque_continue\"")
            .replace(
                "state_target = \"reference_snapshot\"",
                &format!("state_target = \"{target}\""),
            );
        assert_error_contains(
            &source,
            "state-affecting capability cannot be opaque_continue: official.info.all_mids",
        );
    }
}

#[test]
fn reference_snapshot_may_be_opaque_continue() {
    let source = valid_manifest().replace("parser = \"planned\"", "parser = \"opaque_continue\"");
    let manifest = parse_manifest(&source).expect("fixture must parse");
    validate_manifest(&manifest).expect("reference_snapshot opaque_continue must pass");
}

#[test]
fn unknown_state_target_fails_to_parse() {
    let source = valid_manifest().replace(
        "state_target = \"reference_snapshot\"",
        "state_target = \"commited_state\"",
    );
    parse_manifest(&source).expect_err("typo must not deserialize");
}

#[test]
fn status_ladder_requires_fixture_set() {
    assert!(Status::Implemented.requires_fixture_set());
    assert!(Status::ImplementedUnqualified.requires_fixture_set());
    assert!(Status::QualifiedLive.requires_fixture_set());
    assert!(Status::QualifiedReplay.requires_fixture_set());
    assert!(!Status::Planned.requires_fixture_set());
    assert!(!Status::Degraded.requires_fixture_set());
}

#[test]
fn rest_info_base_weight_follows_spec_12_1() {
    assert_eq!(REST_INFO_WEIGHT_2.len(), 6);
    for identifier in REST_INFO_WEIGHT_2 {
        assert_eq!(rest_info_base_weight(identifier), 2, "{identifier}");
    }
    assert_eq!(rest_info_base_weight("userRole"), 60);
    assert_eq!(rest_info_base_weight("userFills"), 20);
    assert_eq!(rest_info_base_weight("borrowLendUserState"), 20);
    assert_eq!(
        parse_request_cost_base_weight("base:20 variable:window"),
        Some(20)
    );
    assert_eq!(parse_request_cost_base_weight("base:60"), Some(60));
}

#[test]
fn rest_info_request_cost_mismatch_fails() {
    let source =
        valid_manifest().replace("request_cost = \"base:2\"", "request_cost = \"base:20\"");
    assert_error_contains(
        &source,
        "request_cost must use base:2: official.info.all_mids",
    );
}

#[test]
fn omitted_freshness_target_is_allowed() {
    let source = valid_manifest().replace("freshness_target_ms = 1000\n", "");
    let manifest = parse_manifest(&source).expect("omitted freshness must parse");
    validate_manifest(&manifest).expect("omitted freshness must pass");
    assert_eq!(manifest.capability[0].freshness_target_ms, None);
}

#[test]
fn evm_fact_and_discovery_only_may_be_opaque_continue() {
    for target in ["evm_fact", "discovery_only"] {
        let source = valid_manifest()
            .replace("parser = \"planned\"", "parser = \"opaque_continue\"")
            .replace(
                "state_target = \"reference_snapshot\"",
                &format!("state_target = \"{target}\""),
            );
        let manifest = parse_manifest(&source).expect("fixture must parse");
        validate_manifest(&manifest)
            .unwrap_or_else(|errors| panic!("{target} opaque_continue must pass: {errors:?}"));
        assert!(!manifest.capability[0].state_target.is_state_affecting());
    }
}

#[test]
fn freshness_target_zero_fails() {
    let source = valid_manifest().replace("freshness_target_ms = 1000", "freshness_target_ms = 0");
    assert_error_contains(
        &source,
        "freshness_target_ms must be greater than 0: official.info.all_mids",
    );
}

#[test]
fn valid_fixture_passes() {
    let manifest = parse_manifest(&valid_manifest()).expect("valid fixture must parse");
    validate_manifest(&manifest).expect("valid fixture must pass");
}
