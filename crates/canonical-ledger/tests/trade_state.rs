use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched, TradeParticipantRoleV1, TradeParticipantV1,
};
use canonical_ledger::{
    ApplyContext, ApplyOutcome, CanonicalLedger, CanonicalTradeReducerSetV2,
    CanonicalTradeReducerV1, CanonicalTradeReducerV2, EventReducer, LedgerLimits, ReducerError,
    StateImageLimits, StateMutation, StateView, TradeParticipantRecordV1, TradeParticipantRecordV2,
    TradeReconciliationRecordV1, TradeReconciliationRecordV2, TradeStateError, TradeStateRecordV1,
    TradeStateRecordV2,
};
use domain_types::{
    Address, BlockHeight, ChainId, ClientOrderId, KnownTime, MarketId, OrderId, PositionQuantity,
    Price, ProtocolTime, Quantity, SourceId, TradeId, TransactionId, TwapId,
};

const V1_EVENT_ID: &str = "evt_2b06fda7df986e0310c53f6394fd50caf1d11d05f594526c5413982778b9030a";
const V1_PAYLOAD_HASH: &str = "76297046b138d7b4333609e10f34b3bf8784eccc0c8bbc31a49074bfbaf0caf2";
const V1_EVIDENCE_HASH: &str = "9cf9ffaa1f59af89374a06eaa74588b10e40436dfa37f4a0efdfba324bab689c";
const V1_TRADE_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-state/v1","event_id":"evt_2b06fda7df986e0310c53f6394fd50caf1d11d05f594526c5413982778b9030a","trade_id":"trd-v1-frozen","market_id":"perp:BTC","price":"65000.000000","quantity":"0.01000000","participant_0":"0x1111111111111111111111111111111111111111","participant_1":"0x2222222222222222222222222222222222222222","block_height":950,"payload_blake3":"76297046b138d7b4333609e10f34b3bf8784eccc0c8bbc31a49074bfbaf0caf2"}"#;
const V1_BUYER_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-participant/v1","event_id":"evt_2b06fda7df986e0310c53f6394fd50caf1d11d05f594526c5413982778b9030a","trade_id":"trd-v1-frozen","ordinal":0,"participant":"0x1111111111111111111111111111111111111111","quantity":"0.01000000","block_height":950}"#;
const V1_SELLER_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-participant/v1","event_id":"evt_2b06fda7df986e0310c53f6394fd50caf1d11d05f594526c5413982778b9030a","trade_id":"trd-v1-frozen","ordinal":1,"participant":"0x2222222222222222222222222222222222222222","quantity":"0.01000000","block_height":950}"#;
const V1_RECONCILIATION_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-quantity-symmetry/v1","check_version":"trade-quantity-symmetry@1.0.0","status":"passed","event_id":"evt_2b06fda7df986e0310c53f6394fd50caf1d11d05f594526c5413982778b9030a","trade_id":"trd-v1-frozen","market_id":"perp:BTC","quantity":"0.01000000","participant_count":2,"block_height":950,"evidence_blake3":"9cf9ffaa1f59af89374a06eaa74588b10e40436dfa37f4a0efdfba324bab689c"}"#;
const V2_EVENT_ID: &str = "evt_713654a539015bd03eaf460f17ba6fbf8c4cc6d3642d6afd1714e8b64dd9fafc";
const V2_PAYLOAD_HASH: &str = "ed63f2a04709ced342b324054881a0fd230c7458c13ff4d76a6e09783f239f29";
const V2_EVIDENCE_HASH: &str = "348706efd6f856ab1f4d23f5c2d9e62a2b30e77b838d104202abdc53554c42e2";
const V2_TRADE_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-state/v2","event_id":"evt_713654a539015bd03eaf460f17ba6fbf8c4cc6d3642d6afd1714e8b64dd9fafc","trade_id":"trd-v2-frozen","market_id":"perp:BTC","price":"65000.000000","quantity":"0.01000000","buyer_account_id":"0x1111111111111111111111111111111111111111","seller_account_id":"0x2222222222222222222222222222222222222222","buyer_start_position":"1.25000000","seller_start_position":"-2.50000000","buyer_order_id":"buyer-order-960","seller_order_id":"seller-order-960","buyer_twap_id":91,"seller_twap_id":null,"buyer_client_order_id":"0x11111111111111111111111111111111","seller_client_order_id":null,"block_height":960,"payload_blake3":"ed63f2a04709ced342b324054881a0fd230c7458c13ff4d76a6e09783f239f29"}"#;
const V2_BUYER_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-participant/v2","event_id":"evt_713654a539015bd03eaf460f17ba6fbf8c4cc6d3642d6afd1714e8b64dd9fafc","trade_id":"trd-v2-frozen","ordinal":0,"role":"buyer","account_id":"0x1111111111111111111111111111111111111111","start_position":"1.25000000","order_id":"buyer-order-960","twap_id":91,"client_order_id":"0x11111111111111111111111111111111","fill_quantity":"0.01000000","position_effect":"0.01000000","block_height":960}"#;
const V2_SELLER_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-participant/v2","event_id":"evt_713654a539015bd03eaf460f17ba6fbf8c4cc6d3642d6afd1714e8b64dd9fafc","trade_id":"trd-v2-frozen","ordinal":1,"role":"seller","account_id":"0x2222222222222222222222222222222222222222","start_position":"-2.50000000","order_id":"seller-order-960","twap_id":null,"client_order_id":null,"fill_quantity":"0.01000000","position_effect":"-0.01000000","block_height":960}"#;
const V2_RECONCILIATION_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/trade-reconciliation/v2","check_version":"trade-position-symmetry@2.0.0","status":"passed","event_id":"evt_713654a539015bd03eaf460f17ba6fbf8c4cc6d3642d6afd1714e8b64dd9fafc","trade_id":"trd-v2-frozen","market_id":"perp:BTC","absolute_quantity":"0.01000000","buyer_effect":"0.01000000","seller_effect":"-0.01000000","participant_count":2,"block_height":960,"evidence_blake3":"348706efd6f856ab1f4d23f5c2d9e62a2b30e77b838d104202abdc53554c42e2"}"#;

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

#[test]
fn enriched_trade_v2_retains_exact_anchors_and_opposite_signed_effects() {
    let event = enriched_trade_event(500, 0, "trd-v2-500");
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(500),
        CanonicalTradeReducerV2,
        LedgerLimits::production(),
    )
    .unwrap();

    let ApplyOutcome::Applied(delta) = ledger
        .apply_block(&block(500, vec![event.clone()]))
        .unwrap()
    else {
        panic!("enriched trade must apply");
    };
    assert_eq!(delta.mutations().len(), 4);
    assert_eq!(
        ledger.state_image().reducer_set_version(),
        "hyperliquid-alpha-desk-canonical-trade@2.0.0"
    );

    let trade_id = TradeId::new("trd-v2-500").unwrap();
    let entries = ledger.state_image().entries();
    let trade_key = TradeStateRecordV2::state_key(&trade_id).unwrap();
    let trade =
        TradeStateRecordV2::decode_at(&trade_key, entries.get(&trade_key).unwrap()).unwrap();
    assert_eq!(trade.event_id(), event.event_id());
    assert_eq!(trade.trade_id(), &trade_id);
    assert_eq!(trade.market_id().as_str(), "perp:BTC");
    assert_eq!(trade.price().to_string(), "65000.000000");
    assert_eq!(trade.quantity().to_string(), "0.01000000");
    assert_eq!(trade.buyer_account_id(), Address::from_bytes([0x11; 20]));
    assert_eq!(trade.seller_account_id(), Address::from_bytes([0x22; 20]));
    assert_eq!(trade.buyer_start_position().to_string(), "1.25000000");
    assert_eq!(trade.seller_start_position().to_string(), "-2.50000000");
    assert_eq!(trade.buyer_order_id().as_str(), "buyer-order-500");
    assert_eq!(trade.seller_order_id().as_str(), "seller-order-500");
    assert_eq!(trade.buyer_twap_id(), Some(TwapId::new(91)));
    assert_eq!(trade.seller_twap_id(), None);
    assert_eq!(
        trade.buyer_client_order_id().unwrap().as_str(),
        "0x11111111111111111111111111111111"
    );
    assert_eq!(trade.seller_client_order_id(), None);
    assert_eq!(trade.block_height(), BlockHeight::new(500));
    assert_eq!(trade.payload_hash(), event.payload_hash());

    for (ordinal, role, account, start, order, twap, cloid, effect) in [
        (
            0,
            TradeParticipantRoleV1::Buyer,
            Address::from_bytes([0x11; 20]),
            "1.25000000",
            "buyer-order-500",
            Some(TwapId::new(91)),
            Some("0x11111111111111111111111111111111"),
            "0.01000000",
        ),
        (
            1,
            TradeParticipantRoleV1::Seller,
            Address::from_bytes([0x22; 20]),
            "-2.50000000",
            "seller-order-500",
            None,
            None,
            "-0.01000000",
        ),
    ] {
        let key = TradeParticipantRecordV2::state_key(&trade_id, ordinal).unwrap();
        let participant =
            TradeParticipantRecordV2::decode_at(&key, entries.get(&key).unwrap()).unwrap();
        assert_eq!(participant.ordinal(), ordinal);
        assert_eq!(participant.role(), role);
        assert_eq!(participant.account_id(), account);
        assert_eq!(participant.start_position().to_string(), start);
        assert_eq!(participant.order_id().as_str(), order);
        assert_eq!(participant.twap_id(), twap);
        assert_eq!(
            participant.client_order_id().map(ClientOrderId::as_str),
            cloid
        );
        assert_eq!(participant.position_effect().to_string(), effect);
    }

    let reconciliation_key = TradeReconciliationRecordV2::state_key(&trade_id).unwrap();
    let reconciliation = TradeReconciliationRecordV2::decode_at(
        &reconciliation_key,
        entries.get(&reconciliation_key).unwrap(),
    )
    .unwrap();
    assert!(reconciliation.passed());
    assert_eq!(reconciliation.trade_id(), &trade_id);
    assert_eq!(reconciliation.absolute_quantity().to_string(), "0.01000000");
    assert_eq!(reconciliation.buyer_effect().to_string(), "0.01000000");
    assert_eq!(reconciliation.seller_effect().to_string(), "-0.01000000");
    assert_eq!(reconciliation.participant_count(), 2);
    assert_eq!(reconciliation.block_height(), BlockHeight::new(500));
}

#[test]
fn trade_reducer_set_preserves_v1_facts_and_adds_v2_only_for_enriched_trades() {
    let legacy = trade_event(
        600,
        0,
        "trd-legacy-600",
        MarketId::new("perp:BTC").unwrap(),
        [0x11; 20],
        [0x22; 20],
    );
    let enriched = enriched_trade_event(601, 0, "trd-enriched-601");
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(600),
        CanonicalTradeReducerSetV2,
        LedgerLimits::production(),
    )
    .unwrap();

    let ApplyOutcome::Applied(legacy_delta) = ledger
        .apply_block(&block(600, vec![legacy.clone()]))
        .unwrap()
    else {
        panic!("legacy trade must apply through V1");
    };
    assert_eq!(legacy_delta.mutations().len(), 4);
    let legacy_id = TradeId::new("trd-legacy-600").unwrap();
    assert!(
        ledger
            .state_image()
            .entries()
            .contains_key(&TradeStateRecordV1::state_key(&legacy_id).unwrap())
    );
    assert!(
        !ledger
            .state_image()
            .entries()
            .contains_key(&TradeStateRecordV2::state_key(&legacy_id).unwrap())
    );

    let ApplyOutcome::Applied(enriched_delta) = ledger
        .apply_block(&block(601, vec![enriched.clone()]))
        .unwrap()
    else {
        panic!("enriched trade must apply through V1 and V2");
    };
    assert_eq!(enriched_delta.mutations().len(), 8);
    assert_eq!(
        ledger.state_image().reducer_set_version(),
        "hyperliquid-alpha-desk-canonical-trade-set@2.0.0"
    );
    let enriched_id = TradeId::new("trd-enriched-601").unwrap();
    let v1_key = TradeStateRecordV1::state_key(&enriched_id).unwrap();
    let v2_key = TradeStateRecordV2::state_key(&enriched_id).unwrap();
    assert_eq!(
        TradeStateRecordV1::decode_at(
            &v1_key,
            ledger.state_image().entries().get(&v1_key).unwrap()
        )
        .unwrap()
        .payload_hash(),
        enriched.payload_hash()
    );
    assert_eq!(
        TradeStateRecordV2::decode_at(
            &v2_key,
            ledger.state_image().entries().get(&v2_key).unwrap()
        )
        .unwrap()
        .payload_hash(),
        enriched.payload_hash()
    );

    let frozen_v1 = TradeStateRecordV1::decode_at(
        &TradeStateRecordV1::state_key(&legacy_id).unwrap(),
        ledger
            .state_image()
            .entries()
            .get(&TradeStateRecordV1::state_key(&legacy_id).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(frozen_v1.participants()[0], Address::from_bytes([0x11; 20]));
    assert_eq!(frozen_v1.participants()[1], Address::from_bytes([0x22; 20]));
}

#[test]
fn v2_support_is_enriched_only_and_v1_checkpoints_do_not_restore_as_v2_or_set() {
    let legacy = trade_event(
        700,
        0,
        "trd-legacy-700",
        MarketId::new("perp:BTC").unwrap(),
        [0x11; 20],
        [0x22; 20],
    );
    let enriched = enriched_trade_event(700, 0, "trd-enriched-700");
    assert!(!CanonicalTradeReducerV2.supports(&legacy));
    assert!(CanonicalTradeReducerV2.supports(&enriched));
    assert!(CanonicalTradeReducerSetV2.supports(&legacy));
    assert!(CanonicalTradeReducerSetV2.supports(&enriched));

    let mut v1 = ledger(700);
    v1.apply_block(&block(700, vec![legacy])).unwrap();
    let restored = canonical_ledger::StateImage::decode_canonical(
        &v1.state_image().canonical_bytes(),
        StateImageLimits::production(),
    )
    .unwrap();
    let v2_error = CanonicalLedger::try_from_state_image(
        restored.clone(),
        CanonicalTradeReducerV2,
        LedgerLimits::production(),
    )
    .expect_err("V1 checkpoint must not restore under V2");
    assert_eq!(v2_error.reason_code(), "ledger.reducer_version_drift");
    let set_error = CanonicalLedger::try_from_state_image(
        restored,
        CanonicalTradeReducerSetV2,
        LedgerLimits::production(),
    )
    .expect_err("V1 checkpoint must not restore under the V2 reducer set");
    assert_eq!(set_error.reason_code(), "ledger.reducer_version_drift");

    let mut v2 = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(700),
        CanonicalTradeReducerV2,
        LedgerLimits::production(),
    )
    .unwrap();
    v2.apply_block(&block(700, vec![enriched])).unwrap();
    let restored_v2 = canonical_ledger::StateImage::decode_canonical(
        &v2.state_image().canonical_bytes(),
        StateImageLimits::production(),
    )
    .unwrap();
    let set_error = CanonicalLedger::try_from_state_image(
        restored_v2,
        CanonicalTradeReducerSetV2,
        LedgerLimits::production(),
    )
    .expect_err("V2 component checkpoint must not restore under the reducer set");
    assert_eq!(set_error.reason_code(), "ledger.reducer_version_drift");
}

#[test]
fn v2_codecs_reject_corrupt_noncanonical_and_key_mismatched_records() {
    let event = enriched_trade_event(800, 0, "trd-codec-v2");
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(800),
        CanonicalTradeReducerV2,
        LedgerLimits::production(),
    )
    .unwrap();
    ledger.apply_block(&block(800, vec![event])).unwrap();
    let trade_id = TradeId::new("trd-codec-v2").unwrap();
    let entries = ledger.state_image().entries();

    let trade_key = TradeStateRecordV2::state_key(&trade_id).unwrap();
    let trade_bytes = entries.get(&trade_key).unwrap();
    assert_v2_codec_boundaries(
        trade_bytes,
        "hyperliquid-alpha-desk/trade-state/v2",
        TradeStateRecordV2::decode,
    );
    let wrong_trade_key =
        TradeStateRecordV2::state_key(&TradeId::new("trd-other-v2").unwrap()).unwrap();
    assert_eq!(
        TradeStateRecordV2::decode_at(&wrong_trade_key, trade_bytes)
            .unwrap_err()
            .reason_code(),
        "trade_state.codec.key_mismatch"
    );

    let participant_key = TradeParticipantRecordV2::state_key(&trade_id, 0).unwrap();
    let participant_bytes = entries.get(&participant_key).unwrap();
    assert_v2_codec_boundaries(
        participant_bytes,
        "hyperliquid-alpha-desk/trade-participant/v2",
        TradeParticipantRecordV2::decode,
    );
    let wrong_participant_key =
        TradeParticipantRecordV2::state_key(&TradeId::new("trd-other-v2").unwrap(), 0).unwrap();
    assert_eq!(
        TradeParticipantRecordV2::decode_at(&wrong_participant_key, participant_bytes)
            .unwrap_err()
            .reason_code(),
        "trade_state.codec.key_mismatch"
    );
    assert!(TradeParticipantRecordV2::state_key(&trade_id, 2).is_err());

    let reconciliation_key = TradeReconciliationRecordV2::state_key(&trade_id).unwrap();
    let reconciliation_bytes = entries.get(&reconciliation_key).unwrap();
    assert_v2_codec_boundaries(
        reconciliation_bytes,
        "hyperliquid-alpha-desk/trade-reconciliation/v2",
        TradeReconciliationRecordV2::decode,
    );
    let wrong_reconciliation_key =
        TradeReconciliationRecordV2::state_key(&TradeId::new("trd-other-v2").unwrap()).unwrap();
    assert_eq!(
        TradeReconciliationRecordV2::decode_at(&wrong_reconciliation_key, reconciliation_bytes)
            .unwrap_err()
            .reason_code(),
        "trade_state.codec.key_mismatch"
    );
}

#[test]
fn v2_trade_identity_collision_rolls_back_without_replacing_prior_facts() {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(900),
        CanonicalTradeReducerV2,
        LedgerLimits::production(),
    )
    .unwrap();
    ledger
        .apply_block(&block(
            900,
            vec![enriched_trade_event(900, 0, "trd-v2-collision")],
        ))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&block(
            901,
            vec![enriched_trade_event(901, 0, "trd-v2-collision")],
        ))
        .expect_err("V2 trade identity collision");
    assert_eq!(
        error.reducer_reason_code(),
        Some("trade_state.trade_id_collision")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn corrupt_or_key_mismatched_prior_v2_fact_rejects_without_replacing_state() {
    let target_id = TradeId::new("trd-v2-prior").unwrap();
    let target_key = TradeStateRecordV2::state_key(&target_id).unwrap();

    let mut source = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(910),
        CanonicalTradeReducerV2,
        LedgerLimits::production(),
    )
    .unwrap();
    source
        .apply_block(&block(
            910,
            vec![enriched_trade_event(910, 0, "trd-v2-wrong-identity")],
        ))
        .unwrap();
    source
        .apply_block(&block(
            911,
            vec![enriched_trade_event(911, 0, target_id.as_str())],
        ))
        .unwrap();
    let wrong_id = TradeId::new("trd-v2-wrong-identity").unwrap();
    let wrong_bytes = source
        .state_image()
        .entries()
        .get(&TradeStateRecordV2::state_key(&wrong_id).unwrap())
        .unwrap()
        .clone();
    let valid_trade_bytes = source
        .state_image()
        .entries()
        .get(&target_key)
        .unwrap()
        .clone();
    let reconciliation_key = TradeReconciliationRecordV2::state_key(&target_id).unwrap();

    for (height, mutations) in [
        (
            920,
            vec![StateMutation::put(target_key.clone(), b"corrupt".to_vec())],
        ),
        (
            930,
            vec![StateMutation::put(target_key.clone(), wrong_bytes)],
        ),
        (
            940,
            vec![
                StateMutation::put(target_key.clone(), valid_trade_bytes),
                StateMutation::put(reconciliation_key.clone(), b"corrupt".to_vec()),
            ],
        ),
    ] {
        let reducer = V2InjectionReducer {
            injection_height: BlockHeight::new(height),
            mutations,
        };
        let mut ledger = CanonicalLedger::try_new(
            ChainId::new("mainnet").unwrap(),
            BlockHeight::new(height),
            reducer,
            LedgerLimits::production(),
        )
        .unwrap();
        ledger
            .apply_block(&block(
                height,
                vec![enriched_trade_event(height, 0, "trd-injection-trigger")],
            ))
            .unwrap();
        let before = ledger.state_image().canonical_bytes();

        let error = ledger
            .apply_block(&block(
                height + 1,
                vec![enriched_trade_event(height + 1, 0, target_id.as_str())],
            ))
            .expect_err("invalid prior V2 fact must fail closed");
        assert_eq!(
            error.reducer_reason_code(),
            Some("trade_state.prior_fact_invalid")
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

#[test]
fn every_v1_trade_value_remains_frozen_as_literal_bytes() {
    let event = trade_event(
        950,
        0,
        "trd-v1-frozen",
        MarketId::new("perp:BTC").unwrap(),
        [0x11; 20],
        [0x22; 20],
    );
    let mut ledger = ledger(950);
    ledger
        .apply_block(&block(950, vec![event.clone()]))
        .unwrap();
    let trade_id = TradeId::new("trd-v1-frozen").unwrap();
    let entries = ledger.state_image().entries();
    let trade_key = TradeStateRecordV1::state_key(&trade_id).unwrap();
    let buyer_key = TradeParticipantRecordV1::state_key(&trade_id, 0).unwrap();
    let seller_key = TradeParticipantRecordV1::state_key(&trade_id, 1).unwrap();
    let reconciliation_key = TradeReconciliationRecordV1::state_key(&trade_id).unwrap();

    assert_eq!(entries.get(&trade_key).unwrap(), V1_TRADE_GOLDEN);
    assert_eq!(entries.get(&buyer_key).unwrap(), V1_BUYER_GOLDEN);
    assert_eq!(entries.get(&seller_key).unwrap(), V1_SELLER_GOLDEN);
    assert_eq!(
        entries.get(&reconciliation_key).unwrap(),
        V1_RECONCILIATION_GOLDEN
    );

    let trade = TradeStateRecordV1::decode_at(&trade_key, V1_TRADE_GOLDEN).unwrap();
    assert_eq!(trade.event_id().as_str(), V1_EVENT_ID);
    assert_eq!(trade.trade_id().as_str(), "trd-v1-frozen");
    assert_eq!(trade.market_id().as_str(), "perp:BTC");
    assert_eq!(trade.price().to_string(), "65000.000000");
    assert_eq!(trade.quantity().to_string(), "0.01000000");
    assert_eq!(
        trade.participants(),
        [
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20])
        ]
    );
    assert_eq!(trade.block_height(), BlockHeight::new(950));
    assert_eq!(hex::encode(trade.payload_hash()), V1_PAYLOAD_HASH);

    for (ordinal, literal, participant) in [
        (0, V1_BUYER_GOLDEN, Address::from_bytes([0x11; 20])),
        (1, V1_SELLER_GOLDEN, Address::from_bytes([0x22; 20])),
    ] {
        let key = TradeParticipantRecordV1::state_key(&trade_id, ordinal).unwrap();
        let record = TradeParticipantRecordV1::decode_at(&key, literal).unwrap();
        assert_eq!(record.event_id().as_str(), V1_EVENT_ID);
        assert_eq!(record.trade_id(), &trade_id);
        assert_eq!(record.ordinal(), ordinal);
        assert_eq!(record.participant(), participant);
        assert_eq!(record.quantity().to_string(), "0.01000000");
        assert_eq!(record.block_height(), BlockHeight::new(950));
    }

    let reconciliation =
        TradeReconciliationRecordV1::decode_at(&reconciliation_key, V1_RECONCILIATION_GOLDEN)
            .unwrap();
    assert!(reconciliation.passed());
    assert_eq!(reconciliation.event_id().as_str(), V1_EVENT_ID);
    assert_eq!(reconciliation.trade_id(), &trade_id);
    assert_eq!(reconciliation.market_id().as_str(), "perp:BTC");
    assert_eq!(reconciliation.quantity().to_string(), "0.01000000");
    assert_eq!(reconciliation.participant_count(), 2);
    assert_eq!(reconciliation.block_height(), BlockHeight::new(950));
    assert_eq!(
        hex::encode(reconciliation.evidence_hash()),
        V1_EVIDENCE_HASH
    );
}

#[test]
fn every_v2_trade_value_is_frozen_as_literal_bytes() {
    let event = enriched_trade_event(960, 0, "trd-v2-frozen");
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(960),
        CanonicalTradeReducerV2,
        LedgerLimits::production(),
    )
    .unwrap();
    ledger.apply_block(&block(960, vec![event])).unwrap();
    let trade_id = TradeId::new("trd-v2-frozen").unwrap();
    let entries = ledger.state_image().entries();
    let trade_key = TradeStateRecordV2::state_key(&trade_id).unwrap();
    let buyer_key = TradeParticipantRecordV2::state_key(&trade_id, 0).unwrap();
    let seller_key = TradeParticipantRecordV2::state_key(&trade_id, 1).unwrap();
    let reconciliation_key = TradeReconciliationRecordV2::state_key(&trade_id).unwrap();

    assert_eq!(entries.get(&trade_key).unwrap(), V2_TRADE_GOLDEN);
    assert_eq!(entries.get(&buyer_key).unwrap(), V2_BUYER_GOLDEN);
    assert_eq!(entries.get(&seller_key).unwrap(), V2_SELLER_GOLDEN);
    assert_eq!(
        entries.get(&reconciliation_key).unwrap(),
        V2_RECONCILIATION_GOLDEN
    );

    let trade = TradeStateRecordV2::decode_at(&trade_key, V2_TRADE_GOLDEN).unwrap();
    assert_eq!(trade.event_id().as_str(), V2_EVENT_ID);
    assert_eq!(trade.trade_id(), &trade_id);
    assert_eq!(trade.market_id().as_str(), "perp:BTC");
    assert_eq!(trade.price().to_string(), "65000.000000");
    assert_eq!(trade.quantity().to_string(), "0.01000000");
    assert_eq!(trade.buyer_account_id(), Address::from_bytes([0x11; 20]));
    assert_eq!(trade.seller_account_id(), Address::from_bytes([0x22; 20]));
    assert_eq!(trade.buyer_start_position().to_string(), "1.25000000");
    assert_eq!(trade.seller_start_position().to_string(), "-2.50000000");
    assert_eq!(trade.buyer_order_id().as_str(), "buyer-order-960");
    assert_eq!(trade.seller_order_id().as_str(), "seller-order-960");
    assert_eq!(trade.buyer_twap_id(), Some(TwapId::new(91)));
    assert_eq!(trade.seller_twap_id(), None);
    assert_eq!(
        trade.buyer_client_order_id().unwrap().as_str(),
        "0x11111111111111111111111111111111"
    );
    assert_eq!(trade.seller_client_order_id(), None);
    assert_eq!(trade.block_height(), BlockHeight::new(960));
    assert_eq!(hex::encode(trade.payload_hash()), V2_PAYLOAD_HASH);

    for (ordinal, literal, role, account, start, order, twap, cloid, effect) in [
        (
            0,
            V2_BUYER_GOLDEN,
            TradeParticipantRoleV1::Buyer,
            Address::from_bytes([0x11; 20]),
            "1.25000000",
            "buyer-order-960",
            Some(TwapId::new(91)),
            Some("0x11111111111111111111111111111111"),
            "0.01000000",
        ),
        (
            1,
            V2_SELLER_GOLDEN,
            TradeParticipantRoleV1::Seller,
            Address::from_bytes([0x22; 20]),
            "-2.50000000",
            "seller-order-960",
            None,
            None,
            "-0.01000000",
        ),
    ] {
        let key = TradeParticipantRecordV2::state_key(&trade_id, ordinal).unwrap();
        let participant = TradeParticipantRecordV2::decode_at(&key, literal).unwrap();
        assert_eq!(participant.event_id().as_str(), V2_EVENT_ID);
        assert_eq!(participant.trade_id(), &trade_id);
        assert_eq!(participant.ordinal(), ordinal);
        assert_eq!(participant.role(), role);
        assert_eq!(participant.account_id(), account);
        assert_eq!(participant.start_position().to_string(), start);
        assert_eq!(participant.order_id().as_str(), order);
        assert_eq!(participant.twap_id(), twap);
        assert_eq!(
            participant.client_order_id().map(ClientOrderId::as_str),
            cloid
        );
        assert_eq!(participant.fill_quantity().to_string(), "0.01000000");
        assert_eq!(participant.position_effect().to_string(), effect);
        assert_eq!(participant.block_height(), BlockHeight::new(960));
    }

    let reconciliation =
        TradeReconciliationRecordV2::decode_at(&reconciliation_key, V2_RECONCILIATION_GOLDEN)
            .unwrap();
    assert!(reconciliation.passed());
    assert_eq!(reconciliation.event_id().as_str(), V2_EVENT_ID);
    assert_eq!(reconciliation.trade_id(), &trade_id);
    assert_eq!(reconciliation.market_id().as_str(), "perp:BTC");
    assert_eq!(reconciliation.absolute_quantity().to_string(), "0.01000000");
    assert_eq!(reconciliation.buyer_effect().to_string(), "0.01000000");
    assert_eq!(reconciliation.seller_effect().to_string(), "-0.01000000");
    assert_eq!(reconciliation.participant_count(), 2);
    assert_eq!(reconciliation.block_height(), BlockHeight::new(960));
    assert_eq!(
        hex::encode(reconciliation.evidence_hash()),
        V2_EVIDENCE_HASH
    );
}

fn assert_v2_codec_boundaries<T: std::fmt::Debug>(
    encoded: &[u8],
    schema: &str,
    decode: fn(&[u8]) -> Result<T, TradeStateError>,
) {
    let truncated = &encoded[..encoded.len() - 1];
    assert_eq!(
        decode(truncated).unwrap_err().reason_code(),
        "trade_state.codec.decode"
    );
    let noncanonical = [encoded, b" "].concat();
    assert_eq!(
        decode(&noncanonical).unwrap_err().reason_code(),
        "trade_state.codec.noncanonical"
    );
    let unknown = [&encoded[..encoded.len() - 1], br#","unknown":true}"#].concat();
    assert_eq!(
        decode(&unknown).unwrap_err().reason_code(),
        "trade_state.codec.decode"
    );
    let duplicate_schema = [
        &encoded[..encoded.len() - 1],
        br#","schema":""#,
        schema.as_bytes(),
        br#""}"#,
    ]
    .concat();
    assert_eq!(
        decode(&duplicate_schema).unwrap_err().reason_code(),
        "trade_state.codec.decode"
    );

    let exact_limit = vec![b' '; 16 * 1024];
    assert_ne!(
        decode(&exact_limit).unwrap_err().reason_code(),
        "trade_state.codec.limit_exceeded"
    );
    let over_limit = vec![b' '; 16 * 1024 + 1];
    assert_eq!(
        decode(&over_limit).unwrap_err().reason_code(),
        "trade_state.codec.limit_exceeded"
    );
}

#[derive(Debug, Clone)]
struct V2InjectionReducer {
    injection_height: BlockHeight,
    mutations: Vec<StateMutation>,
}

impl EventReducer for V2InjectionReducer {
    fn reducer_set_version(&self) -> &str {
        "trade-v2-prior-fact-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        CanonicalTradeReducerV2.supports(event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if context.block_height() == self.injection_height {
            Ok(self.mutations.clone())
        } else {
            CanonicalTradeReducerV2.reduce(state, event, context)
        }
    }
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

fn enriched_trade_event(height: u64, event_index: u32, trade_id: &str) -> CanonicalEventEnvelope {
    let market = MarketId::new("perp:BTC").unwrap();
    let buyer = Address::from_bytes([0x11; 20]);
    let seller = Address::from_bytes([0x22; 20]);
    let payload = EventPayload::TradeMatched(TradeMatched {
        trade_id: Some(TradeId::new(trade_id).unwrap()),
        market_id: Some(market.clone()),
        maker_order_id: None,
        taker_order_id: None,
        price: Price::parse_at_scale("65000", 6).unwrap(),
        quantity: Quantity::parse_at_scale("0.01", 8).unwrap(),
        deterministic_seed: height,
        participants: Some(Box::new([
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Buyer,
                account_id: buyer,
                start_position: PositionQuantity::parse_at_scale("1.25", 8).unwrap(),
                order_id: OrderId::new(format!("buyer-order-{height}")).unwrap(),
                twap_id: Some(TwapId::new(91)),
                client_order_id: Some(
                    ClientOrderId::new("0x11111111111111111111111111111111").unwrap(),
                ),
            },
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Seller,
                account_id: seller,
                start_position: PositionQuantity::parse_at_scale("-2.5", 8).unwrap(),
                order_id: OrderId::new(format!("seller-order-{height}")).unwrap(),
                twap_id: None,
                client_order_id: None,
            },
        ])),
    });
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}")).unwrap(),
        transaction_index: 0,
        canonical_event_index: event_index,
        market_ids: vec![market],
        account_ids: vec![buyer, seller],
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").unwrap(),
                "synthetic-v2",
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
        parser_version: "test-parser-v2".to_owned(),
        payload,
    })
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
