use std::fs;
use std::path::{Path, PathBuf};

use canonical_events::MappingError;
use canonical_inspect::{InspectError, canonicalize};
use sha2::{Digest, Sha256};

fn workspace_fixture() -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/source/node-v1/trade-batch.json"),
    )
    .unwrap()
}

fn write_case(source: &[u8], market_block: &str) -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("source.json"), source).unwrap();
    let sha256 = hex::encode(Sha256::digest(source));
    let manifest = format!(
        r#"schema_version = 1
fixture_id = "test-fixture"
qualification = "normalized-public-documentation-example"
production_recording = false
source_path = "source.json"
source_sha256 = "{sha256}"
stream = "trades"
chain_id = "hyperliquid-mainnet"
source_id = "test-source"
source_version = "node-v1"
source_offset = "source.json"
observed_at_micros = 1721982386000000
ingested_at_micros = 1721982386100000
canonicalized_at_micros = 1721982386200000
mapper_version = "node-v1-mapper-1"
catalog_version = "test-catalog-v1"
expected_disposition = "mapped"
{market_block}
"#
    );
    fs::write(root.path().join("inspect.toml"), manifest).unwrap();
    let output = root.path().join("output.json");
    (root, output)
}

#[test]
fn malformed_source_with_matching_hash_is_rejected_by_the_source_boundary() {
    let (root, output) = write_case(br#"{"not":"a trade""#, "");

    let error = canonicalize(root.path(), Path::new("inspect.toml"), &output).unwrap_err();

    assert!(matches!(error, InspectError::Source(_)));
    assert_eq!(error.reason_code(), "canonical_inspect.source_rejected");
    assert!(!output.exists());
}

#[test]
fn missing_market_mapping_is_not_guessed() {
    let (root, output) = write_case(&workspace_fixture(), "");

    let error = canonicalize(root.path(), Path::new("inspect.toml"), &output).unwrap_err();

    assert!(matches!(
        error,
        InspectError::Mapping(MappingError::UnmappedMarket { .. })
    ));
    assert_eq!(error.reason_code(), "canonical_inspect.mapping_rejected");
    assert!(!output.exists());
}

#[test]
fn standalone_trade_cannot_be_promoted_by_the_inspector() {
    let batch: serde_json::Value = serde_json::from_slice(&workspace_fixture()).unwrap();
    let standalone = serde_json::to_vec(&batch["events"][0]).unwrap();
    let (root, output) = write_case(
        &standalone,
        r#"[[market]]
symbol = "COMP"
market_id = "perp:COMP""#,
    );

    let error = canonicalize(root.path(), Path::new("inspect.toml"), &output).unwrap_err();

    assert!(matches!(error, InspectError::UnexpectedDisposition));
    assert_eq!(
        error.reason_code(),
        "canonical_inspect.unexpected_disposition"
    );
    assert!(!output.exists());
}

#[test]
fn parent_relative_manifest_paths_are_rejected_before_reading() {
    let root = tempfile::tempdir().unwrap();

    let error = canonicalize(
        root.path(),
        Path::new("../inspect.toml"),
        root.path().join("out"),
    )
    .unwrap_err();

    assert!(matches!(error, InspectError::UnsafePath));
    assert_eq!(error.reason_code(), "canonical_inspect.unsafe_input_path");
}
