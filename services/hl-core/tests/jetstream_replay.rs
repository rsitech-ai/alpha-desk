use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, time::Duration};

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::{LedgerLimits, StateImageLimits, WatermarkOnlyReducerV1};
use canonical_state_store::SyncedWriteBatchStore;
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ManifestId, MarketId, Price, ProtocolTime, Quantity,
    SourceId, TransactionId,
};
use hl_core::{
    CanonicalPullSource, CanonicalSubject, InMemoryCanonicalSource, JetStreamReplayAuth,
    JetStreamReplayConfig, JetStreamReplayConfigError, JetStreamReplaySession,
    committed_block_delivery, committed_event_delivery, decode_committed_block_marker,
    encode_committed_block_marker,
};
use storage_ports::ArchiveReceipt;

#[tokio::test]
async fn empty_committed_block_from_the_bus_applies_atomically_and_survives_restart() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let block = empty_block(200, 1);
    let receipt = archive_receipt(&block);
    let delivery = committed_block_delivery(&block, &receipt).expect("delivery");
    let mut source = InMemoryCanonicalSource::new([delivery.clone()]);

    let report = session
        .consume_available(&mut source)
        .await
        .expect("replay");
    assert_eq!(report.applied, 1);
    assert_eq!(report.already_applied, 0);
    assert_eq!(report.last_height, Some(BlockHeight::new(200)));
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
    assert!(source.acked().contains(&delivery.message_id));
    let hash_after_first = report.state_hash;

    let mut duplicate = InMemoryCanonicalSource::new([delivery.clone()]);
    let redone = session
        .consume_available(&mut duplicate)
        .await
        .expect("idempotent");
    assert_eq!(redone.applied, 0);
    assert_eq!(redone.already_applied, 1);
    assert_eq!(redone.state_hash, hash_after_first);
    assert!(!redone.live_qualified);
    assert!(!redone.stage_2_qualified);
    assert!(duplicate.acked().contains(&delivery.message_id));

    drop(session);
    let restarted =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("restart store");
    let mut resumed = open_session(restarted);
    assert_eq!(resumed.ledger().state_hash(), hash_after_first);
    let next = empty_block(201, 1);
    let next_delivery = committed_block_delivery(&next, &archive_receipt(&next)).expect("next");
    let mut continued_source = InMemoryCanonicalSource::new([next_delivery]);
    let continued = resumed
        .consume_available(&mut continued_source)
        .await
        .expect("resume");
    assert_eq!(continued.applied, 1);
    assert_eq!(continued.last_height, Some(BlockHeight::new(201)));
    assert!(!continued.live_qualified);
    assert!(!continued.stage_2_qualified);
}

#[tokio::test]
async fn action_bearing_bus_block_fails_closed_without_ack_or_state_advance() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = trade_block(200);
    let receipt = archive_receipt(&block);
    let marker = committed_block_delivery(&block, &receipt).expect("marker");
    let event = committed_event_delivery(&block.events()[0], &block, &receipt).expect("event");
    let event_id = event.message_id.clone();
    let marker_id = marker.message_id.clone();
    let mut source = InMemoryCanonicalSource::new([event, marker]);

    let error = session
        .consume_available(&mut source)
        .await
        .expect_err("action-bearing mapping/reducer still rejects");
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked().contains(&event_id));
    assert!(!source.acked().contains(&marker_id));
}

#[tokio::test]
async fn event_without_committed_marker_is_incomplete_and_unacked() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = trade_block(200);
    let receipt = archive_receipt(&block);
    let event = committed_event_delivery(&block.events()[0], &block, &receipt).expect("event");
    let event_id = event.message_id.clone();
    let mut source = InMemoryCanonicalSource::new([event]);

    let error = session
        .consume_available(&mut source)
        .await
        .expect_err("incomplete block");
    assert_eq!(error.reason_code(), "core.jetstream_incomplete_block");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(!source.acked().contains(&event_id));
}

#[tokio::test]
async fn payload_hash_mismatch_fails_closed() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let block = empty_block(200, 1);
    let receipt = archive_receipt(&block);
    let mut delivery = committed_block_delivery(&block, &receipt).expect("delivery");
    let last = delivery.payload.len() - 1;
    delivery.payload[last] ^= 0xff;
    let mut source = InMemoryCanonicalSource::new([delivery]);

    let error = session
        .consume_available(&mut source)
        .await
        .expect_err("tampered marker");
    assert_eq!(error.reason_code(), "core.jetstream_hash_mismatch");
    assert!(session.ledger().checkpoint().is_none());
}

#[test]
fn marker_roundtrip_matches_the_frozen_publication_layout() {
    let block = empty_block(200, 7);
    let receipt = archive_receipt(&block);
    let encoded = encode_committed_block_marker(&block, &receipt).expect("encode");
    let decoded = decode_committed_block_marker(&encoded).expect("decode");
    assert_eq!(decoded.chain_id, block.chain_id().clone());
    assert_eq!(decoded.block_height, block.block_height());
    assert_eq!(decoded.canonical_block_hash, block.canonical_block_hash());
    assert!(decoded.events.is_empty());
    assert_eq!(decoded.archive_receipt_id, receipt.receipt_id());
}

#[test]
fn jetstream_replay_configuration_rejects_inline_credentials_and_unbounded_limits() {
    let inline = JetStreamReplayConfig::try_new(
        "nats://user:secret@127.0.0.1:4222",
        JetStreamReplayAuth::Anonymous,
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
        JetStreamReplayConfig::default_durable_name(),
        64,
    )
    .expect_err("inline credentials");
    assert_eq!(inline, JetStreamReplayConfigError::UnsafeServerUrl);

    let zero_inflight = JetStreamReplayConfig::try_new(
        "nats://127.0.0.1:4222",
        JetStreamReplayAuth::Anonymous,
        Duration::from_secs(5),
        Duration::from_secs(5),
        0,
        JetStreamReplayConfig::default_durable_name(),
        64,
    )
    .expect_err("zero in-flight");
    assert_eq!(
        zero_inflight,
        JetStreamReplayConfigError::InvalidMaxAckInflight
    );
}

#[tokio::test]
async fn real_jetstream_file_replay_is_an_opt_in_integration_lane() {
    let Ok(server_url) = std::env::var("ALPHA_DESK_NATS_TEST_URL") else {
        eprintln!(
            "SKIP real JetStream replay test: set ALPHA_DESK_NATS_TEST_URL to select integration lane"
        );
        return;
    };
    let config = JetStreamReplayConfig::try_new(
        server_url,
        match (
            std::env::var("ALPHA_DESK_NATS_TEST_USER"),
            std::env::var("ALPHA_DESK_NATS_TEST_PASSWORD_FILE"),
        ) {
            (Ok(username), Ok(path)) => JetStreamReplayAuth::UserPasswordFile {
                username,
                password_path: std::path::PathBuf::from(path),
            },
            (Err(_), Err(_)) => JetStreamReplayAuth::Anonymous,
            _ => panic!(
                "ALPHA_DESK_NATS_TEST_USER and ALPHA_DESK_NATS_TEST_PASSWORD_FILE must be set together"
            ),
        },
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
        format!(
            "hl-core-file-replay-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("time")
                .as_millis()
        ),
        8,
    )
    .expect("JetStream config");
    let mut source = hl_core::JetStreamPullSource::connect(config)
        .await
        .expect("connect JetStream pull consumer");
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let _ = source.fetch(1).await.expect("fetch must not panic");
    let report = session
        .consume_available(&mut InMemoryCanonicalSource::new([]))
        .await
        .expect("empty drain after optional fetch");
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
}

#[test]
fn committed_subject_rejects_provisional_fanout() {
    let error = CanonicalSubject::parse("hl.v1.block.provisional").expect_err("provisional");
    assert_eq!(error.reason_code(), "core.jetstream_provisional");
}

fn open_session(
    store: SyncedWriteBatchStore,
) -> JetStreamReplaySession<WatermarkOnlyReducerV1, SyncedWriteBatchStore> {
    JetStreamReplaySession::open(
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
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("jetstream-replay").expect("source"),
            [seed; 32],
        )]),
    )
    .expect("block")
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
