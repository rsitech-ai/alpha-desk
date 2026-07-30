use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    ApplyOutcome, CanonicalLedger, CanonicalTradeReducerV1, LedgerLimits, TradeParticipantRecordV1,
    TradeReconciliationRecordV1, TradeStateRecordV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TradeId, TransactionId,
};

#[test]
fn exact_trade_creates_one_fact_two_ordinal_legs_and_stored_reconciliation() {
    let event = trade_event(
        100,
        0,
        "trd-100-0",
        MarketId::new("perp:BTC").unwrap(),
        [0x11; 20],
        [0x22; 20],
    );
    let block = block(100, vec![event.clone()]);
    let mut ledger = ledger(100);

    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block).unwrap() else {
        panic!("new block must apply");
    };

    assert_eq!(delta.mutations().len(), 4);
    let trade_id = TradeId::new("trd-100-0").unwrap();
    let state = ledger.state_image().entries();
    let trade_key = TradeStateRecordV1::state_key(&trade_id).unwrap();
    let trade = TradeStateRecordV1::decode_at(&trade_key, state.get(&trade_key).unwrap()).unwrap();
    assert_eq!(trade.event_id(), event.event_id());
    assert_eq!(trade.trade_id(), &trade_id);
    assert_eq!(trade.market_id().as_str(), "perp:BTC");
    assert_eq!(trade.price().to_string(), "65000.000000");
    assert_eq!(trade.quantity().to_string(), "0.01000000");
    assert_eq!(trade.participants()[0], Address::from_bytes([0x11; 20]));
    assert_eq!(trade.participants()[1], Address::from_bytes([0x22; 20]));
    assert_eq!(trade.block_height(), BlockHeight::new(100));
    assert_eq!(trade.payload_hash(), event.payload_hash());

    for (ordinal, expected) in [(0, [0x11; 20]), (1, [0x22; 20])] {
        let leg_key =
            TradeParticipantRecordV1::state_key(&trade_id, ordinal).expect("participant key");
        let leg = TradeParticipantRecordV1::decode_at(
            &leg_key,
            state.get(&leg_key).expect("participant leg"),
        )
        .expect("participant record");
        assert_eq!(leg.ordinal(), ordinal);
        assert_eq!(leg.participant(), Address::from_bytes(expected));
        assert_eq!(leg.quantity(), trade.quantity());
    }

    let reconciliation_key =
        TradeReconciliationRecordV1::state_key(&trade_id).expect("reconciliation key");
    let reconciliation = TradeReconciliationRecordV1::decode_at(
        &reconciliation_key,
        state
            .get(&reconciliation_key)
            .expect("stored reconciliation"),
    )
    .expect("reconciliation record");
    assert!(reconciliation.passed());
    assert_eq!(reconciliation.trade_id(), &trade_id);
    assert_eq!(reconciliation.quantity(), trade.quantity());
    assert_eq!(reconciliation.participant_count(), 2);
    assert_eq!(reconciliation.block_height(), BlockHeight::new(100));
}

#[test]
fn trade_contract_failures_roll_back_the_complete_block() {
    let cases = [
        invalid_trade(101, InvalidTrade::MissingTradeId),
        invalid_trade(101, InvalidTrade::MissingPayloadMarket),
        invalid_trade(101, InvalidTrade::MismatchedMarket),
        invalid_trade(101, InvalidTrade::OneParticipant),
        invalid_trade(101, InvalidTrade::DuplicateParticipant),
    ];

    for invalid in cases {
        let valid = trade_event(
            101,
            0,
            "trd-101-0",
            MarketId::new("perp:BTC").unwrap(),
            [0x11; 20],
            [0x22; 20],
        );
        let invalid = with_event_order(invalid, 1);
        let mut ledger = ledger(100);
        ledger.apply_block(&block(100, Vec::new())).unwrap();
        let before = ledger.state_image().canonical_bytes();
        let before_hash = ledger.state_hash();

        let error = ledger
            .apply_block(&block(101, vec![valid, invalid]))
            .expect_err("invalid late trade");

        assert_eq!(error.reason_code(), "ledger.reducer_failed");
        assert!(
            error
                .reducer_reason_code()
                .is_some_and(|code| code.starts_with("trade_state."))
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
        assert_eq!(ledger.state_hash(), before_hash);
        assert_eq!(
            ledger.checkpoint().unwrap().block_height(),
            BlockHeight::new(100)
        );
    }
}

#[test]
fn duplicate_trade_identity_in_a_later_block_is_rejected_without_advancing() {
    let mut ledger = ledger(200);
    let first = trade_event(
        200,
        0,
        "trd-shared",
        MarketId::new("perp:BTC").unwrap(),
        [0x11; 20],
        [0x22; 20],
    );
    ledger.apply_block(&block(200, vec![first])).unwrap();
    let before = ledger.state_image().canonical_bytes();

    let conflicting = trade_event(
        201,
        0,
        "trd-shared",
        MarketId::new("perp:ETH").unwrap(),
        [0x33; 20],
        [0x44; 20],
    );
    let error = ledger
        .apply_block(&block(201, vec![conflicting]))
        .expect_err("trade identity collision");

    assert_eq!(
        error.reducer_reason_code(),
        Some("trade_state.trade_id_collision")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn trade_state_codecs_reject_noncanonical_corrupt_and_mismatched_values() {
    let event = trade_event(
        300,
        0,
        "trd-codec",
        MarketId::new("perp:BTC").unwrap(),
        [0x11; 20],
        [0x22; 20],
    );
    let mut ledger = ledger(300);
    ledger.apply_block(&block(300, vec![event])).unwrap();
    let trade_id = TradeId::new("trd-codec").unwrap();
    let encoded = ledger
        .state_image()
        .entries()
        .get(&TradeStateRecordV1::state_key(&trade_id).unwrap())
        .unwrap();

    for corrupt in [
        encoded[..encoded.len() - 1].to_vec(),
        [encoded.as_slice(), b" "].concat(),
        b"{}".to_vec(),
    ] {
        let error = TradeStateRecordV1::decode(&corrupt).expect_err("corrupt state value");
        assert!(error.reason_code().starts_with("trade_state.codec"));
    }

    let wrong_key = TradeStateRecordV1::state_key(&TradeId::new("trd-other").unwrap()).unwrap();
    let mismatch =
        TradeStateRecordV1::decode_at(&wrong_key, encoded).expect_err("key must bind record");
    assert_eq!(mismatch.reason_code(), "trade_state.codec.key_mismatch");
    assert!(TradeParticipantRecordV1::state_key(&trade_id, 2).is_err());
}

#[test]
fn reducer_owns_only_the_exact_trade_schema_and_empty_blocks_still_advance() {
    use canonical_ledger::EventReducer;

    let reducer = CanonicalTradeReducerV1;
    let supported = trade_event(
        400,
        0,
        "trd-400",
        MarketId::new("perp:BTC").unwrap(),
        [0x11; 20],
        [0x22; 20],
    );
    assert!(reducer.supports(&supported));

    let unsupported_schema = event_with_schema(supported, "1.1.0");
    assert!(!reducer.supports(&unsupported_schema));

    let mut ledger = ledger(400);
    ledger.apply_block(&block(400, Vec::new())).unwrap();
    assert!(ledger.state_image().entries().is_empty());
}

fn ledger(first_height: u64) -> CanonicalLedger<CanonicalTradeReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalTradeReducerV1,
        LedgerLimits::production(),
    )
    .unwrap()
}

fn block(height: u64, events: Vec<CanonicalEventEnvelope>) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).unwrap(),
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(SourceId::new("test-primary").unwrap(), [height as u8; 32])]),
    )
    .unwrap()
}

fn trade_event(
    height: u64,
    event_index: u32,
    trade_id: &str,
    market_id: MarketId,
    participant_0: [u8; 20],
    participant_1: [u8; 20],
) -> CanonicalEventEnvelope {
    build_trade(
        height,
        event_index,
        Some(TradeId::new(trade_id).unwrap()),
        Some(market_id.clone()),
        vec![market_id],
        vec![
            Address::from_bytes(participant_0),
            Address::from_bytes(participant_1),
        ],
        Quantity::parse_at_scale("0.01", 8).unwrap(),
        "1.0.0",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InvalidTrade {
    MissingTradeId,
    MissingPayloadMarket,
    MismatchedMarket,
    OneParticipant,
    DuplicateParticipant,
}

fn invalid_trade(height: u64, invalid: InvalidTrade) -> CanonicalEventEnvelope {
    let market = MarketId::new("perp:BTC").unwrap();
    let other = MarketId::new("perp:ETH").unwrap();
    let participant = Address::from_bytes([0x33; 20]);
    build_trade(
        height,
        0,
        (invalid != InvalidTrade::MissingTradeId)
            .then(|| TradeId::new(format!("trd-invalid-{invalid:?}")).unwrap()),
        match invalid {
            InvalidTrade::MissingPayloadMarket => None,
            InvalidTrade::MismatchedMarket => Some(other),
            _ => Some(market.clone()),
        },
        vec![market],
        match invalid {
            InvalidTrade::OneParticipant => vec![participant],
            InvalidTrade::DuplicateParticipant => vec![participant, participant],
            _ => vec![participant, Address::from_bytes([0x44; 20])],
        },
        Quantity::parse_at_scale("0.01", 8).unwrap(),
        "1.0.0",
    )
}

#[allow(clippy::too_many_arguments)]
fn build_trade(
    height: u64,
    event_index: u32,
    trade_id: Option<TradeId>,
    payload_market: Option<MarketId>,
    envelope_markets: Vec<MarketId>,
    participants: Vec<Address>,
    quantity: Quantity,
    schema: &str,
) -> CanonicalEventEnvelope {
    let payload = EventPayload::TradeMatched(TradeMatched {
        trade_id,
        market_id: payload_market,
        maker_order_id: None,
        taker_order_id: None,
        price: Price::parse_at_scale("65000", 6).unwrap(),
        quantity,
        deterministic_seed: 0,
        participants: None,
    });
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}")).unwrap(),
        transaction_index: 0,
        canonical_event_index: event_index,
        market_ids: envelope_markets,
        account_ids: participants,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").unwrap(),
                "v1",
                height.to_string(),
                payload_hash,
                event_index,
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        ingested_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .unwrap()
}

fn with_event_order(event: CanonicalEventEnvelope, event_index: u32) -> CanonicalEventEnvelope {
    let EventPayload::TradeMatched(trade) = event.payload().clone() else {
        unreachable!()
    };
    build_trade(
        event.block_height().get(),
        event_index,
        trade.trade_id,
        trade.market_id,
        event.market_ids().to_vec(),
        event.account_addresses().to_vec(),
        trade.quantity,
        event.schema_version(),
    )
}

fn event_with_schema(event: CanonicalEventEnvelope, schema: &str) -> CanonicalEventEnvelope {
    let EventPayload::TradeMatched(trade) = event.payload().clone() else {
        unreachable!()
    };
    build_trade(
        event.block_height().get(),
        event.canonical_event_index(),
        trade.trade_id,
        trade.market_id,
        event.market_ids().to_vec(),
        event.account_addresses().to_vec(),
        trade.quantity,
        schema,
    )
}
