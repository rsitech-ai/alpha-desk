use std::collections::BTreeMap;

use api_contracts::WireCanonicalEventEnvelope;
use canonical_events::{
    BlockEnvelope, BlockError, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass,
    EventPayload, SourceEvidence, TradeMatched,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}

fn source(source_id: &str, content_byte: u8) -> SourceEvidence {
    SourceEvidence::try_new(
        SourceId::new(source_id).expect("source"),
        "node-v1",
        "session-a:42",
        [content_byte; 32],
    )
    .expect("source evidence")
}

#[allow(clippy::too_many_arguments)]
fn event(
    chain: &str,
    height: u64,
    block_time: i64,
    transaction_id: &str,
    transaction_index: u32,
    event_index: u32,
    source_id: &str,
    confirmation_class: ConfirmationClass,
    lifecycle_offset: i64,
    payload_seed: u64,
) -> CanonicalEventEnvelope {
    scoped_event(
        chain,
        height,
        block_time,
        transaction_id,
        transaction_index,
        event_index,
        source_id,
        confirmation_class,
        lifecycle_offset,
        payload_seed,
        Vec::new(),
        Vec::new(),
    )
}

#[allow(clippy::too_many_arguments)]
fn scoped_event(
    chain: &str,
    height: u64,
    block_time: i64,
    transaction_id: &str,
    transaction_index: u32,
    event_index: u32,
    source_id: &str,
    confirmation_class: ConfirmationClass,
    lifecycle_offset: i64,
    payload_seed: u64,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
) -> CanonicalEventEnvelope {
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new(chain).expect("chain"),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(block_time).expect("block time"),
        transaction_id: TransactionId::new(transaction_id).expect("transaction"),
        transaction_index,
        canonical_event_index: event_index,
        market_ids,
        account_ids,
        source_evidence: vec![source(source_id, payload_seed as u8)],
        confirmation_class,
        observed_at: known(2_000 + lifecycle_offset),
        ingested_at: known(3_000 + lifecycle_offset),
        canonicalized_at: known(4_000 + lifecycle_offset),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            payload_seed,
        )),
    })
    .expect("event")
}

fn source_hashes(source_id: &str, byte: u8) -> BTreeMap<SourceId, [u8; 32]> {
    BTreeMap::from([(SourceId::new(source_id).expect("source"), [byte; 32])])
}

fn block(
    confirmation_class: ConfirmationClass,
    events: Vec<CanonicalEventEnvelope>,
    source_id: &str,
) -> Result<BlockEnvelope, BlockError> {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(42),
        ProtocolTime::from_unix_micros(1_000).expect("block time"),
        confirmation_class,
        events,
        source_hashes(source_id, 0x55),
    )
}

#[test]
fn empty_committed_block_is_valid_when_source_evidence_exists() {
    let block = block(ConfirmationClass::CommittedPrimary, Vec::new(), "primary")
        .expect("empty committed block");

    assert!(block.events().is_empty());
    assert_eq!(block.block_height(), BlockHeight::new(42));
    assert_eq!(block.source_block_hashes().len(), 1);
    assert_eq!(
        block.canonical_block_hash(),
        [
            0xef, 0x6c, 0xc6, 0x2f, 0x51, 0x22, 0xff, 0x79, 0x2f, 0x8e, 0xc4, 0x1f, 0x7d, 0x2d,
            0xc3, 0xee, 0xaf, 0x6e, 0xbb, 0x32, 0x3e, 0xcc, 0x67, 0x1a, 0x55, 0x24, 0x88, 0x91,
            0x80, 0x22, 0xe9, 0x1a,
        ]
    );
}

#[test]
fn canonical_block_hash_excludes_source_confirmation_and_lifecycle_metadata() {
    let primary = block(
        ConfirmationClass::CommittedPrimary,
        vec![event(
            "mainnet",
            42,
            1_000,
            "tx-7",
            3,
            0,
            "primary",
            ConfirmationClass::CommittedPrimary,
            0,
            7,
        )],
        "primary",
    )
    .expect("primary block");
    let independent = block(
        ConfirmationClass::CommittedIndependent,
        vec![event(
            "mainnet",
            42,
            1_000,
            "tx-7",
            3,
            0,
            "secondary",
            ConfirmationClass::CommittedIndependent,
            500,
            7,
        )],
        "secondary",
    )
    .expect("independent block");

    assert_eq!(
        primary.canonical_block_hash(),
        independent.canonical_block_hash()
    );
    assert_ne!(
        primary.source_block_hashes(),
        independent.source_block_hashes()
    );
}

#[test]
fn payload_divergence_changes_block_hash_without_changing_event_identity() {
    let left_event = event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        0,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        7,
    );
    let right_event = event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        0,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        8,
    );
    assert_eq!(left_event.event_id(), right_event.event_id());

    let left = block(
        ConfirmationClass::CommittedPrimary,
        vec![left_event],
        "primary",
    )
    .expect("left");
    let right = block(
        ConfirmationClass::CommittedPrimary,
        vec![right_event],
        "primary",
    )
    .expect("right");

    assert_ne!(left.canonical_block_hash(), right.canonical_block_hash());
}

#[test]
fn routing_metadata_divergence_changes_block_hash_without_changing_event_identity() {
    let left_event = scoped_event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        0,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        7,
        vec![MarketId::new("BTC").expect("market")],
        vec![Address::from_bytes([0x11; 20])],
    );
    let right_event = scoped_event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        0,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        7,
        vec![MarketId::new("ETH").expect("market")],
        vec![Address::from_bytes([0x22; 20])],
    );
    assert_eq!(left_event.event_id(), right_event.event_id());
    assert_eq!(left_event.payload_hash(), right_event.payload_hash());

    let left = block(
        ConfirmationClass::CommittedPrimary,
        vec![left_event],
        "primary",
    )
    .expect("left");
    let right = block(
        ConfirmationClass::CommittedPrimary,
        vec![right_event],
        "primary",
    )
    .expect("right");

    assert_ne!(left.canonical_block_hash(), right.canonical_block_hash());
}

#[test]
fn block_rejects_missing_source_hashes_and_mixed_boundaries() {
    let valid_event = event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        0,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        7,
    );
    assert!(matches!(
        BlockEnvelope::try_new(
            ChainId::new("mainnet").expect("chain"),
            BlockHeight::new(42),
            ProtocolTime::from_unix_micros(1_000).expect("time"),
            ConfirmationClass::CommittedPrimary,
            vec![valid_event],
            BTreeMap::new(),
        ),
        Err(BlockError::MissingSourceBlockHashes)
    ));

    let cases = [
        (
            event(
                "testnet",
                42,
                1_000,
                "tx-7",
                3,
                0,
                "primary",
                ConfirmationClass::CommittedPrimary,
                0,
                7,
            ),
            "chain",
        ),
        (
            event(
                "mainnet",
                43,
                1_000,
                "tx-7",
                3,
                0,
                "primary",
                ConfirmationClass::CommittedPrimary,
                0,
                7,
            ),
            "height",
        ),
        (
            event(
                "mainnet",
                42,
                1_001,
                "tx-7",
                3,
                0,
                "primary",
                ConfirmationClass::CommittedPrimary,
                0,
                7,
            ),
            "time",
        ),
        (
            event(
                "mainnet",
                42,
                1_000,
                "tx-7",
                3,
                0,
                "primary",
                ConfirmationClass::CommittedIndependent,
                0,
                7,
            ),
            "confirmation",
        ),
    ];

    for (event, expected) in cases {
        let error =
            block(ConfirmationClass::CommittedPrimary, vec![event], "primary").expect_err(expected);
        assert_eq!(
            error.reason_code(),
            format!("canonical_block.mixed_{expected}")
        );
    }
}

#[test]
fn block_rejects_duplicate_or_noncanonical_event_identity() {
    let duplicate = event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        0,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        7,
    );
    assert!(matches!(
        block(
            ConfirmationClass::CommittedPrimary,
            vec![duplicate.clone(), duplicate],
            "primary",
        ),
        Err(BlockError::DuplicateEventId(_))
    ));

    let valid = event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        0,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        7,
    );
    let mut wire =
        WireCanonicalEventEnvelope::decode(&valid.encode_to_vec().expect("encode")).expect("wire");
    wire.event_id = "event-id-controlled-by-caller".to_owned();
    let invalid = CanonicalEventEnvelope::decode(&wire.encode_to_vec()).expect("typed envelope");

    assert!(matches!(
        block(
            ConfirmationClass::CommittedPrimary,
            vec![invalid],
            "primary",
        ),
        Err(BlockError::InvalidEventId { .. })
    ));
}

#[test]
fn block_requires_contiguous_event_indices_within_each_transaction() {
    let first_index_not_zero = event(
        "mainnet",
        42,
        1_000,
        "tx-7",
        3,
        1,
        "primary",
        ConfirmationClass::CommittedPrimary,
        0,
        7,
    );
    assert!(matches!(
        block(
            ConfirmationClass::CommittedPrimary,
            vec![first_index_not_zero],
            "primary",
        ),
        Err(BlockError::InvalidEventOrder { .. })
    ));

    let repeated_order = vec![
        event(
            "mainnet",
            42,
            1_000,
            "tx-7",
            3,
            0,
            "primary",
            ConfirmationClass::CommittedPrimary,
            0,
            7,
        ),
        event(
            "mainnet",
            42,
            1_000,
            "tx-8",
            3,
            0,
            "primary",
            ConfirmationClass::CommittedPrimary,
            0,
            7,
        ),
    ];
    assert!(matches!(
        block(
            ConfirmationClass::CommittedPrimary,
            repeated_order,
            "primary",
        ),
        Err(BlockError::InvalidEventOrder { .. })
    ));

    let gap = vec![
        event(
            "mainnet",
            42,
            1_000,
            "tx-7",
            3,
            0,
            "primary",
            ConfirmationClass::CommittedPrimary,
            0,
            7,
        ),
        event(
            "mainnet",
            42,
            1_000,
            "tx-7",
            3,
            2,
            "primary",
            ConfirmationClass::CommittedPrimary,
            0,
            7,
        ),
    ];
    assert!(matches!(
        block(ConfirmationClass::CommittedPrimary, gap, "primary"),
        Err(BlockError::InvalidEventOrder { .. })
    ));
}

#[test]
fn transaction_indices_may_skip_when_each_new_transaction_starts_at_zero() {
    let events = vec![
        event(
            "mainnet",
            42,
            1_000,
            "tx-7",
            3,
            0,
            "primary",
            ConfirmationClass::CommittedPrimary,
            0,
            7,
        ),
        event(
            "mainnet",
            42,
            1_000,
            "tx-9",
            5,
            0,
            "primary",
            ConfirmationClass::CommittedPrimary,
            0,
            8,
        ),
    ];

    let block = block(ConfirmationClass::CommittedPrimary, events, "primary")
        .expect("transaction gaps with no canonical events are valid");
    assert_eq!(block.events().len(), 2);
}
