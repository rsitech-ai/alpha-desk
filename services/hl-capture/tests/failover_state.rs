use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Barrier};

use domain_types::{BlockHeight, ChainId, SourceId};
use hl_capture::{
    FailoverDecision, FailoverError, FailoverReason, FailoverRecordDisposition, FailoverStore,
};
use tempfile::tempdir;

fn canonical_root(root: &tempfile::TempDir) -> std::path::PathBuf {
    root.path().canonicalize().expect("canonical temp root")
}

fn decision(height: u64) -> FailoverDecision {
    FailoverDecision::try_new(
        ChainId::new("mainnet").unwrap(),
        SourceId::new("primary-node").unwrap(),
        SourceId::new("independent-node").unwrap(),
        BlockHeight::new(height),
        FailoverReason::PrimaryRangeUnavailable,
    )
    .unwrap()
}

#[test]
fn decision_is_create_once_idempotent_and_round_trips_with_integrity() {
    let root = tempdir().unwrap();
    let path = canonical_root(&root).join("committed-source-failover.json");
    let store = FailoverStore::new(path.clone()).unwrap();
    let expected = decision(42);

    assert_eq!(
        store.record(&expected).unwrap(),
        FailoverRecordDisposition::Recorded
    );
    assert_eq!(
        store.record(&expected).unwrap(),
        FailoverRecordDisposition::Identical
    );
    assert_eq!(store.load().unwrap(), Some(expected.clone()));
    assert_eq!(
        store.record(&decision(43)).unwrap_err(),
        FailoverError::ConflictingDecision
    );

    let metadata = fs::metadata(&path).unwrap();
    assert_eq!(metadata.permissions().mode() & 0o077, 0);
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert_eq!(value["schema_version"], "hl.capture.failover.v1");
    assert_eq!(value["failover_height"], 42);
    assert_eq!(value["reason"], "primary-range-unavailable");
    assert_eq!(
        value["decision_hash_blake3"]
            .as_str()
            .expect("decision hash")
            .len(),
        64
    );
}

#[test]
fn corruption_and_unsafe_existing_paths_fail_closed() {
    let root = tempdir().unwrap();
    let canonical = canonical_root(&root);
    let path = canonical.join("committed-source-failover.json");
    let store = FailoverStore::new(path.clone()).unwrap();
    store.record(&decision(42)).unwrap();

    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    value["failover_height"] = serde_json::json!(43);
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert_eq!(store.load().unwrap_err(), FailoverError::Integrity);

    fs::remove_file(&path).unwrap();
    let target = canonical.join("target");
    fs::write(&target, b"do-not-touch").unwrap();
    std::os::unix::fs::symlink(&target, &path).unwrap();
    assert_eq!(store.load().unwrap_err(), FailoverError::UnsafePath);
    assert_eq!(fs::read(&target).unwrap(), b"do-not-touch");
}

#[test]
fn symlinked_parent_components_fail_before_state_is_created() {
    let root = tempdir().unwrap();
    let canonical = canonical_root(&root);
    let target = canonical.join("target");
    fs::create_dir(&target).unwrap();
    let linked_parent = canonical.join("linked-parent");
    std::os::unix::fs::symlink(&target, &linked_parent).unwrap();
    let path = linked_parent.join("nested/committed-source-failover.json");

    assert_eq!(
        FailoverStore::new(path).unwrap_err(),
        FailoverError::UnsafePath
    );
    assert!(
        !target.join("nested").exists(),
        "validation must not mutate a symlink target"
    );
}

#[test]
fn concurrent_identical_recording_has_one_durable_decision() {
    let root = tempdir().unwrap();
    let path = canonical_root(&root).join("committed-source-failover.json");
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for _ in 0..2 {
        let worker_path = path.clone();
        let worker_barrier = Arc::clone(&barrier);
        workers.push(std::thread::spawn(move || {
            let store = FailoverStore::new(worker_path).unwrap();
            worker_barrier.wait();
            store.record(&decision(42)).unwrap()
        }));
    }
    barrier.wait();
    let dispositions: Vec<_> = workers
        .into_iter()
        .map(|worker| worker.join().unwrap())
        .collect();

    assert!(dispositions.contains(&FailoverRecordDisposition::Recorded));
    assert!(dispositions.contains(&FailoverRecordDisposition::Identical));
    assert_eq!(
        FailoverStore::new(path).unwrap().load().unwrap(),
        Some(decision(42))
    );
}

#[test]
fn decision_rejects_aliasing_sources() {
    let source = SourceId::new("same-node").unwrap();
    let error = FailoverDecision::try_new(
        ChainId::new("mainnet").unwrap(),
        source.clone(),
        source,
        BlockHeight::new(42),
        FailoverReason::PrimaryRangeUnavailable,
    )
    .unwrap_err();

    assert_eq!(error, FailoverError::InvalidDecision);
}

#[test]
fn durable_decision_is_bound_to_the_configured_chain_and_source_roles() {
    let expected = decision(42);
    let chain = ChainId::new("mainnet").unwrap();
    let primary = SourceId::new("primary-node").unwrap();
    let independent = SourceId::new("independent-node").unwrap();

    expected
        .validate_topology(&chain, &primary, &independent)
        .unwrap();
    assert_eq!(
        expected
            .validate_topology(&ChainId::new("testnet").unwrap(), &primary, &independent)
            .unwrap_err(),
        FailoverError::TopologyMismatch
    );
    assert_eq!(
        expected
            .validate_topology(
                &chain,
                &SourceId::new("replacement-primary").unwrap(),
                &independent
            )
            .unwrap_err(),
        FailoverError::TopologyMismatch
    );
}
