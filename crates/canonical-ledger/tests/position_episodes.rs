use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use canonical_events::{
    AssetContextUpdated, BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput,
    ConfirmationClass, DexCreated, EventKind, EventPayload, FeeCharged, FundingPaid,
    FundingReceived, MarketCreated, MarketMetadataChanged, OrderFilled, SourceEvidence,
    TradeMatched, TradeParticipantRoleV1, TradeParticipantV1,
};
use canonical_ledger::{
    AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1, ApplyContext, ApplyOutcome,
    CanonicalAccountReducerV1, CanonicalLedger, CanonicalMarketReducerV1,
    CanonicalPositionEpisodeReducerV1, CanonicalPositionReducerV1, CanonicalTradeReducerSetV2,
    EventReducer, LedgerLimits, PositionEpisodeCurrentRecordV1, PositionEpisodeEffectFactRecordV1,
    PositionEpisodeRecordV1, PositionQuantityCurrentRecordV1, ReducerError, StateImage,
    StateImageLimits, StateKey, StateMutation, StateView,
};
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, DexId, EventId, FeeRate, FeeTypeV1, FundingRate,
    KnownTime, MarketId, OrderId, PositionQuantity, Price, ProtocolTime, Quantity, QuoteAmount,
    SourceId, TradeId, TransactionId,
};

const BUYER: Address = Address::from_bytes([0x11; 20]);
const SELLER: Address = Address::from_bytes([0x22; 20]);
const OPERATOR: Address = Address::from_bytes([0x33; 20]);

#[derive(Debug, Clone, Copy, Default)]
struct EpisodeDispatcher {
    market: CanonicalMarketReducerV1,
    trade: CanonicalTradeReducerSetV2,
    account: CanonicalAccountReducerV1,
    quantity: CanonicalPositionReducerV1,
    episode: CanonicalPositionEpisodeReducerV1,
}

impl EventReducer for EpisodeDispatcher {
    fn reducer_set_version(&self) -> &str {
        "position-episode-test-dispatcher@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        EventReducer::supports(&self.market, event)
            || EventReducer::supports(&self.trade, event)
            || EventReducer::supports(&self.account, event)
            || EventReducer::supports(&self.quantity, event)
            || EventReducer::supports(&self.episode, event)
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
        if EventReducer::supports(&self.account, event) {
            mutations.extend(EventReducer::reduce(&self.account, state, event, context)?);
        }
        if EventReducer::supports(&self.quantity, event) {
            mutations.extend(EventReducer::reduce(&self.quantity, state, event, context)?);
        }
        if EventReducer::supports(&self.episode, event) {
            mutations.extend(EventReducer::reduce(&self.episode, state, event, context)?);
        }
        let mut keys = BTreeSet::new();
        if !mutations.iter().all(|mutation| keys.insert(mutation.key())) {
            return Err(ReducerError::try_new(
                "position_episode.duplicate_mutation_key",
                "test dispatcher children emitted duplicate keys",
            )
            .unwrap());
        }
        Ok(mutations)
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct CrossChildDuplicateDispatcher {
    market: CanonicalMarketReducerV1,
    episode: CanonicalPositionEpisodeReducerV1,
}

impl EventReducer for CrossChildDuplicateDispatcher {
    fn reducer_set_version(&self) -> &str {
        "position-episode-cross-child-duplicate-test@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        EventReducer::supports(&self.market, event) || EventReducer::supports(&self.episode, event)
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
        let mut mutations = EventReducer::reduce(&self.episode, state, event, context)?;
        let duplicate = mutations
            .first()
            .expect("supported episode trade emits mutations")
            .clone();
        mutations.push(duplicate);
        let mut keys = BTreeSet::new();
        if !mutations.iter().all(|mutation| keys.insert(mutation.key())) {
            return Err(ReducerError::try_new(
                "position_episode.duplicate_mutation_key",
                "test dispatcher children emitted duplicate keys",
            )
            .unwrap());
        }
        Ok(mutations)
    }
}

#[derive(Debug, Clone)]
struct InjectionDispatcher {
    injection_height: BlockHeight,
    injections: Vec<StateMutation>,
    market: CanonicalMarketReducerV1,
    trade: Option<CanonicalTradeReducerSetV2>,
    account: Option<CanonicalAccountReducerV1>,
    quantity: Option<CanonicalPositionReducerV1>,
    episode: CanonicalPositionEpisodeReducerV1,
}

impl EventReducer for InjectionDispatcher {
    fn reducer_set_version(&self) -> &str {
        "position-episode-injection-dispatcher@1.0.0"
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::OrderFilled
            || EventReducer::supports(&self.market, event)
            || self
                .trade
                .is_some_and(|trade| EventReducer::supports(&trade, event))
            || self
                .account
                .is_some_and(|account| EventReducer::supports(&account, event))
            || self
                .quantity
                .is_some_and(|quantity| EventReducer::supports(&quantity, event))
            || EventReducer::supports(&self.episode, event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if context.block_height() == self.injection_height
            && event.event_kind() == EventKind::OrderFilled
        {
            Ok(self.injections.clone())
        } else if EventReducer::supports(&self.market, event) {
            EventReducer::reduce(&self.market, state, event, context)
        } else {
            let mut mutations = Vec::new();
            let mut handled = false;
            if let Some(trade) = self.trade
                && EventReducer::supports(&trade, event)
            {
                handled = true;
                mutations.extend(EventReducer::reduce(&trade, state, event, context)?);
            }
            if let Some(account) = self.account
                && EventReducer::supports(&account, event)
            {
                handled = true;
                mutations.extend(EventReducer::reduce(&account, state, event, context)?);
            }
            if let Some(quantity) = self.quantity
                && EventReducer::supports(&quantity, event)
            {
                handled = true;
                mutations.extend(EventReducer::reduce(&quantity, state, event, context)?);
            }
            if EventReducer::supports(&self.episode, event) {
                handled = true;
                mutations.extend(EventReducer::reduce(&self.episode, state, event, context)?);
            }
            if !handled {
                return Err(ReducerError::try_new(
                    "position_episode.unsupported_event",
                    "injection dispatcher received an unsupported event",
                )
                .unwrap());
            }
            let mut keys = BTreeSet::new();
            if !mutations.iter().all(|mutation| keys.insert(mutation.key())) {
                return Err(ReducerError::try_new(
                    "position_episode.duplicate_mutation_key",
                    "injection dispatcher children emitted duplicate keys",
                )
                .unwrap());
            }
            Ok(mutations)
        }
    }
}

#[test]
fn reducer_version_and_exact_support_boundary_are_frozen() {
    let reducer = CanonicalPositionEpisodeReducerV1;
    assert_eq!(
        CanonicalPositionEpisodeReducerV1::VERSION,
        "hyperliquid-alpha-desk-canonical-position-episode@1.0.0"
    );
    assert_eq!(
        EventReducer::reducer_set_version(&reducer),
        CanonicalPositionEpisodeReducerV1::VERSION
    );
    assert!(EventReducer::supports(
        &reducer,
        &trade_event(true, "1.0.0")
    ));
    assert!(!EventReducer::supports(
        &reducer,
        &trade_event(false, "1.0.0")
    ));
    assert!(!EventReducer::supports(
        &reducer,
        &trade_event(true, "1.1.0")
    ));
    assert!(EventReducer::supports(&reducer, &funding_paid_event()));
    assert!(EventReducer::supports(&reducer, &funding_received_event()));
    assert!(!EventReducer::supports(&reducer, &order_fill_event()));
    assert!(!EventReducer::supports(&reducer, &fee_event()));
}

#[test]
fn component_and_test_dispatcher_checkpoints_refuse_cross_version_restore() {
    let component = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(1),
        CanonicalPositionEpisodeReducerV1,
        LedgerLimits::production(),
    )
    .unwrap();
    let component_state = StateImage::decode_canonical(
        &component.state_image().canonical_bytes(),
        StateImageLimits::production(),
    )
    .unwrap();
    let dispatcher_error = CanonicalLedger::try_from_state_image(
        component_state,
        EpisodeDispatcher::default(),
        LedgerLimits::production(),
    )
    .expect_err("component checkpoint must not restore under the test reducer set");
    assert_eq!(
        dispatcher_error.reason_code(),
        "ledger.reducer_version_drift"
    );

    let dispatcher = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(1),
        EpisodeDispatcher::default(),
        LedgerLimits::production(),
    )
    .unwrap();
    let dispatcher_state = StateImage::decode_canonical(
        &dispatcher.state_image().canonical_bytes(),
        StateImageLimits::production(),
    )
    .unwrap();
    let component_error = CanonicalLedger::try_from_state_image(
        dispatcher_state,
        CanonicalPositionEpisodeReducerV1,
        LedgerLimits::production(),
    )
    .expect_err("test reducer-set checkpoint must not restore under the component");
    assert_eq!(
        component_error.reason_code(),
        "ledger.reducer_version_drift"
    );
}

#[test]
fn observed_signed_trade_notional_delta_is_typed_and_never_divides() {
    let open = canonical_ledger::PositionEpisodeRecordV1::decode(&episode_bytes(
        "complete_from_flat",
        "open",
        None,
        "100",
        "0",
    ))
    .unwrap();
    assert_eq!(open.observed_signed_trade_notional_delta().unwrap(), None);

    let closed = canonical_ledger::PositionEpisodeRecordV1::decode(&episode_bytes(
        "complete_from_flat",
        "closed",
        Some("trade_flat"),
        "100",
        "125",
    ))
    .unwrap();
    assert_eq!(
        closed
            .observed_signed_trade_notional_delta()
            .unwrap()
            .unwrap()
            .to_string(),
        "25"
    );

    let partial = canonical_ledger::PositionEpisodeRecordV1::decode(&episode_bytes(
        "partial_from_first_observation",
        "closed",
        Some("trade_flat"),
        "100",
        "125",
    ))
    .unwrap();
    assert_eq!(
        partial.observed_signed_trade_notional_delta().unwrap(),
        None
    );
}

#[test]
fn flat_open_add_reduce_close_and_reversal_are_exact_for_both_roles() {
    let mut ledger = seeded_ledger(10);

    let open = dynamic_trade_event(11, 0, "trd-open", "100", "1", BUYER, "0", SELLER, "0");
    ledger.apply_block(&block(11, vec![open])).unwrap();
    let buyer_open = resolved_episode(&ledger, BUYER);
    assert_eq!(
        buyer_open.completeness(),
        canonical_ledger::EpisodeCompletenessV1::CompleteFromFlat
    );
    assert_eq!(buyer_open.buy_quantity().to_string(), "1.00000000");
    assert_eq!(buyer_open.buy_notional().to_string(), "100");
    assert_eq!(buyer_open.status(), canonical_ledger::EpisodeStatusV1::Open);

    let add = dynamic_trade_event(12, 0, "trd-add", "100", "0.5", BUYER, "1", SELLER, "-1");
    ledger.apply_block(&block(12, vec![add])).unwrap();
    let buyer_added = resolved_episode(&ledger, BUYER);
    assert_eq!(buyer_added.episode_id(), buyer_open.episode_id());
    assert_eq!(buyer_added.buy_quantity().to_string(), "1.50000000");
    assert_eq!(buyer_added.buy_notional().to_string(), "150");

    let reduce = dynamic_trade_event(
        13,
        0,
        "trd-reduce",
        "100",
        "0.5",
        SELLER,
        "-1.5",
        BUYER,
        "1.5",
    );
    ledger.apply_block(&block(13, vec![reduce])).unwrap();
    let buyer_reduced = resolved_episode(&ledger, BUYER);
    assert_eq!(buyer_reduced.sell_quantity().to_string(), "0.50000000");
    assert_eq!(buyer_reduced.sell_notional().to_string(), "50");

    let close = dynamic_trade_event(14, 0, "trd-close", "100", "1", SELLER, "-1", BUYER, "1");
    ledger.apply_block(&block(14, vec![close])).unwrap();
    let buyer_closed = episode_by_id(&ledger, buyer_open.episode_id());
    assert_eq!(
        buyer_closed.status(),
        canonical_ledger::EpisodeStatusV1::Closed
    );
    assert_eq!(
        buyer_closed.close_cause(),
        Some(canonical_ledger::EpisodeCloseCauseV1::TradeFlat)
    );
    assert_eq!(
        buyer_closed
            .observed_signed_trade_notional_delta()
            .unwrap()
            .unwrap()
            .to_string(),
        "0"
    );
    assert_no_open_episode(&ledger, BUYER);
    assert_no_open_episode(&ledger, SELLER);

    let reopen = dynamic_trade_event(15, 0, "trd-reopen", "100", "1", BUYER, "0", SELLER, "0");
    ledger.apply_block(&block(15, vec![reopen])).unwrap();
    let pre_reversal = resolved_episode(&ledger, BUYER);

    let reversal = dynamic_trade_event(
        16,
        0,
        "trd-reversal",
        "100",
        "1.5",
        SELLER,
        "-1",
        BUYER,
        "1",
    );
    let reversal_event_id = reversal.event_id().clone();
    let outcome = ledger.apply_block(&block(16, vec![reversal])).unwrap();
    let ApplyOutcome::Applied(delta) = outcome else {
        panic!("new reversal block must apply");
    };
    let episode_mutation_order: Vec<_> = delta
        .mutations()
        .iter()
        .map(|mutation| mutation.key().namespace())
        .filter(|namespace| namespace.starts_with("position-episode"))
        .collect();
    assert_eq!(
        episode_mutation_order,
        [
            "position-episode-effect-fact.v1",
            "position-episode-effect-fact.v1",
            "position-episode.v1",
            "position-episode.v1",
            "position-episode-current.v1",
            "position-episode-effect-fact.v1",
            "position-episode-effect-fact.v1",
            "position-episode.v1",
            "position-episode.v1",
            "position-episode-current.v1",
        ]
    );

    let closed_old = episode_by_id(&ledger, pre_reversal.episode_id());
    assert_eq!(
        closed_old.close_cause(),
        Some(canonical_ledger::EpisodeCloseCauseV1::TradeReversal)
    );
    assert_eq!(closed_old.sell_quantity().to_string(), "1.00000000");
    assert_eq!(closed_old.sell_notional().to_string(), "100");

    let residual = resolved_episode(&ledger, BUYER);
    assert_ne!(residual.episode_id(), pre_reversal.episode_id());
    assert_eq!(residual.opening_leg_ordinal(), 1);
    assert_eq!(residual.opening_position().to_string(), "0.00000000");
    assert_eq!(residual.sell_quantity().to_string(), "0.50000000");
    assert_eq!(residual.sell_notional().to_string(), "50");
    assert_eq!(known_quantity(&ledger, BUYER), "-0.50000000");

    for ordinal in [0, 1] {
        let key = PositionEpisodeEffectFactRecordV1::state_key(
            &reversal_event_id,
            &BUYER,
            &market(),
            ordinal,
        )
        .unwrap();
        let effect = PositionEpisodeEffectFactRecordV1::decode_at(
            &key,
            ledger.state_image().entries().get(&key).unwrap(),
        )
        .unwrap();
        assert_eq!(effect.leg_ordinal(), ordinal);
    }
    let seller_residual = resolved_episode(&ledger, SELLER);
    assert_eq!(seller_residual.opening_leg_ordinal(), 1);
    assert_eq!(seller_residual.buy_quantity().to_string(), "0.50000000");
    assert_eq!(known_quantity(&ledger, SELLER), "0.50000000");
}

#[test]
fn first_nonzero_observation_is_partial_and_never_claims_trade_pnl() {
    let mut ledger = seeded_ledger(30);
    let first = dynamic_trade_event(
        31,
        0,
        "trd-partial-open",
        "125",
        "0.5",
        BUYER,
        "2",
        SELLER,
        "-3",
    );
    ledger.apply_block(&block(31, vec![first])).unwrap();
    let partial = resolved_episode(&ledger, BUYER);
    assert_eq!(
        partial.completeness(),
        canonical_ledger::EpisodeCompletenessV1::PartialFromFirstObservation
    );
    assert_eq!(partial.opening_position().to_string(), "2.00000000");
    assert_eq!(partial.buy_quantity().to_string(), "0.50000000");
    assert_eq!(partial.buy_notional().to_string(), "62.5");

    let close = dynamic_trade_event(
        32,
        0,
        "trd-partial-close",
        "120",
        "2.5",
        SELLER,
        "-3.5",
        BUYER,
        "2.5",
    );
    ledger.apply_block(&block(32, vec![close])).unwrap();
    let closed = episode_by_id(&ledger, partial.episode_id());
    assert_eq!(closed.status(), canonical_ledger::EpisodeStatusV1::Closed);
    assert_eq!(closed.observed_signed_trade_notional_delta().unwrap(), None);
}

#[test]
fn first_observation_reversal_closes_partial_leg_zero_and_opens_complete_leg_one() {
    let mut ledger = seeded_ledger(33);
    let reversal = dynamic_trade_event(
        34,
        0,
        "trd-first-reversal",
        "100",
        "1.5",
        SELLER,
        "0",
        BUYER,
        "1",
    );
    let event_id = reversal.event_id().clone();
    ledger.apply_block(&block(34, vec![reversal])).unwrap();

    let partial_id =
        canonical_ledger::derive_position_episode_id(&BUYER, &market(), &event_id, 0).unwrap();
    let partial = episode_by_id(&ledger, &partial_id);
    assert_eq!(
        partial.completeness(),
        canonical_ledger::EpisodeCompletenessV1::PartialFromFirstObservation
    );
    assert_eq!(partial.opening_position().to_string(), "1.00000000");
    assert_eq!(
        partial.close_cause(),
        Some(canonical_ledger::EpisodeCloseCauseV1::TradeReversal)
    );
    assert_eq!(partial.sell_quantity().to_string(), "1.00000000");
    assert_eq!(partial.sell_notional().to_string(), "100");

    let residual = resolved_episode(&ledger, BUYER);
    assert_eq!(residual.opening_leg_ordinal(), 1);
    assert_eq!(
        residual.completeness(),
        canonical_ledger::EpisodeCompletenessV1::CompleteFromFlat
    );
    assert_eq!(residual.sell_quantity().to_string(), "0.50000000");
    assert_eq!(residual.sell_notional().to_string(), "50");
    assert_eq!(known_quantity(&ledger, BUYER), "-0.50000000");

    let first_effect = effect_for(&ledger, &event_id, BUYER, 0);
    let residual_effect = effect_for(&ledger, &event_id, BUYER, 1);
    assert_eq!(first_effect.episode_id(), &partial_id);
    assert_eq!(residual_effect.episode_id(), residual.episode_id());
    assert_eq!(first_effect.sell_quantity_delta().to_string(), "1.00000000");
    assert_eq!(
        residual_effect.sell_quantity_delta().to_string(),
        "0.50000000"
    );
}

#[test]
fn later_events_in_one_block_see_prior_candidate_episode_state() {
    let mut ledger = seeded_ledger(35);
    let open = dynamic_trade_event(
        36,
        0,
        "trd-same-block-open",
        "100",
        "1",
        BUYER,
        "0",
        SELLER,
        "0",
    );
    let add = dynamic_trade_event(
        36,
        1,
        "trd-same-block-add",
        "110",
        "0.5",
        BUYER,
        "1",
        SELLER,
        "-1",
    );
    ledger.apply_block(&block(36, vec![open, add])).unwrap();
    let episode = resolved_episode(&ledger, BUYER);
    assert_eq!(episode.buy_quantity().to_string(), "1.50000000");
    assert_eq!(episode.buy_notional().to_string(), "155");
    assert_eq!(known_quantity(&ledger, BUYER), "1.50000000");
}

#[test]
fn funding_is_exactly_attributed_only_to_a_resolved_open_episode() {
    let mut ledger = seeded_ledger(40);
    let absent_entries = episode_entries(&ledger);
    ledger
        .apply_block(&block(41, vec![funding_event_at(41, true, "1.2500")]))
        .unwrap();
    assert_eq!(episode_entries(&ledger), absent_entries);

    let open = dynamic_trade_event(
        42,
        0,
        "trd-funding-open",
        "100",
        "1",
        BUYER,
        "0",
        SELLER,
        "0",
    );
    ledger.apply_block(&block(42, vec![open])).unwrap();
    let episode_id = resolved_episode(&ledger, BUYER).episode_id().clone();

    let paid = funding_event_at(43, true, "1.2500");
    let paid_event_id = paid.event_id().clone();
    ledger.apply_block(&block(43, vec![paid])).unwrap();
    let after_paid = resolved_episode(&ledger, BUYER);
    assert_eq!(after_paid.funding_paid().to_string(), "1.2500");
    assert_eq!(after_paid.funding_received().to_string(), "0.0000");
    let paid_effect = effect_for(&ledger, &paid_event_id, BUYER, 0);
    assert_eq!(paid_effect.funding_paid_delta().to_string(), "1.2500");
    assert_eq!(paid_effect.funding_received_delta().to_string(), "0.0000");
    assert_eq!(paid_effect.buy_quantity_delta().raw(), 0);
    assert_eq!(paid_effect.sell_quantity_delta().raw(), 0);

    let received = funding_event_at(44, false, "0.125");
    let received_event_id = received.event_id().clone();
    ledger.apply_block(&block(44, vec![received])).unwrap();
    let after_received = resolved_episode(&ledger, BUYER);
    assert_eq!(after_received.episode_id(), &episode_id);
    assert_eq!(after_received.funding_paid().to_string(), "1.2500");
    assert_eq!(after_received.funding_received().to_string(), "0.1250");
    let received_effect = effect_for(&ledger, &received_event_id, BUYER, 0);
    assert_eq!(received_effect.funding_paid_delta().to_string(), "0.0000");
    assert_eq!(
        received_effect.funding_received_delta().to_string(),
        "0.1250"
    );
    let account_funding_key = AccountQuoteFlowCurrentRecordV1::state_key(
        &BUYER,
        &AccountQuoteFlowScopeV1::MarketFunding {
            market_id: market(),
        },
    )
    .unwrap();
    let account_funding = AccountQuoteFlowCurrentRecordV1::decode_at(
        &account_funding_key,
        ledger
            .state_image()
            .entries()
            .get(&account_funding_key)
            .unwrap(),
    )
    .unwrap();
    // The account ledger correctly retains both paid events, including the
    // one that episode attribution suppressed before any position existed.
    assert_eq!(account_funding.debits().to_string(), "2.5000");
    assert_eq!(account_funding.credits().to_string(), "0.1250");

    let close = dynamic_trade_event(
        45,
        0,
        "trd-funding-close",
        "100",
        "1",
        SELLER,
        "-1",
        BUYER,
        "1",
    );
    ledger.apply_block(&block(45, vec![close])).unwrap();
    let flat_entries = episode_entries(&ledger);
    ledger
        .apply_block(&block(46, vec![funding_event_at(46, true, "0.250")]))
        .unwrap();
    assert_eq!(episode_entries(&ledger), flat_entries);

    let mut reverse = seeded_ledger(140);
    let open = dynamic_trade_event(
        141,
        0,
        "trd-funding-reverse",
        "100",
        "1",
        BUYER,
        "0",
        SELLER,
        "0",
    );
    reverse.apply_block(&block(141, vec![open])).unwrap();
    reverse
        .apply_block(&block(142, vec![funding_event_at(142, false, "0.125")]))
        .unwrap();
    let paid = funding_event_at(143, true, "1.2500");
    let paid_id = paid.event_id().clone();
    reverse.apply_block(&block(143, vec![paid])).unwrap();
    let reverse_episode = resolved_episode(&reverse, BUYER);
    assert_eq!(reverse_episode.funding_paid().to_string(), "1.2500");
    assert_eq!(reverse_episode.funding_received().to_string(), "0.1250");
    let reverse_effect = effect_for(&reverse, &paid_id, BUYER, 0);
    assert_eq!(reverse_effect.funding_paid_delta().to_string(), "1.2500");
    assert_eq!(
        reverse_effect.funding_received_delta().to_string(),
        "0.0000"
    );
}

#[test]
fn unresolved_pairs_reanchor_from_zero_or_nonzero_source_state() {
    for (first_height, source_start, expected_completeness) in [
        (
            50,
            "0",
            canonical_ledger::EpisodeCompletenessV1::CompleteFromFlat,
        ),
        (
            52,
            "2",
            canonical_ledger::EpisodeCompletenessV1::PartialFromFirstObservation,
        ),
    ] {
        let quantity_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
        let episode_key = PositionEpisodeCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
        let quantity_bytes = unresolved_quantity_current_bytes();
        let episode_bytes = interrupted_episode_current_bytes();
        PositionQuantityCurrentRecordV1::decode_at(&quantity_key, &quantity_bytes).unwrap();
        PositionEpisodeCurrentRecordV1::decode_at(&episode_key, &episode_bytes).unwrap();

        let mut ledger = injected_episode_ledger_with_quantity(
            first_height,
            vec![
                StateMutation::put(quantity_key, quantity_bytes),
                StateMutation::put(episode_key, episode_bytes),
            ],
            true,
        );
        let trade = dynamic_trade_event(
            first_height + 1,
            0,
            &format!("trd-reanchor-{first_height}"),
            "100",
            "0.5",
            BUYER,
            source_start,
            SELLER,
            "0",
        );
        ledger
            .apply_block(&block(first_height + 1, vec![trade]))
            .unwrap();
        let episode = resolved_episode(&ledger, BUYER);
        assert_eq!(episode.completeness(), expected_completeness);
        assert_eq!(
            episode.opening_position().to_string(),
            if source_start == "0" {
                "0.00000000"
            } else {
                "2.00000000"
            }
        );
        assert_eq!(
            known_quantity(&ledger, BUYER),
            if source_start == "0" {
                "0.50000000"
            } else {
                "2.50000000"
            }
        );
    }
}

#[test]
fn interrupted_pair_suppresses_funding_without_touching_episode_bytes() {
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let episode_key = PositionEpisodeCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let mut ledger = injected_episode_ledger(
        54,
        vec![
            StateMutation::put(quantity_key, unresolved_quantity_current_bytes()),
            StateMutation::put(episode_key, interrupted_episode_current_bytes()),
        ],
    );
    let before = ledger.state_image().entries().clone();
    ledger
        .apply_block(&block(55, vec![funding_event_at(55, true, "1")]))
        .unwrap();
    assert_eq!(ledger.state_image().entries(), &before);
}

#[test]
fn funding_identity_market_and_stale_pair_failures_are_distinct() {
    let mut missing = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(200),
        CanonicalPositionEpisodeReducerV1,
        LedgerLimits::production(),
    )
    .unwrap();
    let error = missing
        .apply_block(&block(200, vec![funding_event_at(200, true, "1")]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.market_prerequisite_missing")
    );

    let mut identity = injected_episode_ledger(202, Vec::new());
    let mismatched = raw_event_at(
        203,
        0,
        "1.0.0",
        EventPayload::FundingPaid(FundingPaid {
            account_id: BUYER,
            market_id: market(),
            amount: QuoteAmount::from_str("1").unwrap(),
            funding_rate: FundingRate::from_str("0.0001").unwrap(),
        }),
        vec![market()],
        vec![SELLER],
    );
    let error = identity
        .apply_block(&block(203, vec![mismatched]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.identity_mismatch")
    );

    let mut unresolved = injected_episode_ledger(204, Vec::new());
    let metadata_change = raw_event_at(
        205,
        0,
        "1.0.0",
        EventPayload::MarketMetadataChanged(MarketMetadataChanged {
            market_id: market(),
            metadata_version: "unresolved-v2".to_owned(),
            metadata_hash: [9; 32],
        }),
        vec![market()],
        Vec::new(),
    );
    unresolved
        .apply_block(&block(205, vec![metadata_change]))
        .unwrap();
    let error = unresolved
        .apply_block(&block(206, vec![funding_event_at(206, true, "1")]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.market_prerequisite_unresolved")
    );

    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    assert_event_failure_with_injections(
        208,
        vec![
            StateMutation::put(quantity_key, quantity_current_bytes(Some("1.00000000"))),
            StateMutation::put(current_key, episode_current_bytes("no_open_episode", None)),
        ],
        funding_event_at(209, true, "1"),
        "position_episode.current_pair_mismatch",
    );
}

#[test]
fn corrupt_or_inconsistent_prestate_fails_with_stable_episode_reasons() {
    let mut source = seeded_ledger(60);
    let open = dynamic_trade_event(
        61,
        0,
        "trd-prestate-source",
        "100",
        "1",
        BUYER,
        "0",
        SELLER,
        "0",
    );
    source.apply_block(&block(61, vec![open])).unwrap();

    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let open_current_bytes = source
        .state_image()
        .entries()
        .get(&current_key)
        .unwrap()
        .clone();
    let quantity_bytes = source
        .state_image()
        .entries()
        .get(&quantity_key)
        .unwrap()
        .clone();
    let open_current =
        PositionEpisodeCurrentRecordV1::decode_at(&current_key, &open_current_bytes).unwrap();
    let episode_key =
        PositionEpisodeRecordV1::state_key(open_current.episode_id().unwrap()).unwrap();
    let open_episode_bytes = source
        .state_image()
        .entries()
        .get(&episode_key)
        .unwrap()
        .clone();

    assert_episode_failure(
        70,
        vec![StateMutation::put(
            quantity_key.clone(),
            quantity_bytes.clone(),
        )],
        "1",
        "position_episode.current_pair_mismatch",
    );
    assert_episode_failure(
        72,
        vec![
            StateMutation::put(quantity_key.clone(), b"corrupt".to_vec()),
            StateMutation::put(current_key.clone(), open_current_bytes.clone()),
            StateMutation::put(episode_key.clone(), open_episode_bytes.clone()),
        ],
        "1",
        "position_episode.quantity_current_invalid",
    );
    assert_episode_failure(
        74,
        vec![
            StateMutation::put(quantity_key.clone(), quantity_bytes.clone()),
            StateMutation::put(current_key.clone(), b"corrupt".to_vec()),
            StateMutation::put(episode_key.clone(), open_episode_bytes.clone()),
        ],
        "1",
        "position_episode.episode_current_invalid",
    );
    assert_episode_failure(
        76,
        vec![
            StateMutation::put(quantity_key.clone(), quantity_bytes.clone()),
            StateMutation::put(current_key.clone(), open_current_bytes.clone()),
        ],
        "1",
        "position_episode.episode_reference_invalid",
    );
    let overflowing_start = PositionQuantity::from_raw(i128::MAX, 8)
        .unwrap()
        .to_string();
    let arithmetic_failure = dynamic_trade_event(
        81,
        0,
        "trd-precedence-overflow",
        "100",
        "1",
        BUYER,
        &overflowing_start,
        SELLER,
        "0",
    );
    for (injections, reason) in [
        (
            vec![
                StateMutation::put(quantity_key.clone(), b"corrupt".to_vec()),
                StateMutation::put(current_key.clone(), open_current_bytes.clone()),
                StateMutation::put(episode_key.clone(), open_episode_bytes.clone()),
            ],
            "position_episode.quantity_current_invalid",
        ),
        (
            vec![
                StateMutation::put(quantity_key.clone(), quantity_bytes.clone()),
                StateMutation::put(current_key.clone(), b"corrupt".to_vec()),
                StateMutation::put(episode_key.clone(), open_episode_bytes.clone()),
            ],
            "position_episode.episode_current_invalid",
        ),
        (
            vec![
                StateMutation::put(quantity_key.clone(), quantity_bytes.clone()),
                StateMutation::put(current_key.clone(), open_current_bytes.clone()),
            ],
            "position_episode.episode_reference_invalid",
        ),
        (
            vec![StateMutation::put(
                quantity_key.clone(),
                quantity_bytes.clone(),
            )],
            "position_episode.current_pair_mismatch",
        ),
    ] {
        assert_event_failure_with_injections(80, injections, arithmetic_failure.clone(), reason);
    }
    assert_event_failure_with_injections(
        80,
        vec![
            StateMutation::put(quantity_key.clone(), quantity_bytes.clone()),
            StateMutation::put(current_key.clone(), open_current_bytes.clone()),
            StateMutation::put(episode_key.clone(), open_episode_bytes.clone()),
        ],
        arithmetic_failure,
        "position_episode.start_position_mismatch",
    );
    let tick_failure = dynamic_trade_event(
        81,
        0,
        "trd-precedence-tick",
        "100.05",
        "1",
        BUYER,
        "2",
        SELLER,
        "0",
    );
    assert_event_failure_with_injections(
        80,
        vec![
            StateMutation::put(quantity_key.clone(), quantity_bytes.clone()),
            StateMutation::put(current_key.clone(), open_current_bytes.clone()),
            StateMutation::put(episode_key.clone(), open_episode_bytes.clone()),
        ],
        tick_failure,
        "position_episode.start_position_mismatch",
    );
    assert_episode_failure(
        78,
        vec![
            StateMutation::put(quantity_key, quantity_bytes),
            StateMutation::put(current_key, open_current_bytes),
            StateMutation::put(episode_key, open_episode_bytes),
        ],
        "2",
        "position_episode.start_position_mismatch",
    );
}

#[test]
fn every_structural_quantity_episode_pair_mismatch_is_rejected() {
    let opening = EventId::new("evt-overflow-open").unwrap();
    let episode_id =
        canonical_ledger::derive_position_episode_id(&BUYER, &market(), &opening, 0).unwrap();
    let episode_record = open_pair_mutations("1.00000000", "100", "0")
        .into_iter()
        .find(|mutation| mutation.key().namespace() == "position-episode.v1")
        .unwrap();
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let pairs = [
        vec![StateMutation::put(
            current_key.clone(),
            episode_current_bytes("no_open_episode", None),
        )],
        vec![StateMutation::put(
            quantity_key.clone(),
            quantity_current_bytes(Some("0.00000000")),
        )],
        vec![
            StateMutation::put(
                quantity_key.clone(),
                quantity_current_bytes(Some("0.00000000")),
            ),
            StateMutation::put(
                current_key.clone(),
                episode_current_bytes("interrupted", None),
            ),
        ],
        vec![
            StateMutation::put(
                quantity_key.clone(),
                quantity_current_bytes(Some("1.00000000")),
            ),
            StateMutation::put(
                current_key.clone(),
                episode_current_bytes("no_open_episode", None),
            ),
        ],
        vec![
            StateMutation::put(quantity_key.clone(), quantity_current_bytes(None)),
            StateMutation::put(
                current_key.clone(),
                episode_current_bytes("no_open_episode", None),
            ),
        ],
        vec![
            StateMutation::put(quantity_key.clone(), quantity_current_bytes(None)),
            StateMutation::put(
                current_key.clone(),
                episode_current_bytes("resolved", Some(&episode_id)),
            ),
            episode_record.clone(),
        ],
        vec![
            StateMutation::put(
                quantity_key.clone(),
                quantity_current_bytes(Some("1.00000000")),
            ),
            StateMutation::put(
                current_key.clone(),
                episode_current_bytes("interrupted", None),
            ),
        ],
        vec![
            StateMutation::put(
                quantity_key.clone(),
                quantity_current_bytes(Some("0.00000000")),
            ),
            StateMutation::put(
                current_key,
                episode_current_bytes("resolved", Some(&episode_id)),
            ),
            episode_record,
        ],
    ];
    for (offset, injections) in pairs.into_iter().enumerate() {
        let first_height = 170 + u64::try_from(offset).unwrap() * 2;
        assert_episode_failure(
            first_height,
            injections,
            "0",
            "position_episode.current_pair_mismatch",
        );
    }
}

#[test]
fn immutable_effect_and_episode_collisions_decode_first_and_roll_back() {
    let mut source = seeded_ledger(90);
    let event = dynamic_trade_event(91, 0, "trd-collision", "100", "1", BUYER, "0", SELLER, "0");
    source.apply_block(&block(91, vec![event.clone()])).unwrap();

    let buyer_effect_key =
        PositionEpisodeEffectFactRecordV1::state_key(event.event_id(), &BUYER, &market(), 0)
            .unwrap();
    let seller_effect_key =
        PositionEpisodeEffectFactRecordV1::state_key(event.event_id(), &SELLER, &market(), 0)
            .unwrap();
    let buyer_episode = resolved_episode(&source, BUYER);
    let seller_episode = resolved_episode(&source, SELLER);
    let buyer_episode_key = PositionEpisodeRecordV1::state_key(buyer_episode.episode_id()).unwrap();
    let seller_episode_key =
        PositionEpisodeRecordV1::state_key(seller_episode.episode_id()).unwrap();

    assert_event_failure_with_injections(
        90,
        vec![StateMutation::put(
            buyer_effect_key.clone(),
            source
                .state_image()
                .entries()
                .get(&buyer_effect_key)
                .unwrap()
                .clone(),
        )],
        event.clone(),
        "position_episode.effect_identity_collision",
    );
    assert_event_failure_with_injections(
        90,
        vec![StateMutation::put(
            buyer_effect_key.clone(),
            b"corrupt".to_vec(),
        )],
        event.clone(),
        "position_episode.effect_prior_invalid",
    );
    assert_event_failure_with_injections(
        90,
        vec![StateMutation::put(
            buyer_effect_key.clone(),
            source
                .state_image()
                .entries()
                .get(&seller_effect_key)
                .unwrap()
                .clone(),
        )],
        event.clone(),
        "position_episode.effect_prior_invalid",
    );
    assert_event_failure_with_injections(
        90,
        vec![StateMutation::put(
            buyer_episode_key.clone(),
            source
                .state_image()
                .entries()
                .get(&buyer_episode_key)
                .unwrap()
                .clone(),
        )],
        event.clone(),
        "position_episode.episode_identity_collision",
    );
    assert_event_failure_with_injections(
        90,
        vec![StateMutation::put(
            buyer_episode_key.clone(),
            b"corrupt".to_vec(),
        )],
        event.clone(),
        "position_episode.episode_prior_invalid",
    );
    assert_event_failure_with_injections(
        90,
        vec![StateMutation::put(
            buyer_episode_key,
            source
                .state_image()
                .entries()
                .get(&seller_episode_key)
                .unwrap()
                .clone(),
        )],
        event.clone(),
        "position_episode.episode_prior_invalid",
    );
    assert_event_failure_with_injections(
        90,
        vec![StateMutation::put(
            seller_effect_key.clone(),
            source
                .state_image()
                .entries()
                .get(&seller_effect_key)
                .unwrap()
                .clone(),
        )],
        event,
        "position_episode.effect_identity_collision",
    );
}

#[test]
fn late_episode_failure_rolls_back_prior_trade_and_quantity_siblings() {
    let mut source = seeded_ledger(220);
    let event = dynamic_trade_event(
        221,
        0,
        "trd-late-child",
        "100",
        "1",
        BUYER,
        "0",
        SELLER,
        "0",
    );
    source
        .apply_block(&block(221, vec![event.clone()]))
        .unwrap();
    let seller_effect_key =
        PositionEpisodeEffectFactRecordV1::state_key(event.event_id(), &SELLER, &market(), 0)
            .unwrap();
    let seller_effect = source
        .state_image()
        .entries()
        .get(&seller_effect_key)
        .unwrap()
        .clone();

    let mut target = injected_full_ledger(
        220,
        vec![StateMutation::put(seller_effect_key, seller_effect)],
    );
    let before = target.state_image().clone();
    let error = target
        .apply_block(&block(221, vec![event]))
        .expect_err("late episode collision must reject the full sibling set");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.effect_identity_collision")
    );
    assert_eq!(target.state_image(), &before);
}

#[test]
fn cross_child_duplicate_episode_key_fails_before_candidate_state_commits() {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(230),
        CrossChildDuplicateDispatcher::default(),
        LedgerLimits::production(),
    )
    .unwrap();
    ledger
        .apply_block(&block(230, market_prerequisites(230)))
        .unwrap();
    let before = ledger.state_image().clone();
    let trade = dynamic_trade_event(
        231,
        0,
        "trd-cross-child-duplicate",
        "100",
        "1",
        BUYER,
        "0",
        SELLER,
        "0",
    );
    let error = ledger
        .apply_block(&block(231, vec![trade]))
        .expect_err("cross-child duplicate mutation key must fail");
    assert_eq!(
        error.reducer_reason_code(),
        Some("position_episode.duplicate_mutation_key")
    );
    assert_eq!(ledger.state_image(), &before);
}

#[test]
fn cumulative_quantity_notional_and_funding_overflow_fail_atomically() {
    let max_quantity = Quantity::from_raw(i128::MAX, 8).unwrap().to_string();
    let quantity_overflow = dynamic_trade_event(
        121,
        0,
        "trd-quantity-overflow",
        "100",
        "1",
        BUYER,
        "1",
        SELLER,
        "0",
    );
    assert_event_failure_with_injections(
        120,
        open_pair_mutations(&max_quantity, "1", "0"),
        quantity_overflow,
        "position_episode.quantity_arithmetic",
    );

    let maximum_512_bit =
        "1340780792994259709957402499820584612747936582059239337772356144372176403007\
         3546976801874298166903427690031858186486050853753882811946569946433649006084095"
            .replace(' ', "");
    let notional_overflow = dynamic_trade_event(
        123,
        0,
        "trd-notional-overflow",
        "100",
        "1",
        BUYER,
        "1",
        SELLER,
        "0",
    );
    assert_event_failure_with_injections(
        122,
        open_pair_mutations("1.00000000", &maximum_512_bit, "0"),
        notional_overflow,
        "position_episode.notional_arithmetic",
    );
    let mismatch_before_notional_overflow = dynamic_trade_event(
        123,
        0,
        "trd-mismatch-before-notional-overflow",
        "100",
        "1",
        BUYER,
        "2",
        SELLER,
        "0",
    );
    assert_event_failure_with_injections(
        122,
        open_pair_mutations("1.00000000", &maximum_512_bit, "0"),
        mismatch_before_notional_overflow,
        "position_episode.start_position_mismatch",
    );

    let maximum_funding = QuoteAmount::from_raw(i128::MAX, 0).unwrap().to_string();
    assert_event_failure_with_injections(
        124,
        open_pair_mutations("1.00000000", "100", &maximum_funding),
        funding_event_at(125, true, "1"),
        "position_episode.funding_arithmetic",
    );
    assert_event_failure_with_injections(
        126,
        open_pair_mutations("1.00000000", "100", &maximum_funding),
        funding_event_at(127, false, "0.1"),
        "position_episode.funding_arithmetic",
    );
}

#[test]
fn episode_trade_validation_maps_tick_lot_and_downscale_failures_stably() {
    for (first_height, price, fill) in [
        (160, "100.05", "1"),
        (162, "100", "0.0005"),
        (164, "100.0000001", "1"),
    ] {
        let event = dynamic_trade_event(
            first_height + 1,
            0,
            &format!("trd-validation-{first_height}"),
            price,
            fill,
            BUYER,
            "0",
            SELLER,
            "0",
        );
        assert_event_failure_with_injections(
            first_height,
            Vec::new(),
            event,
            "position_episode.quantity_arithmetic",
        );
    }
}

fn seeded_ledger(first_height: u64) -> CanonicalLedger<EpisodeDispatcher> {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        EpisodeDispatcher::default(),
        LedgerLimits::production(),
    )
    .unwrap();
    ledger
        .apply_block(&block(first_height, market_prerequisites(first_height)))
        .unwrap();
    ledger
}

fn injected_episode_ledger(
    first_height: u64,
    injections: Vec<StateMutation>,
) -> CanonicalLedger<InjectionDispatcher> {
    injected_episode_ledger_with_quantity(first_height, injections, false)
}

fn injected_episode_ledger_with_quantity(
    first_height: u64,
    injections: Vec<StateMutation>,
    include_quantity: bool,
) -> CanonicalLedger<InjectionDispatcher> {
    let dispatcher = InjectionDispatcher {
        injection_height: BlockHeight::new(first_height),
        injections,
        market: CanonicalMarketReducerV1,
        trade: None,
        account: None,
        quantity: include_quantity.then_some(CanonicalPositionReducerV1),
        episode: CanonicalPositionEpisodeReducerV1,
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        dispatcher,
        LedgerLimits::production(),
    )
    .unwrap();
    let mut events = market_prerequisites(first_height);
    events.push(injection_trigger(
        first_height,
        u32::try_from(events.len()).unwrap(),
    ));
    ledger.apply_block(&block(first_height, events)).unwrap();
    ledger
}

fn injected_full_ledger(
    first_height: u64,
    injections: Vec<StateMutation>,
) -> CanonicalLedger<InjectionDispatcher> {
    let dispatcher = InjectionDispatcher {
        injection_height: BlockHeight::new(first_height),
        injections,
        market: CanonicalMarketReducerV1,
        trade: Some(CanonicalTradeReducerSetV2),
        account: Some(CanonicalAccountReducerV1),
        quantity: Some(CanonicalPositionReducerV1),
        episode: CanonicalPositionEpisodeReducerV1,
    };
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        dispatcher,
        LedgerLimits::production(),
    )
    .unwrap();
    let mut events = market_prerequisites(first_height);
    events.push(injection_trigger(
        first_height,
        u32::try_from(events.len()).unwrap(),
    ));
    ledger.apply_block(&block(first_height, events)).unwrap();
    ledger
}

fn assert_episode_failure(
    first_height: u64,
    injections: Vec<StateMutation>,
    buyer_start: &str,
    expected_reason: &str,
) {
    let mut ledger = injected_episode_ledger(first_height, injections);
    let before = ledger.state_image().clone();
    let trade = dynamic_trade_event(
        first_height + 1,
        0,
        &format!("trd-prestate-{first_height}"),
        "100",
        "0.5",
        BUYER,
        buyer_start,
        SELLER,
        "0",
    );
    let error = ledger
        .apply_block(&block(first_height + 1, vec![trade]))
        .expect_err("invalid episode prestate must fail");
    assert_eq!(error.reducer_reason_code(), Some(expected_reason));
    assert_eq!(ledger.state_image(), &before);
}

fn assert_event_failure_with_injections(
    first_height: u64,
    injections: Vec<StateMutation>,
    event: CanonicalEventEnvelope,
    expected_reason: &str,
) {
    let mut ledger = injected_episode_ledger(first_height, injections);
    let before = ledger.state_image().clone();
    let event_height = event.block_height().get();
    let error = ledger
        .apply_block(&block(event_height, vec![event]))
        .expect_err("immutable identity collision must fail");
    assert_eq!(error.reducer_reason_code(), Some(expected_reason));
    assert_eq!(ledger.state_image(), &before);
}

fn resolved_episode<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    account: Address,
) -> PositionEpisodeRecordV1 {
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&account, &market()).unwrap();
    let current = PositionEpisodeCurrentRecordV1::decode_at(
        &current_key,
        ledger.state_image().entries().get(&current_key).unwrap(),
    )
    .unwrap();
    let episode_id = current.episode_id().expect("resolved episode");
    episode_by_id(ledger, episode_id)
}

fn episode_by_id<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    episode_id: &domain_types::PositionEpisodeId,
) -> PositionEpisodeRecordV1 {
    let key = PositionEpisodeRecordV1::state_key(episode_id).unwrap();
    PositionEpisodeRecordV1::decode_at(&key, ledger.state_image().entries().get(&key).unwrap())
        .unwrap()
}

fn effect_for<R: EventReducer>(
    ledger: &CanonicalLedger<R>,
    event_id: &EventId,
    account: Address,
    ordinal: u8,
) -> PositionEpisodeEffectFactRecordV1 {
    let key = PositionEpisodeEffectFactRecordV1::state_key(event_id, &account, &market(), ordinal)
        .unwrap();
    PositionEpisodeEffectFactRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn episode_entries(ledger: &CanonicalLedger<EpisodeDispatcher>) -> BTreeMap<StateKey, Vec<u8>> {
    ledger
        .state_image()
        .entries()
        .iter()
        .filter(|(key, _)| key.namespace().starts_with("position-episode"))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn known_quantity<R: EventReducer>(ledger: &CanonicalLedger<R>, account: Address) -> String {
    let key = PositionQuantityCurrentRecordV1::state_key(&account, &market()).unwrap();
    PositionQuantityCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
    .known_quantity()
    .unwrap()
    .to_string()
}

fn assert_no_open_episode<R: EventReducer>(ledger: &CanonicalLedger<R>, account: Address) {
    let key = PositionEpisodeCurrentRecordV1::state_key(&account, &market()).unwrap();
    let current = PositionEpisodeCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap();
    assert_eq!(current.episode_id(), None);
    assert_eq!(
        current.attribution_resolution(),
        canonical_ledger::EpisodeAttributionResolutionV1::NoOpenEpisode
    );
}

#[allow(clippy::too_many_arguments)]
fn dynamic_trade_event(
    height: u64,
    event_index: u32,
    trade_id: &str,
    price: &str,
    fill: &str,
    buyer_account: Address,
    buyer_start: &str,
    seller_account: Address,
    seller_start: &str,
) -> CanonicalEventEnvelope {
    raw_event_at(
        height,
        event_index,
        "1.0.0",
        EventPayload::TradeMatched(TradeMatched {
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
                    account_id: buyer_account,
                    start_position: PositionQuantity::from_str(buyer_start).unwrap(),
                    order_id: OrderId::new(format!("buyer-order-{trade_id}")).unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
                TradeParticipantV1 {
                    role: TradeParticipantRoleV1::Seller,
                    account_id: seller_account,
                    start_position: PositionQuantity::from_str(seller_start).unwrap(),
                    order_id: OrderId::new(format!("seller-order-{trade_id}")).unwrap(),
                    twap_id: None,
                    client_order_id: None,
                },
            ])),
        }),
        vec![market()],
        vec![buyer_account, seller_account],
    )
}

fn funding_event_at(height: u64, paid: bool, amount: &str) -> CanonicalEventEnvelope {
    let amount = QuoteAmount::from_str(amount).unwrap();
    let payload = if paid {
        EventPayload::FundingPaid(FundingPaid {
            account_id: BUYER,
            market_id: market(),
            amount,
            funding_rate: FundingRate::from_str("0.0001").unwrap(),
        })
    } else {
        EventPayload::FundingReceived(FundingReceived {
            account_id: BUYER,
            market_id: market(),
            amount,
            funding_rate: FundingRate::from_str("-0.0001").unwrap(),
        })
    };
    raw_event_at(height, 0, "1.0.0", payload, vec![market()], vec![BUYER])
}

fn injection_trigger(height: u64, event_index: u32) -> CanonicalEventEnvelope {
    raw_event_at(
        height,
        event_index,
        "1.0.0",
        EventPayload::OrderFilled(OrderFilled {
            order_id: OrderId::new(format!("injection-order-{height}")).unwrap(),
            trade_id: TradeId::new(format!("injection-trade-{height}")).unwrap(),
            fill_price: Price::from_str("100").unwrap(),
            fill_quantity: Quantity::from_str("1").unwrap(),
        }),
        Vec::new(),
        Vec::new(),
    )
}

fn unresolved_quantity_current_bytes() -> Vec<u8> {
    quantity_current_bytes(None)
}

fn quantity_current_bytes(known_quantity: Option<&str>) -> Vec<u8> {
    let known_quantity = known_quantity
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "null".to_owned());
    let first_anchor = if known_quantity == "null" {
        "null".to_owned()
    } else {
        "\"evt-overflow-open\"".to_owned()
    };
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-quantity-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"known_quantity\":{known_quantity},\"first_anchor_event_id\":{first_anchor},\"last_event_id\":\"evt-unresolved\",\"last_block_height\":1}}",
        BUYER.to_api_string(),
        market().as_str(),
    )
    .into_bytes()
}

fn interrupted_episode_current_bytes() -> Vec<u8> {
    episode_current_bytes("interrupted", None)
}

fn episode_current_bytes(
    resolution: &str,
    episode_id: Option<&domain_types::PositionEpisodeId>,
) -> Vec<u8> {
    let episode_id = episode_id
        .map(|value| format!("\"{}\"", value.as_str()))
        .unwrap_or_else(|| "null".to_owned());
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"episode_id\":{episode_id},\"attribution_resolution\":\"{resolution}\",\"last_event_id\":\"evt-unresolved\",\"last_block_height\":1}}",
        BUYER.to_api_string(),
        market().as_str(),
    )
    .into_bytes()
}

fn open_pair_mutations(
    buy_quantity: &str,
    buy_notional: &str,
    funding_paid: &str,
) -> Vec<StateMutation> {
    let opening = EventId::new("evt-overflow-open").unwrap();
    let episode_id =
        canonical_ledger::derive_position_episode_id(&BUYER, &market(), &opening, 0).unwrap();
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&BUYER, &market()).unwrap();
    let episode_key = PositionEpisodeRecordV1::state_key(&episode_id).unwrap();
    let quantity_bytes = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-quantity-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"known_quantity\":\"1.00000000\",\"first_anchor_event_id\":\"evt-overflow-open\",\"last_event_id\":\"evt-overflow-open\",\"last_block_height\":1}}",
        BUYER.to_api_string(),
        market().as_str(),
    )
    .into_bytes();
    let current_bytes = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode-current/v1\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"episode_id\":\"{}\",\"attribution_resolution\":\"resolved\",\"last_event_id\":\"evt-overflow-open\",\"last_block_height\":1}}",
        BUYER.to_api_string(),
        market().as_str(),
        episode_id.as_str(),
    )
    .into_bytes();
    let episode_bytes = format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"evt-overflow-open\",\"opening_leg_ordinal\":0,\"opening_position\":\"0.00000000\",\"close_event_id\":null,\"close_cause\":null,\"completeness\":\"complete_from_flat\",\"buy_quantity\":\"{buy_quantity}\",\"buy_notional\":\"{buy_notional}\",\"sell_quantity\":\"0.00000000\",\"sell_notional\":\"0\",\"funding_paid\":\"{funding_paid}\",\"funding_received\":\"0\",\"status\":\"open\",\"last_event_id\":\"evt-overflow-open\",\"last_block_height\":1}}",
        episode_id.as_str(),
        BUYER.to_api_string(),
        market().as_str(),
    )
    .into_bytes();
    PositionQuantityCurrentRecordV1::decode_at(&quantity_key, &quantity_bytes).unwrap();
    PositionEpisodeCurrentRecordV1::decode_at(&current_key, &current_bytes).unwrap();
    PositionEpisodeRecordV1::decode_at(&episode_key, &episode_bytes).unwrap();
    vec![
        StateMutation::put(quantity_key, quantity_bytes),
        StateMutation::put(current_key, current_bytes),
        StateMutation::put(episode_key, episode_bytes),
    ]
}

fn market_prerequisites(height: u64) -> Vec<CanonicalEventEnvelope> {
    let base = AssetId::new("BTC").unwrap();
    let quote = AssetId::new("USDC").unwrap();
    vec![
        raw_event_at(
            height,
            0,
            "1.0.0",
            EventPayload::DexCreated(DexCreated {
                dex_id: DexId::new("validator").unwrap(),
                name: "Validator".to_owned(),
                operator_account_id: OPERATOR,
            }),
            Vec::new(),
            vec![OPERATOR],
        ),
        raw_event_at(
            height,
            1,
            "1.0.0",
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: base.clone(),
                context_version: "btc-v1".to_owned(),
                context_hash: [1; 32],
            }),
            Vec::new(),
            Vec::new(),
        ),
        raw_event_at(
            height,
            2,
            "1.0.0",
            EventPayload::AssetContextUpdated(AssetContextUpdated {
                asset_id: quote.clone(),
                context_version: "usdc-v1".to_owned(),
                context_hash: [2; 32],
            }),
            Vec::new(),
            Vec::new(),
        ),
        raw_event_at(
            height,
            3,
            "1.0.0",
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
        ),
    ]
}

fn trade_event(enriched: bool, schema: &str) -> CanonicalEventEnvelope {
    raw_event(
        schema,
        EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(TradeId::new("trd-episode-support").unwrap()),
            market_id: Some(market()),
            maker_order_id: None,
            taker_order_id: None,
            price: Price::from_str("65000").unwrap(),
            quantity: Quantity::from_str("0.25").unwrap(),
            deterministic_seed: 1,
            participants: enriched.then(|| {
                Box::new([
                    TradeParticipantV1 {
                        role: TradeParticipantRoleV1::Buyer,
                        account_id: BUYER,
                        start_position: PositionQuantity::from_str("0").unwrap(),
                        order_id: OrderId::new("buyer-order").unwrap(),
                        twap_id: None,
                        client_order_id: None,
                    },
                    TradeParticipantV1 {
                        role: TradeParticipantRoleV1::Seller,
                        account_id: SELLER,
                        start_position: PositionQuantity::from_str("0").unwrap(),
                        order_id: OrderId::new("seller-order").unwrap(),
                        twap_id: None,
                        client_order_id: None,
                    },
                ])
            }),
        }),
        vec![market()],
        vec![BUYER, SELLER],
    )
}

fn funding_paid_event() -> CanonicalEventEnvelope {
    raw_event(
        "1.0.0",
        EventPayload::FundingPaid(FundingPaid {
            account_id: BUYER,
            market_id: market(),
            amount: QuoteAmount::from_str("1.25").unwrap(),
            funding_rate: FundingRate::from_str("0.0001").unwrap(),
        }),
        vec![market()],
        vec![BUYER],
    )
}

fn funding_received_event() -> CanonicalEventEnvelope {
    raw_event(
        "1.0.0",
        EventPayload::FundingReceived(FundingReceived {
            account_id: BUYER,
            market_id: market(),
            amount: QuoteAmount::from_str("1.25").unwrap(),
            funding_rate: FundingRate::from_str("-0.0001").unwrap(),
        }),
        vec![market()],
        vec![BUYER],
    )
}

fn order_fill_event() -> CanonicalEventEnvelope {
    raw_event(
        "1.0.0",
        EventPayload::OrderFilled(OrderFilled {
            order_id: OrderId::new("order").unwrap(),
            trade_id: TradeId::new("trade").unwrap(),
            fill_price: Price::from_str("65000").unwrap(),
            fill_quantity: Quantity::from_str("0.25").unwrap(),
        }),
        Vec::new(),
        Vec::new(),
    )
}

fn fee_event() -> CanonicalEventEnvelope {
    raw_event(
        "1.0.0",
        EventPayload::FeeCharged(FeeCharged {
            account_id: BUYER,
            asset_id: AssetId::new("USDC").unwrap(),
            amount: Quantity::from_str("1").unwrap(),
            fee_rate: FeeRate::from_str("0.001").unwrap(),
            fee_type: FeeTypeV1::Taker,
        }),
        Vec::new(),
        vec![BUYER],
    )
}

fn raw_event(
    schema: &str,
    payload: EventPayload,
    markets: Vec<MarketId>,
    accounts: Vec<Address>,
) -> CanonicalEventEnvelope {
    raw_event_at(1, 0, schema, payload, markets, accounts)
}

fn raw_event_at(
    height: u64,
    event_index: u32,
    schema: &str,
    payload: EventPayload,
    markets: Vec<MarketId>,
    accounts: Vec<Address>,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-episode-{height}-{event_index}")).unwrap(),
        transaction_index: event_index,
        canonical_event_index: 0,
        market_ids: markets,
        account_ids: accounts,
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").unwrap(),
                "episode-support",
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
        parser_version: "episode-support@1.0.0".to_owned(),
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

fn episode_bytes(
    completeness: &str,
    status: &str,
    close_cause: Option<&str>,
    buy_notional: &str,
    sell_notional: &str,
) -> Vec<u8> {
    let opening_position = if completeness == "complete_from_flat" {
        "0"
    } else {
        "1"
    };
    let close_event = if status == "open" {
        "null".to_owned()
    } else {
        "\"evt-close\"".to_owned()
    };
    let close_cause = close_cause
        .map(|value| format!("\"{value}\""))
        .unwrap_or_else(|| "null".to_owned());
    let opening = EventId::new("evt-open").unwrap();
    let episode_id =
        canonical_ledger::derive_position_episode_id(&BUYER, &market(), &opening, 0).unwrap();
    let buy_quantity = if buy_notional == "0" { "0" } else { "1" };
    let sell_quantity = if sell_notional == "0" { "0" } else { "1" };
    format!(
        "{{\"schema\":\"hyperliquid-alpha-desk/position-episode/v1\",\"episode_id\":\"{}\",\"account_id\":\"{}\",\"market_id\":\"{}\",\"opening_anchor_event_id\":\"evt-open\",\"opening_leg_ordinal\":0,\"opening_position\":\"{opening_position}\",\"close_event_id\":{close_event},\"close_cause\":{close_cause},\"completeness\":\"{completeness}\",\"buy_quantity\":\"{buy_quantity}\",\"buy_notional\":\"{buy_notional}\",\"sell_quantity\":\"{sell_quantity}\",\"sell_notional\":\"{sell_notional}\",\"funding_paid\":\"0\",\"funding_received\":\"0\",\"status\":\"{status}\",\"last_event_id\":\"{}\",\"last_block_height\":1}}",
        episode_id.as_str(),
        BUYER.to_api_string(),
        market().as_str(),
        if status == "open" { "evt-open" } else { "evt-close" },
    )
    .into_bytes()
}
