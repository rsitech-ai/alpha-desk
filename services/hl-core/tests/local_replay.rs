use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt};

use canonical_events::{BlockEnvelope, ConfirmationClass};
use canonical_ledger::{LedgerLimits, StateImageLimits, WatermarkOnlyReducerV1};
use canonical_state_store::SyncedWriteBatchStore;
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};
use hl_core::{
    DirectoryBlockSource, DurableApplyError, InMemoryBlockSource, LocalReplayError,
    LocalReplaySession,
};

#[test]
fn in_memory_source_applies_block_atomically_to_the_file_backed_store_and_survives_restart() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let mut source = InMemoryBlockSource::new([empty_block(200, 1), empty_block(201, 1)]);
    let report = session.replay(&mut source).expect("replay");
    assert_eq!(report.applied, 2);
    assert_eq!(report.already_applied, 0);
    assert_eq!(report.last_height, Some(BlockHeight::new(201)));
    let hash_after_first = report.state_hash;

    let mut duplicate = InMemoryBlockSource::new([empty_block(201, 1)]);
    let redone = session.replay(&mut duplicate).expect("idempotent");
    assert_eq!(redone.applied, 0);
    assert_eq!(redone.already_applied, 1);
    assert_eq!(redone.state_hash, hash_after_first);

    drop(session);
    let restarted =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("restart store");
    let mut resumed = open_session(restarted);
    assert_eq!(resumed.ledger().state_hash(), hash_after_first);
    let mut next = InMemoryBlockSource::new([empty_block(202, 1)]);
    let continued = resumed.replay(&mut next).expect("resume");
    assert_eq!(continued.applied, 1);
    assert_eq!(continued.last_height, Some(BlockHeight::new(202)));
}

#[test]
fn directory_source_replays_synthetic_empty_blocks() {
    let root = private_root();
    let blocks = root.path().join("blocks");
    fs::create_dir_all(&blocks).expect("blocks dir");
    fs::write(
        blocks.join("00000000000000000200.json"),
        block_json(200, "committed-primary"),
    )
    .expect("block 200");
    fs::write(
        blocks.join("00000000000000000201.json"),
        block_json(201, "committed-primary"),
    )
    .expect("block 201");

    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let mut source = DirectoryBlockSource::open(&blocks).expect("directory source");
    let report = session.replay(&mut source).expect("file replay");
    assert_eq!(report.applied, 2);
    assert_eq!(report.last_height, Some(BlockHeight::new(201)));
}

#[test]
fn corrected_source_blocks_fail_closed_and_do_not_advance_durable_state() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let mut source =
        InMemoryBlockSource::new([empty_block_class(200, 1, ConfirmationClass::Corrected)]);
    let error = session.replay(&mut source).expect_err("correction denied");
    match error {
        LocalReplayError::Durable(DurableApplyError::Ledger(ledger)) => {
            assert_eq!(ledger.reason_code(), "ledger.correction_unimplemented");
        }
        other => panic!("expected ledger correction denial, got {other:?}"),
    }
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
}

#[test]
fn directory_source_refuses_qualification_claims() {
    let root = private_root();
    let blocks = root.path().join("blocks");
    fs::create_dir_all(&blocks).expect("blocks dir");
    fs::write(
        blocks.join("00000000000000000200.json"),
        block_json(200, "committed-primary").replace(
            "\"stage_2_qualified\": false",
            "\"stage_2_qualified\": true",
        ),
    )
    .expect("qualified claim");
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let mut source = DirectoryBlockSource::open(&blocks).expect("directory source");
    let error = session.replay(&mut source).expect_err("qualification");
    assert_eq!(error.reason_code(), "core.replay_qualification");
}

fn open_session(
    store: SyncedWriteBatchStore,
) -> LocalReplaySession<WatermarkOnlyReducerV1, SyncedWriteBatchStore> {
    LocalReplaySession::open(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(200),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
        store,
        StateImageLimits::production(),
    )
    .expect("session")
}

fn private_root() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private parent");
    temporary
}

fn empty_block(height: u64, seed: u8) -> BlockEnvelope {
    empty_block_class(height, seed, ConfirmationClass::CommittedPrimary)
}

fn empty_block_class(height: u64, seed: u8, confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        confirmation,
        Vec::new(),
        BTreeMap::from([(SourceId::new("local-replay").expect("source"), [seed; 32])]),
    )
    .expect("block")
}

fn block_json(height: u64, confirmation: &str) -> String {
    format!(
        r#"{{
  "schema": "hl.core.local-replay-block.v1",
  "source_qualification": "synthetic_unassessed",
  "stage_1_qualified": false,
  "stage_2_qualified": false,
  "chain_id": "mainnet",
  "block_height": {height},
  "block_time_micros": {height},
  "confirmation_class": "{confirmation}",
  "source_block_hashes": {{"local-replay": "{hash}"}}
}}
"#,
        hash = hex::encode([1_u8; 32])
    )
}
