use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoError, InfoParseContext, InfoRegistry, parse_outcome_meta,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t07-outcomes"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t07-outcomes").expect("archive ref"),
    )
}

#[test]
fn info_outcome_encoding() {
    let meta = parse_outcome_meta(
        &fs::read(fixture_root().join("response-outcome-meta.json")).expect("fixture"),
        context(),
    )
    .expect("outcome")
    .1;
    let market = &meta.outcomes()[0];
    assert_eq!(market.raw_id(), 123);
    assert_eq!(market.id().as_str(), "123");
    assert_eq!(market.name(), "Recurring");
    assert_eq!(market.sides()[0].name(), "Yes");
    assert_eq!(market.sides()[1].name(), "No");
}

#[test]
fn info_outcome_meta_unavailable_on_mainnet() {
    let endpoint = InfoRegistry::official()
        .get("official.info.outcome_meta")
        .expect("outcomeMeta");
    assert!(endpoint.available_on("testnet"));
    assert!(!endpoint.available_on("mainnet"));
    assert_eq!(endpoint.unsupported_networks()[0].network(), "mainnet");
}

#[test]
fn info_unsupported_network_is_capability_status_not_parser_failure() {
    let endpoint = InfoRegistry::official()
        .get("official.info.outcome_meta")
        .expect("outcomeMeta");
    assert!(!endpoint.available_on("mainnet"));
    parse_outcome_meta(
        &fs::read(fixture_root().join("response-outcome-meta.json")).expect("fixture"),
        context(),
    )
    .expect("parser still accepts the payload");
}

#[test]
fn info_unknown_outcome_side_quarantines() {
    let raw = br#"{"outcomes":[{"outcome":1,"name":"x","description":"d","sideSpecs":[{"name":"Maybe"}]}]}"#;
    let error = parse_outcome_meta(raw, context()).expect_err("side");
    assert!(matches!(
        error,
        InfoError::UnknownStateAffectingVariant { .. }
    ));
}
