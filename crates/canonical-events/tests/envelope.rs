use api_contracts::WireCanonicalEventEnvelope;
use canonical_events::{
    CanonicalEventEnvelope, ConfirmationClass, ContractError, EventKind, EventOrderingKey,
};

fn valid_wire() -> WireCanonicalEventEnvelope {
    let bytes = CanonicalEventEnvelope::fixture()
        .unwrap()
        .encode_to_vec()
        .unwrap();
    WireCanonicalEventEnvelope::decode(&bytes).unwrap()
}

fn decode(wire: WireCanonicalEventEnvelope) -> Result<CanonicalEventEnvelope, ContractError> {
    CanonicalEventEnvelope::decode(&wire.encode_to_vec())
}

#[test]
fn envelope_round_trip_preserves_full_domain_identity_and_order() {
    let envelope = CanonicalEventEnvelope::fixture().unwrap();
    let bytes = envelope.encode_to_vec().unwrap();
    let decoded = CanonicalEventEnvelope::decode(&bytes).unwrap();
    assert_eq!(decoded, envelope);
    assert_eq!(decoded.event_id(), envelope.event_id());
    assert_eq!(
        decoded.ordering_key(),
        EventOrderingKey {
            chain_id: "hyperliquid-mainnet",
            block_height: 42,
            transaction_index: 7,
            event_index: 9,
        }
    );
    assert_eq!(
        decoded.confirmation_class(),
        ConfirmationClass::CommittedPrimary
    );
    assert_eq!(decoded.event_kind(), EventKind::TradeMatched);
}

#[test]
fn source_evidence_round_trip_preserves_parent_offset_and_event_sub_index() {
    let mut wire = valid_wire();
    wire.source_evidence[0].source_event_index = Some(0);

    let decoded = decode(wire).expect("indexed source evidence");
    let evidence = &decoded.source_evidence()[0];

    assert_eq!(evidence.source_offset(), "hyperliquid-mainnet:42:7:9");
    assert_eq!(evidence.source_event_index(), Some(0));
    let encoded = WireCanonicalEventEnvelope::decode(
        &decoded.encode_to_vec().expect("encode indexed evidence"),
    )
    .expect("wire envelope");
    assert_eq!(encoded.source_evidence[0].source_event_index, Some(0));
}

#[test]
fn source_evidence_is_required_and_each_field_is_validated() {
    let mut missing = valid_wire();
    missing.source_evidence.clear();
    assert!(matches!(
        decode(missing),
        Err(ContractError::Missing("source_evidence"))
    ));

    for field in [
        "source_evidence.source_id",
        "source_evidence.source_version",
        "source_evidence.source_offset",
        "source_evidence.content_hash",
    ] {
        let mut wire = valid_wire();
        match field {
            "source_evidence.source_id" => wire.source_evidence[0].source_id.clear(),
            "source_evidence.source_version" => wire.source_evidence[0].source_version.clear(),
            "source_evidence.source_offset" => wire.source_evidence[0].source_offset.clear(),
            "source_evidence.content_hash" => wire.source_evidence[0].content_hash = vec![0; 31],
            _ => unreachable!("the table contains only source evidence fields"),
        }
        let error = decode(wire).unwrap_err();
        if field == "source_evidence.content_hash" {
            assert!(
                matches!(error, ContractError::Invalid { field: actual, .. } if actual == field)
            );
        } else {
            assert!(matches!(error, ContractError::Missing(actual) if actual == field));
        }
    }
}

#[test]
fn semantic_major_one_is_accepted_and_unknown_major_is_rejected() {
    let mut compatible = valid_wire();
    compatible.schema_version = "1.17.3".to_owned();
    assert!(decode(compatible).is_ok());

    let mut unsupported = valid_wire();
    unsupported.schema_version = "2.0.0".to_owned();
    assert!(matches!(
        decode(unsupported),
        Err(ContractError::UnsupportedSchema(version)) if version == "2.0.0"
    ));
}

#[test]
fn malformed_schema_version_is_invalid_not_silently_accepted() {
    let mut wire = valid_wire();
    wire.schema_version = "v1".to_owned();
    assert!(matches!(
        decode(wire),
        Err(ContractError::Invalid {
            field: "schema_version",
            ..
        })
    ));
}

#[test]
fn every_required_string_and_payload_is_rejected_when_missing() {
    for field in [
        "schema_version",
        "chain_id",
        "transaction_id",
        "event_id",
        "event_kind",
        "parser_version",
        "payload",
    ] {
        let mut wire = valid_wire();
        match field {
            "schema_version" => wire.schema_version.clear(),
            "chain_id" => wire.chain_id.clear(),
            "transaction_id" => wire.transaction_id.clear(),
            "event_id" => wire.event_id.clear(),
            "event_kind" => wire.event_kind.clear(),
            "parser_version" => wire.parser_version.clear(),
            "payload" => wire.payload.clear(),
            _ => unreachable!("the table contains only known required fields"),
        }
        assert!(
            matches!(decode(wire), Err(ContractError::Missing(actual)) if actual == field),
            "missing {field} must be rejected"
        );
    }
}

#[test]
fn malformed_identifiers_are_rejected_by_domain_constructors() {
    for field in [
        "chain_id",
        "transaction_id",
        "event_id",
        "market_ids",
        "account_ids",
    ] {
        let mut wire = valid_wire();
        match field {
            "chain_id" => wire.chain_id = " padded".to_owned(),
            "transaction_id" => wire.transaction_id = "tx ".to_owned(),
            "event_id" => wire.event_id = " event".to_owned(),
            "market_ids" => wire.market_ids = vec![String::new()],
            "account_ids" => wire.account_ids = vec!["account ".to_owned()],
            _ => unreachable!("the table contains only identifier fields"),
        }
        assert!(
            matches!(decode(wire), Err(ContractError::Invalid { field: actual, .. }) if actual == field),
            "invalid {field} must be rejected"
        );
    }
}

#[test]
fn negative_protocol_timestamps_are_rejected_individually() {
    for field in [
        "block_time_micros",
        "observed_at_micros",
        "ingested_at_micros",
        "canonicalized_at_micros",
    ] {
        let mut wire = valid_wire();
        match field {
            "block_time_micros" => wire.block_time_micros = -1,
            "observed_at_micros" => wire.observed_at_micros = -1,
            "ingested_at_micros" => wire.ingested_at_micros = -1,
            "canonicalized_at_micros" => wire.canonicalized_at_micros = -1,
            _ => unreachable!("the table contains only timestamp fields"),
        }
        assert!(
            matches!(decode(wire), Err(ContractError::Invalid { field: actual, .. }) if actual == field),
            "negative {field} must be rejected"
        );
    }
}

#[test]
fn unknown_event_kind_and_confirmation_class_are_rejected() {
    let mut kind = valid_wire();
    kind.event_kind = "FutureEvent".to_owned();
    assert!(matches!(
        decode(kind),
        Err(ContractError::Invalid {
            field: "event_kind",
            ..
        })
    ));

    let mut confirmation = valid_wire();
    confirmation.confirmation_class = 99;
    assert!(matches!(
        decode(confirmation),
        Err(ContractError::Invalid {
            field: "confirmation_class",
            ..
        })
    ));
}

#[test]
fn payload_hash_must_be_exactly_32_bytes() {
    for length in [0, 31, 33] {
        let mut wire = valid_wire();
        wire.payload_hash = vec![0; length];
        assert!(matches!(
            decode(wire),
            Err(ContractError::Invalid {
                field: "payload_hash",
                ..
            })
        ));
    }
}

#[test]
fn malformed_protobuf_returns_a_decode_error() {
    assert!(matches!(
        CanonicalEventEnvelope::decode(&[0xff, 0xff, 0xff]),
        Err(ContractError::Decode(_))
    ));
}

#[test]
fn event_kind_mapping_is_exhaustive_unique_and_round_trips() {
    let expected = [
        EventKind::OrderAccepted,
        EventKind::OrderRested,
        EventKind::OrderModified,
        EventKind::OrderPartiallyFilled,
        EventKind::OrderFilled,
        EventKind::OrderCancelled,
        EventKind::OrderRejected,
        EventKind::TriggerOrderActivated,
        EventKind::TwapStarted,
        EventKind::TwapSliceFilled,
        EventKind::TwapCompleted,
        EventKind::TradeMatched,
        EventKind::DepositCredited,
        EventKind::WithdrawalDebited,
        EventKind::SpotTransfer,
        EventKind::PerpTransfer,
        EventKind::SubaccountTransfer,
        EventKind::VaultDeposit,
        EventKind::VaultWithdrawal,
        EventKind::FeeCharged,
        EventKind::BuilderFeeCharged,
        EventKind::FundingPaid,
        EventKind::FundingReceived,
        EventKind::ReferralReward,
        EventKind::AccountModeChanged,
        EventKind::MarginModeChanged,
        EventKind::LeverageChanged,
        EventKind::LiquidationStarted,
        EventKind::LiquidationFill,
        EventKind::BackstopLiquidation,
        EventKind::PositionSettled,
        EventKind::MarketHalted,
        EventKind::MarketResumed,
        EventKind::OpenInterestCapChanged,
        EventKind::MarginTableChanged,
        EventKind::MarketCreated,
        EventKind::MarketMetadataChanged,
        EventKind::OracleUpdated,
        EventKind::FundingRateUpdated,
        EventKind::AssetContextUpdated,
        EventKind::DexCreated,
        EventKind::OutcomeCreated,
        EventKind::OutcomeResolved,
    ];
    assert_eq!(EventKind::ALL, expected);
    assert_eq!(EventKind::ALL.len(), 43);

    let mut names = std::collections::BTreeSet::new();
    for kind in EventKind::ALL {
        assert!(names.insert(kind.as_wire_name()));
        assert_eq!(EventKind::try_from(kind.as_wire_name()).unwrap(), kind);
    }
}
