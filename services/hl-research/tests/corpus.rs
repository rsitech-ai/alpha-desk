use std::path::{Path, PathBuf};

use hl_research::{
    CorpusClass, ResearchError, ResearchStatus, load_corpus_path, refuse_corpus_path,
    run_walk_forward_bytes,
};

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/research")
        .join(name)
}

#[test]
fn live_corpus_path_fails_closed_without_inventing_bytes() {
    let path = Path::new("/tmp/live/corpus.json");
    assert_eq!(CorpusClass::from_path(path), CorpusClass::Live);
    assert_eq!(
        refuse_corpus_path(path).unwrap_err(),
        ResearchError::LiveCorpusForbidden
    );
    assert_eq!(
        load_corpus_path(path).unwrap_err(),
        ResearchError::LiveCorpusForbidden
    );
    assert_eq!(
        ResearchError::LiveCorpusForbidden.reason_code(),
        "hl_research.live_corpus"
    );
}

#[test]
fn replica_command_directory_path_fails_closed_as_live_corpus() {
    let path = Path::new("/var/lib/hyperliquid/hl/data/replica_cmds");
    assert_eq!(CorpusClass::from_path(path), CorpusClass::Live);
    assert_eq!(
        load_corpus_path(path).unwrap_err(),
        ResearchError::LiveCorpusForbidden
    );
}

#[test]
fn locked_holdout_corpus_path_fails_closed_with_a_stable_reason() {
    let path = Path::new("/tmp/locked-holdout/corpus.json");
    assert_eq!(CorpusClass::from_path(path), CorpusClass::LockedHoldout);
    assert_eq!(
        refuse_corpus_path(path).unwrap_err(),
        ResearchError::LockedCorpusForbidden
    );
    assert_eq!(
        load_corpus_path(path).unwrap_err(),
        ResearchError::LockedCorpusForbidden
    );
    assert_eq!(
        ResearchError::LockedCorpusForbidden.reason_code(),
        "hl_research.locked_corpus"
    );

    let lock_path = Path::new("/tmp/external-holdout.lock");
    assert_eq!(
        CorpusClass::from_path(lock_path),
        CorpusClass::LockedHoldout
    );
    assert_eq!(
        load_corpus_path(lock_path).unwrap_err(),
        ResearchError::LockedCorpusForbidden
    );
}

#[test]
fn invented_locked_file_still_cannot_load_as_a_corpus() {
    let fake = std::env::temp_dir().join("locked-holdout.json");
    std::fs::write(
        &fake,
        br#"{"locked":true,"holdout_passed":true,"live_corpus":true}"#,
    )
    .unwrap();
    let classified = CorpusClass::from_path(&fake);
    let error = load_corpus_path(&fake).unwrap_err();
    let _ = std::fs::remove_file(&fake);
    assert_eq!(classified, CorpusClass::LockedHoldout);
    assert_eq!(error, ResearchError::LockedCorpusForbidden);
}

#[test]
fn synthetic_walk_forward_fixture_still_loads() {
    let path = fixture("walk-forward-v1.json");
    assert_eq!(CorpusClass::from_path(&path), CorpusClass::Synthetic);
    let bytes = load_corpus_path(&path).unwrap();
    let report = run_walk_forward_bytes(&bytes).unwrap();
    assert_eq!(report.mode, "synthetic_walk_forward");
    assert_eq!(report.walk_forward, "synthetic_folds");
    assert!(!report.live_corpus);
    assert!(!report.replica_cmds_used);
    let encoded = serde_json::to_value(&report).unwrap();
    assert_eq!(encoded["live_corpus"], false);
    assert_eq!(encoded["replica_cmds_used"], false);
}

#[test]
fn missing_synthetic_path_is_invalid_not_invented() {
    let path = Path::new("/tmp/alpha-desk-missing-synthetic-corpus.json");
    assert_eq!(CorpusClass::from_path(path), CorpusClass::Synthetic);
    assert_eq!(
        load_corpus_path(path).unwrap_err(),
        ResearchError::InvalidFixture
    );
}

#[test]
fn reports_fail_closed_if_live_corpus_would_be_true() {
    let mut status = ResearchStatus::current();
    assert!(!status.live_corpus);
    assert!(!status.replica_cmds_used);
    status.live_corpus = true;
    assert_eq!(
        status.refuse_corpus_claims().unwrap_err(),
        ResearchError::LiveCorpusForbidden
    );
    assert_eq!(
        status.encode_json().unwrap_err(),
        ResearchError::LiveCorpusForbidden
    );
    assert!(serde_json::to_value(&status).is_err());
}

#[test]
fn reports_fail_closed_if_replica_cmds_used_would_be_true() {
    let mut status = ResearchStatus::current();
    status.replica_cmds_used = true;
    assert_eq!(
        status.refuse_corpus_claims().unwrap_err(),
        ResearchError::ReplicaCmdsUsedForbidden
    );
    assert_eq!(
        status.encode_json().unwrap_err(),
        ResearchError::ReplicaCmdsUsedForbidden
    );
    assert_eq!(
        ResearchError::ReplicaCmdsUsedForbidden.reason_code(),
        "hl_research.replica_cmds_used"
    );
    assert!(serde_json::to_value(&status).is_err());
}
