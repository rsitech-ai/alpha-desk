use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, ManifestId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};
use hl_capture::bus::CommittedPublicationBatch;
use sha2::{Digest as _, Sha256};
use storage_ports::ArchiveReceipt;

/// Capture-local empty-primary digest. Height and time are distinct so swapping
/// those 8-byte fields changes the hash. This is no longer the jetstream-marker
/// crate's empty-primary vector (`height == time == 200`); that crate is not on
/// this stack.
const FROZEN_EMPTY_PRIMARY_SHA256: &str =
    "9f76872b4e7cc655c29c1315a4a55dea7e53d30cde5d300999bce44ba99eb34d";

/// Independent confirmation (wire tag `3`) plus one event row. Changing the
/// Independent tag or event-row encoding must fail even if a comment still
/// contains `CommittedIndependent => 3`.
const FROZEN_INDEPENDENT_TRADE_SHA256: &str =
    "e9af3a1e512001b2827bb7a66de6684f1de2c2db47352bf3ce3700cd1040d132";

const BLOCK_MARKER_SCHEMA_V1: &str = "hyperliquid-alpha-desk/block-publication/v1";

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
fn empty_primary_marker_still_emits_the_frozen_v1_layout() {
    let block = empty_primary_block();
    assert_ne!(
        i64::try_from(block.block_height().get()).expect("height fits i64"),
        block.block_time().unix_micros(),
        "empty-primary height and time must stay distinct so a field swap breaks the digest"
    );
    let receipt = archive_receipt(&block);
    let batch = CommittedPublicationBatch::try_new(&block, &receipt).expect("encode marker");
    let payload = batch.block().payload();

    assert_eq!(batch.block().schema_version(), BLOCK_MARKER_SCHEMA_V1);
    assert!(payload.starts_with(&counted_bytes(BLOCK_MARKER_SCHEMA_V1.as_bytes())));
    assert_eq!(wire_confirmation_tag(payload), 2);
    assert_eq!(
        hex::encode(Sha256::digest(payload)),
        FROZEN_EMPTY_PRIMARY_SHA256
    );
    assert_eq!(
        hex::encode(batch.block().publication_sha256()),
        FROZEN_EMPTY_PRIMARY_SHA256
    );
}

#[test]
fn independent_event_marker_still_emits_the_frozen_v1_layout() {
    let block = independent_trade_block();
    assert_eq!(
        block.confirmation_class(),
        ConfirmationClass::CommittedIndependent
    );
    assert_eq!(block.events().len(), 1);
    let receipt = archive_receipt(&block);
    let batch = CommittedPublicationBatch::try_new(&block, &receipt).expect("encode marker");
    let payload = batch.block().payload();

    assert_eq!(batch.block().schema_version(), BLOCK_MARKER_SCHEMA_V1);
    assert!(payload.starts_with(&counted_bytes(BLOCK_MARKER_SCHEMA_V1.as_bytes())));
    assert_eq!(wire_confirmation_tag(payload), 3);
    assert_eq!(batch.events().len(), 1);
    assert_ne!(
        hex::encode(Sha256::digest(payload)),
        FROZEN_EMPTY_PRIMARY_SHA256
    );
    assert_eq!(
        hex::encode(Sha256::digest(payload)),
        FROZEN_INDEPENDENT_TRADE_SHA256
    );
    assert_eq!(
        hex::encode(batch.block().publication_sha256()),
        FROZEN_INDEPENDENT_TRADE_SHA256
    );
}

#[test]
fn private_encoder_source_still_matches_the_frozen_field_order() {
    let source = include_str!("../src/bus/mod.rs");
    let encoder = source
        .split("fn encode_block_marker(")
        .nth(1)
        .expect("hl-capture still has a private encode_block_marker copy");
    assert_tokens_in_order(encoder, &FROZEN_LAYOUT_TOKENS);
}

fn counted_bytes(value: &[u8]) -> Vec<u8> {
    let mut prefix = u64::try_from(value.len())
        .expect("schema length")
        .to_be_bytes()
        .to_vec();
    prefix.extend_from_slice(value);
    prefix
}

fn remaining_after_counted(payload: &[u8]) -> &[u8] {
    let (len_bytes, rest) = payload.split_at(8);
    let len = usize::try_from(u64::from_be_bytes(
        len_bytes.try_into().expect("counted length"),
    ))
    .expect("counted length fits usize");
    rest.get(len..).expect("truncated counted bytes")
}

fn wire_confirmation_tag(payload: &[u8]) -> u8 {
    let after_schema = remaining_after_counted(payload);
    let after_chain = remaining_after_counted(after_schema);
    after_chain
        .get(16)
        .copied()
        .expect("confirmation tag after height and time")
}

fn assert_tokens_in_order(source: &str, tokens: &[&str]) {
    let mut rest = source;
    for token in tokens {
        let Some(index) = rest.find(token) else {
            panic!("capture marker encoder drifted: missing {token}");
        };
        rest = &rest[index + token.len()..];
    }
}

fn empty_primary_block() -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(200),
        ProtocolTime::from_unix_micros(201).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(SourceId::new("jetstream-replay").expect("source"), [7; 32])]),
    )
    .expect("empty primary block")
}

fn independent_trade_block() -> BlockEnvelope {
    let height = 201_u64;
    let block_time = ProtocolTime::from_unix_micros(202).expect("time");
    let source_id = SourceId::new("jetstream-replay").expect("source");
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: Vec::new(),
        source_evidence: vec![
            SourceEvidence::try_new(
                source_id.clone(),
                "node-v1",
                format!("block-{height}:0"),
                [0x44; 32],
            )
            .expect("source evidence"),
        ],
        confirmation_class: ConfirmationClass::CommittedIndependent,
        observed_at: KnownTime::from_unix_micros(202).expect("observed"),
        ingested_at: KnownTime::from_unix_micros(203).expect("ingested"),
        canonicalized_at: KnownTime::from_unix_micros(204).expect("canonicalized"),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            1,
        )),
    })
    .expect("independent trade event");

    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        block_time,
        ConfirmationClass::CommittedIndependent,
        vec![event],
        BTreeMap::from([(source_id, [0x44; 32])]),
    )
    .expect("independent trade block")
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
