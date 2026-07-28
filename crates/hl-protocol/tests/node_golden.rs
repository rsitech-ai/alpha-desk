use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use hl_protocol::node::v1::{NodeRecordKind, NodeStreamKind, parse_node_record};
use hl_protocol::{ObservationClass, SourceError};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    corpus_kind: String,
    production_recording: bool,
    fixture: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    file: String,
    stream: NodeStreamKind,
    kind: Option<NodeRecordKind>,
    sha256: String,
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/source/node-v1")
}

#[test]
fn official_example_corpus_is_hashed_and_truthfully_labeled() {
    let root = fixture_root();
    let manifest: Manifest =
        toml::from_str(&fs::read_to_string(root.join("manifest.toml")).expect("manifest"))
            .expect("valid manifest");

    assert_eq!(manifest.schema_version, 1);
    assert_eq!(
        manifest.corpus_kind,
        "normalized-official-documentation-examples"
    );
    assert!(!manifest.production_recording);

    for fixture in manifest.fixture {
        let payload = fs::read(root.join(&fixture.file)).expect("fixture payload");
        assert_eq!(hex::encode(Sha256::digest(&payload)), fixture.sha256);
        if let Some(expected_kind) = fixture.kind {
            let parsed = parse_node_record(fixture.stream, Bytes::from(payload.clone()))
                .expect("known record");
            assert_eq!(parsed.kind(), expected_kind);
            assert_eq!(parsed.payload().as_ref(), payload);
            assert_eq!(parsed.content_hash(), blake3::hash(&payload));
        }
    }
}

#[test]
fn node_v1_maps_each_stream_to_the_required_observation_class() {
    for (stream, expected) in [
        (
            NodeStreamKind::TransactionBlocks,
            ObservationClass::CommittedBlock,
        ),
        (NodeStreamKind::Fills, ObservationClass::AuxiliaryLedger),
        (
            NodeStreamKind::OrderStatuses,
            ObservationClass::AuxiliaryOrderStatus,
        ),
        (
            NodeStreamKind::RawBookDiffs,
            ObservationClass::AuxiliaryBookDiff,
        ),
        (
            NodeStreamKind::MiscEvents,
            ObservationClass::AuxiliaryLedger,
        ),
        (NodeStreamKind::MarketMetadata, ObservationClass::Snapshot),
    ] {
        assert_eq!(stream.observation_class(), expected);
    }
}

#[test]
fn unknown_misc_variant_is_schema_drift_and_never_silently_skipped() {
    let payload = fs::read(fixture_root().join("unknown-variant.json")).expect("unknown fixture");
    let error = parse_node_record(NodeStreamKind::MiscEvents, Bytes::from(payload))
        .expect_err("unknown variant must fail closed");

    assert!(matches!(error, SourceError::SchemaDrift(_)));
    assert_eq!(error.reason_code(), "source.schema_drift");
}

#[test]
fn malformed_complete_json_is_quarantined() {
    let error = parse_node_record(
        NodeStreamKind::OrderStatuses,
        Bytes::from_static(br#"{"status":"open""#),
    )
    .expect_err("malformed complete record");

    assert!(matches!(error, SourceError::MalformedPayload(_)));
    assert_eq!(error.reason_code(), "source.malformed_payload");
}

#[test]
fn empty_block_batch_retains_its_height_and_is_not_treated_as_corruption() {
    let payload = Bytes::from_static(
        br#"{"local_time":"2026-07-28T12:00:00","block_time":"2026-07-28T12:00:00","block_number":42,"events":[]}"#,
    );
    let parsed = parse_node_record(NodeStreamKind::Fills, payload.clone()).expect("empty batch");

    assert_eq!(parsed.kind(), NodeRecordKind::EmptyBatch);
    assert_eq!(parsed.block_number(), Some(42));
    assert_eq!(parsed.payload(), &payload);
}
