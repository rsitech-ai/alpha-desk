use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use canonical_events::{
    AssetContextUpdated, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    ConfirmationClass, DexCreated, EventKind, EventPayload, MarketCreated, MarketHalted,
    MarketMetadataChanged, OrderFilled, SourceEvidence, TradeMatched, TradeParticipantRoleV1,
    TradeParticipantV1,
};
use canonical_ledger::{
    ApplyContext, ApplyOutcome, CanonicalLedger, CanonicalMarketReducerV1,
    CanonicalPositionReducerV1, CanonicalTradeReducerSetV2, EventReducer, LedgerLimits,
    PositionAnchorTransitionV1, PositionEffectFactRecordV1, PositionQuantityCurrentRecordV1,
    PositionStateError, PositionUnresolvedCauseFactRecordV1, PositionUnresolvedCauseV1,
    ReducerError, StateKey, StateMutation, StateView,
};
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, DexId, EventId, KnownTime, LiquidationId, MarketId,
    OrderId, PositionQuantity, Price, ProtocolTime, Quantity, SourceId, TradeId, TransactionId,
};

const BUYER: Address = Address::from_bytes([0x11; 20]);
const SELLER: Address = Address::from_bytes([0x22; 20]);
const OPERATOR: Address = Address::from_bytes([0x33; 20]);
const CURRENT_KEY_GOLDEN: &[u8] = &[
    0, 0, 0, 0, 0, 0, 0, 20, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
    0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0, 0, 0, 0, 0, 0, 0, 8, b'p', b'e', b'r',
    b'p', b':', b'B', b'T', b'C',
];
const EFFECT_KEY_GOLDEN: &[u8] = &[
    0, 0, 0, 0, 0, 0, 0, 7, b't', b'r', b'd', b'-', b'k', b'e', b'y', 0, 0, 0, 0, 0, 0, 0, 5, b'b',
    b'u', b'y', b'e', b'r',
];
const UNRESOLVED_KEY_GOLDEN: &[u8] = &[
    0, 0, 0, 0, 0, 0, 0, 20, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
    0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0, 0, 0, 0, 0, 0, 0, 8, b'p', b'e', b'r',
    b'p', b':', b'B', b'T', b'C', 0, 0, 0, 0, 0, 0, 0, 7, b'e', b'v', b't', b'-', b'k', b'e', b'y',
    0, 0, 0, 0, 0, 0, 0, 7, b'l', b'i', b'q', b'-', b'k', b'e', b'y',
];
const FROZEN_CURRENT_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/position-quantity-current/v1","account_id":"0x1111111111111111111111111111111111111111","market_id":"perp:BTC","known_quantity":"1.50000000","first_anchor_event_id":"evt_18ac8b13b4ee099927a91a75838317a1ef672ed5bd8b2b7bce036c466785c50e","last_event_id":"evt_18ac8b13b4ee099927a91a75838317a1ef672ed5bd8b2b7bce036c466785c50e","last_block_height":1601}"#;
const FROZEN_BUYER_EFFECT_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/position-effect-fact/v1","event_id":"evt_18ac8b13b4ee099927a91a75838317a1ef672ed5bd8b2b7bce036c466785c50e","trade_id":"trd-position-frozen","account_id":"0x1111111111111111111111111111111111111111","market_id":"perp:BTC","role":"buyer","anchor_transition":"first_observation","start_position":"1.25000000","fill_quantity":"0.25000000","result_position":"1.50000000","rule_version":"hyperliquid-alpha-desk-canonical-position@1.0.0"}"#;
const FROZEN_SELLER_EFFECT_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/position-effect-fact/v1","event_id":"evt_18ac8b13b4ee099927a91a75838317a1ef672ed5bd8b2b7bce036c466785c50e","trade_id":"trd-position-frozen","account_id":"0x2222222222222222222222222222222222222222","market_id":"perp:BTC","role":"seller","anchor_transition":"first_observation","start_position":"-2.50000000","fill_quantity":"0.25000000","result_position":"-2.75000000","rule_version":"hyperliquid-alpha-desk-canonical-position@1.0.0"}"#;
const FROZEN_UNRESOLVED_GOLDEN: &[u8] = br#"{"schema":"hyperliquid-alpha-desk/position-unresolved-cause-fact/v1","account_id":"0x1111111111111111111111111111111111111111","market_id":"perp:BTC","event_id":"evt_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","liquidation_id":"liq-frozen","cause":"backstop_liquidation"}"#;

#[derive(Debug, Clone, Copy, Default)]
struct TestDispatcher {
    market: CanonicalMarketReducerV1,
    trade: CanonicalTradeReducerSetV2,
    position: CanonicalPositionReducerV1,
}

impl EventReducer for TestDispatcher {
    fn reducer_set_version(&self) -> &str {
        "position-quantity-test-dispatcher@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        EventReducer::supports(&self.market, event)
            || EventReducer::supports(&self.trade, event)
            || EventReducer::supports(&self.position, event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if EventReducer::supports(&self.market, event) {
            return EventReducer::reduce(&self.market, state, event, context);
        }

        let mut mutations = Vec::new();
        if EventReducer::supports(&self.trade, event) {
            mutations.extend(EventReducer::reduce(&self.trade, state, event, context)?);
        }
        if EventReducer::supports(&self.position, event) {
            mutations.extend(EventReducer::reduce(&self.position, state, event, context)?);
        }
        reject_cross_child_collisions(&mutations)?;
        Ok(mutations)
    }

    fn validate_block(
        &self,
        state: &StateView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        EventReducer::validate_block(&self.market, state, context)?;
        EventReducer::validate_block(&self.trade, state, context)?;
        EventReducer::validate_block(&self.position, state, context)
    }
}

#[derive(Debug, Clone)]
struct InjectionDispatcher {
    injection_height: BlockHeight,
    injections: Vec<StateMutation>,
    real: TestDispatcher,
}

#[derive(Debug, Clone, Copy)]
struct OversizedMutationKeyReducer;

impl EventReducer for OversizedMutationKeyReducer {
    fn reducer_set_version(&self) -> &str {
        "position-oversized-mutation-key-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::DexCreated
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        Ok(vec![StateMutation::put(
            StateKey::try_new("position-oversized-test.v1", vec![1; 4 * 1024 + 1]).unwrap(),
            vec![1],
        )])
    }
}

impl EventReducer for InjectionDispatcher {
    fn reducer_set_version(&self) -> &str {
        "position-quantity-injection-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::LiquidationStarted || self.real.supports(event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if context.block_height() == self.injection_height
            && event.event_kind() == EventKind::LiquidationStarted
        {
            Ok(self.injections.clone())
        } else {
            self.real.reduce(state, event, context)
        }
    }
}

#[test]
fn owns_only_exact_schema_participant_bearing_trades_and_never_order_fills() {
    let reducer = CanonicalPositionReducerV1;
    let enriched = trade_event(
        101,
        0,
        "trd-supported",
        "65000",
        "0.01",
        "1.25",
        "-2.5",
        [BUYER, SELLER],
        "1.0.0",
    );
    assert!(EventReducer::supports(&reducer, &enriched));

    let legacy = legacy_trade_event(101, 0, "trd-legacy", "1.0.0");
    assert!(!EventReducer::supports(&reducer, &legacy));
    let later_schema = trade_event(
        101,
        0,
        "trd-later",
        "65000",
        "0.01",
        "1.25",
        "-2.5",
        [BUYER, SELLER],
        "1.1.0",
    );
    assert!(!EventReducer::supports(&reducer, &later_schema));

    let order_fill = order_fill_event(101, 0);
    assert!(!EventReducer::supports(&reducer, &order_fill));
}

#[test]
fn missing_corrupt_and_key_mismatched_market_prerequisites_are_distinct() {
    let trade = |height| {
        trade_event(
            height,
            0,
            "trd-market-prerequisite",
            "65000",
            "0.01",
            "0",
            "0",
            [BUYER, SELLER],
            "1.0.0",
        )
    };
    let mut missing = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(1400),
        TestDispatcher::default(),
        LedgerLimits::production(),
    )
    .unwrap();
    let error = missing
        .apply_block(&block(1400, vec![trade(1400)]))
        .expect_err("missing market prerequisite");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_state.market_prerequisite_missing")
    );

    let market_key = canonical_ledger::MarketCurrentRecordV1::state_key(&market()).unwrap();
    for bad_value in [b"corrupt".to_vec(), key_mismatched_market_value()] {
        let mut corrupt = injected_ledger(
            1400,
            vec![StateMutation::put(market_key.clone(), bad_value)],
        );
        let error = corrupt
            .apply_block(&block(1401, vec![trade(1401)]))
            .expect_err("invalid market prerequisite");
        assert_eq!(
            error.reducer_reason_code(),
            Some("position_state.market_prerequisite_invalid")
        );
    }
}

#[test]
fn dispatcher_and_ledger_reject_duplicate_or_oversized_mutation_keys() {
    let duplicate = StateKey::try_new("position-duplicate-test.v1", vec![1]).unwrap();
    let error = reject_cross_child_collisions(&[
        StateMutation::put(duplicate.clone(), vec![1]),
        StateMutation::put(duplicate, vec![2]),
    ])
    .expect_err("test composite must reject a cross-child collision");
    assert_eq!(error.reason_code(), "position_state.duplicate_mutation_key");

    let long_trade_id = TradeId::new("x".repeat(64 * 1024)).unwrap();
    assert_eq!(
        PositionEffectFactRecordV1::state_key(&long_trade_id, TradeParticipantRoleV1::Buyer)
            .unwrap_err()
            .reason_code(),
        "position_state.codec.invalid_key"
    );

    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(1450),
        OversizedMutationKeyReducer,
        LedgerLimits::production(),
    )
    .unwrap();
    let event = raw_event(
        1450,
        0,
        EventPayload::DexCreated(DexCreated {
            dex_id: DexId::new("oversized").unwrap(),
            name: "Oversized".to_owned(),
            operator_account_id: OPERATOR,
        }),
        Vec::new(),
        vec![OPERATOR],
        "1.0.0",
    );
    let error = ledger
        .apply_block(&block(1450, vec![event]))
        .expect_err("production ledger has a stricter 4 KiB encoded-key ceiling");
    assert_eq!(error.reason_code(), "ledger.mutation_limit_exceeded");
}

#[test]
fn framed_keys_and_exact_codec_limits_are_literal_and_inclusive() {
    assert_eq!(
        PositionQuantityCurrentRecordV1::state_key(&BUYER, &market())
            .unwrap()
            .key(),
        CURRENT_KEY_GOLDEN
    );
    assert_eq!(
        PositionEffectFactRecordV1::state_key(
            &TradeId::new("trd-key").unwrap(),
            TradeParticipantRoleV1::Buyer,
        )
        .unwrap()
        .key(),
        EFFECT_KEY_GOLDEN
    );
    assert_eq!(
        PositionUnresolvedCauseFactRecordV1::state_key(
            &BUYER,
            &market(),
            &EventId::new("evt-key").unwrap(),
            &LiquidationId::new("liq-key").unwrap(),
        )
        .unwrap()
        .key(),
        UNRESOLVED_KEY_GOLDEN
    );

    let exact_key_trade = TradeId::new("x".repeat(64 * 1024 - 21)).unwrap();
    let exact_key =
        PositionEffectFactRecordV1::state_key(&exact_key_trade, TradeParticipantRoleV1::Buyer)
            .unwrap();
    assert_eq!(exact_key.key().len(), 64 * 1024);
    let too_long_trade = TradeId::new("x".repeat(64 * 1024 - 20)).unwrap();
    assert_eq!(
        PositionEffectFactRecordV1::state_key(&too_long_trade, TradeParticipantRoleV1::Buyer,)
            .unwrap_err()
            .reason_code(),
        "position_state.codec.invalid_key"
    );

    let exact_record = exact_sized_current_wire(16 * 1024);
    assert_eq!(exact_record.len(), 16 * 1024);
    PositionQuantityCurrentRecordV1::decode(&exact_record).unwrap();
    let mut too_large_record = exact_record;
    too_large_record.push(b' ');
    assert_eq!(
        PositionQuantityCurrentRecordV1::decode(&too_large_record)
            .unwrap_err()
            .reason_code(),
        "position_state.codec.limit_exceeded"
    );
}

#[test]
fn frozen_position_values_decode_at_and_match_reducer_emission() {
    let mut ledger = seeded_ledger(1600);
    let trade = trade_event(
        1601,
        0,
        "trd-position-frozen",
        "65000",
        "0.25",
        "1.25",
        "-2.5",
        [BUYER, SELLER],
        "1.0.0",
    );
    ledger
        .apply_block(&block(1601, vec![trade.clone()]))
        .unwrap();
    let trade_id = TradeId::new("trd-position-frozen").unwrap();
    let current_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let buyer_key =
        PositionEffectFactRecordV1::state_key(&trade_id, TradeParticipantRoleV1::Buyer).unwrap();
    let seller_key =
        PositionEffectFactRecordV1::state_key(&trade_id, TradeParticipantRoleV1::Seller).unwrap();
    let entries = ledger.state_image().entries();
    assert_eq!(entries.get(&current_key).unwrap(), FROZEN_CURRENT_GOLDEN);
    assert_eq!(entries.get(&buyer_key).unwrap(), FROZEN_BUYER_EFFECT_GOLDEN);
    assert_eq!(
        entries.get(&seller_key).unwrap(),
        FROZEN_SELLER_EFFECT_GOLDEN
    );

    let current =
        PositionQuantityCurrentRecordV1::decode_at(&current_key, FROZEN_CURRENT_GOLDEN).unwrap();
    assert_eq!(current.account_id(), BUYER);
    assert_eq!(current.market_id(), &market());
    assert_eq!(current.known_quantity().unwrap().to_string(), "1.50000000");
    assert_eq!(current.first_anchor_event_id(), Some(trade.event_id()));
    assert_eq!(current.last_event_id(), trade.event_id());
    assert_eq!(current.last_block_height(), BlockHeight::new(1601));

    let buyer =
        PositionEffectFactRecordV1::decode_at(&buyer_key, FROZEN_BUYER_EFFECT_GOLDEN).unwrap();
    assert_eq!(buyer.event_id(), trade.event_id());
    assert_eq!(buyer.trade_id(), &trade_id);
    assert_eq!(buyer.role(), TradeParticipantRoleV1::Buyer);
    assert_eq!(buyer.account_id(), BUYER);
    assert_eq!(buyer.market_id(), &market());
    assert_eq!(
        buyer.anchor_transition(),
        PositionAnchorTransitionV1::FirstObservation
    );
    assert_eq!(buyer.start_position().to_string(), "1.25000000");
    assert_eq!(buyer.fill_quantity().to_string(), "0.25000000");
    assert_eq!(buyer.result_position().to_string(), "1.50000000");
    assert_eq!(
        buyer.rule_version(),
        "hyperliquid-alpha-desk-canonical-position@1.0.0"
    );
    let seller =
        PositionEffectFactRecordV1::decode_at(&seller_key, FROZEN_SELLER_EFFECT_GOLDEN).unwrap();
    assert_eq!(seller.event_id(), trade.event_id());
    assert_eq!(seller.trade_id(), &trade_id);
    assert_eq!(seller.role(), TradeParticipantRoleV1::Seller);
    assert_eq!(seller.account_id(), SELLER);
    assert_eq!(seller.market_id(), &market());
    assert_eq!(
        seller.anchor_transition(),
        PositionAnchorTransitionV1::FirstObservation
    );
    assert_eq!(seller.start_position().to_string(), "-2.50000000");
    assert_eq!(seller.fill_quantity().to_string(), "0.25000000");
    assert_eq!(seller.result_position().to_string(), "-2.75000000");
    assert_eq!(
        seller.rule_version(),
        "hyperliquid-alpha-desk-canonical-position@1.0.0"
    );

    let unresolved_event = fixture_event_id();
    let unresolved_key = PositionUnresolvedCauseFactRecordV1::state_key(
        &BUYER,
        &market(),
        &unresolved_event,
        &LiquidationId::new("liq-frozen").unwrap(),
    )
    .unwrap();
    let unresolved =
        PositionUnresolvedCauseFactRecordV1::decode_at(&unresolved_key, FROZEN_UNRESOLVED_GOLDEN)
            .unwrap();
    assert_eq!(unresolved.account_id(), BUYER);
    assert_eq!(unresolved.market_id(), &market());
    assert_eq!(unresolved.event_id(), &unresolved_event);
    assert_eq!(unresolved.liquidation_id().as_str(), "liq-frozen");
    assert_eq!(
        unresolved.cause(),
        PositionUnresolvedCauseV1::BackstopLiquidation
    );
}

#[test]
fn every_invalid_prior_effect_reports_its_codec_cause_without_collision_or_advance() {
    let target_trade = trade_event(
        1701,
        0,
        "trd-prior-effect",
        "65000",
        "0.01",
        "0",
        "0",
        [BUYER, SELLER],
        "1.0.0",
    );
    let target_key = PositionEffectFactRecordV1::state_key(
        &TradeId::new("trd-prior-effect").unwrap(),
        TradeParticipantRoleV1::Buyer,
    )
    .unwrap();
    let valid_wrong_identity = effect_wire(
        target_trade.event_id(),
        "trd-other-effect",
        BUYER,
        TradeParticipantRoleV1::Buyer,
        "0.00000000",
        "0.01000000",
        "0.01000000",
    );
    let valid_target = effect_wire(
        target_trade.event_id(),
        "trd-prior-effect",
        BUYER,
        TradeParticipantRoleV1::Buyer,
        "0.00000000",
        "0.01000000",
        "0.01000000",
    );
    for (prior, expected) in [
        (b"corrupt".to_vec(), "position_state.codec.decode"),
        (valid_wrong_identity, "position_state.codec.key_mismatch"),
        (
            [valid_target.as_slice(), b" "].concat(),
            "position_state.codec.noncanonical",
        ),
    ] {
        let mut ledger = injected_ledger(1700, vec![StateMutation::put(target_key.clone(), prior)]);
        let before_bytes = ledger.state_image().canonical_bytes();
        let before_hash = ledger.state_hash();
        let before_checkpoint = ledger.checkpoint().unwrap();
        let error = ledger
            .apply_block(&block(1701, vec![target_trade.clone()]))
            .expect_err("invalid prior effect must fail before collision");
        assert_eq!(error.reducer_reason_code(), Some(expected));
        assert_ne!(
            error.reducer_reason_code(),
            Some("position_state.effect_collision")
        );
        assert_eq!(ledger.state_image().canonical_bytes(), before_bytes);
        assert_eq!(ledger.state_hash(), before_hash);
        assert_eq!(ledger.checkpoint().unwrap(), before_checkpoint);
    }
}

#[test]
fn first_nonzero_observation_anchors_both_sides_and_applies_opposite_effects() {
    let mut ledger = seeded_ledger(100);
    let trade = trade_event(
        101,
        0,
        "trd-first-anchor",
        "65000",
        "0.25",
        "1.25",
        "-2.5",
        [BUYER, SELLER],
        "1.0.0",
    );
    let event_id = trade.event_id().clone();

    let ApplyOutcome::Applied(delta) = ledger
        .apply_block(&block(101, vec![trade.clone()]))
        .unwrap()
    else {
        panic!("first enriched trade must apply");
    };
    // Four frozen V1 trade mutations, four V2 trade mutations, two immutable
    // position effects, and two position-current updates.
    assert_eq!(delta.mutations().len(), 12);
    assert_eq!(
        delta.mutations()[8..]
            .iter()
            .map(|mutation| mutation.key().namespace())
            .collect::<Vec<_>>(),
        [
            "position-effect-fact.v1",
            "position-effect-fact.v1",
            "position-quantity-current.v1",
            "position-quantity-current.v1",
        ]
    );
    let trade_id = TradeId::new("trd-first-anchor").unwrap();
    assert_eq!(
        delta.mutations()[8].key(),
        &PositionEffectFactRecordV1::state_key(&trade_id, TradeParticipantRoleV1::Buyer).unwrap()
    );
    assert_eq!(
        delta.mutations()[9].key(),
        &PositionEffectFactRecordV1::state_key(&trade_id, TradeParticipantRoleV1::Seller).unwrap()
    );
    assert_eq!(
        delta.mutations()[10].key(),
        &PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap()
    );
    assert_eq!(
        delta.mutations()[11].key(),
        &PositionQuantityCurrentRecordV1::state_key(&SELLER, &market()).unwrap()
    );

    assert_current(
        &ledger,
        BUYER,
        Some("1.50000000"),
        Some(&event_id),
        &event_id,
        101,
    );
    assert_current(
        &ledger,
        SELLER,
        Some("-2.75000000"),
        Some(&event_id),
        &event_id,
        101,
    );
    assert_effect(
        &ledger,
        &trade,
        BUYER,
        TradeParticipantRoleV1::Buyer,
        "1.25000000",
        "0.25000000",
        "1.50000000",
    );
    assert_eq!(
        effect(&ledger, &trade, BUYER).anchor_transition(),
        PositionAnchorTransitionV1::FirstObservation
    );
    assert_effect(
        &ledger,
        &trade,
        SELLER,
        TradeParticipantRoleV1::Seller,
        "-2.50000000",
        "0.25000000",
        "-2.75000000",
    );
    assert_eq!(
        effect(&ledger, &trade, SELLER).anchor_transition(),
        PositionAnchorTransitionV1::FirstObservation
    );
}

#[test]
fn continued_anchor_preserves_first_event_and_halted_exact_market_remains_usable() {
    let mut ledger = seeded_ledger(1500);
    let first = trade_event(
        1501,
        0,
        "trd-continued-first",
        "65000",
        "0.1",
        "0",
        "0",
        [BUYER, SELLER],
        "1.0.0",
    );
    let first_event_id = first.event_id().clone();
    ledger.apply_block(&block(1501, vec![first])).unwrap();
    ledger
        .apply_block(&block(
            1502,
            vec![raw_event(
                1502,
                0,
                EventPayload::MarketHalted(MarketHalted {
                    market_id: market(),
                    reason: "maintenance".to_owned(),
                }),
                vec![market()],
                Vec::new(),
                "1.0.0",
            )],
        ))
        .unwrap();

    let continued = trade_event(
        1503,
        0,
        "trd-continued-second",
        "65000",
        "0.1",
        "0.1",
        "-0.1",
        [BUYER, SELLER],
        "1.0.0",
    );
    ledger
        .apply_block(&block(1503, vec![continued.clone()]))
        .unwrap();
    let buyer_current = current(&ledger, BUYER);
    assert_eq!(buyer_current.first_anchor_event_id(), Some(&first_event_id));
    assert_eq!(
        buyer_current.known_quantity().unwrap().to_string(),
        "0.20000000"
    );
    assert_eq!(
        effect(&ledger, &continued, BUYER).anchor_transition(),
        PositionAnchorTransitionV1::Continued
    );
    let seller_current = current(&ledger, SELLER);
    assert_eq!(
        seller_current.first_anchor_event_id(),
        Some(&first_event_id)
    );
    assert_eq!(
        seller_current.known_quantity().unwrap().to_string(),
        "-0.20000000"
    );
    assert_eq!(
        effect(&ledger, &continued, SELLER).anchor_transition(),
        PositionAnchorTransitionV1::Continued
    );
}

#[test]
fn source_anchored_arithmetic_covers_long_short_add_reduce_flat_and_reversal() {
    let cases = [
        (
            "long-add",
            "1",
            "-1",
            [BUYER, SELLER],
            "0.5",
            "1.50000000",
            "-1.50000000",
        ),
        (
            "long-reduce",
            "-1",
            "1",
            [BUYER, SELLER],
            "0.5",
            "-0.50000000",
            "0.50000000",
        ),
        (
            "flat",
            "-1",
            "1",
            [BUYER, SELLER],
            "1",
            "0.00000000",
            "0.00000000",
        ),
        (
            "reversal",
            "-0.25",
            "0.25",
            [BUYER, SELLER],
            "1",
            "0.75000000",
            "-0.75000000",
        ),
    ];

    for (id, buyer_start, seller_start, identities, fill, buyer_result, seller_result) in cases {
        let mut ledger = seeded_ledger(200);
        let event = trade_event(
            201,
            0,
            id,
            "65000",
            fill,
            buyer_start,
            seller_start,
            identities,
            "1.0.0",
        );
        ledger
            .apply_block(&block(201, vec![event.clone()]))
            .unwrap();
        let [buyer, seller] = event.account_addresses() else {
            panic!("two identities")
        };
        assert_eq!(
            current(&ledger, *buyer)
                .known_quantity()
                .unwrap()
                .to_string(),
            buyer_result
        );
        assert_eq!(
            current(&ledger, *seller)
                .known_quantity()
                .unwrap()
                .to_string(),
            seller_result
        );
    }
}

#[test]
fn mixed_input_scales_normalize_upward_before_alignment_and_exact_arithmetic() {
    let mut ledger = seeded_ledger(300);
    let event = trade_event(
        301,
        0,
        "trd-mixed-scales",
        "65000.0",
        "0.01",
        "1.2",
        "-2.50",
        [BUYER, SELLER],
        "1.0.0",
    );
    ledger
        .apply_block(&block(301, vec![event.clone()]))
        .unwrap();

    assert_effect(
        &ledger,
        &event,
        BUYER,
        TradeParticipantRoleV1::Buyer,
        "1.20000000",
        "0.01000000",
        "1.21000000",
    );
    assert_effect(
        &ledger,
        &event,
        SELLER,
        TradeParticipantRoleV1::Seller,
        "-2.50000000",
        "0.01000000",
        "-2.51000000",
    );
}

#[test]
fn exact_market_metadata_tick_lot_and_upward_only_scale_are_fail_closed() {
    let invalid = [
        (
            trade_event(
                401,
                0,
                "trd-price-downscale",
                "65000.0000001",
                "0.01",
                "0",
                "0",
                [BUYER, SELLER],
                "1.0.0",
            ),
            "position_state.scale_normalization",
        ),
        (
            trade_event(
                401,
                0,
                "trd-fill-downscale",
                "65000",
                "0.000000001",
                "0",
                "0",
                [BUYER, SELLER],
                "1.0.0",
            ),
            "position_state.scale_normalization",
        ),
        (
            trade_event(
                401,
                0,
                "trd-price-tick",
                "65000.05",
                "0.01",
                "0",
                "0",
                [BUYER, SELLER],
                "1.0.0",
            ),
            "position_state.price_tick_misaligned",
        ),
        (
            trade_event(
                401,
                0,
                "trd-fill-lot",
                "65000",
                "0.0005",
                "0",
                "0",
                [BUYER, SELLER],
                "1.0.0",
            ),
            "position_state.quantity_lot_misaligned",
        ),
        (
            trade_event(
                401,
                0,
                "trd-start-lot",
                "65000",
                "0.001",
                "0.0005",
                "0",
                [BUYER, SELLER],
                "1.0.0",
            ),
            "position_state.quantity_lot_misaligned",
        ),
    ];

    for (event, expected) in invalid {
        let mut ledger = seeded_ledger(400);
        let before = ledger.state_image().canonical_bytes();
        let error = ledger
            .apply_block(&block(401, vec![event]))
            .expect_err("invalid market-normalized trade");
        assert_eq!(error.reducer_reason_code(), Some(expected));
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }

    let mut unresolved = seeded_ledger(410);
    unresolved
        .apply_block(&block(
            411,
            vec![market_metadata_changed_event(
                411,
                "metadata-unresolved@2.0.0",
            )],
        ))
        .unwrap();
    let before = unresolved.state_image().canonical_bytes();
    let error = unresolved
        .apply_block(&block(
            412,
            vec![trade_event(
                412,
                0,
                "trd-unresolved-market",
                "65000",
                "0.01",
                "0",
                "0",
                [BUYER, SELLER],
                "1.0.0",
            )],
        ))
        .expect_err("unresolved metadata must suppress position mutation");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_state.market_metadata_unresolved")
    );
    assert_eq!(unresolved.state_image().canonical_bytes(), before);
}

#[test]
fn checked_position_arithmetic_overflow_rejects_the_complete_block() {
    let mut ledger = seeded_ledger(500);
    let maximum_aligned = i128::MAX - i128::MAX.rem_euclid(100_000);
    let maximum = PositionQuantity::from_raw(maximum_aligned, 8)
        .unwrap()
        .to_string();
    let overflow = trade_event(
        501,
        0,
        "trd-overflow",
        "65000",
        "0.001",
        &maximum,
        "0",
        [BUYER, SELLER],
        "1.0.0",
    );
    let before = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&block(501, vec![overflow]))
        .expect_err("position result overflow");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_state.position_arithmetic")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn missing_participants_and_reordered_envelope_identities_are_rejected() {
    let reducer = CanonicalPositionReducerV1;
    let legacy = legacy_trade_event(600, 0, "trd-no-participants", "1.0.0");
    assert!(!EventReducer::supports(&reducer, &legacy));

    let reordered = try_trade_event(
        601,
        0,
        "trd-reordered",
        "65000",
        "0.01",
        "0",
        "0",
        [SELLER, BUYER],
        "1.0.0",
    )
    .expect_err("canonical envelope must reject reordered participant identities");
    assert!(matches!(
        reordered,
        canonical_events::ContractError::Invalid {
            field: "account_ids",
            ..
        }
    ));
}

#[test]
fn later_known_state_requires_exact_upscaled_source_start_and_rolls_back_late_failure() {
    let mut ledger = seeded_ledger(700);
    ledger
        .apply_block(&block(
            701,
            vec![trade_event(
                701,
                0,
                "trd-known-1",
                "65000",
                "0.25",
                "1.25",
                "-2.5",
                [BUYER, SELLER],
                "1.0.0",
            )],
        ))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();
    let before_hash = ledger.state_hash();

    let valid = trade_event(
        702,
        0,
        "trd-known-2",
        "65000",
        "0.25",
        "1.5",
        "-2.75",
        [BUYER, SELLER],
        "1.0.0",
    );
    let mismatch = trade_event(
        702,
        1,
        "trd-known-3",
        "65000",
        "0.25",
        "1.76",
        "-3",
        [BUYER, SELLER],
        "1.0.0",
    );
    let error = ledger
        .apply_block(&block(702, vec![valid, mismatch]))
        .expect_err("late start mismatch must roll back the full block");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_state.start_position_mismatch")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert_eq!(ledger.state_hash(), before_hash);
    assert_eq!(
        ledger.checkpoint().unwrap().block_height(),
        BlockHeight::new(701)
    );
}

#[test]
fn current_codec_accepts_three_valid_anchor_states_and_rejects_known_without_anchor() {
    let event_id = fixture_event_id();
    let account = BUYER.to_api_string();
    let market_text = market().as_str().to_owned();
    let valid = [
        current_wire(&account, &market_text, None, None, &event_id, 800),
        current_wire(
            &account,
            &market_text,
            None,
            Some(&event_id),
            &event_id,
            800,
        ),
        current_wire(
            &account,
            &market_text,
            Some("1.00000000"),
            Some(&event_id),
            &event_id,
            800,
        ),
    ];
    for wire in valid {
        PositionQuantityCurrentRecordV1::decode(&wire).expect("valid current invariant");
    }

    let invalid = current_wire(
        &account,
        &market_text,
        Some("1.00000000"),
        None,
        &event_id,
        800,
    );
    assert_eq!(
        PositionQuantityCurrentRecordV1::decode(&invalid)
            .expect_err("known current without anchor")
            .reason_code(),
        "position_state.codec.invalid_record"
    );
}

#[test]
fn key_bound_codecs_reject_corrupt_noncanonical_oversized_and_mismatched_values() {
    let event_id = fixture_event_id();
    let account = BUYER.to_api_string();
    let market_text = market().as_str().to_owned();
    let encoded = current_wire(
        &account,
        &market_text,
        Some("1.00000000"),
        Some(&event_id),
        &event_id,
        900,
    );

    for (corrupt, expected) in [
        (
            encoded[..encoded.len() - 1].to_vec(),
            "position_state.codec.decode",
        ),
        (
            [encoded.as_slice(), b" "].concat(),
            "position_state.codec.noncanonical",
        ),
        (b"{}".to_vec(), "position_state.codec.decode"),
        (
            vec![b'x'; 16 * 1024 + 1],
            "position_state.codec.limit_exceeded",
        ),
    ] {
        let error =
            PositionQuantityCurrentRecordV1::decode(&corrupt).expect_err("strict current codec");
        assert_eq!(error.reason_code(), expected);
    }

    let key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let wrong_key = PositionQuantityCurrentRecordV1::state_key(&SELLER, &market()).unwrap();
    PositionQuantityCurrentRecordV1::decode_at(&key, &encoded).unwrap();
    assert_eq!(
        PositionQuantityCurrentRecordV1::decode_at(&wrong_key, &encoded)
            .expect_err("record identities must bind the key")
            .reason_code(),
        "position_state.codec.key_mismatch"
    );

    let oversized = vec![b'x'; 64 * 1024 + 1];
    assert_eq!(
        PositionQuantityCurrentRecordV1::decode(&oversized)
            .unwrap_err()
            .reason_code(),
        "position_state.codec.limit_exceeded"
    );
}

#[test]
fn corrupt_current_and_duplicate_effect_fail_closed_without_mutation() {
    let trade = trade_event(
        1001,
        0,
        "trd-corrupt-current",
        "65000",
        "0.01",
        "0",
        "0",
        [BUYER, SELLER],
        "1.0.0",
    );
    let current_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let mut corrupt = injected_ledger(
        1000,
        vec![StateMutation::put(current_key, b"corrupt".to_vec())],
    );
    let before = corrupt.state_image().canonical_bytes();
    let error = corrupt
        .apply_block(&block(1001, vec![trade]))
        .expect_err("corrupt prior current");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_state.current_record_invalid")
    );
    assert_eq!(corrupt.state_image().canonical_bytes(), before);

    let duplicate_trade = trade_event(
        1011,
        0,
        "trd-duplicate-effect",
        "65000",
        "0.01",
        "0",
        "0",
        [BUYER, SELLER],
        "1.0.0",
    );
    let duplicate_key = PositionEffectFactRecordV1::state_key(
        &TradeId::new("trd-duplicate-effect").unwrap(),
        TradeParticipantRoleV1::Buyer,
    )
    .unwrap();
    let duplicate_value = effect_wire(
        duplicate_trade.event_id(),
        "trd-duplicate-effect",
        BUYER,
        TradeParticipantRoleV1::Buyer,
        "0.00000000",
        "0.01000000",
        "0.01000000",
    );
    let mut duplicate = injected_ledger(
        1010,
        vec![StateMutation::put(duplicate_key, duplicate_value)],
    );
    let before = duplicate.state_image().canonical_bytes();
    let error = duplicate
        .apply_block(&block(1011, vec![duplicate_trade]))
        .expect_err("immutable effect collision");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_state.effect_collision")
    );
    assert_eq!(duplicate.state_image().canonical_bytes(), before);
}

#[test]
fn unresolved_backstop_reanchors_without_deleting_any_preserved_cause() {
    let anchor = fixture_event_id();
    let current = current_wire(
        &BUYER.to_api_string(),
        market().as_str(),
        None,
        Some(&anchor),
        &anchor,
        1100,
    );
    let current_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let seller_current = current_wire(
        &SELLER.to_api_string(),
        market().as_str(),
        None,
        Some(&anchor),
        &anchor,
        1100,
    );
    let seller_current_key =
        PositionQuantityCurrentRecordV1::state_key(&SELLER, &market()).unwrap();
    let cause_event_a =
        EventId::new("evt_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
            .unwrap();
    let cause_event_b =
        EventId::new("evt_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
            .unwrap();
    let cause_a = unresolved_wire(&cause_event_a, "liq-a");
    let cause_b = unresolved_wire(&cause_event_b, "liq-b");
    let cause_key_a = PositionUnresolvedCauseFactRecordV1::state_key(
        &BUYER,
        &market(),
        &cause_event_a,
        &LiquidationId::new("liq-a").unwrap(),
    )
    .unwrap();
    let cause_key_b = PositionUnresolvedCauseFactRecordV1::state_key(
        &BUYER,
        &market(),
        &cause_event_b,
        &LiquidationId::new("liq-b").unwrap(),
    )
    .unwrap();
    let mut ledger = injected_ledger(
        1100,
        vec![
            StateMutation::put(current_key, current),
            StateMutation::put(seller_current_key, seller_current),
            StateMutation::put(cause_key_a.clone(), cause_a),
            StateMutation::put(cause_key_b.clone(), cause_b),
        ],
    );
    let trade = trade_event(
        1101,
        0,
        "trd-reanchor",
        "65000",
        "0.25",
        "4",
        "0",
        [BUYER, SELLER],
        "1.0.0",
    );
    let event_id = trade.event_id().clone();
    ledger
        .apply_block(&block(1101, vec![trade.clone()]))
        .unwrap();

    assert_current(
        &ledger,
        BUYER,
        Some("4.25000000"),
        Some(&anchor),
        &event_id,
        1101,
    );
    assert_current(
        &ledger,
        SELLER,
        Some("-0.25000000"),
        Some(&anchor),
        &event_id,
        1101,
    );
    assert!(ledger.state_image().entries().contains_key(&cause_key_a));
    assert!(ledger.state_image().entries().contains_key(&cause_key_b));
    assert_eq!(
        effect(&ledger, &trade, BUYER).anchor_transition(),
        PositionAnchorTransitionV1::ReanchoredFromUnresolved
    );
    assert_eq!(
        effect(&ledger, &trade, SELLER).anchor_transition(),
        PositionAnchorTransitionV1::ReanchoredFromUnresolved
    );
}

#[test]
fn unseen_backstop_state_has_no_anchor_and_first_trade_sets_it() {
    let cause_event = fixture_event_id();
    let current = current_wire(
        &BUYER.to_api_string(),
        market().as_str(),
        None,
        None,
        &cause_event,
        1200,
    );
    let current_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
        &BUYER,
        &market(),
        &cause_event,
        &LiquidationId::new("liq-unseen").unwrap(),
    )
    .unwrap();
    let mut ledger = injected_ledger(
        1200,
        vec![
            StateMutation::put(current_key, current),
            StateMutation::put(cause_key, unresolved_wire(&cause_event, "liq-unseen")),
        ],
    );
    let trade = trade_event(
        1201,
        0,
        "trd-first-after-backstop",
        "65000",
        "0.25",
        "-1",
        "0",
        [BUYER, SELLER],
        "1.0.0",
    );
    let event_id = trade.event_id().clone();
    ledger.apply_block(&block(1201, vec![trade])).unwrap();
    assert_current(
        &ledger,
        BUYER,
        Some("-0.75000000"),
        Some(&event_id),
        &event_id,
        1201,
    );
}

#[test]
fn unresolved_cause_codec_freezes_exact_variant_and_identity_binding() {
    let event_id = fixture_event_id();
    let encoded = unresolved_wire(&event_id, "liq-codec");
    let key = PositionUnresolvedCauseFactRecordV1::state_key(
        &BUYER,
        &market(),
        &event_id,
        &LiquidationId::new("liq-codec").unwrap(),
    )
    .unwrap();
    let decoded = PositionUnresolvedCauseFactRecordV1::decode_at(&key, &encoded).unwrap();
    assert_eq!(
        decoded.cause(),
        PositionUnresolvedCauseV1::BackstopLiquidation
    );
    assert_eq!(decoded.liquidation_id().as_str(), "liq-codec");

    let wrong_key = PositionUnresolvedCauseFactRecordV1::state_key(
        &SELLER,
        &market(),
        &event_id,
        &LiquidationId::new("liq-codec").unwrap(),
    )
    .unwrap();
    assert_eq!(
        PositionUnresolvedCauseFactRecordV1::decode_at(&wrong_key, &encoded)
            .unwrap_err()
            .reason_code(),
        "position_state.codec.key_mismatch"
    );
    let wrong_variant = replace_bytes(&encoded, b"backstop_liquidation", b"other_liquidation");
    assert!(PositionUnresolvedCauseFactRecordV1::decode(&wrong_variant).is_err());
}

#[test]
fn mixed_legacy_and_enriched_blocks_rebuild_byte_identically_without_double_counting() {
    fn rebuild() -> CanonicalLedger<TestDispatcher> {
        let mut ledger = seeded_ledger(1300);
        ledger
            .apply_block(&block(
                1301,
                vec![
                    legacy_trade_event(1301, 0, "trd-legacy-1301", "1.0.0"),
                    trade_event(
                        1301,
                        1,
                        "trd-enriched-1301",
                        "65000",
                        "0.25",
                        "1",
                        "-1",
                        [BUYER, SELLER],
                        "1.0.0",
                    ),
                ],
            ))
            .unwrap();
        ledger
    }

    let first = rebuild();
    let second = rebuild();
    assert_eq!(
        first.state_image().canonical_bytes(),
        second.state_image().canonical_bytes()
    );
    assert_eq!(first.state_hash(), second.state_hash());
    assert_eq!(
        current(&first, BUYER).known_quantity().unwrap().to_string(),
        "1.25000000"
    );
    assert_eq!(
        first
            .state_image()
            .entries()
            .keys()
            .filter(|key| key.namespace() == "position-effect-fact.v1")
            .count(),
        2
    );
}

fn assert_current<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    account: Address,
    expected_quantity: Option<&str>,
    expected_anchor: Option<&EventId>,
    expected_last: &EventId,
    height: u64,
) {
    let record = current(ledger, account);
    assert_eq!(
        record.known_quantity().map(|value| value.to_string()),
        expected_quantity.map(str::to_owned)
    );
    assert_eq!(record.first_anchor_event_id(), expected_anchor);
    assert_eq!(record.last_event_id(), expected_last);
    assert_eq!(record.last_block_height(), BlockHeight::new(height));
}

fn current<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    account: Address,
) -> PositionQuantityCurrentRecordV1 {
    let key = PositionQuantityCurrentRecordV1::state_key(&account, &market()).unwrap();
    PositionQuantityCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn assert_effect<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    event: &CanonicalEventEnvelope,
    account: Address,
    role: TradeParticipantRoleV1,
    start: &str,
    fill: &str,
    result: &str,
) {
    let record = effect(ledger, event, account);
    assert_eq!(record.event_id(), event.event_id());
    assert_eq!(record.account_id(), account);
    assert_eq!(record.market_id(), &market());
    assert_eq!(record.role(), role);
    assert_eq!(record.start_position().to_string(), start);
    assert_eq!(record.fill_quantity().to_string(), fill);
    assert_eq!(record.result_position().to_string(), result);
    assert_eq!(
        record.rule_version(),
        "hyperliquid-alpha-desk-canonical-position@1.0.0"
    );
}

fn effect<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    event: &CanonicalEventEnvelope,
    account: Address,
) -> PositionEffectFactRecordV1 {
    let EventPayload::TradeMatched(trade) = event.payload() else {
        unreachable!()
    };
    let key = PositionEffectFactRecordV1::state_key(
        trade.trade_id.as_ref().unwrap(),
        if account == BUYER {
            TradeParticipantRoleV1::Buyer
        } else {
            TradeParticipantRoleV1::Seller
        },
    )
    .unwrap();
    PositionEffectFactRecordV1::decode_at(&key, ledger.state_image().entries().get(&key).unwrap())
        .unwrap()
}

fn seeded_ledger(first_height: u64) -> CanonicalLedger<TestDispatcher> {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        TestDispatcher::default(),
        LedgerLimits::production(),
    )
    .unwrap();
    ledger
        .apply_block(&block(first_height, market_prerequisites(first_height)))
        .unwrap();
    ledger
}

fn injected_ledger(
    first_height: u64,
    injections: Vec<StateMutation>,
) -> CanonicalLedger<InjectionDispatcher> {
    let dispatcher = InjectionDispatcher {
        injection_height: BlockHeight::new(first_height),
        injections,
        real: TestDispatcher::default(),
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        dispatcher,
        LedgerLimits::production(),
    )
    .unwrap();
    let mut events = market_prerequisites(first_height);
    events.push(liquidation_started_trigger(
        first_height,
        u32::try_from(events.len()).unwrap(),
    ));
    ledger.apply_block(&block(first_height, events)).unwrap();
    ledger
}

fn key_mismatched_market_value() -> Vec<u8> {
    let ledger = seeded_ledger(1390);
    let key = canonical_ledger::MarketCurrentRecordV1::state_key(&market()).unwrap();
    replace_bytes(
        ledger.state_image().entries().get(&key).unwrap(),
        b"perp:BTC",
        b"perp:ETH",
    )
}

fn reject_cross_child_collisions(mutations: &[StateMutation]) -> Result<(), ReducerError> {
    let mut keys = BTreeSet::new();
    if mutations.iter().all(|mutation| keys.insert(mutation.key())) {
        Ok(())
    } else {
        Err(ReducerError::try_new(
            "position_state.duplicate_mutation_key",
            "test dispatcher children emitted the same state key",
        )
        .unwrap())
    }
}

fn market_prerequisites(height: u64) -> Vec<CanonicalEventEnvelope> {
    let base = AssetId::new("BTC").unwrap();
    let quote = AssetId::new("USDC").unwrap();
    vec![
        raw_event(
            height,
            0,
            EventPayload::DexCreated(DexCreated {
                dex_id: DexId::new("validator").unwrap(),
                name: "Validator".to_owned(),
                operator_account_id: OPERATOR,
            }),
            Vec::new(),
            vec![OPERATOR],
            "1.0.0",
        ),
        raw_event(
            height,
            1,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: base.clone(),
                context_version: "btc-v1".to_owned(),
                context_hash: [1; 32],
            }),
            Vec::new(),
            Vec::new(),
            "1.0.0",
        ),
        raw_event(
            height,
            2,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: quote.clone(),
                context_version: "usdc-v1".to_owned(),
                context_hash: [2; 32],
            }),
            Vec::new(),
            Vec::new(),
            "1.0.0",
        ),
        raw_event(
            height,
            3,
            EventPayload::MarketCreated(MarketCreated {
                market_id: market(),
                dex_id: DexId::new("validator").unwrap(),
                base_asset_id: base,
                quote_asset_id: quote,
                tick_size: Price::parse_at_scale("0.1", 6).unwrap(),
                lot_size: Quantity::parse_at_scale("0.001", 8).unwrap(),
            }),
            vec![market()],
            Vec::new(),
            "1.0.0",
        ),
    ]
}

#[allow(clippy::too_many_arguments)]
fn trade_event(
    height: u64,
    event_index: u32,
    trade_id: &str,
    price: &str,
    fill: &str,
    buyer_start: &str,
    seller_start: &str,
    envelope_accounts: [Address; 2],
    schema: &str,
) -> CanonicalEventEnvelope {
    try_trade_event(
        height,
        event_index,
        trade_id,
        price,
        fill,
        buyer_start,
        seller_start,
        envelope_accounts,
        schema,
    )
    .unwrap()
}

#[allow(clippy::too_many_arguments)]
fn try_trade_event(
    height: u64,
    event_index: u32,
    trade_id: &str,
    price: &str,
    fill: &str,
    buyer_start: &str,
    seller_start: &str,
    envelope_accounts: [Address; 2],
    schema: &str,
) -> Result<CanonicalEventEnvelope, canonical_events::ContractError> {
    let payload = EventPayload::TradeMatched(TradeMatched {
        trade_id: Some(TradeId::new(trade_id).unwrap()),
        market_id: Some(market()),
        maker_order_id: None,
        taker_order_id: None,
        price: Price::from_str(price).unwrap(),
        quantity: Quantity::from_str(fill).unwrap(),
        deterministic_seed: height,
        participants: Some(Box::new([
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Buyer,
                account_id: BUYER,
                start_position: PositionQuantity::from_str(buyer_start).unwrap(),
                order_id: OrderId::new(format!("buyer-order-{trade_id}")).unwrap(),
                twap_id: None,
                client_order_id: None,
            },
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Seller,
                account_id: SELLER,
                start_position: PositionQuantity::from_str(seller_start).unwrap(),
                order_id: OrderId::new(format!("seller-order-{trade_id}")).unwrap(),
                twap_id: None,
                client_order_id: None,
            },
        ])),
    });
    try_raw_event(
        height,
        event_index,
        payload,
        vec![market()],
        envelope_accounts.to_vec(),
        schema,
    )
}

fn legacy_trade_event(
    height: u64,
    event_index: u32,
    trade_id: &str,
    schema: &str,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new(trade_id).unwrap()),
            market_id: Some(market()),
            maker_order_id: None,
            taker_order_id: None,
            price: Price::parse_at_scale("65000", 6).unwrap(),
            quantity: Quantity::parse_at_scale("0.01", 8).unwrap(),
            deterministic_seed: height,
            participants: None,
        }),
        vec![market()],
        vec![BUYER, SELLER],
        schema,
    )
}

fn market_metadata_changed_event(height: u64, version: &str) -> CanonicalEventEnvelope {
    raw_event(
        height,
        0,
        EventPayload::MarketMetadataChanged(MarketMetadataChanged {
            market_id: market(),
            metadata_version: version.to_owned(),
            metadata_hash: [4; 32],
        }),
        vec![market()],
        Vec::new(),
        "1.0.0",
    )
}

fn order_fill_event(height: u64, event_index: u32) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::OrderFilled(OrderFilled {
            order_id: OrderId::new("order-fill").unwrap(),
            trade_id: TradeId::new("trade-fill").unwrap(),
            fill_price: Price::parse_at_scale("65000", 6).unwrap(),
            fill_quantity: Quantity::parse_at_scale("0.01", 8).unwrap(),
        }),
        Vec::new(),
        Vec::new(),
        "1.0.0",
    )
}

fn liquidation_started_trigger(height: u64, event_index: u32) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::LiquidationStarted(canonical_events::LiquidationStarted {
            account_id: BUYER,
            liquidation_id: LiquidationId::new(format!("liq-seed-{height}")).unwrap(),
            margin_value: domain_types::UsdAmount::from_str("1").unwrap(),
            maintenance_requirement: domain_types::UsdAmount::from_str("2").unwrap(),
        }),
        Vec::new(),
        vec![BUYER],
        "1.0.0",
    )
}

fn raw_event(
    height: u64,
    event_index: u32,
    payload: EventPayload,
    markets: Vec<MarketId>,
    accounts: Vec<Address>,
    schema: &str,
) -> CanonicalEventEnvelope {
    try_raw_event(height, event_index, payload, markets, accounts, schema).unwrap()
}

fn try_raw_event(
    height: u64,
    event_index: u32,
    payload: EventPayload,
    markets: Vec<MarketId>,
    accounts: Vec<Address>,
    schema: &str,
) -> Result<CanonicalEventEnvelope, canonical_events::ContractError> {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}-{event_index}")).unwrap(),
        transaction_index: event_index,
        canonical_event_index: 0,
        market_ids: markets,
        account_ids: accounts,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").unwrap(),
                "position-fixture",
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
        parser_version: "position-fixture@1.0.0".to_owned(),
        payload,
    })
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

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}

fn fixture_event_id() -> EventId {
    EventId::new("evt_cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc").unwrap()
}

fn exact_sized_current_wire(target_len: usize) -> Vec<u8> {
    let seed = EventId::new("e").unwrap();
    let base = current_wire(
        &BUYER.to_api_string(),
        market().as_str(),
        None,
        None,
        &seed,
        1,
    );
    let expanded = EventId::new("e".repeat(1 + target_len - base.len())).unwrap();
    current_wire(
        &BUYER.to_api_string(),
        market().as_str(),
        None,
        None,
        &expanded,
        1,
    )
}

fn current_wire(
    account: &str,
    market: &str,
    known_quantity: Option<&str>,
    first_anchor_event_id: Option<&EventId>,
    last_event_id: &EventId,
    height: u64,
) -> Vec<u8> {
    let known_quantity = known_quantity
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "null".to_owned());
    let first_anchor_event_id = first_anchor_event_id
        .map(|value| format!("\"{}\"", value.as_str()))
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-quantity-current/v1\",\"account_id\":\"{account}\",\"market_id\":\"{market}\",\"known_quantity\":{known_quantity},\"first_anchor_event_id\":{first_anchor_event_id},\"last_event_id\":\"{}\",\"last_block_height\":{height}}}",
        last_event_id.as_str()
    )
    .into_bytes()
}

fn effect_wire(
    event_id: &EventId,
    trade_id: &str,
    account: Address,
    role: TradeParticipantRoleV1,
    start: &str,
    fill: &str,
    result: &str,
) -> Vec<u8> {
    let role = match role {
        TradeParticipantRoleV1::Buyer => "buyer",
        TradeParticipantRoleV1::Seller => "seller",
    };
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-effect-fact/v1\",\"event_id\":\"{}\",\"trade_id\":\"{trade_id}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"role\":\"{role}\",\"anchor_transition\":\"first_observation\",\"start_position\":\"{start}\",\"fill_quantity\":\"{fill}\",\"result_position\":\"{result}\",\"rule_version\":\"hyperliquid-alpha-desk-canonical-position@1.0.0\"}}",
        event_id.as_str(),
        account.to_api_string(),
        market().as_str()
    )
    .into_bytes()
}

fn unresolved_wire(event_id: &EventId, liquidation_id: &str) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-unresolved-cause-fact/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"event_id\":\"{}\",\"liquidation_id\":\"{liquidation_id}\",\"cause\":\"backstop_liquidation\"}}",
        BUYER.to_api_string(),
        market().as_str(),
        event_id.as_str()
    )
    .into_bytes()
}

fn replace_bytes(input: &[u8], from: &[u8], to: &[u8]) -> Vec<u8> {
    let start = input
        .windows(from.len())
        .position(|window| window == from)
        .unwrap();
    let mut output = Vec::with_capacity(input.len() - from.len() + to.len());
    output.extend_from_slice(&input[..start]);
    output.extend_from_slice(to);
    output.extend_from_slice(&input[start + from.len()..]);
    output
}

const _: fn() = || {
    fn assert_position_error(_: PositionStateError) {}
    let _ = assert_position_error;
    fn assert_state_key(_: StateKey) {}
    let _ = assert_state_key;
};
