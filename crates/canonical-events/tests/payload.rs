use api_contracts::{WireCanonicalEventEnvelope, WireOrderAccepted, encode_order_accepted};
use canonical_events::{
    CanonicalEventEnvelope, ConfirmationClass, ContractError, EventKind, EventPayload, TradeMatched,
};
use domain_types::{
    Address, BlockHeight, EventId, KnownTime, MarketId, Price, ProtocolTime, Quantity,
    TransactionId,
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
        EventPayload::TradeMatched(TradeMatched {
            price: Price::parse_at_scale("65000", 6).unwrap(),
            quantity: Quantity::parse_at_scale("0.01", 8).unwrap(),
            deterministic_seed: 7,
        }),
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
fn opaque_typed_payload_preserves_non_default_message_bytes_exactly() {
    let bytes = encode_order_accepted(&WireOrderAccepted {
        order_id: "order-17".to_owned(),
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        market_id: "perp:BTC".to_owned(),
        side: "buy".to_owned(),
        limit_price: "65000.125".to_owned(),
        quantity: "0.75".to_owned(),
    });

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

    let payload = EventPayload::TradeMatched(TradeMatched {
        price: Price::parse_at_scale("1", 6).unwrap(),
        quantity: Quantity::parse_at_scale("2", 8).unwrap(),
        deterministic_seed: 9,
    });
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
