use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    sync::atomic::{AtomicBool, Ordering},
};

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    CheckpointArtifact, CheckpointCompatibility, LedgerLimits, StateImageLimits,
    WatermarkOnlyReducerV1,
};
use canonical_state_store::{LocalCheckpointStore, SyncedWriteBatchStore};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ManifestId, MarketId, Price, ProtocolTime, Quantity,
    SourceId, TransactionId,
};
use hl_core::{
    CanonicalDelivery, CanonicalPullSource, CoreApp, CoreConfig, CoreInputSubject, DiskReserve,
    DiskSpaceProbe, DurableApplyOutcome, FeatureHealth, InMemoryCanonicalSource, InMemoryDeltaSink,
    JetStreamReplayError, JetStreamReplaySession, PublishDisposition, ResumeMode,
    SNAPSHOT_ACCOUNT_SUBJECT, ShutdownFlag, StateRuntime, admit_resume_height, align_watermarks,
    committed_block_delivery, publish_state_delta,
};
use storage_ports::{ArchiveReceipt, StateCheckpointStore};

struct FixedDisk(u64);

impl DiskSpaceProbe for FixedDisk {
    fn available_bytes(&self) -> Result<u64, hl_core::DiskPressureError> {
        Ok(self.0)
    }
}

struct AckFailingSource {
    inner: InMemoryCanonicalSource,
    fail: AtomicBool,
}

impl CanonicalPullSource for AckFailingSource {
    async fn fetch(
        &mut self,
        max_messages: usize,
    ) -> Result<Vec<CanonicalDelivery>, JetStreamReplayError> {
        self.inner.fetch(max_messages).await
    }

    async fn ack(&mut self, message_ids: &[String]) -> Result<(), JetStreamReplayError> {
        if self.fail.load(Ordering::Acquire) {
            return Err(JetStreamReplayError::Transport);
        }
        self.inner.ack(message_ids).await
    }
}

#[test]
fn resume_refuses_mid_history_without_a_checkpoint() {
    assert_eq!(
        admit_resume_height(BlockHeight::new(200), BlockHeight::new(201), false)
            .unwrap_err()
            .reason_code(),
        "core.resume.mid_history"
    );
    admit_resume_height(BlockHeight::new(200), BlockHeight::new(200), false).unwrap();
    admit_resume_height(BlockHeight::new(200), BlockHeight::new(201), true).unwrap();
}

#[test]
fn genesis_open_refuses_an_existing_durable_image() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut runtime = open_durable(store);
    runtime
        .apply_committed(&empty_block(200, 1), &archive_receipt(&empty_block(200, 1)))
        .expect("apply");
    drop(runtime);
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("reopen");
    let error = match StateRuntime::open(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(200),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
        store,
        StateImageLimits::production(),
        ResumeMode::Genesis,
        None,
    ) {
        Ok(_) => panic!("genesis must not wipe"),
        Err(error) => error,
    };
    assert_eq!(error.reason_code(), "core.resume.mid_history");
}

#[test]
fn durable_resume_replays_from_the_committed_image_not_an_arbitrary_height() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut runtime = open_durable(store);
    let first = empty_block(200, 1);
    runtime
        .apply_committed(&first, &archive_receipt(&first))
        .expect("first");
    let hash = runtime.ledger().state_hash();
    drop(runtime);
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("reopen");
    let mut resumed = open_durable(store);
    assert_eq!(resumed.ledger().state_hash(), hash);
    let second = empty_block(201, 1);
    let outcome = resumed
        .apply_committed(&second, &archive_receipt(&second))
        .expect("second");
    assert!(matches!(outcome, DurableApplyOutcome::Applied { .. }));
    assert_eq!(
        resumed.ledger().checkpoint().unwrap().block_height(),
        BlockHeight::new(201)
    );
}

#[test]
fn checkpoint_restore_yields_the_same_state_hash() {
    let root = private_root();
    let atomic =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("atomic");
    let checkpoints = LocalCheckpointStore::open(
        root.path().join("checkpoints"),
        StateImageLimits::production(),
    )
    .expect("checkpoints");
    let mut runtime = open_durable(atomic);
    let block = empty_block(200, 7);
    runtime
        .apply_committed(&block, &archive_receipt(&block))
        .expect("apply");
    let hash = runtime.ledger().state_hash();
    let artifact = CheckpointArtifact::try_new(
        runtime.ledger().checkpoint().unwrap(),
        runtime.ledger().state_image().clone(),
        ManifestId::new("archive-manifest-v1-test").unwrap(),
        ARCHIVE_HASH,
        SCHEMA_FINGERPRINT,
    )
    .unwrap();
    checkpoints.publish(&artifact).unwrap();
    let checkpoint_id = artifact.checkpoint_id().clone();
    drop(runtime);

    let fresh =
        SyncedWriteBatchStore::open(root.path().join("fresh"), StateImageLimits::production())
            .expect("fresh atomic");
    let compatibility = CheckpointCompatibility::try_new(
        ChainId::new("mainnet").unwrap(),
        WatermarkOnlyReducerV1::VERSION,
        ManifestId::new("archive-manifest-v1-test").unwrap(),
        ARCHIVE_HASH,
        SCHEMA_FINGERPRINT,
    )
    .unwrap();
    let restored = match StateRuntime::open(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(200),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
        fresh,
        StateImageLimits::production(),
        ResumeMode::Checkpoint(checkpoint_id),
        Some((&checkpoints as &dyn StateCheckpointStore, &compatibility)),
    ) {
        Ok(runtime) => runtime,
        Err(error) => panic!("{}", error.reason_code()),
    };
    assert_eq!(restored.ledger().state_hash(), hash);
    assert_eq!(
        restored.ledger().checkpoint().unwrap().block_height(),
        BlockHeight::new(200)
    );
}

#[test]
fn apply_committed_refuses_a_misaligned_archive_receipt() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut runtime = open_durable(store);
    let error = runtime
        .apply_committed(&empty_block(200, 1), &archive_receipt(&empty_block(201, 1)))
        .expect_err("misaligned");
    assert_eq!(error.reason_code(), "core.watermark_misaligned");
    assert!(runtime.ledger().checkpoint().is_none());
}

#[test]
fn archive_and_state_watermarks_must_match() {
    align_watermarks(
        BlockHeight::new(200),
        BlockHeight::new(200),
        BlockHeight::new(200),
    )
    .unwrap();
    assert_eq!(
        align_watermarks(
            BlockHeight::new(200),
            BlockHeight::new(201),
            BlockHeight::new(200),
        )
        .unwrap_err()
        .reason_code(),
        "core.watermark_misaligned"
    );
}

#[test]
fn snapshots_and_provisional_inputs_are_quarantined_and_do_not_enter_the_ledger() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut runtime = open_durable(store);
    let before = runtime.ledger().state_hash();
    let snapshot = CoreInputSubject::parse(SNAPSHOT_ACCOUNT_SUBJECT).unwrap();
    let disposition = runtime.ingest_subject(snapshot);
    assert!(!disposition.may_enter_ledger());
    assert_eq!(runtime.reconciliation().quarantined().len(), 1);
    assert_eq!(
        runtime.reconciliation().quarantined()[0].reason_code(),
        "core.quarantine.provisional_or_snapshot"
    );
    assert_eq!(runtime.ledger().state_hash(), before);
    assert!(runtime.ledger().checkpoint().is_none());
    assert!(!runtime.health().state().suppresses_publication());
}

#[test]
fn red_feature_health_suppresses_state_delta_publication() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut runtime = open_durable(store);
    let block = empty_block(200, 1);
    let DurableApplyOutcome::Applied { delta, .. } = runtime
        .apply_committed(&block, &archive_receipt(&block))
        .unwrap()
    else {
        panic!("applied");
    };
    let mut sink = InMemoryDeltaSink::default();
    let published = publish_state_delta(
        &mut sink,
        runtime.health(),
        &ChainId::new("mainnet").unwrap(),
        &delta,
    )
    .unwrap();
    assert_eq!(published, PublishDisposition::Published);
    assert_eq!(sink.published().len(), 1);

    runtime.health_mut().observe_material_divergence(true);
    assert_eq!(runtime.health().state().as_wire_name(), "RED");
    let suppressed = publish_state_delta(
        &mut sink,
        runtime.health(),
        &ChainId::new("mainnet").unwrap(),
        &delta,
    )
    .unwrap();
    assert_eq!(suppressed, PublishDisposition::Suppressed);
    assert_eq!(sink.published().len(), 1);
}

#[test]
fn core_app_opens_the_file_atomic_store_from_genesis_without_nats() {
    let root = private_root();
    let state = root.path().join("state");
    let checkpoints = root.path().join("checkpoints");
    fs::create_dir_all(&state).unwrap();
    fs::create_dir_all(&checkpoints).unwrap();
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700)).unwrap();
    fs::set_permissions(&checkpoints, fs::Permissions::from_mode(0o700)).unwrap();
    let source = format!(
        r#"
chain_id = "mainnet"
genesis_height = 200
state_path = "{state}"
checkpoint_path = "{checkpoints}"
disk_reserve_bytes = 10
max_pending_blocks = 64
shutdown_grace_millis = 15

[resume]
mode = "genesis"
"#,
        state = state.display(),
        checkpoints = checkpoints.display(),
    );
    let config = CoreConfig::from_toml(&source).expect("config");
    let disk = DiskReserve::try_new(FixedDisk(1_000), 10).unwrap();
    let app = match CoreApp::open(&config, disk) {
        Ok(app) => app,
        Err(error) => panic!("{}", error.reason_code()),
    };
    assert!(app.latest_height().is_none());
    assert!(!app.shutdown().is_stopped());
    assert!(!app.health().state().suppresses_publication());
}

#[test]
fn disk_pressure_and_shutdown_are_bounded() {
    let plenty = DiskReserve::try_new(FixedDisk(1_000), 10).unwrap();
    assert!(plenty.ensure().is_ok());
    let tight = DiskReserve::try_new(FixedDisk(5), 10).unwrap();
    assert_eq!(
        tight.ensure().unwrap_err().reason_code(),
        "core.disk.exhausted"
    );
    let flag = ShutdownFlag::new();
    assert!(!flag.is_stopped());
    flag.request_stop();
    assert!(flag.is_stopped());
    let mut health = FeatureHealth::green();
    health.observe_disk_pressure(true);
    assert!(health.state().suppresses_publication());
    health.observe_backlog(true);
    assert_eq!(health.reason_code(), "core.health.backlog");
}

#[tokio::test]
async fn crash_after_state_write_before_ack_redelivers_as_already_applied() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_jetstream(store);
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).unwrap();
    let mut failing = AckFailingSource {
        inner: InMemoryCanonicalSource::new([delivery.clone()]),
        fail: AtomicBool::new(true),
    };
    let error = session
        .consume_available(&mut failing)
        .await
        .expect_err("ack failed after durable write");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert_eq!(
        session.ledger().checkpoint().unwrap().block_height(),
        BlockHeight::new(200)
    );
    let hash = session.ledger().state_hash();
    drop(session);

    let restarted =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("restart");
    let mut resumed = open_jetstream(restarted);
    assert_eq!(resumed.ledger().state_hash(), hash);
    let mut redelivery = InMemoryCanonicalSource::new([delivery.clone()]);
    let report = resumed.consume_available(&mut redelivery).await.unwrap();
    assert_eq!(report.applied, 0);
    assert_eq!(report.already_applied, 1);
    assert!(redelivery.acked().contains(&delivery.message_id));
}

#[tokio::test]
async fn pending_block_buffer_is_bounded() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_jetstream(store);
    let mut deliveries = Vec::new();
    for height in 200..265_u64 {
        let block = trade_block(height);
        deliveries.push(committed_block_delivery(&block, &archive_receipt(&block)).unwrap());
    }
    let mut source = InMemoryCanonicalSource::new(deliveries);
    let error = session
        .consume_available(&mut source)
        .await
        .expect_err("bounded pending");
    assert_eq!(error.reason_code(), "core.jetstream_pending_limit");
}

fn open_durable(
    store: SyncedWriteBatchStore,
) -> StateRuntime<WatermarkOnlyReducerV1, SyncedWriteBatchStore> {
    match StateRuntime::open(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(200),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
        store,
        StateImageLimits::production(),
        ResumeMode::Durable,
        None,
    ) {
        Ok(runtime) => runtime,
        Err(error) => panic!("{}", error.reason_code()),
    }
}

fn open_jetstream(
    store: SyncedWriteBatchStore,
) -> JetStreamReplaySession<WatermarkOnlyReducerV1, SyncedWriteBatchStore> {
    match JetStreamReplaySession::open(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(200),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
        store,
        StateImageLimits::production(),
    ) {
        Ok(session) => session,
        Err(error) => panic!("{}", error.reason_code()),
    }
}

fn private_root() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private parent");
    temporary
}

fn empty_block(height: u64, seed: u8) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).unwrap(),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(SourceId::new("core-runtime").unwrap(), [seed; 32])]),
    )
    .unwrap()
}

fn trade_block(height: u64) -> BlockEnvelope {
    let event = trade_event(height);
    BlockEnvelope::try_new(
        event.chain_id().clone(),
        event.block_height(),
        event.block_time(),
        ConfirmationClass::CommittedPrimary,
        vec![event],
        BTreeMap::from([(SourceId::new("core-runtime").unwrap(), [0x44; 32])]),
    )
    .unwrap()
}

fn trade_event(height: u64) -> CanonicalEventEnvelope {
    let block_time = ProtocolTime::from_unix_micros(height as i64).unwrap();
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).unwrap(),
        Quantity::parse_at_scale("0.01", 8).unwrap(),
        1,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}")).unwrap(),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC").unwrap()],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("core-runtime").unwrap(),
                "v1",
                height.to_string(),
                payload_hash,
                0,
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        ingested_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .unwrap()
}

const ARCHIVE_HASH: [u8; 32] = [0x44; 32];
const SCHEMA_FINGERPRINT: [u8; 32] = [0x55; 32];

fn archive_receipt(block: &BlockEnvelope) -> ArchiveReceipt {
    ArchiveReceipt::try_new(
        format!("receipt-{}", block.block_height().get()),
        ManifestId::new(format!(
            "manifest-{}",
            hex::encode(block.canonical_block_hash())
        ))
        .unwrap(),
        block.block_height(),
        block.canonical_block_hash(),
        [0x11; 32],
        ARCHIVE_HASH,
        SCHEMA_FINGERPRINT,
        KnownTime::from_unix_micros(1_721_779_300_000_000).unwrap(),
    )
    .unwrap()
}
