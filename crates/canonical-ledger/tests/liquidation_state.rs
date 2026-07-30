use std::collections::BTreeMap;
use std::str::FromStr;

use canonical_events::{
    AssetContextUpdated, BackstopLiquidation, BlockEnvelope, CanonicalEventEnvelope,
    CanonicalEventInput, ConfirmationClass, DexCreated, EventKind, EventPayload, FundingPaid,
    LiquidationFill, LiquidationStarted, MarketCreated, PositionSettled, SourceEvidence,
    TradeMatched, TradeParticipantRoleV1, TradeParticipantV1,
};
use canonical_ledger::{
    ApplyContext, ApplyOutcome, BackstopLiquidationFactRecordV1, CanonicalLedger,
    CanonicalLiquidationReducerV1, CanonicalMarketReducerV1, CanonicalPositionEpisodeReducerV1,
    CanonicalPositionReducerV1, EpisodeAttributionResolutionV1, EpisodeCloseCauseV1,
    EpisodeCompletenessV1, EpisodeEffectKindV1, EpisodeStatusV1, EventReducer, LedgerLimits,
    LiquidationCurrentRecordV1, LiquidationFillFactRecordV1, LiquidationMarketFlowCurrentRecordV1,
    LiquidationObservedStatusV1, LiquidationStartFactRecordV1, PositionEpisodeCurrentRecordV1,
    PositionEpisodeEffectFactRecordV1, PositionEpisodeRecordV1, PositionQuantityCurrentRecordV1,
    PositionSettlementFactRecordV1, PositionUnresolvedCauseFactRecordV1, ReducerError, StateImage,
    StateImageLimits, StateMutation, StateView, derive_position_episode_id,
};
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, DexId, EventId, FundingRate, KnownTime, LiquidationId,
    MarketId, OrderId, PositionQuantity, Price, ProtocolTime, Quantity, QuoteAmount, SourceId,
    TradeId, TransactionId, UsdAmount,
};

const LIQUIDATED: Address = Address::from_bytes([0x11; 20]);
const BACKSTOP: Address = Address::from_bytes([0x22; 20]);

#[derive(Debug, Clone)]
struct InjectionDispatcher {
    injection_height: BlockHeight,
    injections: Vec<StateMutation>,
    liquidation: CanonicalLiquidationReducerV1,
}

impl EventReducer for InjectionDispatcher {
    fn reducer_set_version(&self) -> &str {
        "liquidation-state-injection@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        EventReducer::supports(&self.liquidation, event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if context.block_height() == self.injection_height
            && event.event_kind() == EventKind::PositionSettled
        {
            Ok(self.injections.clone())
        } else {
            EventReducer::reduce(&self.liquidation, state, event, context)
        }
    }
}

#[derive(Debug, Clone)]
struct RecoveryDispatcher {
    injection_height: BlockHeight,
    injections: Vec<StateMutation>,
    market: CanonicalMarketReducerV1,
    liquidation: CanonicalLiquidationReducerV1,
    quantity: CanonicalPositionReducerV1,
    episode: CanonicalPositionEpisodeReducerV1,
}

impl EventReducer for RecoveryDispatcher {
    fn reducer_set_version(&self) -> &str {
        "liquidation-state-recovery@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        EventReducer::supports(&self.market, event)
            || EventReducer::supports(&self.liquidation, event)
            || EventReducer::supports(&self.quantity, event)
            || EventReducer::supports(&self.episode, event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if context.block_height() == self.injection_height
            && event.event_kind() == EventKind::PositionSettled
        {
            return Ok(self.injections.clone());
        }
        if EventReducer::supports(&self.market, event) {
            return EventReducer::reduce(&self.market, state, event, context);
        }
        if EventReducer::supports(&self.liquidation, event) {
            return EventReducer::reduce(&self.liquidation, state, event, context);
        }
        let mut mutations = Vec::new();
        if EventReducer::supports(&self.quantity, event) {
            mutations.extend(EventReducer::reduce(&self.quantity, state, event, context)?);
        }
        if EventReducer::supports(&self.episode, event) {
            mutations.extend(EventReducer::reduce(&self.episode, state, event, context)?);
        }
        let mut keys = std::collections::BTreeSet::new();
        if !mutations.iter().all(|mutation| keys.insert(mutation.key())) {
            return Err(ReducerError::try_new(
                "liquidation_state.duplicate_mutation_key",
                "recovery dispatcher children emitted duplicate keys",
            )
            .unwrap());
        }
        Ok(mutations)
    }
}

#[test]
fn liquidation_reducer_owns_exactly_the_four_frozen_event_kinds_at_schema_v1() {
    let reducer = CanonicalLiquidationReducerV1;
    assert_eq!(
        EventReducer::reducer_set_version(&reducer),
        "hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0"
    );

    for payload in owned_payloads() {
        let exact = event(100, 0, payload.clone(), "1.0.0");
        assert!(EventReducer::supports(&reducer, &exact));

        let later_schema = event(100, 0, payload, "1.1.0");
        assert!(!EventReducer::supports(&reducer, &later_schema));
    }

    assert!(!EventReducer::supports(
        &reducer,
        &CanonicalEventEnvelope::fixture().unwrap()
    ));

    let owned_block = block(
        100,
        vec![event(100, 0, owned_payloads()[0].clone(), "1.0.0")],
    );
    assert_eq!(owned_block.events().len(), 1);
}

#[test]
fn start_atomically_writes_the_immutable_fact_before_the_process_current() {
    let start = event(101, 3, owned_payloads()[0].clone(), "1.0.0");
    let event_id = start.event_id().clone();
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(101),
        CanonicalLiquidationReducerV1,
        LedgerLimits::production(),
    )
    .unwrap();

    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block(101, vec![start])).unwrap() else {
        panic!("fresh start must apply");
    };
    assert_eq!(
        delta
            .mutations()
            .iter()
            .map(|mutation| mutation.key().namespace())
            .collect::<Vec<_>>(),
        ["liquidation-start-fact.v1", "liquidation-current.v1"]
    );

    let fact_key = LiquidationStartFactRecordV1::state_key(&liquidation(), &event_id).unwrap();
    let fact = LiquidationStartFactRecordV1::decode_at(
        &fact_key,
        &ledger.state_image().entries()[&fact_key],
    )
    .unwrap();
    assert_eq!(fact.account_id(), LIQUIDATED);
    assert_eq!(fact.block_height(), BlockHeight::new(101));
    assert_eq!(fact.transaction_index(), 3);

    let current_key = LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap();
    let current = LiquidationCurrentRecordV1::decode_at(
        &current_key,
        &ledger.state_image().entries()[&current_key],
    )
    .unwrap();
    assert_eq!(current.account_id(), LIQUIDATED);
    assert_eq!(
        current.observed_status(),
        LiquidationObservedStatusV1::Started
    );
    assert_eq!(current.start_event_id(), &event_id);
    assert_eq!(current.last_observation_event_id(), &event_id);
}

#[test]
fn fill_without_position_retains_fact_flow_and_process_but_marks_attribution_unknown() {
    let start = event(110, 0, owned_payloads()[0].clone(), "1.0.0");
    let fill = event(110, 1, owned_payloads()[1].clone(), "1.0.0");
    let fill_event_id = fill.event_id().clone();
    let mut ledger = liquidation_ledger(110);

    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block(110, vec![start, fill])).unwrap()
    else {
        panic!("fresh start and fill block must apply");
    };
    let liquidation_namespaces = delta
        .mutations()
        .iter()
        .map(|mutation| mutation.key().namespace())
        .collect::<Vec<_>>();
    assert_eq!(
        liquidation_namespaces,
        [
            "liquidation-start-fact.v1",
            "liquidation-current.v1",
            "liquidation-fill-fact.v1",
            "liquidation-market-flow-current.v1",
            "liquidation-current.v1",
            "position-quantity-current.v1",
            "position-episode-current.v1",
        ]
    );

    let fact_key = LiquidationFillFactRecordV1::state_key(&liquidation(), &fill_event_id).unwrap();
    let fact = LiquidationFillFactRecordV1::decode_at(
        &fact_key,
        &ledger.state_image().entries()[&fact_key],
    )
    .unwrap();
    assert_eq!(fact.quantity().to_string(), "1");
    assert_eq!(fact.price().to_string(), "100");

    let flow_key =
        LiquidationMarketFlowCurrentRecordV1::state_key(&liquidation(), &LIQUIDATED, &market())
            .unwrap();
    let flow = LiquidationMarketFlowCurrentRecordV1::decode_at(
        &flow_key,
        &ledger.state_image().entries()[&flow_key],
    )
    .unwrap();
    assert_eq!(flow.observed_filled_quantity().to_string(), "1");
    assert_eq!(flow.first_fill_event_id(), &fill_event_id);
    assert_eq!(flow.last_fill_event_id(), &fill_event_id);

    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let quantity = PositionQuantityCurrentRecordV1::decode_at(
        &quantity_key,
        &ledger.state_image().entries()[&quantity_key],
    )
    .unwrap();
    assert_eq!(quantity.known_quantity(), None);
    assert_eq!(quantity.first_anchor_event_id(), None);
    assert_eq!(quantity.last_event_id(), &fill_event_id);

    let episode_key = PositionEpisodeCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let episode = PositionEpisodeCurrentRecordV1::decode_at(
        &episode_key,
        &ledger.state_image().entries()[&episode_key],
    )
    .unwrap();
    assert_eq!(
        episode.attribution_resolution(),
        EpisodeAttributionResolutionV1::Interrupted
    );
    assert_eq!(episode.episode_id(), None);
}

#[test]
fn backstop_atomically_marks_both_accounts_unknown_without_entering_fill_flow() {
    let start = event(120, 0, owned_payloads()[0].clone(), "1.0.0");
    let backstop = event(120, 1, owned_payloads()[2].clone(), "1.0.0");
    let backstop_event_id = backstop.event_id().clone();
    let mut ledger = liquidation_ledger(120);

    let ApplyOutcome::Applied(delta) = ledger
        .apply_block(&block(120, vec![start, backstop]))
        .unwrap()
    else {
        panic!("fresh start and backstop block must apply");
    };
    assert_eq!(
        delta
            .mutations()
            .iter()
            .map(|mutation| mutation.key().namespace())
            .collect::<Vec<_>>(),
        [
            "liquidation-start-fact.v1",
            "liquidation-current.v1",
            "backstop-liquidation-fact.v1",
            "liquidation-current.v1",
            "position-unresolved-cause-fact.v1",
            "position-quantity-current.v1",
            "position-episode-current.v1",
            "position-unresolved-cause-fact.v1",
            "position-quantity-current.v1",
            "position-episode-current.v1",
        ]
    );

    let fact_key =
        BackstopLiquidationFactRecordV1::state_key(&liquidation(), &backstop_event_id).unwrap();
    let fact = BackstopLiquidationFactRecordV1::decode_at(
        &fact_key,
        &ledger.state_image().entries()[&fact_key],
    )
    .unwrap();
    assert_eq!(fact.account_id(), LIQUIDATED);
    assert_eq!(fact.backstop_account_id(), BACKSTOP);

    for account in [LIQUIDATED, BACKSTOP] {
        let cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
            &account,
            &market(),
            &backstop_event_id,
            &liquidation(),
        )
        .unwrap();
        assert!(ledger.state_image().entries().contains_key(&cause_key));
        let quantity_key = PositionQuantityCurrentRecordV1::state_key(&account, &market()).unwrap();
        let quantity = PositionQuantityCurrentRecordV1::decode_at(
            &quantity_key,
            &ledger.state_image().entries()[&quantity_key],
        )
        .unwrap();
        assert_eq!(quantity.known_quantity(), None);
        assert_eq!(quantity.last_event_id(), &backstop_event_id);
    }
    let flow_key =
        LiquidationMarketFlowCurrentRecordV1::state_key(&liquidation(), &LIQUIDATED, &market())
            .unwrap();
    assert!(!ledger.state_image().entries().contains_key(&flow_key));
}

#[test]
fn settlement_is_process_independent_and_keeps_pnl_only_in_its_fact() {
    let settlement = event(130, 0, owned_payloads()[3].clone(), "1.0.0");
    let event_id = settlement.event_id().clone();
    let mut ledger = liquidation_ledger(130);
    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block(130, vec![settlement])).unwrap()
    else {
        panic!("standalone settlement must apply");
    };
    assert_eq!(
        delta
            .mutations()
            .iter()
            .map(|mutation| mutation.key().namespace())
            .collect::<Vec<_>>(),
        [
            "position-settlement-fact.v1",
            "position-quantity-current.v1",
            "position-episode-current.v1",
        ]
    );
    let fact_key =
        PositionSettlementFactRecordV1::state_key(&event_id, &LIQUIDATED, &market()).unwrap();
    let fact = PositionSettlementFactRecordV1::decode_at(
        &fact_key,
        &ledger.state_image().entries()[&fact_key],
    )
    .unwrap();
    assert_eq!(fact.realized_pnl().to_string(), "-2.5");
    assert!(
        !ledger
            .state_image()
            .entries()
            .keys()
            .any(|key| key.namespace() == "account-quote-flow-current.v1")
    );
    assert!(
        !ledger
            .state_image()
            .entries()
            .contains_key(&LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap())
    );
}

#[test]
fn exact_partial_fill_interrupts_ord0_and_opens_partial_ord1_at_upward_scale() {
    let mut ledger = injected_liquidation_ledger(200, resolved_pair_mutations("5.00"));
    ledger
        .apply_block(&block(
            200,
            vec![event(200, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    ledger
        .apply_block(&block(
            201,
            vec![event(201, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap();
    let fill = event(
        202,
        0,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: market(),
            price: Price::from_str("99.5").unwrap(),
            quantity: Quantity::from_str("2.5").unwrap(),
        }),
        "1.0.0",
    );
    let fill_event_id = fill.event_id().clone();
    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block(202, vec![fill])).unwrap() else {
        panic!("partial liquidation fill must apply");
    };
    assert_eq!(
        delta
            .mutations()
            .iter()
            .map(|mutation| mutation.key().namespace())
            .collect::<Vec<_>>(),
        [
            "liquidation-fill-fact.v1",
            "liquidation-market-flow-current.v1",
            "liquidation-current.v1",
            "position-episode-effect-fact.v1",
            "position-episode-effect-fact.v1",
            "position-episode.v1",
            "position-episode.v1",
            "position-quantity-current.v1",
            "position-episode-current.v1",
        ]
    );

    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let quantity = PositionQuantityCurrentRecordV1::decode_at(
        &quantity_key,
        &ledger.state_image().entries()[&quantity_key],
    )
    .unwrap();
    assert_eq!(quantity.known_quantity().unwrap().to_string(), "2.50");
    assert_eq!(
        quantity.first_anchor_event_id(),
        Some(&EventId::new("seed-open").unwrap())
    );

    let old_id = derive_position_episode_id(
        &LIQUIDATED,
        &market(),
        &EventId::new("seed-open").unwrap(),
        0,
    )
    .unwrap();
    let old_key = PositionEpisodeRecordV1::state_key(&old_id).unwrap();
    let old =
        PositionEpisodeRecordV1::decode_at(&old_key, &ledger.state_image().entries()[&old_key])
            .unwrap();
    assert_eq!(old.status(), EpisodeStatusV1::Interrupted);
    assert_eq!(
        old.close_cause(),
        Some(EpisodeCloseCauseV1::LiquidationFill)
    );

    let new_id = derive_position_episode_id(&LIQUIDATED, &market(), &fill_event_id, 1).unwrap();
    let new_key = PositionEpisodeRecordV1::state_key(&new_id).unwrap();
    let new =
        PositionEpisodeRecordV1::decode_at(&new_key, &ledger.state_image().entries()[&new_key])
            .unwrap();
    assert_eq!(new.status(), EpisodeStatusV1::Open);
    assert_eq!(
        new.completeness(),
        EpisodeCompletenessV1::PartialFromFirstObservation
    );
    assert_eq!(new.opening_position().to_string(), "2.50");
    assert_eq!(new.buy_quantity().to_string(), "0.00");
    assert_eq!(new.sell_quantity().to_string(), "0.00");
}

#[test]
fn partial_liquidation_fill_ordinals_are_reconciled_when_an_enriched_trade_closes_the_new_episode()
{
    let mut ledger = seeded_recovery_ledger(720, "3.00000000");
    let fill = event(722, 0, owned_payloads()[1].clone(), "1.0.0");
    let fill_event_id = fill.event_id().clone();
    ledger.apply_block(&block(722, vec![fill])).unwrap();

    let old_id = derive_position_episode_id(
        &LIQUIDATED,
        &market(),
        &EventId::new("seed-open").unwrap(),
        0,
    )
    .unwrap();
    let partial_id = derive_position_episode_id(&LIQUIDATED, &market(), &fill_event_id, 1).unwrap();
    let interrupted = episode_by_id(&ledger, &old_id);
    assert_eq!(interrupted.status(), EpisodeStatusV1::Interrupted);
    assert_eq!(
        interrupted.close_cause(),
        Some(EpisodeCloseCauseV1::LiquidationFill)
    );
    let interrupted_effect = episode_effect(&ledger, &fill_event_id, LIQUIDATED, 0);
    assert_eq!(interrupted_effect.episode_id(), &old_id);
    assert_eq!(
        interrupted_effect.effect_kind(),
        EpisodeEffectKindV1::Interrupted
    );
    let opened_effect = episode_effect(&ledger, &fill_event_id, LIQUIDATED, 1);
    assert_eq!(opened_effect.episode_id(), &partial_id);
    assert_eq!(opened_effect.effect_kind(), EpisodeEffectKindV1::Opened);

    let partial = episode_by_id(&ledger, &partial_id);
    assert_eq!(partial.status(), EpisodeStatusV1::Open);
    assert_eq!(
        partial.completeness(),
        EpisodeCompletenessV1::PartialFromFirstObservation
    );
    assert_eq!(partial.opening_position().to_string(), "2.00000000");

    let close = enriched_trade_event(
        723,
        "close-partial-liquidation",
        BACKSTOP,
        "0",
        LIQUIDATED,
        "2.00000000",
        "2",
    );
    let close_event_id = close.event_id().clone();
    ledger.apply_block(&block(723, vec![close])).unwrap();

    let closed = episode_by_id(&ledger, &partial_id);
    assert_eq!(closed.status(), EpisodeStatusV1::Closed);
    assert_eq!(closed.close_event_id(), Some(&close_event_id));
    assert_eq!(closed.close_cause(), Some(EpisodeCloseCauseV1::TradeFlat));
    assert_eq!(closed.sell_quantity().to_string(), "2.00000000");
    assert_eq!(closed.sell_notional().to_string(), "200");
    assert_eq!(closed.observed_signed_trade_notional_delta().unwrap(), None);
    assert_eq!(known_quantity(&ledger, LIQUIDATED), "0.00000000");
    assert_eq!(
        episode_current(&ledger).attribution_resolution(),
        EpisodeAttributionResolutionV1::NoOpenEpisode
    );
    let close_effect = episode_effect(&ledger, &close_event_id, LIQUIDATED, 0);
    assert_eq!(close_effect.episode_id(), &partial_id);
    assert_eq!(close_effect.effect_kind(), EpisodeEffectKindV1::Closed);
    assert_eq!(
        close_effect.close_cause(),
        Some(EpisodeCloseCauseV1::TradeFlat)
    );
}

#[test]
fn exact_flat_fill_keeps_scale_and_overrun_rejects_atomically() {
    let mut flat = injected_liquidation_ledger(210, resolved_pair_mutations("-2.00"));
    flat.apply_block(&block(
        210,
        vec![event(210, 0, owned_payloads()[3].clone(), "1.0.0")],
    ))
    .unwrap();
    flat.apply_block(&block(
        211,
        vec![event(211, 0, owned_payloads()[0].clone(), "1.0.0")],
    ))
    .unwrap();
    let flat_fill = event(
        212,
        0,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: market(),
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("2").unwrap(),
        }),
        "1.0.0",
    );
    flat.apply_block(&block(212, vec![flat_fill])).unwrap();
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let flat_quantity = PositionQuantityCurrentRecordV1::decode_at(
        &quantity_key,
        &flat.state_image().entries()[&quantity_key],
    )
    .unwrap();
    assert_eq!(flat_quantity.known_quantity().unwrap().to_string(), "0.00");
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let current = PositionEpisodeCurrentRecordV1::decode_at(
        &current_key,
        &flat.state_image().entries()[&current_key],
    )
    .unwrap();
    assert_eq!(
        current.attribution_resolution(),
        EpisodeAttributionResolutionV1::NoOpenEpisode
    );
    let before_zero_fill = flat.state_image().clone();
    let error = flat
        .apply_block(&block(
            213,
            vec![event(213, 0, owned_payloads()[1].clone(), "1.0.0")],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.fill_overrun")
    );
    assert_eq!(flat.state_image(), &before_zero_fill);

    let mut overrun = injected_liquidation_ledger(220, resolved_pair_mutations("1.0"));
    overrun
        .apply_block(&block(
            220,
            vec![event(220, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    overrun
        .apply_block(&block(
            221,
            vec![event(221, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap();
    let before = overrun.state_image().clone();
    let error = overrun
        .apply_block(&block(
            222,
            vec![event(
                222,
                0,
                EventPayload::LiquidationFill(LiquidationFill {
                    liquidation_id: liquidation(),
                    account_id: LIQUIDATED,
                    market_id: market(),
                    price: Price::from_str("100").unwrap(),
                    quantity: Quantity::from_str("2").unwrap(),
                }),
                "1.0.0",
            )],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.fill_overrun")
    );
    assert_eq!(overrun.state_image(), &before);
}

#[test]
fn later_fill_against_retained_known_zero_and_no_open_episode_is_an_atomic_overrun() {
    let mut ledger = seeded_pair_ledger(730, "3.00");
    let full_fill = event(
        732,
        0,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: market(),
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("3").unwrap(),
        }),
        "1.0.0",
    );
    ledger.apply_block(&block(732, vec![full_fill])).unwrap();
    assert_quantity(&ledger, Some("0.00"));
    assert_eq!(
        episode_current(&ledger).attribution_resolution(),
        EpisodeAttributionResolutionV1::NoOpenEpisode
    );

    let later_fill = event(733, 0, owned_payloads()[1].clone(), "1.0.0");
    let fact_key =
        LiquidationFillFactRecordV1::state_key(&liquidation(), later_fill.event_id()).unwrap();
    let flow_key =
        LiquidationMarketFlowCurrentRecordV1::state_key(&liquidation(), &LIQUIDATED, &market())
            .unwrap();
    let process_key = LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap();
    let before = ledger.state_image().clone();
    let flow_before = before.entries()[&flow_key].clone();
    let process_before = before.entries()[&process_key].clone();
    assert!(!before.entries().contains_key(&fact_key));

    let error = ledger
        .apply_block(&block(733, vec![later_fill]))
        .expect_err("a later fill against retained known-zero state must overrun");
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.fill_overrun")
    );
    assert_eq!(ledger.state_image(), &before);
    assert!(!ledger.state_image().entries().contains_key(&fact_key));
    assert_eq!(ledger.state_image().entries()[&flow_key], flow_before);
    assert_eq!(ledger.state_image().entries()[&process_key], process_before);
}

#[test]
fn same_block_fills_see_candidate_process_and_accumulate_per_market_upward() {
    let start = event(230, 0, owned_payloads()[0].clone(), "1.0.0");
    let first = event(
        230,
        1,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: market(),
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("1").unwrap(),
        }),
        "1.0.0",
    );
    let second = event(
        230,
        2,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: market(),
            price: Price::from_str("99").unwrap(),
            quantity: Quantity::from_str("0.25").unwrap(),
        }),
        "1.0.0",
    );
    let second_id = second.event_id().clone();
    let mut ledger = liquidation_ledger(230);
    ledger
        .apply_block(&block(230, vec![start, first, second]))
        .unwrap();
    let flow_key =
        LiquidationMarketFlowCurrentRecordV1::state_key(&liquidation(), &LIQUIDATED, &market())
            .unwrap();
    let flow = LiquidationMarketFlowCurrentRecordV1::decode_at(
        &flow_key,
        &ledger.state_image().entries()[&flow_key],
    )
    .unwrap();
    assert_eq!(flow.observed_filled_quantity().to_string(), "1.25");
    assert_eq!(flow.last_fill_event_id(), &second_id);
    let process_key = LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap();
    let process = LiquidationCurrentRecordV1::decode_at(
        &process_key,
        &ledger.state_image().entries()[&process_key],
    )
    .unwrap();
    assert_eq!(process.last_observation_event_id(), &second_id);
}

#[test]
fn per_market_flows_are_isolated_and_overflow_rejects_before_account_state() {
    let other_market = MarketId::new("perp:ETH").unwrap();
    let start = event(233, 0, owned_payloads()[0].clone(), "1.0.0");
    let btc = event(233, 1, owned_payloads()[1].clone(), "1.0.0");
    let eth = event(
        233,
        2,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: other_market.clone(),
            price: Price::from_str("50").unwrap(),
            quantity: Quantity::from_str("2.50").unwrap(),
        }),
        "1.0.0",
    );
    let mut isolated = liquidation_ledger(233);
    isolated
        .apply_block(&block(233, vec![start, btc, eth]))
        .unwrap();
    for (market_id, expected) in [(market(), "1"), (other_market, "2.50")] {
        let key = LiquidationMarketFlowCurrentRecordV1::state_key(
            &liquidation(),
            &LIQUIDATED,
            &market_id,
        )
        .unwrap();
        let flow = LiquidationMarketFlowCurrentRecordV1::decode_at(
            &key,
            &isolated.state_image().entries()[&key],
        )
        .unwrap();
        assert_eq!(flow.observed_filled_quantity().to_string(), expected);
    }

    let mut overflow = injected_liquidation_ledger(
        234,
        vec![
            process_mutation("flow-last", 234, 0, 0),
            flow_mutation_with_quantity(
                "flow-last",
                234,
                0,
                0,
                "170141183460469231731687303715884105727",
            ),
        ],
    );
    overflow
        .apply_block(&block(
            234,
            vec![event(234, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    overflow.apply_block(&block(235, Vec::new())).unwrap();
    let before = overflow.state_image().clone();
    let error = overflow
        .apply_block(&block(
            236,
            vec![event(236, 0, owned_payloads()[1].clone(), "1.0.0")],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.flow_arithmetic")
    );
    assert_eq!(overflow.state_image(), &before);
}

#[test]
fn process_missing_and_repeated_start_fail_with_frozen_precedence_and_no_advance() {
    let mut missing = liquidation_ledger(240);
    let before_missing = missing.state_image().clone();
    let error = missing
        .apply_block(&block(
            240,
            vec![event(240, 0, owned_payloads()[1].clone(), "1.0.0")],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.process_missing")
    );
    assert_eq!(missing.state_image(), &before_missing);

    let mut repeated = liquidation_ledger(241);
    repeated
        .apply_block(&block(
            241,
            vec![event(241, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap();
    let before_repeat = repeated.state_image().clone();
    let error = repeated
        .apply_block(&block(
            242,
            vec![event(242, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.process_identity_collision")
    );
    assert_eq!(repeated.state_image(), &before_repeat);
}

#[test]
fn envelope_process_and_primary_fact_failures_keep_their_frozen_precedence() {
    let mismatched = event_with_identities(
        243,
        0,
        owned_payloads()[1].clone(),
        "1.0.0",
        vec![market()],
        vec![BACKSTOP],
    );
    let mut identity = liquidation_ledger(243);
    let error = identity
        .apply_block(&block(243, vec![mismatched]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.identity_mismatch")
    );

    let mut account = liquidation_ledger(244);
    account
        .apply_block(&block(
            244,
            vec![event(244, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap();
    let wrong_account_fill = event(
        245,
        0,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: BACKSTOP,
            market_id: market(),
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("1").unwrap(),
        }),
        "1.0.0",
    );
    let before_account = account.state_image().clone();
    let error = account
        .apply_block(&block(245, vec![wrong_account_fill]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.process_account_mismatch")
    );
    assert_eq!(account.state_image(), &before_account);

    let target = event(248, 0, owned_payloads()[1].clone(), "1.0.0");
    let fact_key =
        LiquidationFillFactRecordV1::state_key(&liquidation(), target.event_id()).unwrap();
    let fact_bytes = fill_fact_bytes(&target);
    LiquidationFillFactRecordV1::decode_at(&fact_key, &fact_bytes).unwrap();
    let mut collision =
        injected_liquidation_ledger(246, vec![StateMutation::put(fact_key, fact_bytes)]);
    collision
        .apply_block(&block(
            246,
            vec![event(246, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    collision
        .apply_block(&block(
            247,
            vec![event(247, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap();
    let before_collision = collision.state_image().clone();
    let error = collision
        .apply_block(&block(248, vec![target]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.fill_fact_identity_collision")
    );
    assert_eq!(collision.state_image(), &before_collision);
}

#[test]
fn process_and_flow_provenance_require_distinct_coherent_event_identity() {
    let target = event(602, 0, owned_payloads()[1].clone(), "1.0.0");

    assert_fill_failure_with_injections(
        600,
        vec![process_mutation(target.event_id().as_str(), 600, 0, 0)],
        target.clone(),
        "liquidation_state.process_provenance_regression",
    );

    assert_fill_failure_with_injections(
        610,
        vec![
            process_mutation("process-last", 610, 0, 0),
            flow_mutation("flow-ahead", 611, 0, 0),
        ],
        event(612, 0, owned_payloads()[1].clone(), "1.0.0"),
        "liquidation_state.flow_prior_invalid",
    );

    assert_fill_failure_with_injections(
        620,
        vec![
            process_mutation("process-at-tuple", 621, 0, 0),
            flow_mutation("different-flow-at-tuple", 621, 0, 0),
        ],
        event(622, 0, owned_payloads()[1].clone(), "1.0.0"),
        "liquidation_state.flow_prior_invalid",
    );

    let repeated_flow_id = event(632, 0, owned_payloads()[1].clone(), "1.0.0");
    assert_fill_failure_with_injections(
        630,
        vec![
            process_mutation("process-after-flow", 631, 0, 0),
            flow_mutation(repeated_flow_id.event_id().as_str(), 630, 0, 0),
        ],
        repeated_flow_id,
        "liquidation_state.flow_prior_invalid",
    );
}

#[test]
fn secondary_prior_invalid_and_collision_precedence_follow_mutation_and_account_order() {
    let fill = event(
        702,
        0,
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: market(),
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("1").unwrap(),
        }),
        "1.0.0",
    );
    let effect_key =
        PositionEpisodeEffectFactRecordV1::state_key(fill.event_id(), &LIQUIDATED, &market(), 0)
            .unwrap();
    let mut invalid_effect = resolved_pair_mutations("3.00");
    invalid_effect.push(StateMutation::put(effect_key.clone(), b"{}".to_vec()));
    assert_secondary_failure(
        700,
        invalid_effect,
        fill.clone(),
        "liquidation_state.episode_effect_prior_invalid",
    );

    let mut colliding_effect = resolved_pair_mutations("3.00");
    let valid_effect = interrupted_effect_bytes(
        &fill,
        LIQUIDATED,
        &EventId::new("seed-open").unwrap(),
        "0.00",
        "liquidation_fill",
    );
    PositionEpisodeEffectFactRecordV1::decode_at(&effect_key, &valid_effect).unwrap();
    colliding_effect.push(StateMutation::put(effect_key, valid_effect));
    assert_secondary_failure(
        700,
        colliding_effect,
        fill.clone(),
        "liquidation_state.episode_effect_identity_collision",
    );

    let ordinal_one_key =
        PositionEpisodeEffectFactRecordV1::state_key(fill.event_id(), &LIQUIDATED, &market(), 1)
            .unwrap();
    let new_episode_id =
        derive_position_episode_id(&LIQUIDATED, &market(), fill.event_id(), 1).unwrap();
    let new_episode_key = PositionEpisodeRecordV1::state_key(&new_episode_id).unwrap();
    let valid_new_episode = partial_episode_bytes(&fill, "2.00");
    PositionEpisodeRecordV1::decode_at(&new_episode_key, &valid_new_episode).unwrap();
    let mut ordinal_one_invalid = resolved_pair_mutations("3.00");
    ordinal_one_invalid.push(StateMutation::put(ordinal_one_key.clone(), b"{}".to_vec()));
    ordinal_one_invalid.push(StateMutation::put(
        new_episode_key.clone(),
        valid_new_episode.clone(),
    ));
    assert_secondary_failure(
        700,
        ordinal_one_invalid,
        fill.clone(),
        "liquidation_state.episode_effect_prior_invalid",
    );

    let valid_ordinal_one = opened_effect_bytes(&fill, "0.00");
    PositionEpisodeEffectFactRecordV1::decode_at(&ordinal_one_key, &valid_ordinal_one).unwrap();
    let mut ordinal_one_collision = resolved_pair_mutations("3.00");
    ordinal_one_collision.push(StateMutation::put(
        ordinal_one_key.clone(),
        valid_ordinal_one,
    ));
    ordinal_one_collision.push(StateMutation::put(new_episode_key.clone(), b"{}".to_vec()));
    assert_secondary_failure(
        700,
        ordinal_one_collision,
        fill.clone(),
        "liquidation_state.episode_effect_identity_collision",
    );

    let mut new_episode_invalid = resolved_pair_mutations("3.00");
    new_episode_invalid.push(StateMutation::put(new_episode_key.clone(), b"{}".to_vec()));
    assert_secondary_failure(
        700,
        new_episode_invalid,
        fill.clone(),
        "liquidation_state.episode_prior_invalid",
    );

    let mut new_episode_collision = resolved_pair_mutations("3.00");
    new_episode_collision.push(StateMutation::put(new_episode_key, valid_new_episode));
    assert_secondary_failure(
        700,
        new_episode_collision,
        fill,
        "liquidation_state.episode_identity_collision",
    );

    let backstop = event(712, 0, owned_payloads()[2].clone(), "1.0.0");
    let first_cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
        &LIQUIDATED,
        &market(),
        backstop.event_id(),
        &liquidation(),
    )
    .unwrap();
    let second_cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
        &BACKSTOP,
        &market(),
        backstop.event_id(),
        &liquidation(),
    )
    .unwrap();
    let first_cause = unresolved_cause_bytes(&backstop, LIQUIDATED);
    PositionUnresolvedCauseFactRecordV1::decode_at(&first_cause_key, &first_cause).unwrap();
    let mut first_account_collision = resolved_pair_mutations("3.00");
    first_account_collision.push(StateMutation::put(first_cause_key, first_cause));
    first_account_collision.push(StateMutation::put(second_cause_key.clone(), b"{}".to_vec()));
    assert_secondary_failure(
        710,
        first_account_collision,
        backstop.clone(),
        "liquidation_state.unresolved_identity_collision",
    );

    let liquidated_effect_key = PositionEpisodeEffectFactRecordV1::state_key(
        backstop.event_id(),
        &LIQUIDATED,
        &market(),
        0,
    )
    .unwrap();
    let second_cause = unresolved_cause_bytes(&backstop, BACKSTOP);
    PositionUnresolvedCauseFactRecordV1::decode_at(&second_cause_key, &second_cause).unwrap();
    let mut first_account_invalid = resolved_pair_mutations("3.00");
    first_account_invalid.push(StateMutation::put(liquidated_effect_key, b"{}".to_vec()));
    first_account_invalid.push(StateMutation::put(second_cause_key, second_cause));
    assert_secondary_failure(
        710,
        first_account_invalid,
        backstop,
        "liquidation_state.episode_effect_prior_invalid",
    );
}

#[test]
fn settlement_partial_full_and_overrun_ambiguity_are_distinct() {
    let mut partial = seeded_pair_ledger(250, "5.00");
    let partial_event = event(252, 0, settlement_payload("2", "-3.25"), "1.0.0");
    let partial_id = partial_event.event_id().clone();
    partial
        .apply_block(&block(252, vec![partial_event]))
        .unwrap();
    assert_quantity(&partial, Some("3.00"));
    let partial_current = episode_current(&partial);
    assert_eq!(
        partial_current.attribution_resolution(),
        EpisodeAttributionResolutionV1::Resolved
    );
    assert_eq!(
        partial_current.episode_id(),
        Some(&derive_position_episode_id(&LIQUIDATED, &market(), &partial_id, 1).unwrap())
    );

    let mut full = seeded_pair_ledger(260, "-2.00");
    full.apply_block(&block(
        262,
        vec![event(262, 0, settlement_payload("2", "4"), "1.0.0")],
    ))
    .unwrap();
    assert_quantity(&full, Some("0.00"));
    assert_eq!(
        episode_current(&full).attribution_resolution(),
        EpisodeAttributionResolutionV1::NoOpenEpisode
    );

    let mut ambiguous = seeded_pair_ledger(270, "1.0");
    ambiguous
        .apply_block(&block(
            272,
            vec![event(272, 0, settlement_payload("2", "-9"), "1.0.0")],
        ))
        .unwrap();
    assert_quantity(&ambiguous, None);
    assert_eq!(
        episode_current(&ambiguous).attribution_resolution(),
        EpisodeAttributionResolutionV1::Interrupted
    );
}

#[test]
fn funding_after_partial_settlement_attaches_only_to_the_new_partial_episode() {
    let mut ledger = seeded_recovery_ledger(740, "3.00000000");
    let settlement = event(742, 0, settlement_payload("1.25", "-2"), "1.0.0");
    let settlement_event_id = settlement.event_id().clone();
    ledger.apply_block(&block(742, vec![settlement])).unwrap();

    let old_id = derive_position_episode_id(
        &LIQUIDATED,
        &market(),
        &EventId::new("seed-open").unwrap(),
        0,
    )
    .unwrap();
    let partial_id =
        derive_position_episode_id(&LIQUIDATED, &market(), &settlement_event_id, 1).unwrap();
    let old_key = PositionEpisodeRecordV1::state_key(&old_id).unwrap();
    let old_after_settlement = ledger.state_image().entries()[&old_key].clone();
    let interrupted = episode_by_id(&ledger, &old_id);
    assert_eq!(interrupted.status(), EpisodeStatusV1::Interrupted);
    assert_eq!(
        interrupted.close_cause(),
        Some(EpisodeCloseCauseV1::Settlement)
    );
    let interrupted_effect = episode_effect(&ledger, &settlement_event_id, LIQUIDATED, 0);
    assert_eq!(interrupted_effect.episode_id(), &old_id);
    assert_eq!(
        interrupted_effect.effect_kind(),
        EpisodeEffectKindV1::Interrupted
    );
    let opened_effect = episode_effect(&ledger, &settlement_event_id, LIQUIDATED, 1);
    assert_eq!(opened_effect.episode_id(), &partial_id);
    assert_eq!(opened_effect.effect_kind(), EpisodeEffectKindV1::Opened);

    let partial = episode_by_id(&ledger, &partial_id);
    assert_eq!(partial.status(), EpisodeStatusV1::Open);
    assert_eq!(
        partial.completeness(),
        EpisodeCompletenessV1::PartialFromFirstObservation
    );
    assert_eq!(partial.opening_position().to_string(), "1.75000000");
    assert_eq!(partial.funding_paid().to_string(), "0");

    let funding = funding_paid_event(743, "0.50");
    let funding_event_id = funding.event_id().clone();
    ledger.apply_block(&block(743, vec![funding])).unwrap();

    assert_eq!(
        ledger.state_image().entries()[&old_key],
        old_after_settlement
    );
    let funded = episode_by_id(&ledger, &partial_id);
    assert_eq!(funded.funding_paid().to_string(), "0.50");
    assert_eq!(funded.funding_received().to_string(), "0.00");
    assert_eq!(episode_current(&ledger).episode_id(), Some(&partial_id));
    let funding_effect = episode_effect(&ledger, &funding_event_id, LIQUIDATED, 0);
    assert_eq!(funding_effect.episode_id(), &partial_id);
    assert_eq!(funding_effect.effect_kind(), EpisodeEffectKindV1::Updated);
    assert_eq!(funding_effect.funding_paid_delta().to_string(), "0.50");
    assert_eq!(funding_effect.funding_received_delta().to_string(), "0.00");
}

#[test]
fn minimum_signed_position_reduces_without_absolute_value_overflow() {
    let mut ledger = seeded_pair_ledger(280, "-170141183460469231731687303715884105728");
    ledger
        .apply_block(&block(
            282,
            vec![event(
                282,
                0,
                EventPayload::LiquidationFill(LiquidationFill {
                    liquidation_id: liquidation(),
                    account_id: LIQUIDATED,
                    market_id: market(),
                    price: Price::from_str("100").unwrap(),
                    quantity: Quantity::from_str("1").unwrap(),
                }),
                "1.0.0",
            )],
        ))
        .unwrap();
    assert_quantity(&ledger, Some("-170141183460469231731687303715884105727"));
}

#[test]
fn backstop_prepares_known_and_unseen_accounts_before_committing_either() {
    let mut ledger = seeded_pair_ledger(290, "3.00");
    let backstop = event(292, 0, owned_payloads()[2].clone(), "1.0.0");
    ledger.apply_block(&block(292, vec![backstop])).unwrap();
    for account in [LIQUIDATED, BACKSTOP] {
        let key = PositionQuantityCurrentRecordV1::state_key(&account, &market()).unwrap();
        let quantity =
            PositionQuantityCurrentRecordV1::decode_at(&key, &ledger.state_image().entries()[&key])
                .unwrap();
        assert_eq!(quantity.known_quantity(), None);
    }

    let mut bad_injections = resolved_pair_mutations("3.00");
    bad_injections.push(StateMutation::put(
        PositionEpisodeCurrentRecordV1::state_key(&BACKSTOP, &market()).unwrap(),
        b"{}".to_vec(),
    ));
    let mut rollback = injected_liquidation_ledger(300, bad_injections);
    rollback
        .apply_block(&block(
            300,
            vec![event(300, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    rollback
        .apply_block(&block(
            301,
            vec![event(301, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap();
    let before = rollback.state_image().clone();
    let error = rollback
        .apply_block(&block(
            302,
            vec![event(302, 0, owned_payloads()[2].clone(), "1.0.0")],
        ))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.episode_current_invalid")
    );
    assert_eq!(rollback.state_image(), &before);
}

#[test]
fn repeated_backstop_fill_and_settlement_preserve_first_provenance_anchor_and_history() {
    let mut ledger = seeded_pair_ledger(310, "3.00");
    let first = event(312, 0, owned_payloads()[2].clone(), "1.0.0");
    let first_id = first.event_id().clone();
    ledger.apply_block(&block(312, vec![first])).unwrap();
    assert_anchor(&ledger, LIQUIDATED, Some("seed-open"));

    let repeated = event(313, 0, owned_payloads()[2].clone(), "1.0.0");
    let repeated_id = repeated.event_id().clone();
    ledger.apply_block(&block(313, vec![repeated])).unwrap();
    let process_key = LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap();
    let repeated_process = LiquidationCurrentRecordV1::decode_at(
        &process_key,
        &ledger.state_image().entries()[&process_key],
    )
    .unwrap();
    assert_eq!(
        repeated_process.observed_status(),
        LiquidationObservedStatusV1::BackstopObserved
    );
    assert_eq!(repeated_process.first_backstop_event_id(), Some(&first_id));
    assert_eq!(repeated_process.last_observation_event_id(), &repeated_id);
    assert_anchor(&ledger, LIQUIDATED, Some("seed-open"));

    for event_id in [&first_id, &repeated_id] {
        for account in [LIQUIDATED, BACKSTOP] {
            let cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
                &account,
                &market(),
                event_id,
                &liquidation(),
            )
            .unwrap();
            assert!(ledger.state_image().entries().contains_key(&cause_key));
        }
    }

    let fill = event(314, 0, owned_payloads()[1].clone(), "1.0.0");
    let fill_id = fill.event_id().clone();
    ledger.apply_block(&block(314, vec![fill])).unwrap();
    let fill_process = LiquidationCurrentRecordV1::decode_at(
        &process_key,
        &ledger.state_image().entries()[&process_key],
    )
    .unwrap();
    assert_eq!(
        fill_process.observed_status(),
        LiquidationObservedStatusV1::BackstopObserved
    );
    assert_eq!(fill_process.first_backstop_event_id(), Some(&first_id));
    assert_eq!(fill_process.last_observation_event_id(), &fill_id);
    assert_anchor(&ledger, LIQUIDATED, Some("seed-open"));

    let process_before_settlement = ledger.state_image().entries()[&process_key].clone();
    ledger
        .apply_block(&block(
            315,
            vec![event(315, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    assert_eq!(
        ledger.state_image().entries()[&process_key],
        process_before_settlement
    );
    assert_anchor(&ledger, LIQUIDATED, Some("seed-open"));
}

#[test]
fn fresh_repeated_and_checkpoint_resumed_replay_are_byte_and_hash_identical() {
    let settlement = event(402, 0, settlement_payload("1.25", "-2.75"), "1.0.0");
    let settlement_id = settlement.event_id().clone();
    let blocks = [
        block(
            400,
            vec![event(400, 0, owned_payloads()[3].clone(), "1.0.0")],
        ),
        block(
            401,
            vec![event(401, 0, owned_payloads()[0].clone(), "1.0.0")],
        ),
        block(402, vec![settlement]),
        block(
            403,
            vec![event(403, 0, owned_payloads()[2].clone(), "1.0.0")],
        ),
        block(
            404,
            vec![event(404, 0, owned_payloads()[2].clone(), "1.0.0")],
        ),
        block(
            405,
            vec![event(405, 0, owned_payloads()[1].clone(), "1.0.0")],
        ),
        block(
            406,
            vec![event(406, 0, owned_payloads()[3].clone(), "1.0.0")],
        ),
    ];
    let mut fresh = injected_liquidation_ledger(400, resolved_pair_mutations("5.000"));
    for replay_block in &blocks[..6] {
        fresh.apply_block(replay_block).unwrap();
    }
    let process_key = LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap();
    let process_before_settlement = fresh.state_image().entries()[&process_key].clone();
    fresh.apply_block(&blocks[6]).unwrap();
    assert_eq!(
        fresh.state_image().entries()[&process_key],
        process_before_settlement
    );
    for ordinal in [0, 1] {
        let key = PositionEpisodeEffectFactRecordV1::state_key(
            &settlement_id,
            &LIQUIDATED,
            &market(),
            ordinal,
        )
        .unwrap();
        assert!(fresh.state_image().entries().contains_key(&key));
    }
    let bytes = fresh.state_image().canonical_bytes();
    let hash = fresh.state_hash();

    let repeated = fresh.apply_block(&blocks[6]).unwrap();
    assert!(matches!(repeated, ApplyOutcome::AlreadyApplied(_)));
    assert_eq!(fresh.state_image().canonical_bytes(), bytes);
    assert_eq!(fresh.state_hash(), hash);

    let mut independent = injected_liquidation_ledger(400, resolved_pair_mutations("5.000"));
    for replay_block in &blocks {
        independent.apply_block(replay_block).unwrap();
    }
    assert_eq!(independent.state_image().canonical_bytes(), bytes);
    assert_eq!(independent.state_hash(), hash);

    let mut prefix = injected_liquidation_ledger(400, resolved_pair_mutations("5.000"));
    for replay_block in &blocks[..4] {
        prefix.apply_block(replay_block).unwrap();
    }
    let restored = StateImage::decode_canonical(
        &prefix.state_image().canonical_bytes(),
        StateImageLimits::production(),
    )
    .unwrap();
    let mut resumed = CanonicalLedger::try_from_state_image(
        restored,
        InjectionDispatcher {
            injection_height: BlockHeight::new(400),
            injections: resolved_pair_mutations("5.000"),
            liquidation: CanonicalLiquidationReducerV1,
        },
        LedgerLimits::production(),
    )
    .unwrap();
    for replay_block in &blocks[4..] {
        resumed.apply_block(replay_block).unwrap();
    }
    assert_eq!(resumed.state_image().canonical_bytes(), bytes);
    assert_eq!(resumed.state_hash(), hash);

    let before_late_failure = fresh.state_image().clone();
    let late_failure = block(
        407,
        vec![
            event(407, 0, owned_payloads()[3].clone(), "1.0.0"),
            event(407, 1, owned_payloads()[0].clone(), "1.0.0"),
        ],
    );
    let error = fresh.apply_block(&late_failure).unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("liquidation_state.process_identity_collision")
    );
    assert_eq!(fresh.state_image(), &before_late_failure);

    let wrong_version = CanonicalLedger::try_from_state_image(
        StateImage::decode_canonical(&bytes, StateImageLimits::production()).unwrap(),
        CanonicalLiquidationReducerV1,
        LedgerLimits::production(),
    )
    .expect_err("checkpoint reducer-set version substitution must be refused");
    assert_eq!(wrong_version.reason_code(), "ledger.reducer_version_drift");
}

#[test]
fn backstop_interrupts_funding_attribution_until_authoritative_trade_recovery() {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(500),
        RecoveryDispatcher {
            injection_height: BlockHeight::new(500),
            injections: Vec::new(),
            market: CanonicalMarketReducerV1,
            liquidation: CanonicalLiquidationReducerV1,
            quantity: CanonicalPositionReducerV1,
            episode: CanonicalPositionEpisodeReducerV1,
        },
        LedgerLimits::production(),
    )
    .unwrap();
    let mut first_events = market_prerequisites(500);
    first_events.push(event_with_identities(
        500,
        4,
        owned_payloads()[3].clone(),
        "1.0.0",
        vec![market()],
        vec![LIQUIDATED],
    ));
    ledger.apply_block(&block(500, first_events)).unwrap();
    ledger
        .apply_block(&block(
            501,
            vec![event(501, 0, owned_payloads()[0].clone(), "1.0.0")],
        ))
        .unwrap();
    ledger
        .apply_block(&block(
            502,
            vec![event(502, 0, owned_payloads()[2].clone(), "1.0.0")],
        ))
        .unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let interrupted_bytes = ledger.state_image().entries()[&current_key].clone();
    let ApplyOutcome::Applied(suppressed) = ledger
        .apply_block(&block(503, vec![funding_paid_event(503, "1.25")]))
        .unwrap()
    else {
        panic!("funding block must advance");
    };
    assert!(suppressed.mutations().is_empty());
    assert_eq!(
        ledger.state_image().entries()[&current_key],
        interrupted_bytes
    );

    ledger
        .apply_block(&block(504, vec![recovery_trade_event(504)]))
        .unwrap();
    let recovered = PositionEpisodeCurrentRecordV1::decode_at(
        &current_key,
        &ledger.state_image().entries()[&current_key],
    )
    .unwrap();
    assert_eq!(
        recovered.attribution_resolution(),
        EpisodeAttributionResolutionV1::Resolved
    );
    ledger
        .apply_block(&block(505, vec![funding_paid_event(505, "0.75")]))
        .unwrap();
    let recovered = PositionEpisodeCurrentRecordV1::decode_at(
        &current_key,
        &ledger.state_image().entries()[&current_key],
    )
    .unwrap();
    let episode_key = PositionEpisodeRecordV1::state_key(recovered.episode_id().unwrap()).unwrap();
    let episode = PositionEpisodeRecordV1::decode_at(
        &episode_key,
        &ledger.state_image().entries()[&episode_key],
    )
    .unwrap();
    assert_eq!(episode.funding_paid().to_string(), "0.75");
}

fn liquidation_ledger(first_height: u64) -> CanonicalLedger<CanonicalLiquidationReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalLiquidationReducerV1,
        LedgerLimits::production(),
    )
    .unwrap()
}

fn injected_liquidation_ledger(
    first_height: u64,
    injections: Vec<StateMutation>,
) -> CanonicalLedger<InjectionDispatcher> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        InjectionDispatcher {
            injection_height: BlockHeight::new(first_height),
            injections,
            liquidation: CanonicalLiquidationReducerV1,
        },
        LedgerLimits::production(),
    )
    .unwrap()
}

fn assert_fill_failure_with_injections(
    first_height: u64,
    injections: Vec<StateMutation>,
    target: CanonicalEventEnvelope,
    expected_reason: &str,
) {
    let mut ledger = injected_liquidation_ledger(first_height, injections);
    ledger
        .apply_block(&block(
            first_height,
            vec![event(first_height, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    for height in first_height + 1..target.block_height().get() {
        ledger.apply_block(&block(height, Vec::new())).unwrap();
    }
    let before = ledger.state_image().clone();
    let error = ledger
        .apply_block(&block(target.block_height().get(), vec![target]))
        .expect_err("invalid retained provenance must reject the fill");
    assert_eq!(error.reducer_reason_code(), Some(expected_reason));
    assert_eq!(ledger.state_image(), &before);
}

fn assert_secondary_failure(
    first_height: u64,
    injections: Vec<StateMutation>,
    target: CanonicalEventEnvelope,
    expected_reason: &str,
) {
    let mut ledger = injected_liquidation_ledger(first_height, injections);
    ledger
        .apply_block(&block(
            first_height,
            vec![event(first_height, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    ledger
        .apply_block(&block(
            first_height + 1,
            vec![event(
                first_height + 1,
                0,
                owned_payloads()[0].clone(),
                "1.0.0",
            )],
        ))
        .unwrap();
    let before = ledger.state_image().clone();
    let error = ledger
        .apply_block(&block(target.block_height().get(), vec![target]))
        .expect_err("secondary immutable state must reject the event");
    assert_eq!(error.reducer_reason_code(), Some(expected_reason));
    assert_eq!(ledger.state_image(), &before);
}

fn interrupted_effect_bytes(
    event: &CanonicalEventEnvelope,
    account: Address,
    opening_anchor: &EventId,
    zero_quantity: &str,
    close_cause: &str,
) -> Vec<u8> {
    let episode_id = derive_position_episode_id(&account, &market(), opening_anchor, 0).unwrap();
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode-effect-fact/v1\",\"event_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"leg_ordinal\":0,\"episode_id\":\"{}\",\"effect_kind\":\"interrupted\",\"buy_quantity_delta\":\"{zero_quantity}\",\"buy_notional_delta\":\"0\",\"sell_quantity_delta\":\"{zero_quantity}\",\"sell_notional_delta\":\"0\",\"funding_paid_delta\":\"0\",\"funding_received_delta\":\"0\",\"close_cause\":\"{close_cause}\",\"rule_version\":\"hyperliquid-alpha-desk-canonical-position-episode@1.0.0\"}}",
        event.event_id().as_str(),
        account.to_api_string(),
        market().as_str(),
        episode_id.as_str(),
    )
    .into_bytes()
}

fn opened_effect_bytes(event: &CanonicalEventEnvelope, zero_quantity: &str) -> Vec<u8> {
    let episode_id =
        derive_position_episode_id(&LIQUIDATED, &market(), event.event_id(), 1).unwrap();
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode-effect-fact/v1\",\"event_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"leg_ordinal\":1,\"episode_id\":\"{}\",\"effect_kind\":\"opened\",\"buy_quantity_delta\":\"{zero_quantity}\",\"buy_notional_delta\":\"0\",\"sell_quantity_delta\":\"{zero_quantity}\",\"sell_notional_delta\":\"0\",\"funding_paid_delta\":\"0\",\"funding_received_delta\":\"0\",\"close_cause\":null,\"rule_version\":\"hyperliquid-alpha-desk-canonical-position-episode@1.0.0\"}}",
        event.event_id().as_str(),
        LIQUIDATED.to_api_string(),
        market().as_str(),
        episode_id.as_str(),
    )
    .into_bytes()
}

fn partial_episode_bytes(event: &CanonicalEventEnvelope, opening_position: &str) -> Vec<u8> {
    let episode_id =
        derive_position_episode_id(&LIQUIDATED, &market(), event.event_id(), 1).unwrap();
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"{}\",\"opening_leg_ordinal\":1,\"opening_position\":\"{opening_position}\",\"close_event_id\":null,\"close_cause\":null,\"completeness\":\"partial_from_first_observation\",\"buy_quantity\":\"0.00\",\"buy_notional\":\"0\",\"sell_quantity\":\"0.00\",\"sell_notional\":\"0\",\"funding_paid\":\"0\",\"funding_received\":\"0\",\"status\":\"open\",\"last_event_id\":\"{}\",\"last_block_height\":{}}}",
        episode_id.as_str(),
        LIQUIDATED.to_api_string(),
        market().as_str(),
        event.event_id().as_str(),
        event.event_id().as_str(),
        event.block_height().get(),
    )
    .into_bytes()
}

fn unresolved_cause_bytes(event: &CanonicalEventEnvelope, account: Address) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-unresolved-cause-fact/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"event_id\":\"{}\",\"liquidation_id\":\"{}\",\"cause\":\"backstop_liquidation\"}}",
        account.to_api_string(),
        market().as_str(),
        event.event_id().as_str(),
        liquidation().as_str(),
    )
    .into_bytes()
}

fn process_mutation(
    last_event_id: &str,
    last_height: u64,
    last_transaction_index: u32,
    last_event_index: u32,
) -> StateMutation {
    let key = LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap();
    let bytes = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/liquidation-current/v1\",\"liquidation_id\":\"{}\",\"account_id\":\"{}\",\"start_margin_value\":\"9\",\"start_maintenance_requirement\":\"10\",\"observed_status\":\"started\",\"start_event_id\":\"seed-start\",\"start_block_height\":1,\"start_transaction_index\":0,\"start_canonical_event_index\":0,\"first_backstop_event_id\":null,\"first_backstop_block_height\":null,\"first_backstop_transaction_index\":null,\"first_backstop_canonical_event_index\":null,\"last_observation_event_id\":\"{last_event_id}\",\"last_observation_block_height\":{last_height},\"last_observation_transaction_index\":{last_transaction_index},\"last_observation_canonical_event_index\":{last_event_index},\"rule_version\":\"hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0\"}}",
        liquidation().as_str(),
        LIQUIDATED.to_api_string(),
    )
    .into_bytes();
    LiquidationCurrentRecordV1::decode_at(&key, &bytes).unwrap();
    StateMutation::put(key, bytes)
}

fn flow_mutation(
    last_event_id: &str,
    last_height: u64,
    last_transaction_index: u32,
    last_event_index: u32,
) -> StateMutation {
    flow_mutation_with_quantity(
        last_event_id,
        last_height,
        last_transaction_index,
        last_event_index,
        "1",
    )
}

fn flow_mutation_with_quantity(
    last_event_id: &str,
    last_height: u64,
    last_transaction_index: u32,
    last_event_index: u32,
    quantity: &str,
) -> StateMutation {
    let key =
        LiquidationMarketFlowCurrentRecordV1::state_key(&liquidation(), &LIQUIDATED, &market())
            .unwrap();
    let bytes = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/liquidation-market-flow-current/v1\",\"liquidation_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"observed_filled_quantity\":\"{quantity}\",\"first_fill_event_id\":\"{last_event_id}\",\"first_fill_block_height\":{last_height},\"first_fill_transaction_index\":{last_transaction_index},\"first_fill_canonical_event_index\":{last_event_index},\"last_fill_event_id\":\"{last_event_id}\",\"last_fill_block_height\":{last_height},\"last_fill_transaction_index\":{last_transaction_index},\"last_fill_canonical_event_index\":{last_event_index},\"rule_version\":\"hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0\"}}",
        liquidation().as_str(),
        LIQUIDATED.to_api_string(),
        market().as_str(),
    )
    .into_bytes();
    LiquidationMarketFlowCurrentRecordV1::decode_at(&key, &bytes).unwrap();
    StateMutation::put(key, bytes)
}

fn seeded_pair_ledger(first_height: u64, quantity: &str) -> CanonicalLedger<InjectionDispatcher> {
    let mut ledger = injected_liquidation_ledger(first_height, resolved_pair_mutations(quantity));
    ledger
        .apply_block(&block(
            first_height,
            vec![event(first_height, 0, owned_payloads()[3].clone(), "1.0.0")],
        ))
        .unwrap();
    ledger
        .apply_block(&block(
            first_height + 1,
            vec![event(
                first_height + 1,
                0,
                owned_payloads()[0].clone(),
                "1.0.0",
            )],
        ))
        .unwrap();
    ledger
}

fn seeded_recovery_ledger(
    first_height: u64,
    quantity: &str,
) -> CanonicalLedger<RecoveryDispatcher> {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        RecoveryDispatcher {
            injection_height: BlockHeight::new(first_height),
            injections: resolved_pair_mutations(quantity),
            market: CanonicalMarketReducerV1,
            liquidation: CanonicalLiquidationReducerV1,
            quantity: CanonicalPositionReducerV1,
            episode: CanonicalPositionEpisodeReducerV1,
        },
        LedgerLimits::production(),
    )
    .unwrap();
    let mut initial = market_prerequisites(first_height);
    initial.push(event_with_identities(
        first_height,
        4,
        owned_payloads()[3].clone(),
        "1.0.0",
        vec![market()],
        vec![LIQUIDATED],
    ));
    ledger.apply_block(&block(first_height, initial)).unwrap();
    ledger
        .apply_block(&block(
            first_height + 1,
            vec![event(
                first_height + 1,
                0,
                owned_payloads()[0].clone(),
                "1.0.0",
            )],
        ))
        .unwrap();
    ledger
}

fn settlement_payload(quantity: &str, realized_pnl: &str) -> EventPayload {
    EventPayload::PositionSettled(PositionSettled {
        account_id: LIQUIDATED,
        market_id: market(),
        settlement_price: Price::from_str("0").unwrap(),
        settled_quantity: Quantity::from_str(quantity).unwrap(),
        realized_pnl: QuoteAmount::from_str(realized_pnl).unwrap(),
    })
}

fn assert_quantity<R: EventReducer>(ledger: &CanonicalLedger<R>, expected: Option<&str>) {
    let key = PositionQuantityCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let record =
        PositionQuantityCurrentRecordV1::decode_at(&key, &ledger.state_image().entries()[&key])
            .unwrap();
    assert_eq!(
        record.known_quantity().map(|value| value.to_string()),
        expected.map(str::to_owned)
    );
}

fn assert_anchor(
    ledger: &CanonicalLedger<InjectionDispatcher>,
    account: Address,
    expected: Option<&str>,
) {
    let key = PositionQuantityCurrentRecordV1::state_key(&account, &market()).unwrap();
    let record =
        PositionQuantityCurrentRecordV1::decode_at(&key, &ledger.state_image().entries()[&key])
            .unwrap();
    assert_eq!(
        record.first_anchor_event_id().map(EventId::as_str),
        expected
    );
}

fn episode_current<R: EventReducer>(ledger: &CanonicalLedger<R>) -> PositionEpisodeCurrentRecordV1 {
    let key = PositionEpisodeCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    PositionEpisodeCurrentRecordV1::decode_at(&key, &ledger.state_image().entries()[&key]).unwrap()
}

fn episode_by_id<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    episode_id: &domain_types::PositionEpisodeId,
) -> PositionEpisodeRecordV1 {
    let key = PositionEpisodeRecordV1::state_key(episode_id).unwrap();
    PositionEpisodeRecordV1::decode_at(&key, &ledger.state_image().entries()[&key]).unwrap()
}

fn episode_effect<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    event_id: &EventId,
    account: Address,
    ordinal: u8,
) -> PositionEpisodeEffectFactRecordV1 {
    let key = PositionEpisodeEffectFactRecordV1::state_key(event_id, &account, &market(), ordinal)
        .unwrap();
    PositionEpisodeEffectFactRecordV1::decode_at(&key, &ledger.state_image().entries()[&key])
        .unwrap()
}

fn known_quantity<R: EventReducer>(ledger: &CanonicalLedger<R>, account: Address) -> String {
    let key = PositionQuantityCurrentRecordV1::state_key(&account, &market()).unwrap();
    PositionQuantityCurrentRecordV1::decode_at(&key, &ledger.state_image().entries()[&key])
        .unwrap()
        .known_quantity()
        .unwrap()
        .to_string()
}

fn resolved_pair_mutations(known_quantity: &str) -> Vec<StateMutation> {
    let anchor = EventId::new("seed-open").unwrap();
    let episode_id = derive_position_episode_id(&LIQUIDATED, &market(), &anchor, 0).unwrap();
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&LIQUIDATED, &market()).unwrap();
    let episode_key = PositionEpisodeRecordV1::state_key(&episode_id).unwrap();
    let address = LIQUIDATED.to_api_string();
    let market_id = market();
    let quantity = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-quantity-current/v1\",\"account_id\":\"{address}\",\"market_id\":\"{}\",\"known_quantity\":\"{known_quantity}\",\"first_anchor_event_id\":\"seed-open\",\"last_event_id\":\"seed-open\",\"last_block_height\":199}}",
        market_id.as_str()
    )
    .into_bytes();
    let current = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode-current/v1\",\"account_id\":\"{address}\",\"market_id\":\"{}\",\"episode_id\":\"{}\",\"attribution_resolution\":\"resolved\",\"last_event_id\":\"seed-open\",\"last_block_height\":199}}",
        market_id.as_str(),
        episode_id.as_str()
    )
    .into_bytes();
    let episode = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{address}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"seed-open\",\"opening_leg_ordinal\":0,\"opening_position\":\"0\",\"close_event_id\":null,\"close_cause\":null,\"completeness\":\"complete_from_flat\",\"buy_quantity\":\"5\",\"buy_notional\":\"500\",\"sell_quantity\":\"0\",\"sell_notional\":\"0\",\"funding_paid\":\"0\",\"funding_received\":\"0\",\"status\":\"open\",\"last_event_id\":\"seed-open\",\"last_block_height\":199}}",
        episode_id.as_str(),
        market_id.as_str()
    )
    .into_bytes();
    vec![
        StateMutation::put(quantity_key, quantity),
        StateMutation::put(current_key, current),
        StateMutation::put(episode_key, episode),
    ]
}

fn fill_fact_bytes(event: &CanonicalEventEnvelope) -> Vec<u8> {
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/liquidation-fill-fact/v1\",\"liquidation_id\":\"{}\",\"event_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"price\":\"100\",\"quantity\":\"1\",\"block_height\":{},\"transaction_index\":{},\"canonical_event_index\":{},\"payload_blake3\":\"{}\",\"rule_version\":\"hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0\"}}",
        liquidation().as_str(),
        event.event_id().as_str(),
        LIQUIDATED.to_api_string(),
        market().as_str(),
        event.block_height().get(),
        event.transaction_index(),
        event.canonical_event_index(),
        hex::encode(event.payload_hash()),
    )
    .into_bytes()
}

fn market_prerequisites(height: u64) -> Vec<CanonicalEventEnvelope> {
    let operator = Address::from_bytes([0x33; 20]);
    let base = AssetId::new("BTC").unwrap();
    let quote = AssetId::new("USDC").unwrap();
    vec![
        event_with_identities(
            height,
            0,
            EventPayload::DexCreated(DexCreated {
                dex_id: DexId::new("validator").unwrap(),
                name: "Validator".to_owned(),
                operator_account_id: operator,
            }),
            "1.0.0",
            Vec::new(),
            vec![operator],
        ),
        event_with_identities(
            height,
            1,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: base.clone(),
                context_version: "btc-v1".to_owned(),
                context_hash: [1; 32],
            }),
            "1.0.0",
            Vec::new(),
            Vec::new(),
        ),
        event_with_identities(
            height,
            2,
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: quote.clone(),
                context_version: "usdc-v1".to_owned(),
                context_hash: [2; 32],
            }),
            "1.0.0",
            Vec::new(),
            Vec::new(),
        ),
        event_with_identities(
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
            "1.0.0",
            vec![market()],
            Vec::new(),
        ),
    ]
}

fn funding_paid_event(height: u64, amount: &str) -> CanonicalEventEnvelope {
    event_with_identities(
        height,
        0,
        EventPayload::FundingPaid(FundingPaid {
            account_id: LIQUIDATED,
            market_id: market(),
            amount: QuoteAmount::from_str(amount).unwrap(),
            funding_rate: FundingRate::from_str("0.0001").unwrap(),
        }),
        "1.0.0",
        vec![market()],
        vec![LIQUIDATED],
    )
}

fn recovery_trade_event(height: u64) -> CanonicalEventEnvelope {
    event_with_identities(
        height,
        0,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new("recovery-trade").unwrap()),
            market_id: Some(market()),
            maker_order_id: None,
            taker_order_id: None,
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("1").unwrap(),
            deterministic_seed: height,
            participants: Some(Box::new([
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Buyer,
                    account_id: LIQUIDATED,
                    start_position: PositionQuantity::from_str("0").unwrap(),
                    order_id: OrderId::new("recovery-buyer").unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Seller,
                    account_id: BACKSTOP,
                    start_position: PositionQuantity::from_str("0").unwrap(),
                    order_id: OrderId::new("recovery-seller").unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
            ])),
        }),
        "1.0.0",
        vec![market()],
        vec![LIQUIDATED, BACKSTOP],
    )
}

#[allow(clippy::too_many_arguments)]
fn enriched_trade_event(
    height: u64,
    trade_id: &str,
    buyer_account: Address,
    buyer_start: &str,
    seller_account: Address,
    seller_start: &str,
    quantity: &str,
) -> CanonicalEventEnvelope {
    event_with_identities(
        height,
        0,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new(trade_id).unwrap()),
            market_id: Some(market()),
            maker_order_id: None,
            taker_order_id: None,
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str(quantity).unwrap(),
            deterministic_seed: height,
            participants: Some(Box::new([
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Buyer,
                    account_id: buyer_account,
                    start_position: PositionQuantity::from_str(buyer_start).unwrap(),
                    order_id: OrderId::new(format!("{trade_id}-buyer")).unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Seller,
                    account_id: seller_account,
                    start_position: PositionQuantity::from_str(seller_start).unwrap(),
                    order_id: OrderId::new(format!("{trade_id}-seller")).unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
            ])),
        }),
        "1.0.0",
        vec![market()],
        vec![buyer_account, seller_account],
    )
}

fn owned_payloads() -> Vec<EventPayload> {
    vec![
        EventPayload::LiquidationStarted(LiquidationStarted {
            account_id: LIQUIDATED,
            liquidation_id: liquidation(),
            margin_value: UsdAmount::from_str("9").unwrap(),
            maintenance_requirement: UsdAmount::from_str("10").unwrap(),
        }),
        EventPayload::LiquidationFill(LiquidationFill {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            market_id: market(),
            price: Price::from_str("100").unwrap(),
            quantity: Quantity::from_str("1").unwrap(),
        }),
        EventPayload::BackstopLiquidation(BackstopLiquidation {
            liquidation_id: liquidation(),
            account_id: LIQUIDATED,
            backstop_account_id: BACKSTOP,
            market_id: market(),
            quantity: Quantity::from_str("1").unwrap(),
        }),
        EventPayload::PositionSettled(PositionSettled {
            account_id: LIQUIDATED,
            market_id: market(),
            settlement_price: Price::from_str("0").unwrap(),
            settled_quantity: Quantity::from_str("1").unwrap(),
            realized_pnl: QuoteAmount::from_str("-2.5").unwrap(),
        }),
    ]
}

fn event(
    height: u64,
    transaction_index: u32,
    payload: EventPayload,
    schema: &str,
) -> CanonicalEventEnvelope {
    let (markets, accounts) = match &payload {
        EventPayload::LiquidationStarted(value) => (Vec::new(), vec![value.account_id]),
        EventPayload::LiquidationFill(value) => {
            (vec![value.market_id.clone()], vec![value.account_id])
        }
        EventPayload::BackstopLiquidation(value) => (
            vec![value.market_id.clone()],
            vec![value.account_id, value.backstop_account_id],
        ),
        EventPayload::PositionSettled(value) => {
            (vec![value.market_id.clone()], vec![value.account_id])
        }
        _ => unreachable!("owned payload helper"),
    };
    event_with_identities(
        height,
        transaction_index,
        payload,
        schema,
        markets,
        accounts,
    )
}

fn event_with_identities(
    height: u64,
    transaction_index: u32,
    payload: EventPayload,
    schema: &str,
    markets: Vec<MarketId>,
    accounts: Vec<Address>,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}-{transaction_index}")).unwrap(),
        transaction_index,
        canonical_event_index: 0,
        market_ids: markets,
        account_ids: accounts,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("liquidation-state-test").unwrap(),
                "task6b-fixture",
                height.to_string(),
                payload_hash,
                transaction_index,
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        ingested_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).unwrap(),
        parser_version: "liquidation-state-test@1.0.0".to_owned(),
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
        BTreeMap::from([(
            SourceId::new("liquidation-state-test").unwrap(),
            [height as u8; 32],
        )]),
    )
    .unwrap()
}

fn liquidation() -> LiquidationId {
    LiquidationId::new("liq-task6b").unwrap()
}

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}
