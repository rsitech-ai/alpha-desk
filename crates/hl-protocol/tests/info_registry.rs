use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoEnumField, InfoError, InfoPagination, InfoParseContext, InfoRegistry,
    InfoStateTarget, REST_INFO_ENDPOINTS, encode_info_request,
};
use hl_protocol::{
    ErrorDisposition, ObservationClass, SourceTrust, observation_qualifies_committed_source,
};
use serde::Deserialize;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn received_at() -> KnownTime {
    KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time")
}

fn archive_ref() -> ArchiveRef {
    ArchiveRef::new("fixture:official-info/response-all-mids.json").expect("archive ref")
}

fn opaque_context(request_hash: blake3::Hash) -> InfoParseContext {
    InfoParseContext::new(request_hash, received_at(), archive_ref())
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[derive(Debug, Deserialize)]
struct FixtureManifest {
    schema_version: u32,
    corpus_kind: String,
    production_recording: bool,
    fixture: Vec<FixtureRow>,
}

#[derive(Debug, Deserialize)]
struct FixtureRow {
    file: String,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct CapabilityManifest {
    capability: Vec<CapabilityRow>,
}

#[derive(Debug, Deserialize)]
struct CapabilityRow {
    id: String,
    transport: String,
    identifier: String,
    domain: String,
    pagination: String,
    state_target: String,
    request_cost: String,
    source_role: String,
    network: Vec<String>,
    #[serde(default)]
    unsupported_networks: Vec<UnsupportedRow>,
}

#[derive(Debug, Deserialize)]
struct UnsupportedRow {
    network: String,
    reason: String,
}

#[test]
fn official_info_fixtures_are_hashed_and_not_production_recordings() {
    let root = fixture_root();
    let manifest: FixtureManifest =
        toml::from_str(&fs::read_to_string(root.join("manifest.toml")).expect("manifest"))
            .expect("valid manifest");
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(manifest.corpus_kind, "synthetic-framework-examples");
    assert!(!manifest.production_recording);
    for fixture in manifest.fixture {
        let payload = fs::read(root.join(&fixture.file)).expect("fixture payload");
        assert_eq!(hex::encode(Sha256::digest(&payload)), fixture.sha256);
    }
}

#[test]
fn rest_info_registry_matches_capability_manifest_one_to_one() {
    let manifest: CapabilityManifest = toml::from_str(
        &fs::read_to_string(workspace_root().join("config/hyperliquid/capabilities.toml"))
            .expect("capabilities"),
    )
    .expect("valid capabilities");
    let rows: Vec<&CapabilityRow> = manifest
        .capability
        .iter()
        .filter(|row| row.transport == "rest_info")
        .collect();
    assert_eq!(rows.len(), REST_INFO_ENDPOINTS.len());
    assert_eq!(InfoRegistry::official().len(), 61);
    InfoRegistry::try_new(REST_INFO_ENDPOINTS).expect("unique inventory");

    for (row, endpoint) in rows.iter().zip(REST_INFO_ENDPOINTS) {
        assert_eq!(row.id, endpoint.capability_id());
        assert_eq!(row.identifier, endpoint.identifier());
        assert_eq!(row.domain, endpoint.domain());
        assert_eq!(row.pagination, endpoint.pagination().as_manifest_str());
        assert_eq!(row.state_target, endpoint.state_target().as_manifest_str());
        assert_eq!(row.request_cost, endpoint.request_cost());
        assert_eq!(row.source_role, "reconciliation");
        assert_eq!(row.network, endpoint.networks());
        assert_eq!(
            row.unsupported_networks.len(),
            endpoint.unsupported_networks().len()
        );
        for (expected, actual) in row
            .unsupported_networks
            .iter()
            .zip(endpoint.unsupported_networks())
        {
            assert_eq!(expected.network, actual.network());
            assert_eq!(expected.reason, actual.reason());
        }
        assert_eq!(
            InfoRegistry::official()
                .get(&row.id)
                .expect("id")
                .identifier(),
            row.identifier
        );
        assert_eq!(
            InfoRegistry::official()
                .get_by_identifier(&row.identifier)
                .expect("type")
                .capability_id(),
            row.id
        );
    }
}

#[test]
fn info_each_manifest_entry_resolves_to_exactly_one_encoder_and_parser() {
    let registry = InfoRegistry::official();
    let empty = BTreeMap::new();
    let response = br#"{"ok":true}"#;
    for endpoint in registry.endpoints() {
        let encoded = endpoint.encode(&empty).expect("encode");
        let again = registry
            .encode(endpoint.capability_id(), &empty)
            .expect("registry encode");
        assert_eq!(encoded.body(), again.body());
        assert_eq!(encoded.content_hash(), again.content_hash());
        assert_eq!(encoded.identifier(), endpoint.identifier());
        let parsed = endpoint
            .parse(response, &opaque_context(encoded.content_hash()))
            .expect("parse");
        let parsed_again = registry
            .parse(
                endpoint.capability_id(),
                response,
                &opaque_context(encoded.content_hash()),
            )
            .expect("registry parse");
        assert_eq!(parsed.response_hash(), parsed_again.response_hash());
        assert_eq!(parsed.raw().as_ref(), response);
    }

    static DUPLICATE_ID: &[hl_protocol::info::InfoEndpoint] =
        &[REST_INFO_ENDPOINTS[0], REST_INFO_ENDPOINTS[0]];
    assert_eq!(
        InfoRegistry::try_new(DUPLICATE_ID).expect_err("duplicate id"),
        InfoError::DuplicateCapability
    );
}

#[test]
fn info_request_serialization_is_deterministic_and_stable_hashed() {
    let mut first = Map::new();
    first.insert("z".to_owned(), Value::from(1));
    first.insert("a".to_owned(), Value::from(2));
    let mut second = Map::new();
    second.insert("a".to_owned(), Value::from(2));
    second.insert("z".to_owned(), Value::from(1));

    let mut params_a = BTreeMap::new();
    params_a.insert("user".to_owned(), Value::from("0xabc"));
    params_a.insert("req".to_owned(), Value::Object(first));
    let mut params_b = BTreeMap::new();
    params_b.insert("req".to_owned(), Value::Object(second));
    params_b.insert("user".to_owned(), Value::from("0xabc"));

    let left = InfoRegistry::official()
        .encode("official.info.open_orders", &params_a)
        .expect("left");
    let right = InfoRegistry::official()
        .encode("official.info.open_orders", &params_b)
        .expect("right");
    assert_eq!(left.body(), right.body());
    assert_eq!(left.content_hash(), right.content_hash());
    assert_eq!(left.content_hash(), blake3::hash(left.body()));
    assert_eq!(
        left.body().as_ref(),
        br#"{"req":{"a":2,"z":1},"type":"openOrders","user":"0xabc"}"#
    );

    let mut aggregated = BTreeMap::new();
    aggregated.insert("aggregateByTime".to_owned(), Value::from(true));
    let with_flag = InfoRegistry::official()
        .encode("official.info.user_fills_by_time", &aggregated)
        .expect("aggregated");
    let without_flag = InfoRegistry::official()
        .encode("official.info.user_fills_by_time", &BTreeMap::new())
        .expect("plain");
    assert_ne!(with_flag.content_hash(), without_flag.content_hash());

    let mut conflict = BTreeMap::new();
    conflict.insert("type".to_owned(), Value::from("nope"));
    assert_eq!(
        encode_info_request("official.info.all_mids", "allMids", &conflict)
            .expect_err("type reserved"),
        InfoError::TypeFieldConflict
    );

    let fixture = read_fixture("request-all-mids.json");
    let encoded = InfoRegistry::official()
        .encode("official.info.all_mids", &BTreeMap::new())
        .expect("allMids");
    assert_eq!(encoded.body().as_ref(), fixture);
}

#[test]
fn info_observation_hash_is_stable_and_bytes_are_preserved() {
    let raw = read_fixture("response-all-mids.json");
    let encoded = InfoRegistry::official()
        .encode("official.info.all_mids", &BTreeMap::new())
        .expect("encode");
    let first = InfoRegistry::official()
        .parse(
            "official.info.all_mids",
            &raw,
            &opaque_context(encoded.content_hash()),
        )
        .expect("parse");
    let second = InfoRegistry::official()
        .parse(
            "official.info.all_mids",
            &raw,
            &opaque_context(encoded.content_hash()),
        )
        .expect("parse again");
    assert_eq!(first.response_hash(), blake3::hash(&raw));
    assert_eq!(first.response_hash(), second.response_hash());
    assert_eq!(first.schema_fingerprint(), second.schema_fingerprint());
    assert_eq!(first.raw().as_ref(), raw);
    assert_eq!(first.request_hash(), encoded.content_hash());
}

#[test]
fn info_unknown_fields_preserve_raw_evidence_and_change_schema_fingerprint() {
    let extra = read_fixture("response-unknown-field.json");
    const KNOWN: &[&str] = &["/px", "/sz"];
    let encoded = InfoRegistry::official()
        .encode("official.info.l2_book", &BTreeMap::new())
        .expect("encode");
    let with_extra = InfoRegistry::official()
        .get("official.info.l2_book")
        .expect("endpoint")
        .parse(
            &extra,
            &opaque_context(encoded.content_hash()).with_known_fields(KNOWN),
        )
        .expect("parse extra");
    let without_extra = InfoRegistry::official()
        .get("official.info.l2_book")
        .expect("endpoint")
        .parse(
            br#"{"px":"1.0","sz":"2.0"}"#,
            &opaque_context(encoded.content_hash()).with_known_fields(KNOWN),
        )
        .expect("parse known");
    assert_eq!(with_extra.raw().as_ref(), extra);
    assert_eq!(
        with_extra
            .unknown_fields()
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>(),
        ["/note"]
    );
    assert_eq!(without_extra.unknown_fields(), &[]);
    assert_ne!(
        with_extra.schema_fingerprint(),
        without_extra.schema_fingerprint()
    );
    assert!(
        with_extra
            .warnings()
            .iter()
            .any(|warning| warning.code() == "info.unknown_field")
    );
}

#[test]
fn info_schema_fingerprint_ignores_array_cardinality_and_dynamic_map_keys() {
    let fills = InfoRegistry::official()
        .encode("official.info.user_fills_by_time", &BTreeMap::new())
        .expect("encode fills");
    let two = InfoRegistry::official()
        .parse(
            "official.info.user_fills_by_time",
            br#"[{"px":"1.0"},{"px":"2.0"}]"#,
            &opaque_context(fills.content_hash()),
        )
        .expect("two rows");
    let three = InfoRegistry::official()
        .parse(
            "official.info.user_fills_by_time",
            br#"[{"px":"1.0"},{"px":"2.0"},{"px":"3.0"}]"#,
            &opaque_context(fills.content_hash()),
        )
        .expect("three rows");
    assert_eq!(two.schema_fingerprint(), three.schema_fingerprint());

    let mids = InfoRegistry::official()
        .encode("official.info.all_mids", &BTreeMap::new())
        .expect("encode mids");
    let listed = InfoRegistry::official()
        .parse(
            "official.info.all_mids",
            br#"{"BTC":"1.0","ETH":"2.0"}"#,
            &opaque_context(mids.content_hash()),
        )
        .expect("two markets");
    let new_market = InfoRegistry::official()
        .parse(
            "official.info.all_mids",
            br#"{"BTC":"1.0","ETH":"2.0","SOL":"3.0"}"#,
            &opaque_context(mids.content_hash()),
        )
        .expect("three markets");
    assert_eq!(listed.schema_fingerprint(), new_market.schema_fingerprint());
    assert_ne!(
        two.schema_fingerprint(),
        listed.schema_fingerprint(),
        "array-of-fields and mid-price map must not collapse to the same shape"
    );
}

#[test]
fn info_known_fields_match_array_shaped_payloads() {
    const KNOWN: &[&str] = &["/px"];
    let encoded = InfoRegistry::official()
        .encode("official.info.user_fills_by_time", &BTreeMap::new())
        .expect("encode");
    let known_only = InfoRegistry::official()
        .get("official.info.user_fills_by_time")
        .expect("endpoint")
        .parse(
            br#"[{"px":"1.0"},{"px":"2.0"}]"#,
            &opaque_context(encoded.content_hash()).with_known_fields(KNOWN),
        )
        .expect("array of known fields");
    assert_eq!(known_only.unknown_fields(), &[]);

    let with_extra = InfoRegistry::official()
        .get("official.info.user_fills_by_time")
        .expect("endpoint")
        .parse(
            br#"[{"px":"1.0"},{"px":"2.0","note":"x"}]"#,
            &opaque_context(encoded.content_hash()).with_known_fields(KNOWN),
        )
        .expect("array with extra");
    assert_eq!(
        with_extra
            .unknown_fields()
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>(),
        ["/1/note"]
    );
}

#[test]
fn info_unknown_state_affecting_enum_variants_quarantine() {
    const STATUS: &[InfoEnumField] = &[InfoEnumField::new("/status", &["open", "filled"])];
    let raw = read_fixture("response-unknown-variant.json");
    let encoded = InfoRegistry::official()
        .encode("official.info.open_orders", &BTreeMap::new())
        .expect("encode");
    let error = InfoRegistry::official()
        .get("official.info.open_orders")
        .expect("openOrders is state-affecting")
        .parse(
            &raw,
            &opaque_context(encoded.content_hash()).with_enum_fields(STATUS),
        )
        .expect_err("quarantine");
    assert_eq!(
        error,
        InfoError::UnknownStateAffectingVariant {
            path: "/status".to_owned(),
            value: "brand_new_status".to_owned(),
        }
    );
    assert_eq!(error.disposition(), ErrorDisposition::Quarantine);

    let warning = InfoRegistry::official()
        .get("official.info.all_mids")
        .expect("allMids is not state-affecting")
        .parse(
            &raw,
            &opaque_context(encoded.content_hash()).with_enum_fields(STATUS),
        )
        .expect("continue");
    assert!(
        warning
            .warnings()
            .iter()
            .any(|item| item.code() == "info.unknown_enum_variant")
    );
    assert_eq!(warning.raw().as_ref(), raw);
}

#[test]
fn info_decimals_reject_overflow_invalid_scale_and_json_floats() {
    let encoded = InfoRegistry::official()
        .encode("official.info.l2_book", &BTreeMap::new())
        .expect("encode");
    let overflow = InfoRegistry::official()
        .parse(
            "official.info.l2_book",
            &read_fixture("response-decimal-overflow.json"),
            &opaque_context(encoded.content_hash()),
        )
        .expect_err("overflow");
    assert_eq!(
        overflow,
        InfoError::DecimalOverflow {
            path: "/px".to_owned()
        }
    );
    assert_eq!(overflow.disposition(), ErrorDisposition::Quarantine);

    let scale = InfoRegistry::official()
        .parse(
            "official.info.l2_book",
            &read_fixture("response-decimal-invalid-scale.json"),
            &opaque_context(encoded.content_hash()),
        )
        .expect_err("scale");
    assert_eq!(
        scale,
        InfoError::DecimalInvalidScale {
            path: "/px".to_owned()
        }
    );

    let json_float = InfoRegistry::official()
        .parse(
            "official.info.l2_book",
            &read_fixture("response-json-float.json"),
            &opaque_context(encoded.content_hash()),
        )
        .expect_err("f64");
    assert_eq!(
        json_float,
        InfoError::ForbiddenJsonNumber {
            path: "/px".to_owned()
        }
    );
    assert_eq!(json_float.disposition(), ErrorDisposition::Quarantine);
}

#[test]
fn official_info_is_reconciliation_not_committed_primary() {
    for endpoint in InfoRegistry::official().endpoints() {
        assert_ne!(endpoint.state_target(), InfoStateTarget::CommittedState);
        assert_eq!(endpoint.observation_class(), ObservationClass::Snapshot);
        assert_eq!(endpoint.source_trust(), SourceTrust::ReconciledSnapshot);
        assert!(!observation_qualifies_committed_source(
            endpoint.observation_class()
        ));
        endpoint.admission().expect("reconciled snapshot admission");
        match endpoint.pagination() {
            InfoPagination::SinglePage | InfoPagination::ByTime => {}
        }
    }
    let outcome = InfoRegistry::official()
        .get("official.info.outcome_meta")
        .expect("outcomeMeta");
    assert!(outcome.available_on("testnet"));
    assert!(!outcome.available_on("mainnet"));
}

#[test]
fn info_error_variants_have_reason_codes() {
    let errors = [
        InfoError::UnknownCapability,
        InfoError::UnknownIdentifier,
        InfoError::DuplicateCapability,
        InfoError::DuplicateIdentifier,
        InfoError::InvalidCapabilityId,
        InfoError::InvalidArchiveRef,
        InfoError::InvalidJsonPath,
        InfoError::TypeFieldConflict,
        InfoError::EmptyPayload,
        InfoError::MalformedJson,
        InfoError::DecimalOverflow {
            path: "/px".to_owned(),
        },
        InfoError::DecimalInvalidScale {
            path: "/px".to_owned(),
        },
        InfoError::DecimalInvalid {
            path: "/px".to_owned(),
        },
        InfoError::ForbiddenJsonNumber {
            path: "/px".to_owned(),
        },
        InfoError::UnknownStateAffectingVariant {
            path: "/status".to_owned(),
            value: "x".to_owned(),
        },
        InfoError::InvalidCursor,
        InfoError::InvalidCoverage,
    ];
    for error in errors {
        assert!(!error.reason_code().is_empty());
        let _ = error.disposition();
    }
}
