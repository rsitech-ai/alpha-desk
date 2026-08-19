use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, time::Duration};

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::StateImageLimits;
use canonical_state_store::SyncedWriteBatchStore;
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ManifestId, MarketId, Price, ProtocolTime, Quantity,
    SourceId, TransactionId,
};
use hl_core::{
    CoreConfig, CoreRuntime, InMemoryCanonicalSource, committed_block_delivery,
    committed_event_delivery,
};
use storage_ports::{ArchiveReceipt, AtomicStateStore};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn missing_store_parent_fails_closed_without_opening_nats() {
    let root = private_root();
    let missing = root.path().join("absent").join("state");
    let config = CoreConfig::from_toml(&valid_toml(&missing, 200)).expect("config");
    let error = match CoreRuntime::open(config) {
        Ok(_) => panic!("missing store must fail closed"),
        Err(error) => error,
    };
    assert_eq!(error.reason_code(), "core_runtime.store");
}

#[tokio::test]
async fn action_bearing_source_still_fails_closed_without_ack_or_state_advance() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200)).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let block = trade_block(200);
    let receipt = archive_receipt(&block);
    let marker = committed_block_delivery(&block, &receipt).expect("marker");
    let event = committed_event_delivery(&block.events()[0], &block, &receipt).expect("event");
    let event_id = event.message_id.clone();
    let marker_id = marker.message_id.clone();
    let mut source = InMemoryCanonicalSource::new([event, marker]);
    let cancellation = CancellationToken::new();

    let error = runtime
        .run_source(&mut source, cancellation)
        .await
        .expect_err("action-bearing mapping/reducer still rejects");
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert!(!source.acked().contains(&event_id));
    assert!(!source.acked().contains(&marker_id));

    let store =
        SyncedWriteBatchStore::open(&store_path, StateImageLimits::production()).expect("reopen");
    assert!(
        store
            .load_latest(StateImageLimits::production())
            .expect("load")
            .is_none()
    );
}

#[tokio::test]
async fn empty_source_shuts_down_cleanly_without_qualification_claims() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200)).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut source = InMemoryCanonicalSource::new([]);
    let report = runtime
        .run_source(&mut source, cancellation)
        .await
        .expect("clean shutdown");
    assert_eq!(report.applied, 0);
    assert!(report.last_height.is_none());
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
}

#[tokio::test]
async fn idle_loop_observes_cancellation_without_connecting_to_nats() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200)).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let cancellation = CancellationToken::new();
    let mut source = InMemoryCanonicalSource::new([]);
    let run = runtime.run_source(&mut source, cancellation.clone());
    let shutdown = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancellation.cancel();
    };
    let (report, ()) = tokio::join!(run, shutdown);
    let report = report.expect("cancelled idle loop");
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
}

fn valid_toml(store_path: &std::path::Path, first_height: u64) -> String {
    format!(
        r#"
chain_id = "mainnet"
first_height = {first_height}
shutdown_grace_millis = 15000
idle_poll_millis = 50

[store]
path = "{path}"

[nats]
server_url = "nats://127.0.0.1:4222"
stream = "HL_CANONICAL"
username = "core"
password_path = "/run/secrets/alpha-desk-nats-core-password"
connect_timeout_millis = 5000
acknowledgement_timeout_millis = 5000
max_ack_inflight = 64
durable_name = "hl-core-file-replay"
fetch_batch = 64
"#,
        path = store_path.display()
    )
}

fn private_root() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private parent");
    temporary
}

fn trade_block(height: u64) -> BlockEnvelope {
    let event = trade_event(height);
    BlockEnvelope::try_new(
        event.chain_id().clone(),
        event.block_height(),
        event.block_time(),
        ConfirmationClass::CommittedPrimary,
        vec![event],
        BTreeMap::from([(
            SourceId::new("jetstream-replay").expect("source"),
            [0x44; 32],
        )]),
    )
    .expect("action-bearing block")
}

fn trade_event(height: u64) -> CanonicalEventEnvelope {
    let block_time = ProtocolTime::from_unix_micros(height as i64).expect("time");
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).expect("price"),
        Quantity::parse_at_scale("0.01", 8).expect("quantity"),
        1,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec().expect("payload bytes")).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC").expect("market")],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("jetstream-replay").expect("source"),
                "v1",
                height.to_string(),
                payload_hash,
                0,
            )
            .expect("evidence"),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        ingested_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .expect("event")
}

fn archive_receipt(block: &BlockEnvelope) -> ArchiveReceipt {
    ArchiveReceipt::try_new(
        format!("receipt-{}", block.block_height().get()),
        ManifestId::new(format!(
            "manifest-{}",
            hex::encode(block.canonical_block_hash())
        ))
        .expect("manifest"),
        block.block_height(),
        block.canonical_block_hash(),
        [0x11; 32],
        [0x22; 32],
        [0x33; 32],
        KnownTime::from_unix_micros(1_721_779_300_000_000).expect("durable at"),
    )
    .expect("archive receipt")
}
