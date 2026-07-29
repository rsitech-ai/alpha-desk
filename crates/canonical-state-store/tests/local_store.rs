use std::{
    collections::BTreeMap,
    fs,
    io::{Seek, SeekFrom, Write},
    os::unix::fs::{PermissionsExt, symlink},
};

use canonical_events::{BlockEnvelope, ConfirmationClass};
use canonical_ledger::{
    CanonicalLedger, CheckpointArtifact, CheckpointCompatibility, LedgerLimits, StateImageLimits,
    WatermarkOnlyReducerV1,
};
use canonical_state_store::LocalCheckpointStore;
use domain_types::{BlockHeight, ChainId, CheckpointId, ManifestId, ProtocolTime, SourceId};
use storage_ports::{CheckpointPublishDisposition, StateCheckpointStore};

const ARCHIVE_HASH: [u8; 32] = [0x44; 32];
const SCHEMA_FINGERPRINT: [u8; 32] = [0x55; 32];

#[test]
fn publish_load_and_identical_republish_are_private_and_exact() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let root = temporary.path().join("checkpoints");
    let store = LocalCheckpointStore::open(&root, StateImageLimits::production()).expect("store");
    let artifact = artifact(400);

    let first = store.publish(&artifact).expect("publish");
    assert!(matches!(first, CheckpointPublishDisposition::Published(_)));
    let second = store.publish(&artifact).expect("idempotent publish");
    assert!(matches!(second, CheckpointPublishDisposition::Identical(_)));
    assert_eq!(first.receipt(), second.receipt());

    let loaded = store
        .load(
            artifact.checkpoint_id(),
            &compatibility(),
            StateImageLimits::production(),
        )
        .expect("load");
    assert_eq!(loaded, artifact);

    assert_eq!(mode(&root), 0o700);
    let generation = root.join(artifact.checkpoint_id().as_str());
    assert_eq!(mode(&generation), 0o700);
    assert_eq!(mode(&generation.join("state.bin")), 0o600);
    assert_eq!(mode(&generation.join("manifest.json")), 0o600);
}

#[test]
fn retained_directory_descriptor_prevents_parent_path_retargeting() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let root = temporary.path().join("checkpoints");
    let retained = temporary.path().join("retained");
    let attacker = temporary.path().join("attacker");
    let store = LocalCheckpointStore::open(&root, StateImageLimits::production()).expect("store");
    fs::create_dir(&attacker).expect("attacker directory");
    fs::set_permissions(&attacker, fs::Permissions::from_mode(0o700))
        .expect("attacker permissions");
    fs::rename(&root, &retained).expect("retain opened directory");
    symlink(&attacker, &root).expect("retarget public path");
    let artifact = artifact(410);

    store
        .publish(&artifact)
        .expect("descriptor-relative publish");
    let loaded = store
        .load(
            artifact.checkpoint_id(),
            &compatibility(),
            StateImageLimits::production(),
        )
        .expect("descriptor-relative load");

    assert_eq!(loaded, artifact);
    assert!(retained.join(artifact.checkpoint_id().as_str()).is_dir());
    assert!(
        fs::read_dir(&attacker)
            .expect("attacker directory")
            .next()
            .is_none()
    );
}

#[test]
fn symlinked_root_or_generation_is_rejected_without_following_it() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let target = temporary.path().join("target");
    fs::create_dir(&target).expect("target");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o700)).expect("permissions");
    let root_link = temporary.path().join("root-link");
    symlink(&target, &root_link).expect("root symlink");
    let root_error = LocalCheckpointStore::open(&root_link, StateImageLimits::production())
        .expect_err("symlinked root");
    assert_eq!(root_error.reason_code(), "checkpoint_store.unsafe_path");

    let root = temporary.path().join("checkpoints");
    let store = LocalCheckpointStore::open(&root, StateImageLimits::production()).expect("store");
    let artifact = artifact(420);
    let generation = root.join(artifact.checkpoint_id().as_str());
    symlink(&target, &generation).expect("generation symlink");

    let error = store
        .load(
            artifact.checkpoint_id(),
            &compatibility(),
            StateImageLimits::production(),
        )
        .expect_err("symlinked generation");
    assert_eq!(error.reason_code(), "checkpoint_store.unsafe_object");
    assert!(fs::read_dir(&target).expect("target").next().is_none());
}

#[test]
fn corrupted_or_incomplete_generation_never_restores() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let root = temporary.path().join("checkpoints");
    let store = LocalCheckpointStore::open(&root, StateImageLimits::production()).expect("store");
    let published_artifact = artifact(430);
    store.publish(&published_artifact).expect("publish");
    let generation = root.join(published_artifact.checkpoint_id().as_str());
    let state_path = generation.join("state.bin");
    let mut state = fs::OpenOptions::new()
        .write(true)
        .open(&state_path)
        .expect("state");
    state.seek(SeekFrom::Start(16)).expect("seek");
    state.write_all(&[0xff]).expect("tamper");
    state.sync_all().expect("sync tamper");

    let corruption = store
        .load(
            published_artifact.checkpoint_id(),
            &compatibility(),
            StateImageLimits::production(),
        )
        .expect_err("corruption");
    assert_eq!(corruption.reason_code(), "checkpoint_store.contract");

    let incomplete_artifact = artifact(431);
    let incomplete = root.join(incomplete_artifact.checkpoint_id().as_str());
    fs::create_dir(&incomplete).expect("incomplete generation");
    fs::set_permissions(&incomplete, fs::Permissions::from_mode(0o700)).expect("permissions");
    fs::write(
        incomplete.join("state.bin"),
        incomplete_artifact.state_image_bytes(),
    )
    .expect("partial state");
    fs::set_permissions(
        incomplete.join("state.bin"),
        fs::Permissions::from_mode(0o600),
    )
    .expect("state permissions");

    let missing_manifest = store
        .load(
            incomplete_artifact.checkpoint_id(),
            &compatibility(),
            StateImageLimits::production(),
        )
        .expect_err("manifest-last contract");
    assert_eq!(missing_manifest.reason_code(), "checkpoint_store.not_found");
}

#[test]
fn untrusted_checkpoint_ids_permissions_and_size_limits_fail_before_publication() {
    let temporary = tempfile::tempdir().expect("temporary root");
    let root = temporary.path().join("checkpoints");
    let store = LocalCheckpointStore::open(&root, StateImageLimits::production()).expect("store");
    let malicious = CheckpointId::new("../outside").expect("generic ID boundary");
    let path_error = store
        .load(&malicious, &compatibility(), StateImageLimits::production())
        .expect_err("path-bearing checkpoint ID");
    assert_eq!(path_error.reason_code(), "checkpoint_store.unsafe_path");
    assert!(!temporary.path().join("outside").exists());

    let permission_artifact = artifact(440);
    let generation = root.join(permission_artifact.checkpoint_id().as_str());
    fs::create_dir(&generation).expect("generation");
    fs::set_permissions(&generation, fs::Permissions::from_mode(0o755))
        .expect("unsafe permissions");
    let permission_error = store
        .load(
            permission_artifact.checkpoint_id(),
            &compatibility(),
            StateImageLimits::production(),
        )
        .expect_err("group-readable generation");
    assert_eq!(
        permission_error.reason_code(),
        "checkpoint_store.unsafe_object"
    );

    let small_root = temporary.path().join("small");
    let small_limits = StateImageLimits::try_new(32, 1, 16, 16).expect("small limits");
    let small_store = LocalCheckpointStore::open(&small_root, small_limits).expect("small store");
    let size_error = small_store
        .publish(&artifact(441))
        .expect_err("state exceeds configured store bound");
    assert_eq!(size_error.reason_code(), "checkpoint_store.too_large");
    assert!(
        fs::read_dir(&small_root)
            .expect("small root")
            .next()
            .is_none(),
        "oversized state must fail before creating a staging generation"
    );
}

fn artifact(height: u64) -> CheckpointArtifact {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("ledger");
    ledger
        .apply_block(&empty_block(height))
        .expect("empty block");
    CheckpointArtifact::try_new(
        ledger.checkpoint().expect("checkpoint"),
        ledger.state_image().clone(),
        ManifestId::new("archive-manifest-v1-test").expect("manifest"),
        ARCHIVE_HASH,
        SCHEMA_FINGERPRINT,
    )
    .expect("artifact")
}

fn compatibility() -> CheckpointCompatibility {
    CheckpointCompatibility::try_new(
        ChainId::new("mainnet").expect("chain"),
        WatermarkOnlyReducerV1::VERSION,
        ManifestId::new("archive-manifest-v1-test").expect("manifest"),
        ARCHIVE_HASH,
        SCHEMA_FINGERPRINT,
    )
    .expect("compatibility")
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

fn mode(path: &std::path::Path) -> u32 {
    fs::symlink_metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}
