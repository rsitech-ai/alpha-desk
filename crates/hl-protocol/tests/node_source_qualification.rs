use domain_types::SourceId;
use hl_protocol::{
    ErrorDisposition,
    node::qualification::{
        Blake3Digest, MAX_IDENTITY_BYTES, MAX_QUALIFICATION_MANIFEST_BYTES,
        NodeRecordingFileRoleV1, NodeSourceQualificationError, Sha256Digest,
        decode_node_source_qualification_manifest_v1, qualify_node_source_v1,
    },
};
use sha2::{Digest, Sha256};

const SCHEMA: &str = "hyperliquid-alpha-desk/node-source-qualification/v1";

fn sha256(bytes: &[u8]) -> Sha256Digest {
    Sha256Digest::from_bytes(Sha256::digest(bytes).into())
}

fn valid_manifest() -> Vec<u8> {
    format!(
        concat!(
            "{{\"schema\":\"{}\",",
            "\"recording_id\":\"recording-2026-08-03-a\",",
            "\"chain_id\":\"hyperliquid-mainnet\",",
            "\"node_instance_id\":\"operator-node-a\",",
            "\"source_group\":{{\"committed_source_id\":\"node-a-replica-cmds\",\"trade_source_id\":\"node-a-block-trades\"}},",
            "\"artifact\":{{\"name\":\"hyperliquid-node\",\"version\":\"v1.2.3\",\"repository_commit\":\"0123456789abcdef0123456789abcdef01234567\",\"build_argv\":[\"cargo\",\"build\",\"--release\",\"--locked\"],\"binary_sha256\":\"{}\",\"build_material_sha256\":\"{}\",\"signature_fingerprint\":\"0123456789ABCDEF0123456789ABCDEF01234567\",\"signature_material_sha256\":\"{}\"}},",
            "\"capture\":{{\"argv\":[\"/opt/hyperliquid-node\",\"--write-trades\",\"--batch-by-block\",\"--replica-cmds-style\",\"actions-and-responses\",\"--disable-output-file-buffering\"],\"output_file_buffering\":\"disabled\",\"production_recording\":true,\"same_node_instance\":true,\"byte_exact\":true,\"corpus_coverage_complete\":true,\"runtime_material_sha256\":\"{}\"}},",
            "\"profile\":{{\"qualification_profile\":\"hl-node-source-v1\",\"committed_parser_version\":\"node-committed-parser-1\",\"committed_parser_material_sha256\":\"{}\",\"trade_parser_version\":\"node-trade-parser-1\",\"trade_parser_material_sha256\":\"{}\",\"mapper_version\":\"node-v1-mapper-1\",\"mapper_material_sha256\":\"{}\",\"catalog_version\":\"mainnet-catalog-2026-08-03\",\"catalog_sha256\":\"{}\",\"time_normalization_rule\":\"node-block-time-naive-utc-v1\",\"time_normalization_material_sha256\":\"{}\"}},",
            "\"redistribution\":\"private-operator-evidence\",",
            "\"files\":[",
            "{{\"relative_path\":\"replica_cmds/000000.ndjson\",\"role\":\"committed\",\"rotation_sequence\":0,\"size_bytes\":123,\"sha256\":\"{}\",\"first_cursor\":{{\"epoch\":\"replica-epoch-a\",\"position\":{{\"kind\":\"block-height\",\"height\":992814678}},\"content_blake3\":\"{}\"}},\"last_cursor\":{{\"epoch\":\"replica-epoch-a\",\"position\":{{\"kind\":\"block-height\",\"height\":992814679}},\"content_blake3\":\"{}\"}}}},",
            "{{\"relative_path\":\"block_trades/000000.ndjson\",\"role\":\"trade\",\"rotation_sequence\":0,\"size_bytes\":456,\"sha256\":\"{}\",\"first_cursor\":{{\"epoch\":\"trade-epoch-a\",\"position\":{{\"kind\":\"byte-offset\",\"end_offset\":123}},\"content_blake3\":\"{}\"}},\"last_cursor\":{{\"epoch\":\"trade-epoch-a\",\"position\":{{\"kind\":\"byte-offset\",\"end_offset\":456}},\"content_blake3\":\"{}\"}}}}]}}"
        ),
        SCHEMA,
        "11".repeat(32),
        "22".repeat(32),
        "33".repeat(32),
        "44".repeat(32),
        "55".repeat(32),
        "66".repeat(32),
        "0d".repeat(32),
        "0e".repeat(32),
        "0f".repeat(32),
        "77".repeat(32),
        "88".repeat(32),
        "99".repeat(32),
        "aa".repeat(32),
        "bb".repeat(32),
        "cc".repeat(32),
    )
    .into_bytes()
}

fn decode(
    bytes: &[u8],
) -> Result<
    hl_protocol::node::qualification::NodeSourceQualificationManifestV1,
    NodeSourceQualificationError,
> {
    decode_node_source_qualification_manifest_v1(bytes, sha256(bytes))
}

#[test]
fn canonical_manifest_roundtrips_and_computes_its_sha256_internally() {
    let bytes = valid_manifest();
    let expected = sha256(&bytes);

    let manifest =
        decode_node_source_qualification_manifest_v1(&bytes, expected).expect("canonical manifest");

    assert_eq!(manifest.canonical_bytes(), bytes);
    assert_eq!(manifest.manifest_sha256(), expected);
    assert_eq!(manifest.node_instance_id().as_str(), "operator-node-a");
    assert_eq!(manifest.artifact().name().as_str(), "hyperliquid-node");
    assert_eq!(manifest.artifact().build_argv()[0], "cargo");
    assert!(manifest.capture().production_recording());
    assert!(manifest.capture().same_node_instance());
    assert!(manifest.capture().byte_exact());
    assert!(manifest.capture().corpus_coverage_complete());
    assert_eq!(
        manifest.profile().time_normalization_rule().as_str(),
        "node-block-time-naive-utc-v1"
    );
    assert_eq!(
        manifest.profile().time_normalization_material_sha256(),
        Sha256Digest::parse_lower_hex(&"0f".repeat(32)).unwrap()
    );
    assert_eq!(manifest.files().len(), 2);
}

#[test]
fn rejects_unknown_fields_and_noncanonical_json() {
    let bytes = valid_manifest();
    let unknown = String::from_utf8(bytes.clone())
        .unwrap()
        .replacen(
            &format!("{{\"schema\":\"{SCHEMA}\","),
            &format!("{{\"schema\":\"{SCHEMA}\",\"caller_qualified\":true,"),
            1,
        )
        .into_bytes();
    let error = decode(&unknown).expect_err("unknown caller claim must be rejected");
    assert_eq!(
        error.reason_code(),
        "source_join.invalid_qualification_manifest"
    );

    let mut noncanonical = bytes;
    noncanonical.push(b'\n');
    let error = decode(&noncanonical).expect_err("trailing whitespace is not canonical");
    assert_eq!(
        error.reason_code(),
        "source_join.noncanonical_qualification_manifest"
    );
}

#[test]
fn expected_sha256_mismatch_precedes_json_parsing_and_registry_lookup() {
    let malformed = b"not-json";
    let wrong_expected = Sha256Digest::from_bytes([0x5a; 32]);

    let error = qualify_node_source_v1(malformed, wrong_expected)
        .expect_err("digest mismatch must fail before decoding");

    assert_eq!(
        error.reason_code(),
        "source_join.qualification_manifest_digest_mismatch"
    );
    assert_eq!(error.disposition(), ErrorDisposition::Quarantine);
}

#[test]
fn self_declared_or_unlisted_profile_cannot_produce_a_qualified_token() {
    let bytes = valid_manifest();
    let error = qualify_node_source_v1(&bytes, sha256(&bytes))
        .expect_err("the built-in production registry is intentionally empty");

    assert_eq!(
        error.reason_code(),
        "source_join.unqualified_source_profile"
    );
    assert_eq!(error.disposition(), ErrorDisposition::Stop);
}

#[test]
fn wrong_flags_build_or_catalog_remain_unqualified() {
    let valid = String::from_utf8(valid_manifest()).unwrap();
    let variants = [
        valid.replacen(
            "\"corpus_coverage_complete\":true",
            "\"corpus_coverage_complete\":false",
            1,
        ),
        valid.replacen(&"22".repeat(32), &"2f".repeat(32), 1),
        valid.replacen(&"0e".repeat(32), &"0f".repeat(32), 1),
    ];

    for bytes in variants.map(String::into_bytes) {
        let error = qualify_node_source_v1(&bytes, sha256(&bytes))
            .expect_err("caller-controlled material cannot qualify a source");
        assert_eq!(
            error.reason_code(),
            "source_join.unqualified_source_profile"
        );
    }
}

#[test]
fn required_runtime_flags_are_exact_nonduplicated_and_value_bound() {
    let valid = String::from_utf8(valid_manifest()).unwrap();
    let variants = [
        valid.replacen("\"--write-trades\",", "", 1),
        valid.replacen("\"--batch-by-block\",", "", 1),
        valid.replacen(
            "\"--write-trades\",",
            "\"--write-trades\",\"--write-trades\",",
            1,
        ),
        valid.replacen(
            "\"--write-trades\",",
            "\"--write-trades\",\"--write-fills\",",
            1,
        ),
        valid.replacen(
            "\"--replica-cmds-style\",\"actions-and-responses\"",
            "\"--replica-cmds-style\",\"actions\"",
            1,
        ),
        valid.replacen(
            "\"--replica-cmds-style\",\"actions-and-responses\"",
            "\"--replica-cmds-style\",\"actions-and-responses\",\"--replica-cmds-style\",\"actions-and-responses\"",
            1,
        ),
    ];

    for bytes in variants.map(String::into_bytes) {
        let error = decode(&bytes).expect_err("required runtime flag contract must fail closed");
        assert_eq!(
            error.reason_code(),
            "source_join.invalid_qualification_manifest"
        );
    }
}

#[test]
fn time_normalization_material_is_independently_digest_bound() {
    let original = valid_manifest();
    let changed = String::from_utf8(original.clone())
        .unwrap()
        .replacen(&"0f".repeat(32), &"f0".repeat(32), 1)
        .into_bytes();

    let original_manifest = decode(&original).expect("original manifest");
    let changed_manifest = decode(&changed).expect("changed manifest");

    assert_ne!(
        original_manifest.manifest_sha256(),
        changed_manifest.manifest_sha256()
    );
}

#[test]
fn runtime_argv_and_buffering_mode_must_agree() {
    let valid = String::from_utf8(valid_manifest()).unwrap();
    let disabled_without_flag = valid
        .replacen(
            "\"--disable-output-file-buffering\"",
            "\"--not-buffering-flag\"",
            1,
        )
        .into_bytes();
    assert!(decode(&disabled_without_flag).is_err());

    let enabled_with_flag = valid
        .replacen(
            "\"output_file_buffering\":\"disabled\"",
            "\"output_file_buffering\":\"enabled\"",
            1,
        )
        .into_bytes();
    assert!(decode(&enabled_with_flag).is_err());
}

#[test]
fn committed_and_trade_sources_must_be_distinct_bounded_source_ids() {
    let valid = String::from_utf8(valid_manifest()).unwrap();
    let same = valid
        .replacen("node-a-block-trades", "node-a-replica-cmds", 1)
        .into_bytes();
    let error = decode(&same).expect_err("source roles require distinct identities");
    assert_eq!(
        error.reason_code(),
        "source_join.invalid_qualification_manifest"
    );

    let oversized_id = "x".repeat(MAX_IDENTITY_BYTES + 1);
    let oversized = valid
        .replacen("operator-node-a", &oversized_id, 1)
        .into_bytes();
    assert!(decode(&oversized).is_err());

    let control = valid
        .replacen("operator-node-a", "operator-node\\u0000a", 1)
        .into_bytes();
    assert!(decode(&control).is_err());

    assert!(SourceId::new("node-a-replica-cmds").is_ok());
}

#[test]
fn native_cursor_evidence_preserves_epoch_offset_and_blake3_without_spool_sequence() {
    let bytes = valid_manifest();
    let manifest = decode(&bytes).expect("valid manifest");
    let committed = &manifest.files()[0];
    let trade = &manifest.files()[1];

    assert_eq!(committed.role(), NodeRecordingFileRoleV1::Committed);
    assert_eq!(committed.first_cursor().epoch().as_str(), "replica-epoch-a");
    assert_eq!(
        committed.last_cursor().position(),
        hl_protocol::node::qualification::NodeNativePositionV1::BlockHeight { height: 992814679 }
    );
    assert_eq!(trade.role(), NodeRecordingFileRoleV1::Trade);
    assert_eq!(
        trade.last_cursor().position(),
        hl_protocol::node::qualification::NodeNativePositionV1::ByteOffset { end_offset: 456 }
    );
    assert_eq!(
        trade.last_cursor().content_blake3(),
        Blake3Digest::parse_lower_hex(&"cc".repeat(32)).unwrap()
    );

    let with_local_sequence = String::from_utf8(bytes)
        .unwrap()
        .replacen(
            "\"position\":",
            "\"local_spool_sequence\":0,\"position\":",
            1,
        )
        .into_bytes();
    assert!(decode(&with_local_sequence).is_err());
}

#[test]
fn cursor_position_kind_must_match_the_source_role() {
    let valid = String::from_utf8(valid_manifest()).unwrap();
    let committed_as_bytes = valid
        .replacen(
            "\"kind\":\"block-height\",\"height\":992814678",
            "\"kind\":\"byte-offset\",\"end_offset\":1",
            1,
        )
        .into_bytes();
    assert!(decode(&committed_as_bytes).is_err());

    let zero_trade_end = valid
        .replacen("\"end_offset\":123", "\"end_offset\":0", 1)
        .into_bytes();
    assert!(decode(&zero_trade_end).is_err());
}

#[test]
fn digest_algorithms_are_typed_and_wire_algorithm_confusion_is_rejected() {
    let lowercase = "ab".repeat(32);
    let sha = Sha256Digest::parse_lower_hex(&lowercase).expect("sha-256");
    let blake = Blake3Digest::parse_lower_hex(&lowercase).expect("blake3");
    assert_eq!(sha.to_string(), lowercase);
    assert_eq!(blake.to_string(), lowercase);
    assert!(Sha256Digest::parse_lower_hex(&"AB".repeat(32)).is_err());
    assert!(Blake3Digest::parse_lower_hex("abcd").is_err());

    let confused = String::from_utf8(valid_manifest())
        .unwrap()
        .replacen("\"content_blake3\"", "\"content_sha256\"", 1)
        .into_bytes();
    let error = decode(&confused).expect_err("wrong algorithm field name is not accepted");
    assert_eq!(
        error.reason_code(),
        "source_join.invalid_qualification_manifest"
    );
}

#[test]
fn unsafe_paths_and_duplicate_rotation_descriptors_are_rejected() {
    let valid = String::from_utf8(valid_manifest()).unwrap();
    for unsafe_path in [
        "/tmp/000000.ndjson",
        "../000000.ndjson",
        "replica_cmds\\\\000000.ndjson",
    ] {
        let bytes = valid
            .replacen("replica_cmds/000000.ndjson", unsafe_path, 1)
            .into_bytes();
        assert!(
            decode(&bytes).is_err(),
            "accepted unsafe path {unsafe_path}"
        );
    }

    let duplicate_rotation = valid
        .replacen(
            "\"role\":\"trade\",\"rotation_sequence\":0",
            "\"role\":\"committed\",\"rotation_sequence\":0",
            1,
        )
        .into_bytes();
    assert!(decode(&duplicate_rotation).is_err());
}

#[test]
fn empty_and_oversized_inputs_fail_with_stable_reasons_and_dispositions() {
    let empty = b"";
    let error = decode_node_source_qualification_manifest_v1(empty, sha256(empty))
        .expect_err("empty input");
    assert_eq!(
        error.reason_code(),
        "source_join.empty_qualification_manifest"
    );
    assert_eq!(error.disposition(), ErrorDisposition::Quarantine);

    let oversized = vec![b' '; MAX_QUALIFICATION_MANIFEST_BYTES + 1];
    let error = decode_node_source_qualification_manifest_v1(&oversized, sha256(&oversized))
        .expect_err("oversized input");
    assert_eq!(
        error.reason_code(),
        "source_join.qualification_manifest_too_large"
    );
    assert_eq!(error.disposition(), ErrorDisposition::Stop);
}
