use api_contracts::{
    WireCanonicalEventEnvelope, WireMarketCreated, WireOrderAccepted, encode_market_created,
    encode_order_accepted,
};
use canonical_events::{
    AssetContextUpdated, CanonicalEventEnvelope, ConfirmationClass, ContractError, DexCreated,
    EventKind, EventPayload, FundingRateUpdated, MarginTableChanged, MarketCreated, MarketHalted,
    MarketMetadataChanged, MarketResumed, OpenInterestCapChanged, OracleUpdated, OutcomeCreated,
    OutcomeResolved, TradeMatched, TradeParticipantRoleV1, TradeParticipantV1,
};
use domain_types::{
    Address, AssetId, BlockHeight, ClientOrderId, DexId, EventId, FundingRate, KnownTime, MarketId,
    OrderId, OutcomeId, PositionQuantity, Price, ProtocolTime, Quantity, QuoteAmount, TradeId,
    TransactionId, TwapId,
};

fn task_4_envelope(schema_version: &str) -> Result<CanonicalEventEnvelope, ContractError> {
    let buyer = Address::from_bytes([0x11; 20]);
    let seller = Address::from_bytes([0x22; 20]);
    CanonicalEventEnvelope::try_new(
        schema_version,
        "mainnet",
        BlockHeight::new(42),
        ProtocolTime::from_unix_micros(1_721_779_200_000_042).unwrap(),
        TransactionId::new("fixture-tx-42-0").unwrap(),
        0,
        0,
        EventId::new("fixture-mainnet-42-0-0").unwrap(),
        vec![MarketId::new("perp:BTC").unwrap()],
        vec![buyer, seller],
        ConfirmationClass::CommittedPrimary,
        EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).unwrap(),
            Quantity::parse_at_scale("0.01", 8).unwrap(),
            7,
        )),
        "fixture-parser-v1",
    )
}

fn assert_protocol_time(_: ProtocolTime) {}
fn assert_known_time(_: KnownTime) {}

#[test]
fn exact_task_4_constructor_is_deterministic_typed_and_round_trips() {
    let first = task_4_envelope("1.0.0").unwrap();
    let second = task_4_envelope("1.0.0").unwrap();

    assert_eq!(first, second);
    assert_eq!(first.event_kind(), EventKind::TradeMatched);
    assert!(matches!(
        first.payload(),
        EventPayload::TradeMatched(TradeMatched {
            deterministic_seed: 7,
            ..
        })
    ));
    assert_protocol_time(first.block_time());
    assert_known_time(first.observed_at());
    assert_known_time(first.ingested_at());
    assert_known_time(first.canonicalized_at());
    assert_eq!(
        first.observed_at().unix_micros(),
        first.block_time().unix_micros()
    );
    assert_eq!(first.observed_at(), first.ingested_at());
    assert_eq!(first.ingested_at(), first.canonicalized_at());
    assert_eq!(first.account_addresses().len(), 2);
    assert_eq!(first.payload_hash(), second.payload_hash());
    assert_eq!(
        CanonicalEventEnvelope::decode(&first.encode_to_vec().unwrap()).unwrap(),
        first
    );
}

#[test]
fn trade_payload_round_trip_preserves_non_empty_v1_identities() {
    let payload = EventPayload::TradeMatched(TradeMatched {
        trade_id: Some(TradeId::new("trade-42").unwrap()),
        market_id: Some(MarketId::new("perp:BTC").unwrap()),
        maker_order_id: Some(OrderId::new("maker-7").unwrap()),
        taker_order_id: Some(OrderId::new("taker-9").unwrap()),
        price: Price::parse_at_scale("65000", 6).unwrap(),
        quantity: Quantity::parse_at_scale("0.01", 8).unwrap(),
        deterministic_seed: 7,
        participants: None,
    });
    let encoded = payload.encode_to_vec().unwrap();

    assert_eq!(
        EventPayload::decode(EventKind::TradeMatched, &encoded).unwrap(),
        payload
    );

    let mut wire = WireCanonicalEventEnvelope::decode(
        &task_4_envelope("1.0.0").unwrap().encode_to_vec().unwrap(),
    )
    .unwrap();
    wire.payload_hash = blake3::hash(&encoded).as_bytes().to_vec();
    wire.payload = encoded.clone();
    let decoded = CanonicalEventEnvelope::decode(&wire.encode_to_vec()).unwrap();
    let reencoded = WireCanonicalEventEnvelope::decode(&decoded.encode_to_vec().unwrap()).unwrap();

    assert_eq!(reencoded.payload, encoded);
    assert_eq!(
        reencoded.payload_hash,
        blake3::hash(&reencoded.payload).as_bytes()
    );
}

fn enriched_trade() -> TradeMatched {
    TradeMatched {
        trade_id: Some(TradeId::new("trade-42").unwrap()),
        market_id: Some(MarketId::new("perp:BTC").unwrap()),
        maker_order_id: Some(OrderId::new("maker-7").unwrap()),
        taker_order_id: Some(OrderId::new("taker-9").unwrap()),
        price: Price::parse_at_scale("65000", 6).unwrap(),
        quantity: Quantity::parse_at_scale("0.01", 8).unwrap(),
        deterministic_seed: 7,
        participants: Some(Box::new([
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Buyer,
                account_id: Address::from_bytes([0x11; 20]),
                start_position: PositionQuantity::parse_at_scale("996.67", 2).unwrap(),
                order_id: OrderId::new("12212201265").unwrap(),
                twap_id: Some(TwapId::new(91)),
                client_order_id: Some(
                    ClientOrderId::new("0x11111111111111111111111111111111").unwrap(),
                ),
            },
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Seller,
                account_id: Address::from_bytes([0x22; 20]),
                start_position: PositionQuantity::parse_at_scale("-996.7", 1).unwrap(),
                order_id: OrderId::new("12212198275").unwrap(),
                twap_id: None,
                client_order_id: None,
            },
        ])),
    }
}

#[test]
fn enriched_trade_round_trip_preserves_exact_signed_participant_anchors() {
    let payload = EventPayload::TradeMatched(enriched_trade());
    let encoded = payload.encode_to_vec().unwrap();

    assert_eq!(
        EventPayload::decode(EventKind::TradeMatched, &encoded).unwrap(),
        payload
    );
}

#[test]
fn canonical_trade_rejects_nonpositive_price_or_fill_before_envelope_construction() {
    let mut zero_price = enriched_trade();
    zero_price.price = Price::parse_at_scale("0", 6).unwrap();
    assert!(
        EventPayload::TradeMatched(zero_price)
            .encode_to_vec()
            .is_err()
    );

    let mut zero_quantity = enriched_trade();
    zero_quantity.quantity = Quantity::parse_at_scale("0", 8).unwrap();
    assert!(
        EventPayload::TradeMatched(zero_quantity)
            .encode_to_vec()
            .is_err()
    );
}

#[test]
fn enriched_trade_requires_exact_envelope_account_order() {
    let build = |accounts| {
        CanonicalEventEnvelope::try_new(
            "1.0.0",
            "mainnet",
            BlockHeight::new(42),
            ProtocolTime::from_unix_micros(1_721_779_200_000_042).unwrap(),
            TransactionId::new("fixture-tx-42-0").unwrap(),
            0,
            0,
            EventId::new("fixture-mainnet-42-0-0").unwrap(),
            vec![MarketId::new("perp:BTC").unwrap()],
            accounts,
            ConfirmationClass::CommittedPrimary,
            EventPayload::TradeMatched(enriched_trade()),
            "fixture-parser-v1",
        )
    };

    let valid = build(vec![
        Address::from_bytes([0x11; 20]),
        Address::from_bytes([0x22; 20]),
    ])
    .unwrap();
    let mut swapped_wire =
        WireCanonicalEventEnvelope::decode(&valid.encode_to_vec().unwrap()).unwrap();
    swapped_wire.account_ids.swap(0, 1);
    assert!(CanonicalEventEnvelope::decode(&swapped_wire.encode_to_vec()).is_err());
    assert!(
        build(vec![
            Address::from_bytes([0x22; 20]),
            Address::from_bytes([0x11; 20])
        ])
        .is_err()
    );
    assert!(build(vec![Address::from_bytes([0x11; 20])]).is_err());
}

#[test]
fn participant_absent_trade_payload_retains_the_frozen_v1_bytes() {
    let encoded = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).unwrap(),
        Quantity::parse_at_scale("0.01", 8).unwrap(),
        7,
    ))
    .encode_to_vec()
    .unwrap();

    assert_eq!(
        encoded.as_slice(),
        b"\x0a\x0cTradeMatched\x12\x20\x2a\x0e\x0a\x0c65000.000000\
          \x32\x0c\x0a\x0a0.01000000\x38\x07"
    );
}

#[test]
fn trade_decode_rejects_one_or_three_participants() {
    let encoded = EventPayload::TradeMatched(enriched_trade())
        .encode_to_vec()
        .unwrap();
    let with_participant_count = |count: usize| {
        mutate_typed_payload(&encoded, |message| {
            let fields = split_protobuf_fields(message);
            let first_participant = fields
                .iter()
                .find(|field| field.number == 8)
                .expect("enriched fixture participant")
                .encoded
                .clone();
            let mut result = fields
                .into_iter()
                .filter(|field| field.number != 8)
                .flat_map(|field| field.encoded)
                .collect::<Vec<_>>();
            for _ in 0..count {
                result.extend_from_slice(&first_participant);
            }
            result
        })
    };

    assert!(EventPayload::decode(EventKind::TradeMatched, &with_participant_count(1)).is_err());
    assert!(EventPayload::decode(EventKind::TradeMatched, &with_participant_count(3)).is_err());
}

#[test]
fn all_43_typed_payload_variants_encode_decode_and_preserve_kind() {
    let fixtures = EventPayload::fixtures().unwrap();
    assert_eq!(fixtures.len(), 43);
    let mut kinds = std::collections::BTreeSet::new();

    for payload in fixtures {
        let kind = payload.kind();
        assert!(kinds.insert(kind));
        let bytes = payload.encode_to_vec().unwrap();
        let decoded = EventPayload::decode(kind, &bytes).unwrap();
        assert_eq!(decoded, payload);
        assert_eq!(decoded.kind(), kind);
    }
    assert_eq!(kinds.len(), 43);
}

#[test]
fn market_metadata_payloads_are_typed_and_round_trip_exactly() {
    let payloads = [
        EventPayload::DexCreated(DexCreated {
            dex_id: DexId::new("validator").unwrap(),
            name: "Hyperliquid Validator Perpetuals".to_owned(),
            operator_account_id: Address::from_bytes([0x11; 20]),
        }),
        EventPayload::AssetContextUpdated(AssetContextUpdated {
            asset_id: AssetId::new("USDC").unwrap(),
            context_version: "asset-context-7".to_owned(),
            context_hash: [0x22; 32],
        }),
        EventPayload::MarketCreated(MarketCreated {
            market_id: MarketId::new("perp:BTC").unwrap(),
            dex_id: DexId::new("validator").unwrap(),
            base_asset_id: AssetId::new("BTC").unwrap(),
            quote_asset_id: AssetId::new("USDC").unwrap(),
            tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
            lot_size: Quantity::parse_at_scale("0.00001", 8).unwrap(),
        }),
        EventPayload::MarketMetadataChanged(MarketMetadataChanged {
            market_id: MarketId::new("perp:BTC").unwrap(),
            metadata_version: "market-metadata-8".to_owned(),
            metadata_hash: [0x33; 32],
        }),
    ];

    for payload in payloads {
        let kind = payload.kind();
        let encoded = payload.encode_to_vec().unwrap();
        assert_eq!(EventPayload::decode(kind, &encoded).unwrap(), payload);
    }
}

#[test]
fn market_metadata_payloads_reject_semantically_invalid_direct_values() {
    let invalid_market = EventPayload::MarketCreated(MarketCreated {
        market_id: MarketId::new("perp:BTC").unwrap(),
        dex_id: DexId::new("validator").unwrap(),
        base_asset_id: AssetId::new("BTC").unwrap(),
        quote_asset_id: AssetId::new("USDC").unwrap(),
        tick_size: Price::from_raw(0, 6).unwrap(),
        lot_size: Quantity::parse_at_scale("0.00001", 8).unwrap(),
    });
    assert!(matches!(
        invalid_market.encode_to_vec(),
        Err(ContractError::Invalid { .. })
    ));

    let invalid_dex = EventPayload::DexCreated(DexCreated {
        dex_id: DexId::new("validator").unwrap(),
        name: "Validator\nPerpetuals".to_owned(),
        operator_account_id: Address::from_bytes([0x11; 20]),
    });
    assert!(matches!(
        invalid_dex.encode_to_vec(),
        Err(ContractError::Invalid { .. })
    ));

    let invalid_wire = encode_market_created(&WireMarketCreated {
        market_id: "perp:BTC".to_owned(),
        dex_id: "validator".to_owned(),
        base_asset_id: "BTC".to_owned(),
        quote_asset_id: "BTC".to_owned(),
        tick_size: "0.100000".to_owned(),
        lot_size: "0.00001000".to_owned(),
    })
    .unwrap();
    assert!(matches!(
        EventPayload::decode(EventKind::MarketCreated, &invalid_wire),
        Err(ContractError::Invalid { .. })
    ));
}

#[test]
fn market_state_payloads_are_typed_and_round_trip_exactly() {
    let market_id = MarketId::new("perp:BTC").unwrap();
    let outcome_market_id = MarketId::new("outcome:presidential-election").unwrap();
    let payloads = [
        EventPayload::MarketHalted(MarketHalted {
            market_id: market_id.clone(),
            reason: "scheduled_upgrade".to_owned(),
        }),
        EventPayload::MarketResumed(MarketResumed {
            market_id: market_id.clone(),
            reason: "upgrade_complete".to_owned(),
        }),
        EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
            market_id: market_id.clone(),
            previous_cap: QuoteAmount::from_raw(100_000_000, 0).unwrap(),
            new_cap: QuoteAmount::from_raw(125_000_000, 0).unwrap(),
        }),
        EventPayload::MarginTableChanged(MarginTableChanged {
            market_id: market_id.clone(),
            previous_table_hash: "margin-table-v7".to_owned(),
            new_table_hash: "margin-table-v8".to_owned(),
        }),
        EventPayload::OracleUpdated(OracleUpdated {
            market_id: market_id.clone(),
            oracle_price: Price::parse_at_scale("65000.125", 6).unwrap(),
            source: "hyperliquid-validator-oracle".to_owned(),
            effective_at: ProtocolTime::from_unix_micros(1_721_779_200_000_042).unwrap(),
        }),
        EventPayload::FundingRateUpdated(FundingRateUpdated {
            market_id,
            funding_rate: "-0.00001250".parse::<FundingRate>().unwrap(),
            effective_at: ProtocolTime::from_unix_micros(1_721_779_200_000_043).unwrap(),
        }),
        EventPayload::OutcomeCreated(OutcomeCreated {
            market_id: outcome_market_id.clone(),
            outcome_id: OutcomeId::new("candidate-a").unwrap(),
            description: "Candidate A wins the election".to_owned(),
        }),
        EventPayload::OutcomeResolved(OutcomeResolved {
            market_id: outcome_market_id,
            outcome_id: OutcomeId::new("candidate-a").unwrap(),
            settlement_value: Price::parse_at_scale("1", 6).unwrap(),
            resolved_at: ProtocolTime::from_unix_micros(1_730_000_000_000_000).unwrap(),
        }),
    ];

    for payload in payloads {
        let kind = payload.kind();
        let bytes = payload.encode_to_vec().unwrap();
        assert_eq!(EventPayload::decode(kind, &bytes).unwrap(), payload);
    }
}

#[test]
fn market_state_payloads_reject_invalid_direct_values() {
    let invalid_oracle = EventPayload::OracleUpdated(OracleUpdated {
        market_id: MarketId::new("perp:BTC").unwrap(),
        oracle_price: Price::from_raw(0, 6).unwrap(),
        source: "validator".to_owned(),
        effective_at: ProtocolTime::from_unix_micros(1).unwrap(),
    });
    assert!(matches!(
        invalid_oracle.encode_to_vec(),
        Err(ContractError::Invalid { .. })
    ));

    let invalid_cap = EventPayload::OpenInterestCapChanged(OpenInterestCapChanged {
        market_id: MarketId::new("perp:BTC").unwrap(),
        previous_cap: QuoteAmount::from_raw(100, 0).unwrap(),
        new_cap: QuoteAmount::from_raw(100, 0).unwrap(),
    });
    assert!(matches!(
        invalid_cap.encode_to_vec(),
        Err(ContractError::Invalid { .. })
    ));

    let invalid_settlement = EventPayload::OutcomeResolved(OutcomeResolved {
        market_id: MarketId::new("outcome:test").unwrap(),
        outcome_id: OutcomeId::new("yes").unwrap(),
        settlement_value: Price::from_raw(-1, 6).unwrap(),
        resolved_at: ProtocolTime::from_unix_micros(1).unwrap(),
    });
    assert!(matches!(
        invalid_settlement.encode_to_vec(),
        Err(ContractError::Invalid { .. })
    ));
}

#[test]
fn opaque_typed_payload_preserves_non_default_message_bytes_exactly() {
    let bytes = encode_order_accepted(&WireOrderAccepted {
        order_id: "order-17".to_owned(),
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        market_id: "perp:BTC".to_owned(),
        side: "buy".to_owned(),
        limit_price: "65000.125".to_owned(),
        quantity: "0.75".to_owned(),
    })
    .unwrap();

    let payload = EventPayload::decode(EventKind::OrderAccepted, &bytes).unwrap();
    assert_eq!(payload.encode_to_vec().unwrap(), bytes);
    assert!(EventPayload::decode(EventKind::OrderRested, &bytes).is_err());
}

#[test]
fn payload_decode_rejects_malformed_bytes_and_wrong_event_kind() {
    assert!(matches!(
        EventPayload::decode(EventKind::TradeMatched, &[0xff, 0xff]),
        Err(ContractError::Decode(_)) | Err(ContractError::Invalid { .. })
    ));

    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("1", 6).unwrap(),
        Quantity::parse_at_scale("2", 8).unwrap(),
        9,
    ));
    let bytes = payload.encode_to_vec().unwrap();
    assert!(EventPayload::decode(EventKind::OrderAccepted, &bytes).is_err());
}

#[test]
fn envelope_decode_rejects_event_kind_payload_mismatch_and_hash_divergence() {
    let envelope = task_4_envelope("1.0.0").unwrap();
    let mut wire = WireCanonicalEventEnvelope::decode(&envelope.encode_to_vec().unwrap()).unwrap();
    wire.event_kind = "OrderAccepted".to_owned();
    assert!(CanonicalEventEnvelope::decode(&wire.encode_to_vec()).is_err());

    let mut wire = WireCanonicalEventEnvelope::decode(&envelope.encode_to_vec().unwrap()).unwrap();
    wire.payload_hash[0] ^= 0xff;
    assert!(matches!(
        CanonicalEventEnvelope::decode(&wire.encode_to_vec()),
        Err(ContractError::Invalid {
            field: "payload_hash",
            ..
        })
    ));
}

#[test]
fn schema_version_is_strict_numeric_core_semver_with_major_one() {
    for valid in ["1.0.0", "1.17.3", "1.18446744073709551615.0"] {
        assert!(task_4_envelope(valid).is_ok(), "{valid} must be accepted");
    }

    for invalid in [
        "01.0.0",
        "1.02.3",
        "1.2.03",
        "+1.0.0",
        "1.+2.0",
        "1.0.-1",
        " 1.0.0",
        "1.0.0 ",
        "1.0.0-alpha",
        "1.0.0+build",
        "1.18446744073709551616.0",
        "1.0.18446744073709551616",
        "１.0.0",
    ] {
        assert!(
            matches!(
                task_4_envelope(invalid),
                Err(ContractError::Invalid {
                    field: "schema_version",
                    ..
                })
            ),
            "{invalid} must be rejected as malformed"
        );
    }

    assert!(matches!(
        task_4_envelope("2.0.0"),
        Err(ContractError::UnsupportedSchema(version)) if version == "2.0.0"
    ));
}

#[test]
fn trade_envelope_preserves_valid_forward_compatible_payload_encodings() {
    let canonical = task_4_envelope("1.0.0").unwrap();
    let canonical_wire =
        WireCanonicalEventEnvelope::decode(&canonical.encode_to_vec().unwrap()).unwrap();

    let cases = [
        (
            "unknown outer field",
            append_varint_field(canonical_wire.payload.clone(), 100, 1),
            7,
        ),
        (
            "unknown inner field",
            mutate_typed_payload(&canonical_wire.payload, |message| {
                append_varint_field(message.to_vec(), 100, 1)
            }),
            7,
        ),
        (
            "duplicate singular field",
            mutate_typed_payload(&canonical_wire.payload, |message| {
                append_varint_field(message.to_vec(), 7, 99)
            }),
            99,
        ),
        (
            "noncanonical outer and inner field order",
            reorder_typed_payload_fields(&canonical_wire.payload),
            7,
        ),
    ];

    for (case, payload, expected_seed) in cases {
        assert!(
            EventPayload::decode(EventKind::TradeMatched, &payload).is_err(),
            "standalone typed decode must reject {case} because it cannot reproduce those bytes"
        );

        let mut wire = canonical_wire.clone();
        wire.payload_hash = blake3::hash(&payload).as_bytes().to_vec();
        wire.payload = payload.clone();

        let decoded = CanonicalEventEnvelope::decode(&wire.encode_to_vec())
            .unwrap_or_else(|error| panic!("{case} must decode: {error}"));
        assert!(matches!(
            decoded.payload(),
            EventPayload::TradeMatched(TradeMatched {
                deterministic_seed,
                ..
            }) if *deterministic_seed == expected_seed
        ));

        let reencoded = WireCanonicalEventEnvelope::decode(
            &decoded
                .encode_to_vec()
                .unwrap_or_else(|error| panic!("{case} must re-encode: {error}")),
        )
        .unwrap();
        assert_eq!(reencoded.payload, payload, "{case} lost wire data");
        assert_eq!(
            reencoded.payload_hash,
            blake3::hash(&reencoded.payload).as_bytes(),
            "{case} changed the stored payload hash"
        );
    }
}

fn mutate_typed_payload(encoded: &[u8], mutate_message: impl Fn(&[u8]) -> Vec<u8>) -> Vec<u8> {
    let fields = split_protobuf_fields(encoded);
    let mut result = Vec::new();
    for field in fields {
        if field.number == 2 {
            let message = length_delimited_body(&field.encoded);
            append_length_delimited_field(&mut result, 2, &mutate_message(message));
        } else {
            result.extend_from_slice(&field.encoded);
        }
    }
    result
}

fn reorder_typed_payload_fields(encoded: &[u8]) -> Vec<u8> {
    let fields = split_protobuf_fields(encoded);
    let mut result = Vec::new();
    for field in fields.into_iter().rev() {
        if field.number == 2 {
            let mut message_fields = split_protobuf_fields(length_delimited_body(&field.encoded));
            message_fields.reverse();
            let message = message_fields
                .into_iter()
                .flat_map(|field| field.encoded)
                .collect::<Vec<_>>();
            append_length_delimited_field(&mut result, 2, &message);
        } else {
            result.extend_from_slice(&field.encoded);
        }
    }
    result
}

fn append_varint_field(mut encoded: Vec<u8>, number: u64, value: u64) -> Vec<u8> {
    append_varint(&mut encoded, number << 3);
    append_varint(&mut encoded, value);
    encoded
}

fn append_length_delimited_field(encoded: &mut Vec<u8>, number: u64, body: &[u8]) {
    append_varint(encoded, (number << 3) | 2);
    append_varint(encoded, u64::try_from(body.len()).unwrap());
    encoded.extend_from_slice(body);
}

fn append_varint(encoded: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        encoded.push((value as u8) | 0x80);
        value >>= 7;
    }
    encoded.push(value as u8);
}

#[derive(Debug)]
struct EncodedField {
    number: u64,
    encoded: Vec<u8>,
}

fn split_protobuf_fields(encoded: &[u8]) -> Vec<EncodedField> {
    let mut fields = Vec::new();
    let mut position = 0;
    while position < encoded.len() {
        let start = position;
        let key = read_varint(encoded, &mut position);
        match key & 7 {
            0 => {
                read_varint(encoded, &mut position);
            }
            1 => position += 8,
            2 => {
                let length = usize::try_from(read_varint(encoded, &mut position)).unwrap();
                position += length;
            }
            5 => position += 4,
            wire_type => panic!("unsupported test wire type {wire_type}"),
        }
        fields.push(EncodedField {
            number: key >> 3,
            encoded: encoded[start..position].to_vec(),
        });
    }
    fields
}

fn length_delimited_body(field: &[u8]) -> &[u8] {
    let mut position = 0;
    let key = read_varint(field, &mut position);
    assert_eq!(key & 7, 2);
    let length = usize::try_from(read_varint(field, &mut position)).unwrap();
    &field[position..position + length]
}

fn read_varint(encoded: &[u8], position: &mut usize) -> u64 {
    let mut value = 0_u64;
    let mut shift = 0;
    loop {
        let byte = encoded[*position];
        *position += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
        shift += 7;
    }
}
