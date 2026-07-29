use canonical_events::{
    CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    EvidenceMergeError, SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};

#[allow(clippy::too_many_arguments)]
fn event(
    source_id: &str,
    source_offset: &str,
    source_content_byte: u8,
    market_id: &str,
    confirmation_class: ConfirmationClass,
    lifecycle_offset: i64,
) -> CanonicalEventEnvelope {
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(42),
        block_time: ProtocolTime::from_unix_micros(1_000).expect("block time"),
        transaction_id: TransactionId::new("tx-42").expect("transaction"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new(market_id).expect("market")],
        account_ids: Vec::new(),
        source_evidence: vec![
            SourceEvidence::try_new(
                SourceId::new(source_id).expect("source"),
                "node-v1",
                source_offset,
                [source_content_byte; 32],
            )
            .expect("evidence"),
        ],
        confirmation_class,
        observed_at: KnownTime::from_unix_micros(2_000 + lifecycle_offset).expect("observed time"),
        ingested_at: KnownTime::from_unix_micros(3_000 + lifecycle_offset).expect("ingested time"),
        canonicalized_at: KnownTime::from_unix_micros(4_000 + lifecycle_offset)
            .expect("canonicalized time"),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            7,
        )),
    })
    .expect("event")
}

#[test]
fn matching_canonical_content_merges_and_sorts_independent_evidence() {
    let primary = event(
        "primary",
        "block:42",
        0x11,
        "BTC",
        ConfirmationClass::CommittedPrimary,
        0,
    );
    let secondary = event(
        "secondary",
        "block:42",
        0x22,
        "BTC",
        ConfirmationClass::CommittedIndependent,
        500,
    );

    let merged = secondary
        .merge_matching_source_evidence(&primary)
        .expect("matching content");
    let source_ids = merged
        .source_evidence()
        .iter()
        .map(|evidence| evidence.source_id().as_str())
        .collect::<Vec<_>>();

    assert_eq!(source_ids, vec!["primary", "secondary"]);
    assert_eq!(
        merged.confirmation_class(),
        ConfirmationClass::CommittedIndependent
    );
}

#[test]
fn same_source_locator_with_different_content_hash_is_rejected() {
    let first = event(
        "primary",
        "block:42",
        0x11,
        "BTC",
        ConfirmationClass::CommittedPrimary,
        0,
    );
    let conflicting = event(
        "primary",
        "block:42",
        0x22,
        "BTC",
        ConfirmationClass::CommittedPrimary,
        0,
    );

    assert!(matches!(
        first.merge_matching_source_evidence(&conflicting),
        Err(EvidenceMergeError::SourceEvidenceConflict {
            source_id,
            existing_hash,
            conflicting_hash,
        }) if source_id.as_str() == "primary"
            && existing_hash == [0x11; 32]
            && conflicting_hash == [0x22; 32]
    ));
}

#[test]
fn routing_metadata_difference_is_not_merged_as_source_only_evidence() {
    let btc = event(
        "primary",
        "block:42",
        0x11,
        "BTC",
        ConfirmationClass::CommittedPrimary,
        0,
    );
    let eth = event(
        "secondary",
        "block:42",
        0x22,
        "ETH",
        ConfirmationClass::CommittedIndependent,
        500,
    );

    assert_eq!(
        btc.merge_matching_source_evidence(&eth),
        Err(EvidenceMergeError::CanonicalContentMismatch)
    );
}
