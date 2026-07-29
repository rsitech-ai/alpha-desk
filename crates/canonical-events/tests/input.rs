use canonical_events::{
    CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, ContractError, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("valid known time")
}

fn source(source_id: &str, content_byte: u8) -> SourceEvidence {
    SourceEvidence::try_new(
        SourceId::new(source_id).expect("valid source"),
        "node-v1",
        "session-a:42",
        [content_byte; 32],
    )
    .expect("valid evidence")
}

fn payload(seed: u64) -> EventPayload {
    EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).expect("price"),
        Quantity::parse_at_scale("0.01", 8).expect("quantity"),
        seed,
    ))
}

fn input(
    evidence: SourceEvidence,
    observed_at: KnownTime,
    ingested_at: KnownTime,
    canonicalized_at: KnownTime,
    canonical_event_index: u32,
    payload: EventPayload,
) -> CanonicalEventInput {
    CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(42),
        block_time: ProtocolTime::from_unix_micros(1_000).expect("block time"),
        transaction_id: TransactionId::new("tx-7").expect("transaction"),
        transaction_index: 3,
        canonical_event_index,
        market_ids: vec![MarketId::new("perp:BTC").expect("market")],
        account_ids: vec![Address::from_bytes([0x11; 20])],
        source_evidence: vec![evidence],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at,
        ingested_at,
        canonicalized_at,
        parser_version: "canonical-parser-v1".to_owned(),
        payload,
    }
}

#[test]
fn source_and_lifecycle_metadata_do_not_change_canonical_identity() {
    let primary = CanonicalEventEnvelope::from_input(input(
        source("primary", 0x11),
        known(1_000),
        known(2_000),
        known(3_000),
        0,
        payload(7),
    ))
    .expect("primary event");
    let secondary = CanonicalEventEnvelope::from_input(input(
        source("secondary", 0x22),
        known(1_500),
        known(2_500),
        known(3_500),
        0,
        payload(7),
    ))
    .expect("secondary event");

    assert_eq!(primary.event_id(), secondary.event_id());
    assert_eq!(primary.payload_hash(), secondary.payload_hash());
    assert_ne!(primary.source_evidence(), secondary.source_evidence());
    assert_ne!(primary.observed_at(), secondary.observed_at());
}

#[test]
fn payload_content_is_separate_from_identity_and_index_is_not() {
    let baseline = CanonicalEventEnvelope::from_input(input(
        source("primary", 0x11),
        known(1_000),
        known(2_000),
        known(3_000),
        0,
        payload(7),
    ))
    .expect("baseline");
    let changed_payload = CanonicalEventEnvelope::from_input(input(
        source("primary", 0x11),
        known(1_000),
        known(2_000),
        known(3_000),
        0,
        payload(8),
    ))
    .expect("changed payload");
    let changed_index = CanonicalEventEnvelope::from_input(input(
        source("primary", 0x11),
        known(1_000),
        known(2_000),
        known(3_000),
        1,
        payload(7),
    ))
    .expect("changed index");

    assert_eq!(baseline.event_id(), changed_payload.event_id());
    assert_ne!(baseline.payload_hash(), changed_payload.payload_hash());
    assert_ne!(baseline.event_id(), changed_index.event_id());
}

#[test]
fn source_evidence_rejects_blank_or_padded_identity_fields() {
    let source_id = SourceId::new("primary").expect("source");

    assert!(matches!(
        SourceEvidence::try_new(source_id.clone(), "", "offset", [0; 32]),
        Err(ContractError::Missing("source_evidence.source_version"))
    ));
    assert!(matches!(
        SourceEvidence::try_new(source_id.clone(), " node-v1", "offset", [0; 32]),
        Err(ContractError::Invalid {
            field: "source_evidence.source_version",
            ..
        })
    ));
    assert!(matches!(
        SourceEvidence::try_new(source_id.clone(), "node-v1", "", [0; 32]),
        Err(ContractError::Missing("source_evidence.source_offset"))
    ));
    assert!(matches!(
        SourceEvidence::try_new(source_id, "node-v1", "offset ", [0; 32]),
        Err(ContractError::Invalid {
            field: "source_evidence.source_offset",
            ..
        })
    ));
}

#[test]
fn production_input_rejects_missing_evidence_and_invalid_versions() {
    let mut missing_evidence = input(
        source("primary", 0x11),
        known(1_000),
        known(2_000),
        known(3_000),
        0,
        payload(7),
    );
    missing_evidence.source_evidence.clear();
    assert!(matches!(
        CanonicalEventEnvelope::from_input(missing_evidence),
        Err(ContractError::Missing("source_evidence"))
    ));

    for version in ["v1", "2.0.0"] {
        let mut invalid = input(
            source("primary", 0x11),
            known(1_000),
            known(2_000),
            known(3_000),
            0,
            payload(7),
        );
        invalid.schema_version = version.to_owned();
        assert!(CanonicalEventEnvelope::from_input(invalid).is_err());
    }

    for parser_version in ["", " parser-v1", "parser-v1 "] {
        let mut invalid = input(
            source("primary", 0x11),
            known(1_000),
            known(2_000),
            known(3_000),
            0,
            payload(7),
        );
        invalid.parser_version = parser_version.to_owned();
        assert!(CanonicalEventEnvelope::from_input(invalid).is_err());
    }
}

#[test]
fn production_input_rejects_non_monotonic_lifecycle_times() {
    for (observed, ingested, canonicalized) in [(2_001, 2_000, 3_000), (1_000, 3_001, 3_000)] {
        let invalid = input(
            source("primary", 0x11),
            known(observed),
            known(ingested),
            known(canonicalized),
            0,
            payload(7),
        );
        assert!(matches!(
            CanonicalEventEnvelope::from_input(invalid),
            Err(ContractError::Invalid {
                field: "lifecycle_times",
                ..
            })
        ));
    }
}
