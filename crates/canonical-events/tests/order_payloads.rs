use api_contracts::{
    WireCanonicalEventEnvelope, WireOrderAccepted, WireOrderModified, WireOrderRested,
    encode_order_accepted, encode_order_modified, encode_order_rested,
};
use canonical_events::{
    CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, ContractError, EventKind,
    EventPayload, OrderAccepted, OrderModified, OrderRested,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, OrderId, OrderSide, Price, ProtocolTime,
    Quantity, SourceId, TransactionId,
};

#[test]
fn admission_and_modification_payloads_decode_to_exact_domain_values() {
    let account = Address::from_bytes([0x11; 20]);
    let accepted_bytes = encode_order_accepted(&WireOrderAccepted {
        order_id: "order-17".to_owned(),
        account_id: account.to_api_string(),
        market_id: "perp:BTC".to_owned(),
        side: "buy".to_owned(),
        limit_price: "65000.125000".to_owned(),
        quantity: "0.75000000".to_owned(),
    })
    .unwrap();
    let accepted = EventPayload::decode(EventKind::OrderAccepted, &accepted_bytes).unwrap();
    assert_eq!(
        accepted,
        EventPayload::OrderAccepted(OrderAccepted {
            order_id: OrderId::new("order-17").unwrap(),
            account_id: account,
            market_id: MarketId::new("perp:BTC").unwrap(),
            side: OrderSide::Buy,
            limit_price: Price::parse_at_scale("65000.125", 6).unwrap(),
            quantity: Quantity::parse_at_scale("0.75", 8).unwrap(),
        })
    );
    assert_eq!(accepted.encode_to_vec().unwrap(), accepted_bytes);

    let rested_bytes = encode_order_rested(&WireOrderRested {
        order_id: "order-17".to_owned(),
        market_id: "perp:BTC".to_owned(),
        remaining_quantity: "0.50000000".to_owned(),
        limit_price: "65000.125000".to_owned(),
    })
    .unwrap();
    assert_eq!(
        EventPayload::decode(EventKind::OrderRested, &rested_bytes).unwrap(),
        EventPayload::OrderRested(OrderRested {
            order_id: OrderId::new("order-17").unwrap(),
            market_id: MarketId::new("perp:BTC").unwrap(),
            remaining_quantity: Quantity::parse_at_scale("0.5", 8).unwrap(),
            limit_price: Price::parse_at_scale("65000.125", 6).unwrap(),
        })
    );

    let modified_bytes = encode_order_modified(&WireOrderModified {
        order_id: "order-17".to_owned(),
        previous_price: "65000.125000".to_owned(),
        new_price: "65001.000000".to_owned(),
        previous_quantity: "0.50000000".to_owned(),
        new_quantity: "0.25000000".to_owned(),
    })
    .unwrap();
    assert_eq!(
        EventPayload::decode(EventKind::OrderModified, &modified_bytes).unwrap(),
        EventPayload::OrderModified(OrderModified {
            order_id: OrderId::new("order-17").unwrap(),
            previous_price: Price::parse_at_scale("65000.125", 6).unwrap(),
            new_price: Price::parse_at_scale("65001", 6).unwrap(),
            previous_quantity: Quantity::parse_at_scale("0.5", 8).unwrap(),
            new_quantity: Quantity::parse_at_scale("0.25", 8).unwrap(),
        })
    );
}

#[test]
fn typed_order_payloads_reject_invalid_semantics() {
    let account = Address::from_bytes([0x11; 20]).to_api_string();
    for (field, value) in [
        ("side", "long"),
        ("side", "Buy"),
        ("limit_price", "0"),
        ("limit_price", "-1"),
        ("quantity", "0"),
        ("quantity", "-0.01"),
    ] {
        let mut wire = WireOrderAccepted {
            order_id: "order-17".to_owned(),
            account_id: account.clone(),
            market_id: "perp:BTC".to_owned(),
            side: "buy".to_owned(),
            limit_price: "65000.125000".to_owned(),
            quantity: "0.75000000".to_owned(),
        };
        match field {
            "side" => wire.side = value.to_owned(),
            "limit_price" => wire.limit_price = value.to_owned(),
            "quantity" => wire.quantity = value.to_owned(),
            _ => unreachable!(),
        }
        let bytes = encode_order_accepted(&wire).unwrap();
        assert!(matches!(
            EventPayload::decode(EventKind::OrderAccepted, &bytes),
            Err(ContractError::Invalid {
                field: "payload",
                ..
            })
        ));
    }

    let unchanged = encode_order_modified(&WireOrderModified {
        order_id: "order-17".to_owned(),
        previous_price: "65000.125000".to_owned(),
        new_price: "65000.125000".to_owned(),
        previous_quantity: "0.50000000".to_owned(),
        new_quantity: "0.50000000".to_owned(),
    })
    .unwrap();
    assert!(EventPayload::decode(EventKind::OrderModified, &unchanged).is_err());
}

#[test]
fn enclosing_order_event_preserves_valid_forward_compatible_payload_bytes() {
    let account = Address::from_bytes([0x11; 20]);
    let payload = EventPayload::OrderAccepted(OrderAccepted {
        order_id: OrderId::new("order-17").unwrap(),
        account_id: account,
        market_id: MarketId::new("perp:BTC").unwrap(),
        side: OrderSide::Buy,
        limit_price: Price::parse_at_scale("65000.125", 6).unwrap(),
        quantity: Quantity::parse_at_scale("0.75", 8).unwrap(),
    });
    let time = ProtocolTime::from_unix_micros(1_721_779_200_000_042).unwrap();
    let envelope = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(42),
        block_time: time,
        transaction_id: TransactionId::new("order-tx-42").unwrap(),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC").unwrap()],
        account_ids: vec![account],
        source_evidence: vec![
            canonical_events::SourceEvidence::try_new(
                SourceId::new("order-test").unwrap(),
                "v1",
                "42",
                [0x42; 32],
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        ingested_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        parser_version: "order-test-v1".to_owned(),
        payload,
    })
    .unwrap();
    let mut wire = WireCanonicalEventEnvelope::decode(&envelope.encode_to_vec().unwrap()).unwrap();
    wire.payload = append_varint_field(wire.payload, 100, 1);
    wire.payload_hash = blake3::hash(&wire.payload).as_bytes().to_vec();

    let decoded = CanonicalEventEnvelope::decode(&wire.encode_to_vec()).unwrap();
    let reencoded = WireCanonicalEventEnvelope::decode(&decoded.encode_to_vec().unwrap()).unwrap();
    assert_eq!(reencoded.payload, wire.payload);
}

fn append_varint_field(mut encoded: Vec<u8>, field: u64, value: u64) -> Vec<u8> {
    append_varint(&mut encoded, field << 3);
    append_varint(&mut encoded, value);
    encoded
}

fn append_varint(encoded: &mut Vec<u8>, mut value: u64) {
    while value >= 0x80 {
        encoded.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    encoded.push(value as u8);
}
