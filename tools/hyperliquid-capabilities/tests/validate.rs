use hyperliquid_capabilities::{parse_manifest, validate_manifest};

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
fn missing_parser_owner_fails() {
    let source = valid_manifest().replace("owner = \"hl-capture\"", "owner = \"\"");
    assert_error_contains(&source, "missing parser owner: official.info.all_mids");
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
fn state_affecting_capabilities_cannot_be_opaque_continue() {
    let source = valid_manifest()
        .replace("parser = \"planned\"", "parser = \"opaque_continue\"")
        .replace(
            "state_target = \"reference_snapshot\"",
            "state_target = \"committed_state\"",
        );
    assert_error_contains(
        &source,
        "state-affecting capability cannot be opaque_continue: official.info.all_mids",
    );
}

#[test]
fn implemented_star_is_the_serde_name_prefix() {
    use hyperliquid_capabilities::Status;

    assert!(Status::Implemented.requires_fixture_set());
    assert!(Status::ImplementedUnqualified.requires_fixture_set());
    assert!(!Status::QualifiedLive.requires_fixture_set());
    assert!(!Status::Planned.requires_fixture_set());
}

#[test]
fn valid_fixture_passes() {
    let manifest = parse_manifest(&valid_manifest()).expect("valid fixture must parse");
    validate_manifest(&manifest).expect("valid fixture must pass");
}
