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
    CanonicalPullSource, CanonicalSubject, DEAD_LETTER_SCHEMA_V1, DeadLetterError,
    DeadLetterRecord, DeadLetterSink, FileDeadLetterSink, InMemoryCanonicalSource,
    InMemoryDeadLetterSink, InMemoryFetchSource, JetStreamFetchFrame, JetStreamReplayAuth,
    JetStreamReplayConfig, JetStreamReplayConfigError, JetStreamReplayError,
    JetStreamReplaySession, committed_block_delivery, committed_event_delivery,
    decode_committed_block_marker, encode_committed_block_marker,
};
use sha2::{Digest as _, Sha256};
use storage_ports::ArchiveReceipt;

mod common;

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
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let report = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect("replay");
    assert_eq!(report.applied, 1);
    assert_eq!(report.already_applied, 0);
    assert_eq!(report.last_height, Some(BlockHeight::new(200)));
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
    assert!(source.acked().contains(&delivery.message_id));
    assert!(dead_letter.records().is_empty());
    let hash_after_first = report.state_hash;

    let mut duplicate = InMemoryCanonicalSource::new([delivery.clone()]);
    let mut duplicate_dead_letter = InMemoryDeadLetterSink::default();
    let redone = session
        .consume_available(&mut duplicate, &mut duplicate_dead_letter)
        .await
        .expect("idempotent");
    assert_eq!(redone.applied, 0);
    assert_eq!(redone.already_applied, 1);
    assert_eq!(redone.state_hash, hash_after_first);
    assert!(!redone.live_qualified);
    assert!(!redone.stage_2_qualified);
    assert!(duplicate.acked().contains(&delivery.message_id));
    assert!(duplicate_dead_letter.records().is_empty());

    drop(session);
    let restarted =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("restart store");
    let mut resumed = open_session(restarted);
    assert_eq!(resumed.ledger().state_hash(), hash_after_first);
    let next = empty_block(201, 1);
    let next_delivery = committed_block_delivery(&next, &archive_receipt(&next)).expect("next");
    let mut continued_source = InMemoryCanonicalSource::new([next_delivery]);
    let mut continued_dead_letter = InMemoryDeadLetterSink::default();
    let continued = resumed
        .consume_available(&mut continued_source, &mut continued_dead_letter)
        .await
        .expect("resume");
    assert_eq!(continued.applied, 1);
    assert_eq!(continued.last_height, Some(BlockHeight::new(201)));
    assert!(!continued.live_qualified);
    assert!(!continued.stage_2_qualified);
    assert!(continued_dead_letter.records().is_empty());
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
    let mut source = InMemoryCanonicalSource::new([event, marker.clone()]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("action-bearing mapping/reducer still rejects");
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked().contains(&event_id));
    assert!(!source.acked().contains(&marker_id));
    assert_one_dead_letter(
        &dead_letter,
        "ledger.unsupported_event",
        &marker_id,
        marker.publication_sha256,
        marker.block_hash,
    );
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
    let mut source = InMemoryCanonicalSource::new([event.clone()]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("incomplete block");
    assert_eq!(error.reason_code(), "core.jetstream_incomplete_block");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(!source.acked().contains(&event_id));
    assert_one_dead_letter(
        &dead_letter,
        "core.jetstream_incomplete_block",
        &event_id,
        event.publication_sha256,
        event.block_hash,
    );
}

#[tokio::test]
async fn payload_hash_mismatch_fails_closed() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let receipt = archive_receipt(&block);
    let mut delivery = committed_block_delivery(&block, &receipt).expect("delivery");
    let last = delivery.payload.len() - 1;
    delivery.payload[last] ^= 0xff;
    let poison_hash: [u8; 32] = Sha256::digest(&delivery.payload).into();
    let message_id = delivery.message_id.clone();
    let block_hash = delivery.block_hash;
    let mut source = InMemoryCanonicalSource::new([delivery]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("tampered marker");
    assert_eq!(error.reason_code(), "core.jetstream_hash_mismatch");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked().contains(&message_id));
    assert_one_dead_letter(
        &dead_letter,
        "core.jetstream_hash_mismatch",
        &message_id,
        poison_hash,
        block_hash,
    );
}

#[tokio::test]
async fn fetch_missing_headers_records_dlq_without_ack_or_state_advance() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let payload_hash: [u8; 32] = Sha256::digest(&delivery.payload).into();
    let mut frame = JetStreamFetchFrame::from_delivery(&delivery);
    frame.headers = None;
    frame.stream_sequence = Some(11);
    frame.consumer_sequence = Some(2);
    frame.delivery_count = 4;
    let mut source = InMemoryFetchSource::new([frame]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("missing headers");
    assert_eq!(error.reason_code(), "core.jetstream_decode");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(source.acked().is_empty());
    assert!(source.terminated());
    assert_fetch_dead_letter(
        &dead_letter,
        "core.jetstream_decode",
        "hl.v1.block.committed",
        "undecodable",
        payload_hash,
        [0; 32],
        Some(11),
        Some(2),
        4,
    );
}

#[tokio::test]
async fn fetch_publication_hash_mismatch_records_dlq_with_payload_hash_and_stream_seq() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let payload_hash: [u8; 32] = Sha256::digest(&delivery.payload).into();
    let message_id = delivery.message_id.clone();
    let block_hash = delivery.block_hash;
    let mut frame = JetStreamFetchFrame::from_delivery(&delivery);
    frame.stream_sequence = Some(42);
    frame.consumer_sequence = Some(7);
    frame.delivery_count = 3;
    if let Some(headers) = frame.headers.as_mut() {
        headers.insert("Alpha-Desk-Publication-SHA256", hex::encode([0xab; 32]));
    }
    let mut source = InMemoryFetchSource::new([frame]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("fetch hash mismatch");
    assert_eq!(error.reason_code(), "core.jetstream_hash_mismatch");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked().contains(&message_id));
    assert!(source.terminated());
    assert_fetch_dead_letter(
        &dead_letter,
        "core.jetstream_hash_mismatch",
        "hl.v1.block.committed",
        &message_id,
        payload_hash,
        block_hash,
        Some(42),
        Some(7),
        3,
    );
}

#[tokio::test]
async fn fetch_malformed_publication_hash_records_dlq() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let payload_hash: [u8; 32] = Sha256::digest(&delivery.payload).into();
    let message_id = delivery.message_id.clone();
    let block_hash = delivery.block_hash;
    let mut frame = JetStreamFetchFrame::from_delivery(&delivery);
    frame.stream_sequence = Some(9);
    if let Some(headers) = frame.headers.as_mut() {
        headers.insert("Alpha-Desk-Publication-SHA256", "not-a-hash");
    }
    let mut source = InMemoryFetchSource::new([frame]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("malformed publication hash");
    assert_eq!(error.reason_code(), "core.jetstream_decode");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(source.acked().is_empty());
    assert!(source.terminated());
    assert_fetch_dead_letter(
        &dead_letter,
        "core.jetstream_decode",
        "hl.v1.block.committed",
        &message_id,
        payload_hash,
        block_hash,
        Some(9),
        None,
        0,
    );
}

#[tokio::test]
async fn fetch_provisional_subject_records_dlq_without_ack() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let payload_hash: [u8; 32] = Sha256::digest(&delivery.payload).into();
    let message_id = delivery.message_id.clone();
    let mut frame = JetStreamFetchFrame::from_delivery(&delivery);
    frame.subject = "hl.v1.block.provisional".to_owned();
    frame.stream_sequence = Some(5);
    let mut source = InMemoryFetchSource::new([frame]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("provisional subject");
    assert_eq!(error.reason_code(), "core.jetstream_provisional");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(!source.acked().contains(&message_id));
    assert!(source.terminated());
    assert_eq!(
        dead_letter.records()[0].subject(),
        "hl.v1.block.provisional"
    );
    assert_eq!(dead_letter.records()[0].payload_sha256(), payload_hash);
    assert_eq!(dead_letter.records()[0].stream_sequence(), Some(5));
}

#[tokio::test]
async fn valid_fetch_frame_applies_without_dlq_or_term() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let message_id = delivery.message_id.clone();
    let mut source = InMemoryFetchSource::new([JetStreamFetchFrame::from_delivery(&delivery)]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let report = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect("valid fetch frame");
    assert_eq!(report.applied, 1);
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
    assert!(source.acked().contains(&message_id));
    assert!(!source.terminated());
    assert!(dead_letter.records().is_empty());
}

#[tokio::test]
async fn ack_transport_after_apply_records_dlq_without_success_ack_or_double_apply() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let block = empty_block(200, 1);
    let mut delivery =
        committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    delivery.stream_sequence = Some(13);
    delivery.consumer_sequence = Some(4);
    delivery.delivery_count = 2;
    let payload_hash: [u8; 32] = Sha256::digest(&delivery.payload).into();
    let message_id = delivery.message_id.clone();
    let block_hash = delivery.block_hash;
    let mut source = AckFailSource::new([delivery.clone()]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("ack transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert_eq!(
        session
            .ledger()
            .checkpoint()
            .map(|checkpoint| checkpoint.block_height()),
        Some(BlockHeight::new(200))
    );
    assert!(!source.acked().contains(&message_id));
    assert_fetch_dead_letter(
        &dead_letter,
        "core.jetstream_transport",
        "hl.v1.block.committed",
        &message_id,
        payload_hash,
        block_hash,
        Some(13),
        Some(4),
        2,
    );

    let mut retry = InMemoryCanonicalSource::new([delivery]);
    let mut retry_dead_letter = InMemoryDeadLetterSink::default();
    let redone = session
        .consume_available(&mut retry, &mut retry_dead_letter)
        .await
        .expect("already applied after ack failure");
    assert_eq!(redone.applied, 0);
    assert_eq!(redone.already_applied, 1);
    assert_eq!(redone.last_height, Some(BlockHeight::new(200)));
    assert!(!redone.live_qualified);
    assert!(!redone.stage_2_qualified);
    assert!(retry.acked().contains(&message_id));
    assert!(retry_dead_letter.records().is_empty());
}

#[tokio::test]
async fn connect_transport_error_records_sentinel_dlq_then_fails_closed() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .connect_source(
            || async { Err::<InMemoryCanonicalSource, _>(JetStreamReplayError::Transport) },
            &mut dead_letter,
        )
        .await
        .expect_err("connect transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert_fetch_dead_letter(
        &dead_letter,
        "core.jetstream_transport",
        "hl.v1.connect.transport",
        "connect",
        [0; 32],
        [0; 32],
        None,
        None,
        0,
    );
}

#[tokio::test]
async fn fetch_transport_error_records_sentinel_dlq_without_ack_or_state_advance() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let mut source = TransportFailSource::default();
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked);
    assert_fetch_dead_letter(
        &dead_letter,
        "core.jetstream_transport",
        "hl.v1.fetch.transport",
        "transport",
        [0; 32],
        [0; 32],
        None,
        None,
        0,
    );
}

#[tokio::test]
async fn fetch_duplicate_inflight_id_records_dlq_without_ack_or_state_advance() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let payload_hash: [u8; 32] = Sha256::digest(&delivery.payload).into();
    let message_id = delivery.message_id.clone();
    let block_hash = delivery.block_hash;
    let first = JetStreamFetchFrame::from_delivery(&delivery);
    let mut duplicate = JetStreamFetchFrame::from_delivery(&delivery);
    duplicate.stream_sequence = Some(17);
    duplicate.consumer_sequence = Some(8);
    duplicate.delivery_count = 2;
    let mut source = InMemoryFetchSource::new([first, duplicate]);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("duplicate in-flight id");
    assert_eq!(error.reason_code(), "core.jetstream_pending_limit");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked().contains(&message_id));
    assert!(source.terminated());
    assert_fetch_dead_letter(
        &dead_letter,
        "core.jetstream_pending_limit",
        "hl.v1.block.committed",
        &message_id,
        payload_hash,
        block_hash,
        Some(17),
        Some(8),
        2,
    );
}

#[tokio::test]
async fn file_dead_letter_sink_records_fetch_hash_mismatch_with_stream_sequence() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let poison_hash = hex::encode(Sha256::digest(&delivery.payload));
    let mut frame = JetStreamFetchFrame::from_delivery(&delivery);
    frame.stream_sequence = Some(99);
    if let Some(headers) = frame.headers.as_mut() {
        headers.insert("Alpha-Desk-Publication-SHA256", hex::encode([0xcd; 32]));
    }
    let path = root.path().join("dead-letter.jsonl");
    let mut dead_letter = FileDeadLetterSink::open(&path).expect("file dlq");
    let mut source = InMemoryFetchSource::new([frame]);

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("fetch hash mismatch");
    assert_eq!(error.reason_code(), "core.jetstream_hash_mismatch");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(source.terminated());
    drop(dead_letter);

    let encoded = fs::read_to_string(&path).expect("dlq file");
    let value: serde_json::Value = serde_json::from_str(encoded.trim()).expect("json");
    assert_eq!(value["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(value["reason_code"], "core.jetstream_hash_mismatch");
    assert_eq!(value["payload_sha256"], poison_hash);
    assert_eq!(value["stream_sequence"], 99);
    assert!(value.get("live_qualified").is_none());
    assert!(value.get("stage_2_qualified").is_none());
}

#[tokio::test]
async fn file_dead_letter_sink_records_fetch_pending_limit_with_stream_sequence() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let poison_hash = hex::encode(Sha256::digest(&delivery.payload));
    let message_id = delivery.message_id.clone();
    let block_hash = hex::encode(delivery.block_hash);
    let first = JetStreamFetchFrame::from_delivery(&delivery);
    let mut duplicate = JetStreamFetchFrame::from_delivery(&delivery);
    duplicate.stream_sequence = Some(21);
    duplicate.consumer_sequence = Some(4);
    duplicate.delivery_count = 6;
    let path = root.path().join("dead-letter.jsonl");
    let mut dead_letter = FileDeadLetterSink::open(&path).expect("file dlq");
    let mut source = InMemoryFetchSource::new([first, duplicate]);

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("duplicate in-flight id");
    assert_eq!(error.reason_code(), "core.jetstream_pending_limit");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked().contains(&message_id));
    assert!(source.terminated());
    drop(dead_letter);

    let encoded = fs::read_to_string(&path).expect("dlq file");
    let value: serde_json::Value = serde_json::from_str(encoded.trim()).expect("json");
    assert_eq!(value["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(value["reason_code"], "core.jetstream_pending_limit");
    assert_eq!(value["message_id"], message_id);
    assert_eq!(value["subject"], "hl.v1.block.committed");
    assert_eq!(value["payload_sha256"], poison_hash);
    assert_eq!(value["block_hash"], block_hash);
    assert_eq!(value["stream_sequence"], 21);
    assert_eq!(value["consumer_sequence"], 4);
    assert_eq!(value["retry_count"], 6);
    assert!(value.get("live_qualified").is_none());
    assert!(value.get("stage_2_qualified").is_none());
}

#[tokio::test]
async fn file_dead_letter_sink_records_fetch_transport_sentinel() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let path = root.path().join("dead-letter.jsonl");
    let mut dead_letter = FileDeadLetterSink::open(&path).expect("file dlq");
    let mut source = TransportFailSource::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(!source.acked);
    drop(dead_letter);

    let encoded = fs::read_to_string(&path).expect("dlq file");
    let value: serde_json::Value = serde_json::from_str(encoded.trim()).expect("json");
    assert_eq!(value["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(value["reason_code"], "core.jetstream_transport");
    assert_eq!(value["subject"], "hl.v1.fetch.transport");
    assert_eq!(value["message_id"], "transport");
    assert_eq!(value["payload_sha256"], hex::encode([0u8; 32]));
    assert_eq!(value["block_hash"], hex::encode([0u8; 32]));
    assert!(value.get("stream_sequence").is_none());
    assert!(value.get("consumer_sequence").is_none());
    assert_eq!(value["retry_count"], 0);
    assert!(value.get("live_qualified").is_none());
    assert!(value.get("stage_2_qualified").is_none());
}

#[tokio::test]
async fn file_dead_letter_sink_records_ack_transport_after_apply() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let block = empty_block(200, 1);
    let mut delivery =
        committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    delivery.stream_sequence = Some(31);
    delivery.consumer_sequence = Some(5);
    delivery.delivery_count = 1;
    let payload_hash = hex::encode(Sha256::digest(&delivery.payload));
    let message_id = delivery.message_id.clone();
    let block_hash = hex::encode(delivery.block_hash);
    let path = root.path().join("dead-letter.jsonl");
    let mut dead_letter = FileDeadLetterSink::open(&path).expect("file dlq");
    let mut source = AckFailSource::new([delivery.clone()]);

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("ack transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert_eq!(
        session
            .ledger()
            .checkpoint()
            .map(|checkpoint| checkpoint.block_height()),
        Some(BlockHeight::new(200))
    );
    assert!(!source.acked().contains(&message_id));
    drop(dead_letter);

    let encoded = fs::read_to_string(&path).expect("dlq file");
    let value: serde_json::Value = serde_json::from_str(encoded.trim()).expect("json");
    assert_eq!(value["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(value["reason_code"], "core.jetstream_transport");
    assert_eq!(value["subject"], "hl.v1.block.committed");
    assert_eq!(value["message_id"], message_id);
    assert_eq!(value["payload_sha256"], payload_hash);
    assert_eq!(value["block_hash"], block_hash);
    assert_eq!(value["stream_sequence"], 31);
    assert_eq!(value["consumer_sequence"], 5);
    assert_eq!(value["retry_count"], 1);
    assert!(value.get("live_qualified").is_none());
    assert!(value.get("stage_2_qualified").is_none());
}

#[tokio::test]
async fn file_dead_letter_sink_records_connect_transport_sentinel() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let path = root.path().join("dead-letter.jsonl");
    let mut dead_letter = FileDeadLetterSink::open(&path).expect("file dlq");

    let error = session
        .connect_source(
            || async { Err::<InMemoryCanonicalSource, _>(JetStreamReplayError::Transport) },
            &mut dead_letter,
        )
        .await
        .expect_err("connect transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    drop(dead_letter);

    let encoded = fs::read_to_string(&path).expect("dlq file");
    let value: serde_json::Value = serde_json::from_str(encoded.trim()).expect("json");
    assert_eq!(value["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(value["reason_code"], "core.jetstream_transport");
    assert_eq!(value["subject"], "hl.v1.connect.transport");
    assert_eq!(value["message_id"], "connect");
    assert_eq!(value["payload_sha256"], hex::encode([0u8; 32]));
    assert_eq!(value["block_hash"], hex::encode([0u8; 32]));
    assert!(value.get("stream_sequence").is_none());
    assert!(value.get("consumer_sequence").is_none());
    assert_eq!(value["retry_count"], 0);
    assert!(value.get("live_qualified").is_none());
    assert!(value.get("stage_2_qualified").is_none());
}

#[tokio::test]
async fn assembler_pending_block_cap_records_dlq_without_ack_or_state_advance() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let deliveries: Vec<_> = (200..265)
        .map(|height| {
            let block = trade_block(height);
            committed_event_delivery(&block.events()[0], &block, &archive_receipt(&block))
                .expect("event")
        })
        .collect();
    let poison = deliveries[64].clone();
    let payload_hash: [u8; 32] = Sha256::digest(&poison.payload).into();
    let poison_id = poison.message_id.clone();
    let mut source = InMemoryCanonicalSource::new(deliveries);
    let mut dead_letter = InMemoryDeadLetterSink::default();

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("pending-block cap");
    assert_eq!(error.reason_code(), "core.jetstream_pending_limit");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    assert!(session.ledger().checkpoint().is_none());
    assert!(source.acked().is_empty());
    assert_one_dead_letter(
        &dead_letter,
        "core.jetstream_pending_limit",
        &poison_id,
        payload_hash,
        poison.block_hash,
    );
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
        .consume_available(
            &mut InMemoryCanonicalSource::new([]),
            &mut InMemoryDeadLetterSink::default(),
        )
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

#[test]
fn file_dead_letter_sink_open_does_not_leave_empty_jsonl() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    let sink = FileDeadLetterSink::open(&path).expect("file dlq");
    assert_dead_letter_file_absent_or_typed_sentinel(&path);
    drop(sink);
    assert_dead_letter_file_absent_or_typed_sentinel(&path);

    fs::write(&path, []).expect("seed empty leftover");
    let leftover = FileDeadLetterSink::open(&path).expect("reopen leftover");
    assert_dead_letter_file_absent_or_typed_sentinel(&path);
    drop(leftover);
    assert_dead_letter_file_absent_or_typed_sentinel(&path);
}

#[test]
fn file_dead_letter_sink_open_rejects_truncated_jsonl() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    {
        let mut sink = FileDeadLetterSink::open(&path).expect("file dlq");
        sink.persist(&sample_dead_letter("complete"))
            .expect("first record");
        sink.persist(&sample_dead_letter("partial"))
            .expect("second record");
    }
    let mut leftover = fs::read(&path).expect("complete jsonl");
    leftover.truncate(leftover.len().saturating_sub(24));
    assert!(
        !leftover.is_empty(),
        "truncated leftover must remain non-empty"
    );
    fs::write(&path, &leftover).expect("truncate last line");

    let error = FileDeadLetterSink::open(&path).expect_err("truncated jsonl");
    assert_eq!(error, DeadLetterError::Corrupt);
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    assert_eq!(fs::read(&path).expect("left in place"), leftover);
}

#[test]
fn file_dead_letter_sink_open_rejects_non_json_bytes() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    let leftover = b"\x00\xff not-jsonl\n{";
    fs::write(&path, leftover).expect("seed garbage");

    let error = FileDeadLetterSink::open(&path).expect_err("non-json leftover");
    assert_eq!(error, DeadLetterError::Corrupt);
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    assert_eq!(fs::read(&path).expect("left in place"), leftover);
}

#[test]
fn file_dead_letter_sink_open_rejects_json_that_is_not_a_record() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    let leftover = b"{\"foo\":1}\n";
    fs::write(&path, leftover).expect("seed garbage json");

    let error = FileDeadLetterSink::open(&path).expect_err("non-record json");
    assert_eq!(error, DeadLetterError::Corrupt);
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    assert_eq!(fs::read(&path).expect("left in place"), leftover);
}

#[test]
fn file_dead_letter_sink_open_rejects_oversized_jsonl() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    // One byte over `MAX_DEAD_LETTER_FILE_BYTES` (256 * 4 KiB records).
    // Valid ExistingRecord jsonl, not `vec![b'x'; 1 MiB + 1]`: garbage of that
    // size would still `Corrupt` via decode if the leftover cap were removed.
    let leftover = common::valid_dead_letter_jsonl_of_len(1_048_576 + 1);
    fs::write(&path, &leftover).expect("seed oversized leftover");

    let error = FileDeadLetterSink::open(&path).expect_err("oversized leftover");
    assert_eq!(error, DeadLetterError::Corrupt);
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    assert_eq!(fs::read(&path).expect("left in place"), leftover);
}

#[test]
fn file_dead_letter_sink_open_reopens_valid_jsonl_at_file_cap() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    let leftover = common::valid_dead_letter_jsonl_of_len(1_048_576);
    fs::write(&path, &leftover).expect("seed at-cap leftover");

    // Leftover-open at the cap still succeeds; persist must not grow past it.

    let mut sink = FileDeadLetterSink::open(&path).expect("reopen at cap");
    let error = sink
        .persist(&sample_dead_letter("after-cap"))
        .expect_err("persist past file cap");
    assert_eq!(error, DeadLetterError::Corrupt);
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    drop(sink);
    assert_eq!(fs::read(&path).expect("unchanged at cap"), leftover);
}

#[test]
fn file_dead_letter_sink_persist_refuses_record_that_would_exceed_file_cap() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    let record = sample_dead_letter("would-exceed");
    let record_len = persist_probe_len(root.path(), "would-exceed");
    let leftover = common::valid_dead_letter_jsonl_of_len(1_048_576 - record_len + 1);
    fs::write(&path, &leftover).expect("seed leftover that cannot fit one more record");
    let previous_len = leftover.len();

    let mut sink = FileDeadLetterSink::open(&path).expect("reopen under leftover-open cap");
    let error = sink
        .persist(&record)
        .expect_err("persist that would grow past cap");
    assert_eq!(error, DeadLetterError::Corrupt);
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    drop(sink);
    assert_eq!(fs::read(&path).expect("previous valid size"), leftover);
    assert_eq!(
        usize::try_from(fs::metadata(&path).expect("metadata").len()).expect("len"),
        previous_len
    );
}

#[test]
fn file_dead_letter_sink_persist_writes_up_to_file_cap() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    let record = sample_dead_letter("under-cap");
    let record_len = persist_probe_len(root.path(), "under-cap");
    let leftover = common::valid_dead_letter_jsonl_of_len(1_048_576 - record_len);
    fs::write(&path, &leftover).expect("seed leftover that fits one more record");

    let mut sink = FileDeadLetterSink::open(&path).expect("reopen under cap");
    sink.persist(&record).expect("persist that fills the cap");
    assert_eq!(
        usize::try_from(fs::metadata(&path).expect("at cap").len()).expect("len"),
        1_048_576
    );
    let error = sink
        .persist(&sample_dead_letter("past-cap"))
        .expect_err("persist past cap");
    assert_eq!(error, DeadLetterError::Corrupt);
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    drop(sink);
    assert_eq!(
        usize::try_from(fs::metadata(&path).expect("still at cap").len()).expect("len"),
        1_048_576
    );
}

#[test]
fn file_dead_letter_sink_open_keeps_valid_records_readable() {
    let root = private_root();
    let path = root.path().join("dead-letter.jsonl");
    {
        let mut sink = FileDeadLetterSink::open(&path).expect("file dlq");
        sink.persist(&sample_dead_letter("first"))
            .expect("first record");
    }
    {
        let mut sink = FileDeadLetterSink::open(&path).expect("reopen valid jsonl");
        sink.persist(&sample_dead_letter("second"))
            .expect("second record");
    }
    let encoded = fs::read_to_string(&path).expect("readable jsonl");
    let lines: Vec<_> = encoded.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(lines.len(), 2);
    let first: serde_json::Value = serde_json::from_str(lines[0]).expect("first json");
    let second: serde_json::Value = serde_json::from_str(lines[1]).expect("second json");
    assert_eq!(first["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(first["message_id"], "first");
    assert_eq!(second["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(second["message_id"], "second");
}

#[tokio::test]
async fn file_dead_letter_sink_persists_poison_without_applying_state() {
    let root = private_root();
    let store =
        SyncedWriteBatchStore::open(root.path().join("state"), StateImageLimits::production())
            .expect("store");
    let mut session = open_session(store);
    let before = session.ledger().state_image().canonical_bytes();
    let block = empty_block(200, 1);
    let receipt = archive_receipt(&block);
    let mut delivery = committed_block_delivery(&block, &receipt).expect("delivery");
    let last = delivery.payload.len() - 1;
    delivery.payload[last] ^= 0xff;
    let poison_hash = hex::encode(Sha256::digest(&delivery.payload));
    let path = root.path().join("dead-letter.jsonl");
    let mut dead_letter = FileDeadLetterSink::open(&path).expect("file dlq");
    let mut source = InMemoryCanonicalSource::new([delivery]);

    let error = session
        .consume_available(&mut source, &mut dead_letter)
        .await
        .expect_err("tampered marker");
    assert_eq!(error.reason_code(), "core.jetstream_hash_mismatch");
    assert_eq!(session.ledger().state_image().canonical_bytes(), before);
    drop(dead_letter);

    let encoded = fs::read_to_string(&path).expect("dlq file");
    let value: serde_json::Value = serde_json::from_str(encoded.trim()).expect("json");
    assert_eq!(value["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(value["reason_code"], "core.jetstream_hash_mismatch");
    assert_eq!(value["payload_sha256"], poison_hash);
    assert!(value.get("stream_sequence").is_none());
    assert!(value.get("live_qualified").is_none());
    assert!(value.get("stage_2_qualified").is_none());
}

fn assert_dead_letter_file_absent_or_typed_sentinel(path: &std::path::Path) {
    match fs::metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("dead-letter metadata: {error}"),
        Ok(metadata) => {
            assert!(
                metadata.len() > 0,
                "empty dead-letter.jsonl must not persist without a typed sentinel"
            );
            let encoded = fs::read_to_string(path).expect("dlq file");
            let first = encoded
                .lines()
                .find(|line| !line.is_empty())
                .expect("non-empty jsonl");
            let record: serde_json::Value = serde_json::from_str(first).expect("jsonl");
            assert_eq!(record["schema_version"], DEAD_LETTER_SCHEMA_V1);
            let reason = record["reason_code"].as_str().expect("typed reason_code");
            assert!(!reason.is_empty());
            assert!(
                record["subject"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
            assert!(
                record["message_id"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
            );
        }
    }
}

fn assert_one_dead_letter(
    sink: &InMemoryDeadLetterSink,
    reason_code: &str,
    message_id: &str,
    payload_sha256: [u8; 32],
    block_hash: [u8; 32],
) {
    assert_eq!(sink.records().len(), 1);
    let record = &sink.records()[0];
    assert_eq!(record.reason_code(), reason_code);
    assert_eq!(record.message_id(), message_id);
    assert_eq!(record.payload_sha256(), payload_sha256);
    assert_eq!(record.block_hash(), block_hash);
    assert!(record.stream_sequence().is_none());
    assert!(record.consumer_sequence().is_none());
    assert_eq!(
        record.consumer(),
        JetStreamReplayConfig::default_durable_name()
    );
    assert!(record.failed_at_unix_micros() >= 0);
}

#[allow(clippy::too_many_arguments)]
fn assert_fetch_dead_letter(
    sink: &InMemoryDeadLetterSink,
    reason_code: &str,
    subject: &str,
    message_id: &str,
    payload_sha256: [u8; 32],
    block_hash: [u8; 32],
    stream_sequence: Option<u64>,
    consumer_sequence: Option<u64>,
    retry_count: u64,
) {
    assert_eq!(sink.records().len(), 1);
    let record = &sink.records()[0];
    assert_eq!(record.reason_code(), reason_code);
    assert_eq!(record.subject(), subject);
    assert_eq!(record.message_id(), message_id);
    assert_eq!(record.payload_sha256(), payload_sha256);
    assert_eq!(record.block_hash(), block_hash);
    assert_eq!(record.stream_sequence(), stream_sequence);
    assert_eq!(record.consumer_sequence(), consumer_sequence);
    assert_eq!(record.retry_count(), retry_count);
    assert_eq!(
        record.consumer(),
        JetStreamReplayConfig::default_durable_name()
    );
    assert!(record.failed_at_unix_micros() >= 0);
}

struct AckFailSource {
    inner: InMemoryCanonicalSource,
}

impl AckFailSource {
    fn new(deliveries: impl IntoIterator<Item = hl_core::CanonicalDelivery>) -> Self {
        Self {
            inner: InMemoryCanonicalSource::new(deliveries),
        }
    }

    fn acked(&self) -> &std::collections::BTreeSet<String> {
        self.inner.acked()
    }
}

impl CanonicalPullSource for AckFailSource {
    async fn fetch(
        &mut self,
        max_messages: usize,
    ) -> Result<Vec<hl_core::CanonicalDelivery>, JetStreamReplayError> {
        self.inner.fetch(max_messages).await
    }

    async fn ack(&mut self, _message_ids: &[String]) -> Result<(), JetStreamReplayError> {
        Err(JetStreamReplayError::Transport)
    }
}

#[derive(Default)]
struct TransportFailSource {
    acked: bool,
}

impl CanonicalPullSource for TransportFailSource {
    async fn fetch(
        &mut self,
        _max_messages: usize,
    ) -> Result<Vec<hl_core::CanonicalDelivery>, JetStreamReplayError> {
        Err(JetStreamReplayError::Transport)
    }

    async fn ack(&mut self, _message_ids: &[String]) -> Result<(), JetStreamReplayError> {
        self.acked = true;
        Ok(())
    }
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

fn sample_dead_letter(message_id: &str) -> DeadLetterRecord {
    DeadLetterRecord::try_new(
        "core.jetstream_transport",
        "hl.v1.connect.transport",
        message_id,
        None,
        None,
        [0x11; 32],
        [0x22; 32],
        JetStreamReplayConfig::default_durable_name(),
        0,
        1,
    )
    .expect("sample dead-letter record")
}

fn persist_probe_len(parent: &std::path::Path, message_id: &str) -> usize {
    let path = parent.join(format!("probe-{message_id}.jsonl"));
    let mut sink = FileDeadLetterSink::open(&path).expect("probe sink");
    sink.persist(&sample_dead_letter(message_id))
        .expect("probe persist");
    drop(sink);
    usize::try_from(fs::metadata(&path).expect("probe metadata").len()).expect("probe len")
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
