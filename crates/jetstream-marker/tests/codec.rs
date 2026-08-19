use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, SourceEvidence, TradeMatched,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ManifestId, MarketId, Price, ProtocolTime, Quantity,
    SourceId, TransactionId,
};
use jetstream_marker::{
    BLOCK_MARKER_SCHEMA_V1, MarkerCodecError, decode_committed_block_marker,
    encode_committed_block_marker,
};
use sha2::{Digest as _, Sha256};
use storage_ports::ArchiveReceipt;

const FROZEN_EMPTY_PRIMARY_SHA256: &str =
    "3d6f5627cfe8713c5538f1721d5a63b997f3d32f93078a83eb5bdcff70d23e71";

const FROZEN_LAYOUT_TOKENS: [&str; 16] = [
    "BLOCK_MARKER_SCHEMA_V1",
    "chain_id()",
    "block_height().get().to_be_bytes()",
    "block_time().unix_micros().to_be_bytes()",
    "CommittedPrimary => 2",
    "CommittedIndependent => 3",
    "canonical_block_hash()",
    "receipt_id()",
    "manifest_id()",
    "manifest_sha256()",
    "schema_fingerprint()",
    "source_block_hashes()",
    "event_id()",
    "event_kind().as_wire_name()",
    "payload_hash()",
    "Sha256::digest",
];

#[test]
fn empty_committed_marker_roundtrips_identically() {
    let block = empty_block(200, 7, ConfirmationClass::CommittedPrimary);
    let receipt = archive_receipt(&block);
    let encoded = encode_committed_block_marker(&block, &receipt).expect("encode");
    let again = encode_committed_block_marker(&block, &receipt).expect("repeat encode");
    assert_eq!(encoded, again);

    let decoded = decode_committed_block_marker(&encoded).expect("decode");
    assert_eq!(decoded.chain_id, block.chain_id().clone());
    assert_eq!(decoded.block_height, block.block_height());
    assert_eq!(decoded.block_time, block.block_time());
    assert_eq!(
        decoded.confirmation_class,
        ConfirmationClass::CommittedPrimary
    );
    assert_eq!(decoded.canonical_block_hash, block.canonical_block_hash());
    assert_eq!(decoded.archive_receipt_id, receipt.receipt_id());
    assert_eq!(decoded.archive_manifest_id, receipt.manifest_id().as_str());
    assert_eq!(decoded.archive_manifest_sha256, receipt.manifest_sha256());
    assert_eq!(decoded.schema_fingerprint, receipt.schema_fingerprint());
    assert_eq!(decoded.source_block_hashes, *block.source_block_hashes());
    assert!(decoded.events.is_empty());
}

#[test]
fn independent_confirmation_roundtrips_with_event_rows() {
    let block = trade_block(201, ConfirmationClass::CommittedIndependent);
    let receipt = archive_receipt(&block);
    let encoded = encode_committed_block_marker(&block, &receipt).expect("encode");
    let decoded = decode_committed_block_marker(&encoded).expect("decode");
    let event = &block.events()[0];
    let envelope = event.encode_to_vec().expect("envelope");
    let envelope_sha256: [u8; 32] = Sha256::digest(&envelope).into();

    assert_eq!(
        decoded.confirmation_class,
        ConfirmationClass::CommittedIndependent
    );
    assert_eq!(decoded.events.len(), 1);
    assert_eq!(decoded.events[0].event_id, event.event_id().as_str());
    assert_eq!(decoded.events[0].event_kind, EventKind::TradeMatched);
    assert_eq!(decoded.events[0].payload_hash, event.payload_hash());
    assert_eq!(decoded.events[0].envelope_sha256, envelope_sha256);
    assert_eq!(
        decode_committed_block_marker(&encoded).expect("second decode"),
        decoded
    );
}

#[test]
fn encoded_marker_starts_with_the_frozen_schema_frame() {
    let block = empty_block(200, 7, ConfirmationClass::CommittedPrimary);
    let encoded = encode_committed_block_marker(&block, &archive_receipt(&block)).expect("encode");
    let schema = BLOCK_MARKER_SCHEMA_V1.as_bytes();
    let mut prefix = u64::try_from(schema.len())
        .expect("schema length")
        .to_be_bytes()
        .to_vec();
    prefix.extend_from_slice(schema);
    assert!(encoded.starts_with(&prefix));
    assert_eq!(
        hex::encode(Sha256::digest(&encoded)),
        FROZEN_EMPTY_PRIMARY_SHA256
    );
}

#[test]
fn decode_rejects_truncated_schema_mismatch_trailing_bytes_and_non_committed_tags() {
    let block = empty_block(200, 7, ConfirmationClass::CommittedPrimary);
    let encoded = encode_committed_block_marker(&block, &archive_receipt(&block)).expect("encode");

    assert_eq!(
        decode_committed_block_marker(&encoded[..encoded.len() - 1]),
        Err(MarkerCodecError::Malformed)
    );

    let mut trailing = encoded.clone();
    trailing.push(0);
    assert_eq!(
        decode_committed_block_marker(&trailing),
        Err(MarkerCodecError::Malformed)
    );

    let mut wrong_schema = encoded.clone();
    wrong_schema[15] ^= 0x01;
    assert_eq!(
        decode_committed_block_marker(&wrong_schema),
        Err(MarkerCodecError::UnsupportedSchema)
    );

    let schema = BLOCK_MARKER_SCHEMA_V1.as_bytes();
    let tag_at = 8 + schema.len() + 8 + b"mainnet".len() + 8 + 8;
    assert_eq!(encoded[tag_at], 2);
    let mut not_committed = encoded.clone();
    not_committed[tag_at] = 1;
    assert_eq!(
        decode_committed_block_marker(&not_committed),
        Err(MarkerCodecError::NotCommitted)
    );
}

#[test]
fn encode_rejects_receipt_mismatch_and_provisional_blocks() {
    let block = empty_block(200, 7, ConfirmationClass::CommittedPrimary);
    let other = empty_block(201, 8, ConfirmationClass::CommittedPrimary);
    assert_eq!(
        encode_committed_block_marker(&block, &archive_receipt(&other)),
        Err(MarkerCodecError::Malformed)
    );

    let provisional = empty_block(200, 7, ConfirmationClass::ProvisionalSource);
    assert_eq!(
        encode_committed_block_marker(&provisional, &archive_receipt(&provisional)),
        Err(MarkerCodecError::NotCommitted)
    );
}

#[test]
fn shared_codec_bytes_match_the_capture_copy() {
    let block = trade_block(200, ConfirmationClass::CommittedPrimary);
    let receipt = archive_receipt(&block);
    let shared = encode_committed_block_marker(&block, &receipt).expect("shared");
    let capture = capture_encode_block_marker(&block, &receipt).expect("capture copy");
    assert_eq!(shared, capture);
    assert_eq!(
        decode_committed_block_marker(&capture).expect("decode capture copy"),
        decode_committed_block_marker(&shared).expect("decode shared")
    );
}

#[test]
fn capture_copy_still_encodes_the_frozen_layout() {
    let capture = include_str!("../../../services/hl-capture/src/bus/mod.rs");
    let encoder = capture
        .split("fn encode_block_marker(")
        .nth(1)
        .expect("hl-capture still has a private encode_block_marker copy");
    assert_tokens_in_order(encoder, &FROZEN_LAYOUT_TOKENS);

    let shared = include_str!("../src/lib.rs");
    let shared_encoder = shared
        .split("pub fn encode_committed_block_marker(")
        .nth(1)
        .expect("shared crate encoder");
    assert_tokens_in_order(shared_encoder, &FROZEN_LAYOUT_TOKENS);
}

fn assert_tokens_in_order(source: &str, tokens: &[&str]) {
    let mut rest = source;
    for token in tokens {
        let Some(index) = rest.find(token) else {
            panic!("marker codec drifted: missing {token}");
        };
        rest = &rest[index + token.len()..];
    }
}

#[derive(Debug, PartialEq, Eq)]
enum CaptureEncodeError {
    NotCommitted,
    CanonicalCodec,
    CountOverflow,
}

fn capture_encode_block_marker(
    block: &BlockEnvelope,
    receipt: &ArchiveReceipt,
) -> Result<Vec<u8>, CaptureEncodeError> {
    let mut output = Vec::new();
    capture_push_bytes(&mut output, BLOCK_MARKER_SCHEMA_V1.as_bytes())?;
    capture_push_bytes(&mut output, block.chain_id().as_str().as_bytes())?;
    output.extend_from_slice(&block.block_height().get().to_be_bytes());
    output.extend_from_slice(&block.block_time().unix_micros().to_be_bytes());
    output.push(match block.confirmation_class() {
        ConfirmationClass::CommittedPrimary => 2,
        ConfirmationClass::CommittedIndependent => 3,
        _ => return Err(CaptureEncodeError::NotCommitted),
    });
    output.extend_from_slice(&block.canonical_block_hash());
    capture_push_bytes(&mut output, receipt.receipt_id().as_bytes())?;
    capture_push_bytes(&mut output, receipt.manifest_id().as_str().as_bytes())?;
    output.extend_from_slice(&receipt.manifest_sha256());
    output.extend_from_slice(&receipt.schema_fingerprint());

    capture_push_count(&mut output, block.source_block_hashes().len())?;
    for (source_id, source_hash) in block.source_block_hashes() {
        capture_push_bytes(&mut output, source_id.as_str().as_bytes())?;
        output.extend_from_slice(source_hash);
    }

    capture_push_count(&mut output, block.events().len())?;
    for event in block.events() {
        capture_push_bytes(&mut output, event.event_id().as_str().as_bytes())?;
        capture_push_bytes(&mut output, event.event_kind().as_wire_name().as_bytes())?;
        output.extend_from_slice(&event.payload_hash());
        let encoded = event
            .encode_to_vec()
            .map_err(|_| CaptureEncodeError::CanonicalCodec)?;
        output.extend_from_slice(&Sha256::digest(encoded));
    }
    Ok(output)
}

fn capture_push_count(output: &mut Vec<u8>, count: usize) -> Result<(), CaptureEncodeError> {
    let count = u64::try_from(count).map_err(|_| CaptureEncodeError::CountOverflow)?;
    output.extend_from_slice(&count.to_be_bytes());
    Ok(())
}

fn capture_push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), CaptureEncodeError> {
    capture_push_count(output, value.len())?;
    output.extend_from_slice(value);
    Ok(())
}

fn empty_block(height: u64, seed: u8, confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        confirmation,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("jetstream-replay").expect("source"),
            [seed; 32],
        )]),
    )
    .expect("block")
}

fn trade_block(height: u64, confirmation: ConfirmationClass) -> BlockEnvelope {
    let event = trade_event(height, confirmation);
    BlockEnvelope::try_new(
        event.chain_id().clone(),
        event.block_height(),
        event.block_time(),
        confirmation,
        vec![event],
        BTreeMap::from([(
            SourceId::new("jetstream-replay").expect("source"),
            [0x44; 32],
        )]),
    )
    .expect("action-bearing block")
}

fn trade_event(height: u64, confirmation: ConfirmationClass) -> CanonicalEventEnvelope {
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
        confirmation_class: confirmation,
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
