use std::{
    collections::BTreeMap,
    fs,
    io::{Seek, SeekFrom, Write},
    os::unix::fs::PermissionsExt,
};

use canonical_events::{BlockEnvelope, ConfirmationClass};
use canonical_ledger::{
    ApplyOutcome, CanonicalLedger, LedgerLimits, StateImageLimits, WatermarkOnlyReducerV1,
};
use canonical_state_store::SyncedWriteBatchStore;
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};
use storage_ports::{AtomicStateCommit, AtomicStateStore, StateCommitDisposition, StateStoreError};

#[test]
fn empty_store_commits_restarts_and_rejects_conflicting_history() {
    let temporary = tempfile::tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private parent");
    let root = temporary.path().join("atomic-state");
    let store = SyncedWriteBatchStore::open(&root, StateImageLimits::production()).expect("store");
    assert!(
        store
            .load_latest(StateImageLimits::production())
            .expect("empty")
            .is_none()
    );

    let mut first_ledger = ledger(200);
    let first = apply(&mut first_ledger, 200, 1);
    let commit = AtomicStateCommit::try_new(first.delta(), first.image()).expect("commit");
    let first_hash = first.image().state_hash();
    match store.commit(&commit).expect("first commit") {
        StateCommitDisposition::Committed(receipt) => {
            assert_eq!(receipt.state_hash(), first_hash);
        }
        other => panic!("first commit must be new: {other:?}"),
    }
    match store.commit(&commit).expect("identical replay") {
        StateCommitDisposition::AlreadyCommitted(receipt) => {
            assert_eq!(receipt.state_hash(), first_hash);
        }
        other => panic!("identical height+hash must be already committed: {other:?}"),
    }

    let mut other_ledger = ledger(200);
    let conflicting = apply(&mut other_ledger, 200, 2);
    let conflict =
        AtomicStateCommit::try_new(conflicting.delta(), conflicting.image()).expect("conflict");
    assert!(matches!(
        store.commit(&conflict),
        Err(StateStoreError::Conflict)
    ));

    let second = apply(&mut first_ledger, 201, 1);
    let second_commit = AtomicStateCommit::try_new(second.delta(), second.image()).expect("second");
    store.commit(&second_commit).expect("height 201");
    let latest = store
        .load_latest(StateImageLimits::production())
        .expect("load")
        .expect("present");
    assert_eq!(latest.state_hash(), second.image().state_hash());
    assert_eq!(latest.block_height(), Some(BlockHeight::new(201)));

    drop(store);
    let restarted =
        SyncedWriteBatchStore::open(&root, StateImageLimits::production()).expect("restart");
    let restored = restarted
        .load_latest(StateImageLimits::production())
        .expect("restart load")
        .expect("restored");
    assert_eq!(restored.canonical_bytes(), second.image().canonical_bytes());
}

#[test]
fn second_open_is_locked_and_tampered_state_is_corrupt() {
    let temporary = tempfile::tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private parent");
    let root = temporary.path().join("atomic-lock");
    let store = SyncedWriteBatchStore::open(&root, StateImageLimits::production()).expect("store");
    let locked = SyncedWriteBatchStore::open(&root, StateImageLimits::production())
        .expect_err("exclusive lock");
    assert!(matches!(locked, StateStoreError::Locked));

    let mut first_ledger = ledger(300);
    let applied = apply(&mut first_ledger, 300, 1);
    let commit = AtomicStateCommit::try_new(applied.delta(), applied.image()).expect("commit");
    store.commit(&commit).expect("commit");
    drop(store);

    let generation = fs::read_dir(&root)
        .expect("root")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("gen-"))
        })
        .expect("generation");
    let mut state = fs::OpenOptions::new()
        .write(true)
        .open(generation.join("state.bin"))
        .expect("state");
    state.seek(SeekFrom::Start(16)).expect("seek");
    state.write_all(&[0xff]).expect("tamper");
    state.sync_all().expect("sync");

    let restarted =
        SyncedWriteBatchStore::open(&root, StateImageLimits::production()).expect("reopen");
    let error = restarted
        .load_latest(StateImageLimits::production())
        .expect_err("tamper");
    assert!(matches!(error, StateStoreError::Corrupt));
}

struct Applied {
    delta: canonical_ledger::StateDelta,
    image: canonical_ledger::StateImage,
}

impl Applied {
    fn delta(&self) -> &canonical_ledger::StateDelta {
        &self.delta
    }

    fn image(&self) -> &canonical_ledger::StateImage {
        &self.image
    }
}

fn apply(ledger: &mut CanonicalLedger<WatermarkOnlyReducerV1>, height: u64, seed: u8) -> Applied {
    let ApplyOutcome::Applied(delta) = ledger.apply_block(&empty_block(height, seed)).unwrap()
    else {
        panic!("new block must apply");
    };
    Applied {
        delta,
        image: ledger.state_image().clone(),
    }
}

fn ledger(first_height: u64) -> CanonicalLedger<WatermarkOnlyReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .unwrap()
}

fn empty_block(height: u64, seed: u8) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64 + i64::from(seed)).unwrap(),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(SourceId::new("test-primary").unwrap(), [seed; 32])]),
    )
    .unwrap()
}
