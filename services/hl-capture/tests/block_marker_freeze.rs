use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, KnownTime, ManifestId, ProtocolTime, SourceId};
use hl_capture::bus::CommittedPublicationBatch;
use sha2::{Digest as _, Sha256};
use storage_ports::ArchiveReceipt;

/// Same empty-primary digest as hl-core's frozen `jetstream-marker` copy.
const FROZEN_EMPTY_PRIMARY_SHA256: &str =
    "3d6f5627cfe8713c5538f1721d5a63b997f3d32f93078a83eb5bdcff70d23e71";

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
    let receipt = archive_receipt(&block);
    let batch = CommittedPublicationBatch::try_new(&block, &receipt).expect("encode marker");
    let payload = batch.block().payload();

    assert_eq!(batch.block().schema_version(), BLOCK_MARKER_SCHEMA_V1);
    assert!(payload.starts_with(&counted_bytes(BLOCK_MARKER_SCHEMA_V1.as_bytes())));
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
        ProtocolTime::from_unix_micros(200).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(SourceId::new("jetstream-replay").expect("source"), [7; 32])]),
    )
    .expect("empty primary block")
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
