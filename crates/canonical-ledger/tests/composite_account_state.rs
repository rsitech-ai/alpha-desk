use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::str::FromStr;

use canonical_events::{
    AssetContextUpdated, BackstopLiquidation, BlockEnvelope, CanonicalEventEnvelope,
    CanonicalEventInput, ConfirmationClass, DepositCredited, DexCreated, EventKind, EventPayload,
    FundingPaid, LiquidationStarted, MarketCreated, OrderAccepted, PositionSettled, SourceEvidence,
    TradeMatched, TradeParticipantRoleV1, TradeParticipantV1,
};
use canonical_ledger::{
    ApplyContext, ApplyOutcome, BlockDeltaView, CanonicalAccountReducerV1, CanonicalLedger,
    CanonicalLiquidationReducerV1, CanonicalMarketReducerV1, CanonicalOrderReducerV1,
    CanonicalPositionEpisodeReducerV1, CanonicalPositionReducerV1, CanonicalStateReducerV1,
    CanonicalTradeReducerSetV2, CanonicalTradeReducerV1, CanonicalTradeReducerV2, EventReducer,
    LedgerLimits, PositionEffectFactRecordV1, PositionQuantityCurrentRecordV1, ReducerError,
    StateImage, StateImageLimits, StateKey, StateMutation, StateView,
};
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, DexId, FundingRate, KnownTime, LiquidationId, MarketId,
    OrderId, OrderSide, PositionQuantity, Price, ProtocolTime, Quantity, QuoteAmount, SourceId,
    TradeId, TransactionId, UsdAmount,
};

const BUYER: Address = Address::from_bytes([0x11; 20]);
const SELLER: Address = Address::from_bytes([0x22; 20]);
const OPERATOR: Address = Address::from_bytes([0x33; 20]);

#[test]
fn validated_constructor_exposes_the_exact_ordered_component_manifest() {
    let reducer = CanonicalStateReducerV1::try_new().expect("frozen manifest must validate");

    assert_eq!(
        reducer.reducer_set_version(),
        "hyperliquid-alpha-desk-canonical-state@1.0.0"
    );
    assert_eq!(
        reducer
            .component_manifest()
            .map(|component| (component.name(), component.version())),
        [
            ("market", "hyperliquid-alpha-desk-canonical-market@1.0.0"),
            ("order", "hyperliquid-alpha-desk-canonical-order@1.0.0"),
            ("trade_v1", "hyperliquid-alpha-desk-canonical-trade@1.0.0"),
            ("trade_v2", "hyperliquid-alpha-desk-canonical-trade@2.0.0"),
            ("account", "hyperliquid-alpha-desk-canonical-account@1.0.0"),
            (
                "position_quantity",
                "hyperliquid-alpha-desk-canonical-position@1.0.0"
            ),
            (
                "position_episode",
                "hyperliquid-alpha-desk-canonical-position-episode@1.0.0"
            ),
            (
                "position_liquidation",
                "hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0"
            ),
        ]
    );
}

#[test]
fn ledger_and_direct_reducer_keep_unsupported_boundaries_separate() {
    let trigger = EventPayload::fixtures()
        .unwrap()
        .into_iter()
        .find(|payload| payload.kind() == EventKind::TriggerOrderActivated)
        .unwrap();
    let event = raw_event(1, 0, trigger, Vec::new(), Vec::new(), "1.0.0");
    let mut ledger = composite_ledger(1);

    let error = ledger
        .apply_block(&block(1, vec![event]))
        .expect_err("ledger must reject unsupported ownership before reduce");

    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert_eq!(error.reducer_reason_code(), None);
    assert_eq!(error.event_kind(), Some(EventKind::TriggerOrderActivated));
    assert!(ledger.checkpoint().is_none());
    assert!(ledger.state_image().entries().is_empty());

    let non_v1 = legacy_trade_event(1, 0, "legacy-non-v1", "1.1.0");
    let error = ledger
        .apply_block(&block(1, vec![non_v1]))
        .expect_err("non-1.0.0 events are unsupported");
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert_eq!(error.schema_version(), Some("1.1.0"));
}

#[test]
fn every_named_pre_composite_checkpoint_is_refused() {
    let component_states = [
        state_for(CanonicalMarketReducerV1),
        state_for(CanonicalOrderReducerV1),
        state_for(CanonicalTradeReducerV1),
        state_for(CanonicalTradeReducerV2),
        state_for(CanonicalTradeReducerSetV2),
        state_for(CanonicalAccountReducerV1),
        state_for(CanonicalPositionReducerV1),
        state_for(CanonicalPositionEpisodeReducerV1),
        state_for(CanonicalLiquidationReducerV1),
    ];
    assert_eq!(
        component_states
            .iter()
            .map(StateImage::reducer_set_version)
            .collect::<Vec<_>>(),
        [
            "hyperliquid-alpha-desk-canonical-market@1.0.0",
            "hyperliquid-alpha-desk-canonical-order@1.0.0",
            "hyperliquid-alpha-desk-canonical-trade@1.0.0",
            "hyperliquid-alpha-desk-canonical-trade@2.0.0",
            "hyperliquid-alpha-desk-canonical-trade-set@2.0.0",
            "hyperliquid-alpha-desk-canonical-account@1.0.0",
            "hyperliquid-alpha-desk-canonical-position@1.0.0",
            "hyperliquid-alpha-desk-canonical-position-episode@1.0.0",
            "hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0",
        ]
    );

    for state in component_states {
        let error = CanonicalLedger::try_from_state_image(
            state,
            CanonicalStateReducerV1::try_new().unwrap(),
            LedgerLimits::production(),
        )
        .expect_err("component checkpoint is never a composite migration");
        assert_eq!(error.reason_code(), "ledger.reducer_version_drift");
    }
}

#[test]
fn order_lifecycle_writes_no_positions_and_enriched_trade_applies_each_participant_once() {
    let mut ledger = composite_ledger(10);
    ledger
        .apply_block(&block(10, market_prerequisites(10)))
        .unwrap();

    let order_delta = applied(
        ledger
            .apply_block(&block(
                11,
                vec![
                    order_accepted_event(11, 0, "buyer-order", BUYER, OrderSide::Buy),
                    order_accepted_event(11, 1, "seller-order", SELLER, OrderSide::Sell),
                ],
            ))
            .unwrap(),
    );
    assert!(
        order_delta
            .mutations()
            .iter()
            .all(|mutation| !mutation.key().namespace().starts_with("position-"))
    );

    let trade_id = TradeId::new("composite-enriched").unwrap();
    ledger
        .apply_block(&block(
            12,
            vec![enriched_trade_event(
                12,
                0,
                trade_id.as_str(),
                "1.00000000",
                "-1.00000000",
            )],
        ))
        .unwrap();

    assert_eq!(
        current_quantity(&ledger, BUYER),
        PositionQuantity::from_str("1.25000000").unwrap()
    );
    assert_eq!(
        current_quantity(&ledger, SELLER),
        PositionQuantity::from_str("-1.25000000").unwrap()
    );
    assert_eq!(namespace_count(&ledger, "position-effect-fact.v1"), 2);
    assert_eq!(
        namespace_count(&ledger, "position-episode-effect-fact.v1"),
        2
    );
    assert_eq!(namespace_count(&ledger, "trade.v1"), 1);
    assert_eq!(namespace_count(&ledger, "trade-participant.v1"), 2);
    assert_eq!(namespace_count(&ledger, "trade.v2"), 1);
    assert_eq!(namespace_count(&ledger, "trade-participant.v2"), 2);

    for role in [
        TradeParticipantRoleV1::Buyer,
        TradeParticipantRoleV1::Seller,
    ] {
        let key = PositionEffectFactRecordV1::state_key(&trade_id, role).unwrap();
        PositionEffectFactRecordV1::decode_at(
            &key,
            ledger.state_image().entries().get(&key).unwrap(),
        )
        .unwrap();
    }

    let position_entries_before_legacy = ledger
        .state_image()
        .entries()
        .keys()
        .filter(|key| key.namespace().starts_with("position-"))
        .count();
    ledger
        .apply_block(&block(
            13,
            vec![legacy_trade_event(13, 0, "composite-legacy", "1.0.0")],
        ))
        .unwrap();

    assert_eq!(namespace_count(&ledger, "trade.v1"), 2);
    assert_eq!(namespace_count(&ledger, "trade.v2"), 1);
    assert_eq!(
        ledger
            .state_image()
            .entries()
            .keys()
            .filter(|key| key.namespace().starts_with("position-"))
            .count(),
        position_entries_before_legacy
    );
}

#[test]
fn account_only_and_funding_fanout_remain_distinct_and_repeated_writes_are_counted() {
    let observed_delta = Rc::new(RefCell::new(Vec::new()));
    let reducer = DeltaRecordingComposite {
        real: CanonicalStateReducerV1::try_new().unwrap(),
        observed_delta: Rc::clone(&observed_delta),
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(30),
        reducer,
        LedgerLimits::production(),
    )
    .unwrap();
    ledger
        .apply_block(&block(30, market_prerequisites(30)))
        .unwrap();

    let account_only = applied(
        ledger
            .apply_block(&block(
                31,
                vec![deposit_event(31, 0, "deposit-account-only", BUYER)],
            ))
            .unwrap(),
    );
    assert_eq!(
        account_only
            .mutations()
            .iter()
            .map(|mutation| mutation.key().namespace())
            .collect::<Vec<_>>(),
        ["account-fact.v1", "account-quantity-flow-current.v1"]
    );

    ledger
        .apply_block(&block(
            32,
            vec![
                order_accepted_event(32, 0, "buyer-order", BUYER, OrderSide::Buy),
                order_accepted_event(32, 1, "seller-order", SELLER, OrderSide::Sell),
            ],
        ))
        .unwrap();
    ledger
        .apply_block(&block(
            33,
            vec![enriched_trade_event(
                33,
                0,
                "funding-open",
                "1.00000000",
                "-1.00000000",
            )],
        ))
        .unwrap();

    observed_delta.borrow_mut().clear();
    let funding_delta = applied(
        ledger
            .apply_block(&block(
                34,
                vec![
                    funding_paid_event(34, 0, "1.25"),
                    funding_paid_event(34, 1, "0.75"),
                ],
            ))
            .unwrap(),
    );
    assert_eq!(
        funding_delta
            .mutations()
            .iter()
            .filter(|mutation| mutation.key().namespace() == "account-fact.v1")
            .count(),
        2
    );
    assert_eq!(
        funding_delta
            .mutations()
            .iter()
            .filter(|mutation| { mutation.key().namespace() == "position-episode-effect-fact.v1" })
            .count(),
        2
    );

    let observed = observed_delta.borrow();
    assert_eq!(observed.len(), 1);
    assert_eq!(
        observed[0]
            .iter()
            .filter(|entry| entry.namespace == "account-quote-flow-current.v1")
            .map(|entry| entry.write_count)
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        observed[0]
            .iter()
            .filter(|entry| entry.namespace == "position-episode-current.v1")
            .map(|entry| entry.write_count)
            .collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn liquidation_backstop_and_settlement_fanout_once_without_account_cashflow() {
    let mut liquidation_ledger = composite_ledger(40);
    liquidation_ledger
        .apply_block(&block(40, market_prerequisites(40)))
        .unwrap();
    let liquidation_delta = applied(
        liquidation_ledger
            .apply_block(&block(
                41,
                vec![liquidation_started_event(41, 0), backstop_event(41, 1)],
            ))
            .unwrap(),
    );
    assert_eq!(
        liquidation_delta
            .mutations()
            .iter()
            .filter(|mutation| mutation.key().namespace() == "liquidation-start-fact.v1")
            .count(),
        1
    );
    assert_eq!(
        liquidation_delta
            .mutations()
            .iter()
            .filter(|mutation| mutation.key().namespace() == "backstop-liquidation-fact.v1")
            .count(),
        1
    );
    assert!(
        liquidation_delta
            .mutations()
            .iter()
            .all(|mutation| !mutation.key().namespace().starts_with("account-"))
    );

    let settlement_delta = applied(
        liquidation_ledger
            .apply_block(&block(42, vec![settlement_event(42, 0)]))
            .unwrap(),
    );
    assert_eq!(
        settlement_delta
            .mutations()
            .iter()
            .filter(|mutation| mutation.key().namespace() == "position-settlement-fact.v1")
            .count(),
        1
    );
    assert!(
        settlement_delta
            .mutations()
            .iter()
            .all(|mutation| mutation.key().namespace() != "account-quote-flow-current.v1")
    );
}

#[test]
fn later_immutable_trade_collision_rolls_back_all_earlier_block_mutations() {
    let mut ledger = composite_ledger(50);
    ledger
        .apply_block(&block(50, market_prerequisites(50)))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();
    let checkpoint_before = ledger.checkpoint();

    let error = ledger
        .apply_block(&block(
            51,
            vec![
                deposit_event(51, 0, "must-roll-back", BUYER),
                legacy_trade_event(51, 1, "duplicate-trade", "1.0.0"),
                legacy_trade_event(51, 2, "duplicate-trade", "1.0.0"),
            ],
        ))
        .expect_err("the second immutable trade fact must reject the complete block");

    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(
        error.reducer_reason_code(),
        Some("trade_state.trade_id_collision")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert_eq!(ledger.checkpoint(), checkpoint_before);
}

#[test]
fn late_real_episode_delta_failure_rolls_back_the_complete_block() {
    let reducer = CorruptingComposite {
        real: CanonicalStateReducerV1::try_new().unwrap(),
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(20),
        reducer,
        LedgerLimits::production(),
    )
    .unwrap();
    let before = ledger.state_image().canonical_bytes();

    let error = ledger
        .apply_block(&block(20, market_prerequisites(20)))
        .expect_err("episode delta validation must reject the corrupt current");

    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.episode_current_invalid")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

#[derive(Debug, Clone, Copy)]
struct CorruptingComposite {
    real: CanonicalStateReducerV1,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ObservedDeltaEntry {
    namespace: String,
    write_count: u32,
}

#[derive(Debug, Clone)]
struct DeltaRecordingComposite {
    real: CanonicalStateReducerV1,
    observed_delta: Rc<RefCell<Vec<Vec<ObservedDeltaEntry>>>>,
}

impl EventReducer for DeltaRecordingComposite {
    fn reducer_set_version(&self) -> &str {
        CanonicalStateReducerV1::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        self.real.supports(event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        self.real.reduce(state, event, context)
    }

    fn validate_block(
        &self,
        state: &StateView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        self.real.validate_block(state, context)
    }

    fn validate_block_delta(
        &self,
        final_state: &StateView<'_>,
        delta: &BlockDeltaView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        self.real
            .validate_block_delta(final_state, delta, context)?;
        self.observed_delta.borrow_mut().push(
            delta
                .iter()
                .map(|entry| ObservedDeltaEntry {
                    namespace: entry.key().namespace().to_owned(),
                    write_count: entry.write_count(),
                })
                .collect(),
        );
        Ok(())
    }
}

impl EventReducer for CorruptingComposite {
    fn reducer_set_version(&self) -> &str {
        CanonicalStateReducerV1::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        self.real.supports(event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let mut mutations = self.real.reduce(state, event, context)?;
        if event.event_kind() == EventKind::MarketCreated {
            mutations.push(StateMutation::put(
                StateKey::try_new(
                    "position-episode-current.v1",
                    b"malformed-composite-current".to_vec(),
                )
                .unwrap(),
                b"malformed".to_vec(),
            ));
        }
        Ok(mutations)
    }

    fn validate_block(
        &self,
        state: &StateView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        self.real.validate_block(state, context)
    }

    fn validate_block_delta(
        &self,
        final_state: &StateView<'_>,
        delta: &BlockDeltaView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        self.real.validate_block_delta(final_state, delta, context)
    }
}

fn composite_ledger(first_height: u64) -> CanonicalLedger<CanonicalStateReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalStateReducerV1::try_new().unwrap(),
        LedgerLimits::production(),
    )
    .unwrap()
}

fn state_for<R: EventReducer>(reducer: R) -> StateImage {
    let ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(1),
        reducer,
        LedgerLimits::production(),
    )
    .unwrap();
    StateImage::decode_canonical(
        &ledger.state_image().canonical_bytes(),
        StateImageLimits::production(),
    )
    .unwrap()
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

fn order_accepted_event(
    height: u64,
    event_index: u32,
    order_id: &str,
    account: Address,
    side: OrderSide,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::OrderAccepted(OrderAccepted {
            order_id: OrderId::new(order_id).unwrap(),
            account_id: account,
            market_id: market(),
            side,
            limit_price: Price::parse_at_scale("65000", 6).unwrap(),
            quantity: Quantity::parse_at_scale("1", 8).unwrap(),
        }),
        vec![market()],
        vec![account],
        "1.0.0",
    )
}

fn enriched_trade_event(
    height: u64,
    event_index: u32,
    trade_id: &str,
    buyer_start: &str,
    seller_start: &str,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new(trade_id).unwrap()),
            market_id: Some(market()),
            maker_order_id: Some(OrderId::new("seller-order").unwrap()),
            taker_order_id: Some(OrderId::new("buyer-order").unwrap()),
            price: Price::parse_at_scale("65000", 6).unwrap(),
            quantity: Quantity::parse_at_scale("0.25", 8).unwrap(),
            deterministic_seed: height,
            participants: Some(Box::new([
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Buyer,
                    account_id: BUYER,
                    start_position: PositionQuantity::from_str(buyer_start).unwrap(),
                    order_id: OrderId::new("buyer-order").unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Seller,
                    account_id: SELLER,
                    start_position: PositionQuantity::from_str(seller_start).unwrap(),
                    order_id: OrderId::new("seller-order").unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
            ])),
        }),
        vec![market()],
        vec![BUYER, SELLER],
        "1.0.0",
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
            quantity: Quantity::parse_at_scale("0.25", 8).unwrap(),
            deterministic_seed: height,
            participants: None,
        }),
        vec![market()],
        vec![BUYER, SELLER],
        schema,
    )
}

fn deposit_event(
    height: u64,
    event_index: u32,
    reference: &str,
    account: Address,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::DepositCredited(DepositCredited {
            account_id: account,
            asset_id: AssetId::new("USDC").unwrap(),
            amount: Quantity::from_str("10").unwrap(),
            deposit_reference: reference.to_owned(),
        }),
        Vec::new(),
        vec![account],
        "1.0.0",
    )
}

fn funding_paid_event(height: u64, event_index: u32, amount: &str) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::FundingPaid(FundingPaid {
            account_id: BUYER,
            market_id: market(),
            amount: QuoteAmount::from_str(amount).unwrap(),
            funding_rate: FundingRate::from_str("0.0001").unwrap(),
        }),
        vec![market()],
        vec![BUYER],
        "1.0.0",
    )
}

fn liquidation_started_event(height: u64, event_index: u32) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::LiquidationStarted(LiquidationStarted {
            account_id: BUYER,
            liquidation_id: liquidation(),
            margin_value: UsdAmount::from_str("9").unwrap(),
            maintenance_requirement: UsdAmount::from_str("10").unwrap(),
        }),
        Vec::new(),
        vec![BUYER],
        "1.0.0",
    )
}

fn backstop_event(height: u64, event_index: u32) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::BackstopLiquidation(BackstopLiquidation {
            liquidation_id: liquidation(),
            account_id: BUYER,
            backstop_account_id: SELLER,
            market_id: market(),
            quantity: Quantity::from_str("1").unwrap(),
        }),
        vec![market()],
        vec![BUYER, SELLER],
        "1.0.0",
    )
}

fn settlement_event(height: u64, event_index: u32) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::PositionSettled(PositionSettled {
            account_id: BUYER,
            market_id: market(),
            settlement_price: Price::from_str("0").unwrap(),
            settled_quantity: Quantity::from_str("1").unwrap(),
            realized_pnl: QuoteAmount::from_str("-2.5").unwrap(),
        }),
        vec![market()],
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
                "composite-fixture",
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
        parser_version: "composite-fixture@1.0.0".to_owned(),
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

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}

fn liquidation() -> LiquidationId {
    LiquidationId::new("liq-composite").unwrap()
}

fn current_quantity(
    ledger: &CanonicalLedger<CanonicalStateReducerV1>,
    account: Address,
) -> PositionQuantity {
    let key = PositionQuantityCurrentRecordV1::state_key(&account, &market()).unwrap();
    PositionQuantityCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
    .known_quantity()
    .unwrap()
}

fn namespace_count(ledger: &CanonicalLedger<CanonicalStateReducerV1>, namespace: &str) -> usize {
    ledger
        .state_image()
        .entries()
        .keys()
        .filter(|key| key.namespace() == namespace)
        .count()
}

fn applied(outcome: ApplyOutcome) -> canonical_ledger::StateDelta {
    match outcome {
        ApplyOutcome::Applied(delta) => delta,
        ApplyOutcome::AlreadyApplied(_) => panic!("test block must be new"),
    }
}
