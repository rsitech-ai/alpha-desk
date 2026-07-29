use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, ConfirmationClass};
use canonical_ledger::{
    CanonicalLedger, CheckpointArtifact, CheckpointCompatibility, LedgerLimits, StateImage,
    StateImageLimits, WatermarkOnlyReducerV1,
};
use domain_types::{BlockHeight, ChainId, ManifestId, ProtocolTime, SourceId};

const ARCHIVE_HASH: [u8; 32] = [0x44; 32];
const SCHEMA_FINGERPRINT: [u8; 32] = [0x55; 32];

#[test]
fn state_image_round_trip_is_exact_and_resumes_to_the_uninterrupted_hash() {
    let mut uninterrupted = ledger(300);
    uninterrupted
        .apply_block(&empty_block(300))
        .expect("height 300");
    uninterrupted
        .apply_block(&empty_block(301))
        .expect("height 301");

    let checkpoint_bytes = uninterrupted.state_image().canonical_bytes();
    let restored = StateImage::decode_canonical(&checkpoint_bytes, StateImageLimits::production())
        .expect("decode state image");
    assert_eq!(restored.canonical_bytes(), checkpoint_bytes);
    assert_eq!(restored.state_hash(), uninterrupted.state_hash());

    let mut resumed = CanonicalLedger::try_from_state_image(
        restored,
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("resume ledger");
    uninterrupted
        .apply_block(&empty_block(302))
        .expect("uninterrupted height 302");
    resumed
        .apply_block(&empty_block(302))
        .expect("resumed height 302");

    assert_eq!(resumed.state_hash(), uninterrupted.state_hash());
    assert_eq!(
        resumed.state_image().canonical_bytes(),
        uninterrupted.state_image().canonical_bytes()
    );
}

#[test]
fn state_image_decoder_rejects_truncation_trailing_bytes_schema_drift_and_limits() {
    let ledger = applied_ledger(310);
    let bytes = ledger.state_image().canonical_bytes();

    let truncated =
        StateImage::decode_canonical(&bytes[..bytes.len() - 1], StateImageLimits::production())
            .expect_err("truncated");
    assert_eq!(truncated.reason_code(), "state_image.truncated");

    let mut trailing = bytes.clone();
    trailing.push(0);
    let trailing_error = StateImage::decode_canonical(&trailing, StateImageLimits::production())
        .expect_err("trailing");
    assert_eq!(trailing_error.reason_code(), "state_image.trailing_bytes");

    let mut wrong_schema = bytes.clone();
    wrong_schema[8] ^= 1;
    let schema_error = StateImage::decode_canonical(&wrong_schema, StateImageLimits::production())
        .expect_err("schema drift");
    assert_eq!(schema_error.reason_code(), "state_image.invalid_schema");

    let limits =
        StateImageLimits::try_new(bytes.len() - 1, 1, 64, 64).expect("small deterministic limit");
    let limit_error =
        StateImage::decode_canonical(&bytes, limits).expect_err("state image byte limit");
    assert_eq!(limit_error.reason_code(), "state_image.limit_exceeded");
}

#[test]
fn checkpoint_manifest_round_trip_binds_state_archive_schema_and_reducer() {
    let ledger = applied_ledger(320);
    let artifact = artifact(&ledger);
    let manifest = artifact.encode_manifest().expect("manifest");
    let decoded = CheckpointArtifact::decode(
        &manifest,
        artifact.state_image_bytes(),
        StateImageLimits::production(),
    )
    .expect("decode artifact");

    assert_eq!(decoded, artifact);
    assert_eq!(
        decoded.encode_manifest().expect("manifest"),
        manifest,
        "canonical manifest bytes must be stable"
    );
    assert!(
        decoded
            .checkpoint_id()
            .as_str()
            .starts_with("state-checkpoint-v1-")
    );
    assert_eq!(
        decoded.checkpoint().state_hash(),
        decoded.state_image().state_hash()
    );
}

#[test]
fn checkpoint_decode_rejects_tampered_state_and_noncanonical_manifest_bytes() {
    let ledger = applied_ledger(330);
    let artifact = artifact(&ledger);
    let manifest = artifact.encode_manifest().expect("manifest");

    let mut tampered_state = artifact.state_image_bytes().to_vec();
    let hash_byte = tampered_state.len() - 9;
    tampered_state[hash_byte] ^= 1;
    let state_error =
        CheckpointArtifact::decode(&manifest, &tampered_state, StateImageLimits::production())
            .expect_err("state tamper");
    assert_eq!(state_error.reason_code(), "checkpoint.integrity");

    let mut noncanonical_manifest = manifest;
    noncanonical_manifest.push(b'\n');
    let manifest_error = CheckpointArtifact::decode(
        &noncanonical_manifest,
        artifact.state_image_bytes(),
        StateImageLimits::production(),
    )
    .expect_err("noncanonical manifest");
    assert_eq!(
        manifest_error.reason_code(),
        "checkpoint.noncanonical_manifest"
    );
}

#[test]
fn compatibility_checks_fail_closed_for_every_bound_identity() {
    let ledger = applied_ledger(340);
    let artifact = artifact(&ledger);
    let exact = CheckpointCompatibility::try_new(
        ChainId::new("mainnet").expect("chain"),
        WatermarkOnlyReducerV1::VERSION,
        ManifestId::new("archive-manifest-v1-test").expect("manifest"),
        ARCHIVE_HASH,
        SCHEMA_FINGERPRINT,
    )
    .expect("compatibility");
    artifact
        .validate_compatibility(&exact)
        .expect("exact compatibility");

    let cases = [
        CheckpointCompatibility::try_new(
            ChainId::new("testnet").expect("chain"),
            WatermarkOnlyReducerV1::VERSION,
            ManifestId::new("archive-manifest-v1-test").expect("manifest"),
            ARCHIVE_HASH,
            SCHEMA_FINGERPRINT,
        )
        .expect("case"),
        CheckpointCompatibility::try_new(
            ChainId::new("mainnet").expect("chain"),
            "other-reducer@1.0.0",
            ManifestId::new("archive-manifest-v1-test").expect("manifest"),
            ARCHIVE_HASH,
            SCHEMA_FINGERPRINT,
        )
        .expect("case"),
        CheckpointCompatibility::try_new(
            ChainId::new("mainnet").expect("chain"),
            WatermarkOnlyReducerV1::VERSION,
            ManifestId::new("archive-manifest-v1-other").expect("manifest"),
            ARCHIVE_HASH,
            SCHEMA_FINGERPRINT,
        )
        .expect("case"),
        CheckpointCompatibility::try_new(
            ChainId::new("mainnet").expect("chain"),
            WatermarkOnlyReducerV1::VERSION,
            ManifestId::new("archive-manifest-v1-test").expect("manifest"),
            [0x66; 32],
            SCHEMA_FINGERPRINT,
        )
        .expect("case"),
        CheckpointCompatibility::try_new(
            ChainId::new("mainnet").expect("chain"),
            WatermarkOnlyReducerV1::VERSION,
            ManifestId::new("archive-manifest-v1-test").expect("manifest"),
            ARCHIVE_HASH,
            [0x77; 32],
        )
        .expect("case"),
    ];

    for compatibility in cases {
        let error = artifact
            .validate_compatibility(&compatibility)
            .expect_err("identity mismatch");
        assert_eq!(error.reason_code(), "checkpoint.incompatible");
    }
}

fn artifact(ledger: &CanonicalLedger<WatermarkOnlyReducerV1>) -> CheckpointArtifact {
    CheckpointArtifact::try_new(
        ledger.checkpoint().expect("checkpoint"),
        ledger.state_image().clone(),
        ManifestId::new("archive-manifest-v1-test").expect("manifest"),
        ARCHIVE_HASH,
        SCHEMA_FINGERPRINT,
    )
    .expect("artifact")
}

fn ledger(first_height: u64) -> CanonicalLedger<WatermarkOnlyReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(first_height),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("ledger")
}

fn applied_ledger(height: u64) -> CanonicalLedger<WatermarkOnlyReducerV1> {
    let mut ledger = ledger(height);
    ledger
        .apply_block(&empty_block(height))
        .expect("empty block");
    ledger
}

fn empty_block(height: u64) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("test-primary").expect("source"),
            *blake3::hash(&height.to_be_bytes()).as_bytes(),
        )]),
    )
    .expect("block")
}
