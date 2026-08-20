use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, OrderAccepted, OrderCancelled, OrderFilled, OrderModified, OrderPartiallyFilled,
    OrderRejected, OrderRested, SourceEvidence,
};
use canonical_ledger::{
    ApplyOutcome, CanonicalLedger, CanonicalOrderReducerV1, LedgerLimits, OrderCurrentRecordV1,
    OrderFactRecordV1, OrderLifecycleV1, OrderStateError, OrderTransitionRecordV1,
    OrderTransitionStatusV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, ClientOrderId, KnownTime, MarketId, OrderId, OrderSide, Price,
    ProtocolTime, Quantity, SourceId, TransactionId,
};

const ACCOUNT_BYTES: [u8; 20] = [0x11; 20];
const OTHER_ACCOUNT_BYTES: [u8; 20] = [0x22; 20];

#[test]
fn full_fill_lifecycle_creates_immutable_facts_current_state_and_hash_linked_transitions() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("order-full").unwrap();
    let events = [
        order_event(
            100,
            0,
            EventPayload::OrderAccepted(OrderAccepted {
                order_id: order_id.clone(),
                account_id: account,
                market_id: market.clone(),
                side: OrderSide::Buy,
                limit_price: price("65000"),
                quantity: quantity("1"),
            }),
            vec![market.clone()],
            vec![account],
            "1.0.0",
        ),
        order_event(
            100,
            1,
            EventPayload::OrderRested(OrderRested {
                order_id: order_id.clone(),
                market_id: market.clone(),
                remaining_quantity: quantity("1"),
                limit_price: price("65000"),
            }),
            vec![market.clone()],
            vec![account],
            "1.0.0",
        ),
        order_event(
            100,
            2,
            EventPayload::OrderModified(OrderModified {
                order_id: order_id.clone(),
                previous_price: price("65000"),
                new_price: price("65010"),
                previous_quantity: quantity("1"),
                new_quantity: quantity("1.25"),
            }),
            vec![market.clone()],
            vec![account],
            "1.0.0",
        ),
        order_event(
            100,
            3,
            EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
                order_id: order_id.clone(),
                trade_id: domain_types::TradeId::new("trade-partial").unwrap(),
                fill_price: price("65001"),
                fill_quantity: quantity("0.25"),
                remaining_quantity: quantity("1"),
            }),
            vec![market.clone()],
            vec![account],
            "1.0.0",
        ),
        order_event(
            100,
            4,
            EventPayload::OrderFilled(OrderFilled {
                order_id: order_id.clone(),
                trade_id: domain_types::TradeId::new("trade-terminal").unwrap(),
                fill_price: price("65002"),
                fill_quantity: quantity("1"),
            }),
            vec![market.clone()],
            vec![account],
            "1.0.0",
        ),
    ];
    let block = block(100, events.to_vec());
    let mut ledger = ledger(100);

    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block).unwrap() else {
        panic!("new block must apply");
    };

    assert_eq!(delta.mutations().len(), 15);
    assert_eq!(namespace_count(&ledger, "order-fact.v1"), 5);
    assert_eq!(namespace_count(&ledger, "order-current.v1"), 1);
    assert_eq!(namespace_count(&ledger, "order-transition.v1"), 5);

    let current_key = OrderCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let current = OrderCurrentRecordV1::decode_at(
        &current_key,
        ledger
            .state_image()
            .entries()
            .get(&current_key)
            .expect("current order"),
    )
    .unwrap();
    assert_eq!(current.order_id(), &order_id);
    assert_eq!(current.account_id(), account);
    assert_eq!(current.market_id(), &market);
    assert_eq!(current.side(), OrderSide::Buy);
    assert_eq!(current.lifecycle(), OrderLifecycleV1::Filled);
    assert_eq!(current.limit_price(), price("65010"));
    assert_eq!(current.accepted_quantity(), quantity("1.25"));
    assert_eq!(current.filled_quantity(), quantity("1.25"));
    assert_eq!(current.remaining_quantity(), quantity("0"));
    assert_eq!(current.last_event_id(), events[4].event_id());

    let mut previous_result = None;
    for event in &events {
        let fact_key =
            OrderFactRecordV1::state_key_for_order(&market, &order_id, event.event_id()).unwrap();
        let fact = OrderFactRecordV1::decode_at(
            &fact_key,
            ledger.state_image().entries().get(&fact_key).unwrap(),
        )
        .unwrap();
        assert_eq!(fact.event_id(), event.event_id());
        assert_eq!(fact.event_kind(), event.event_kind());
        assert_eq!(fact.order_id(), Some(&order_id));
        assert_eq!(fact.market_id(), Some(&market));
        assert_eq!(fact.account_id(), account);
        assert_eq!(fact.payload_hash(), event.payload_hash());

        let transition_key =
            OrderTransitionRecordV1::state_key_for_order(&market, &order_id, event.event_id())
                .unwrap();
        let transition = OrderTransitionRecordV1::decode_at(
            &transition_key,
            ledger.state_image().entries().get(&transition_key).unwrap(),
        )
        .unwrap();
        assert_eq!(transition.event_id(), event.event_id());
        assert_eq!(transition.payload_hash(), event.payload_hash());
        assert_eq!(transition.prior_state_hash(), previous_result);
        assert_eq!(transition.status(), OrderTransitionStatusV1::Applied);
        assert_eq!(
            transition.rule_version(),
            "hyperliquid-alpha-desk-canonical-order@1.0.0"
        );
        previous_result = transition.result_state_hash();
        assert!(previous_result.is_some());
    }
}

#[test]
fn cancellation_and_rejection_are_terminal_or_fact_only_as_applicable() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:ETH").unwrap();
    let order_id = OrderId::new("order-cancel").unwrap();
    let accepted = accepted_event(200, 0, &order_id, &market, account, quantity("2"));
    let cancelled = order_event(
        200,
        1,
        EventPayload::OrderCancelled(OrderCancelled {
            order_id: order_id.clone(),
            reason: "operator_requested".to_owned(),
            remaining_quantity: quantity("2"),
        }),
        vec![market.clone()],
        vec![account],
        "1.0.0",
    );
    let client_order_id = ClientOrderId::new("client-rejected").unwrap();
    let rejected = order_event(
        200,
        2,
        EventPayload::OrderRejected(OrderRejected {
            client_order_id: client_order_id.clone(),
            account_id: account,
            reason_code: "invalid_tick".to_owned(),
            reason: "limit price is not aligned to the active tick".to_owned(),
        }),
        Vec::new(),
        vec![account],
        "1.0.0",
    );
    let mut ledger = ledger(200);
    ledger
        .apply_block(&block(200, vec![accepted, cancelled, rejected.clone()]))
        .unwrap();

    let current_key = OrderCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let current = OrderCurrentRecordV1::decode_at(
        &current_key,
        ledger.state_image().entries().get(&current_key).unwrap(),
    )
    .unwrap();
    assert_eq!(current.lifecycle(), OrderLifecycleV1::Cancelled);
    assert_eq!(current.filled_quantity(), quantity("0"));
    assert_eq!(current.remaining_quantity(), quantity("2"));

    let rejection_key =
        OrderFactRecordV1::state_key_for_rejection(&account, &client_order_id, rejected.event_id())
            .unwrap();
    let rejection = OrderFactRecordV1::decode_at(
        &rejection_key,
        ledger.state_image().entries().get(&rejection_key).unwrap(),
    )
    .unwrap();
    assert_eq!(rejection.event_kind(), EventKind::OrderRejected);
    assert_eq!(rejection.order_id(), None);
    assert_eq!(rejection.client_order_id(), Some(&client_order_id));
    assert_eq!(rejection.market_id(), None);

    let transition_key = OrderTransitionRecordV1::state_key_for_rejection(
        &account,
        &client_order_id,
        rejected.event_id(),
    )
    .unwrap();
    let transition = OrderTransitionRecordV1::decode_at(
        &transition_key,
        ledger.state_image().entries().get(&transition_key).unwrap(),
    )
    .unwrap();
    assert_eq!(
        transition.status(),
        OrderTransitionStatusV1::RecordedRejection
    );
    assert_eq!(transition.prior_state_hash(), None);
    assert_eq!(transition.result_state_hash(), None);
    assert_eq!(namespace_count(&ledger, "order-current.v1"), 1);
}

#[test]
fn invalid_transitions_and_identity_mismatches_fail_closed_without_advancing() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let other_account = Address::from_bytes(OTHER_ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();

    let fill_before_accept = order_event(
        300,
        0,
        EventPayload::OrderFilled(OrderFilled {
            order_id: OrderId::new("missing-order").unwrap(),
            trade_id: domain_types::TradeId::new("trade-early").unwrap(),
            fill_price: price("1"),
            fill_quantity: quantity("1"),
        }),
        vec![market.clone()],
        vec![account],
        "1.0.0",
    );
    assert_reducer_failure(
        ledger(300)
            .apply_block(&block(300, vec![fill_before_accept]))
            .unwrap_err(),
        "order_state.order_not_found",
    );

    let cases = [
        (
            "identity mismatch",
            order_event(
                301,
                0,
                EventPayload::OrderRested(OrderRested {
                    order_id: OrderId::new("seed").unwrap(),
                    market_id: market.clone(),
                    remaining_quantity: quantity("1"),
                    limit_price: price("1"),
                }),
                vec![market.clone()],
                vec![other_account],
                "1.0.0",
            ),
            "order_state.identity_mismatch",
        ),
        (
            "overfill",
            order_event(
                301,
                0,
                EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
                    order_id: OrderId::new("seed").unwrap(),
                    trade_id: domain_types::TradeId::new("trade-overfill").unwrap(),
                    fill_price: price("1"),
                    fill_quantity: quantity("1.25"),
                    remaining_quantity: quantity("0.25"),
                }),
                vec![market.clone()],
                vec![account],
                "1.0.0",
            ),
            "order_state.overfill",
        ),
        (
            "inconsistent remaining",
            order_event(
                301,
                0,
                EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
                    order_id: OrderId::new("seed").unwrap(),
                    trade_id: domain_types::TradeId::new("trade-bad-remaining").unwrap(),
                    fill_price: price("1"),
                    fill_quantity: quantity("0.25"),
                    remaining_quantity: quantity("0.50"),
                }),
                vec![market.clone()],
                vec![account],
                "1.0.0",
            ),
            "order_state.remaining_mismatch",
        ),
        (
            "filled quantity not terminal",
            order_event(
                301,
                0,
                EventPayload::OrderFilled(OrderFilled {
                    order_id: OrderId::new("seed").unwrap(),
                    trade_id: domain_types::TradeId::new("trade-underfill").unwrap(),
                    fill_price: price("1"),
                    fill_quantity: quantity("0.75"),
                }),
                vec![market.clone()],
                vec![account],
                "1.0.0",
            ),
            "order_state.remaining_mismatch",
        ),
    ];

    for (label, invalid, reason) in cases {
        let mut seeded = ledger(300);
        seeded
            .apply_block(&block(
                300,
                vec![accepted_event(
                    300,
                    0,
                    &OrderId::new("seed").unwrap(),
                    &market,
                    account,
                    quantity("1"),
                )],
            ))
            .unwrap();
        let before = seeded.state_image().canonical_bytes();
        let before_hash = seeded.state_hash();
        let error = seeded.apply_block(&block(301, vec![invalid])).unwrap_err();
        assert_reducer_failure(error, reason);
        assert_eq!(seeded.state_image().canonical_bytes(), before, "{label}");
        assert_eq!(seeded.state_hash(), before_hash, "{label}");
    }
}

#[test]
fn terminal_orders_never_resurrect_and_order_id_collisions_are_rejected() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("terminal").unwrap();
    let mut ledger = ledger(400);
    ledger
        .apply_block(&block(
            400,
            vec![
                accepted_event(400, 0, &order_id, &market, account, quantity("1")),
                order_event(
                    400,
                    1,
                    EventPayload::OrderCancelled(OrderCancelled {
                        order_id: order_id.clone(),
                        reason: "operator_requested".to_owned(),
                        remaining_quantity: quantity("1"),
                    }),
                    vec![market.clone()],
                    vec![account],
                    "1.0.0",
                ),
            ],
        ))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();

    let modified = order_event(
        401,
        0,
        EventPayload::OrderModified(OrderModified {
            order_id: order_id.clone(),
            previous_price: price("65000"),
            new_price: price("65001"),
            previous_quantity: quantity("1"),
            new_quantity: quantity("1"),
        }),
        vec![market.clone()],
        vec![account],
        "1.0.0",
    );
    assert_reducer_failure(
        ledger.apply_block(&block(401, vec![modified])).unwrap_err(),
        "order_state.terminal_order",
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);

    let duplicate = accepted_event(401, 0, &order_id, &market, account, quantity("1"));
    assert_reducer_failure(
        ledger
            .apply_block(&block(401, vec![duplicate]))
            .unwrap_err(),
        "order_state.order_id_collision",
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn lifecycle_table_rejects_a_resting_regression_after_partial_fill() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("no-regression").unwrap();
    let mut ledger = ledger(450);
    ledger
        .apply_block(&block(
            450,
            vec![
                accepted_event(450, 0, &order_id, &market, account, quantity("1")),
                order_event(
                    450,
                    1,
                    EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
                        order_id: order_id.clone(),
                        trade_id: domain_types::TradeId::new("trade-partial-regression").unwrap(),
                        fill_price: price("65000"),
                        fill_quantity: quantity("0.25"),
                        remaining_quantity: quantity("0.75"),
                    }),
                    vec![market.clone()],
                    vec![account],
                    "1.0.0",
                ),
            ],
        ))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();

    let regressed = order_event(
        451,
        0,
        EventPayload::OrderRested(OrderRested {
            order_id,
            market_id: market.clone(),
            remaining_quantity: quantity("0.75"),
            limit_price: price("65000"),
        }),
        vec![market],
        vec![account],
        "1.0.0",
    );
    assert_reducer_failure(
        ledger
            .apply_block(&block(451, vec![regressed]))
            .unwrap_err(),
        "order_state.invalid_transition",
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn a_late_invalid_event_rolls_back_the_complete_candidate_block() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let mut ledger = ledger(500);
    ledger.apply_block(&block(500, Vec::new())).unwrap();
    let before = ledger.state_image().canonical_bytes();
    let before_hash = ledger.state_hash();
    let order_id = OrderId::new("atomic").unwrap();

    let valid = accepted_event(501, 0, &order_id, &market, account, quantity("1"));
    let invalid = order_event(
        501,
        1,
        EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
            order_id,
            trade_id: domain_types::TradeId::new("atomic-overfill").unwrap(),
            fill_price: price("1"),
            fill_quantity: quantity("2"),
            remaining_quantity: quantity("1"),
        }),
        vec![market],
        vec![account],
        "1.0.0",
    );
    assert_reducer_failure(
        ledger
            .apply_block(&block(501, vec![valid, invalid]))
            .unwrap_err(),
        "order_state.overfill",
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert_eq!(ledger.state_hash(), before_hash);
    assert_eq!(
        ledger.checkpoint().unwrap().block_height(),
        BlockHeight::new(500)
    );
}

#[test]
fn reducer_owns_only_exact_order_schema_and_records_are_canonical_and_key_bound() {
    use canonical_ledger::EventReducer;

    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("codec").unwrap();
    let supported = accepted_event(600, 0, &order_id, &market, account, quantity("1"));
    let unsupported = order_event(
        600,
        0,
        supported.payload().clone(),
        vec![market.clone()],
        vec![account],
        "1.1.0",
    );
    assert!(CanonicalOrderReducerV1.supports(&supported));
    assert!(!CanonicalOrderReducerV1.supports(&unsupported));

    let mut ledger = ledger(600);
    let error = ledger
        .apply_block(&block(600, vec![unsupported]))
        .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert_eq!(ledger.checkpoint(), None);

    ledger
        .apply_block(&block(600, vec![supported.clone()]))
        .unwrap();
    let key = OrderCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let encoded = ledger.state_image().entries().get(&key).unwrap();
    for corrupt in [
        encoded[..encoded.len() - 1].to_vec(),
        [encoded.as_slice(), b" "].concat(),
        b"{}".to_vec(),
    ] {
        assert!(OrderCurrentRecordV1::decode(&corrupt).is_err());
    }
    let wrong_key =
        OrderCurrentRecordV1::state_key(&market, &OrderId::new("other").unwrap()).unwrap();
    assert!(OrderCurrentRecordV1::decode_at(&wrong_key, encoded).is_err());

    let fact_key =
        OrderFactRecordV1::state_key_for_order(&market, &order_id, supported.event_id()).unwrap();
    let fact_bytes = ledger.state_image().entries().get(&fact_key).unwrap();
    let wrong_fact_key = OrderFactRecordV1::state_key_for_order(
        &market,
        &OrderId::new("other").unwrap(),
        supported.event_id(),
    )
    .unwrap();
    assert!(OrderFactRecordV1::decode_at(&wrong_fact_key, fact_bytes).is_err());

    let oversized = OrderId::new("x".repeat(70_000)).unwrap();
    assert!(OrderCurrentRecordV1::state_key(&market, &oversized).is_err());
}

#[test]
fn reducer_preserves_valid_nondefault_decimal_scale_and_rejects_zero_admission() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("scale-zero").unwrap();
    let mut ledger = ledger(700);
    ledger
        .apply_block(&block(
            700,
            vec![
                accepted_event(
                    700,
                    0,
                    &order_id,
                    &market,
                    account,
                    Quantity::parse_at_scale("1", 0).unwrap(),
                ),
                order_event(
                    700,
                    1,
                    EventPayload::OrderFilled(OrderFilled {
                        order_id: order_id.clone(),
                        trade_id: domain_types::TradeId::new("scale-fill").unwrap(),
                        fill_price: price("65000"),
                        fill_quantity: Quantity::parse_at_scale("1", 0).unwrap(),
                    }),
                    vec![market.clone()],
                    vec![account],
                    "1.0.0",
                ),
            ],
        ))
        .unwrap();
    let key = OrderCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let current =
        OrderCurrentRecordV1::decode_at(&key, ledger.state_image().entries().get(&key).unwrap())
            .unwrap();
    assert_eq!(current.accepted_quantity().scale(), 0);
    assert_eq!(current.filled_quantity().scale(), 0);
    assert_eq!(current.remaining_quantity().scale(), 0);

    let zero_order = OrderId::new("zero-admission").unwrap();
    let zero = accepted_event(701, 0, &zero_order, &market, account, quantity("0"));
    assert_reducer_failure(
        ledger.apply_block(&block(701, vec![zero])).unwrap_err(),
        "order_state.invalid_quantity",
    );
}

#[test]
fn filled_remaining_admission_covers_every_constructible_order_lifecycle() {
    fn lifecycle_wire(lifecycle: OrderLifecycleV1) -> &'static str {
        match lifecycle {
            OrderLifecycleV1::Accepted => "accepted",
            OrderLifecycleV1::Rested => "rested",
            OrderLifecycleV1::Modified => "modified",
            OrderLifecycleV1::PartiallyFilled => "partially_filled",
            OrderLifecycleV1::Filled => "filled",
            OrderLifecycleV1::Cancelled => "cancelled",
        }
    }

    fn current_bytes(
        lifecycle: OrderLifecycleV1,
        accepted: Quantity,
        filled: Quantity,
        remaining: Quantity,
    ) -> Vec<u8> {
        format!(
            concat!(
                r#"{{"schema":"hyperliquid-alpha-desk/order-current/v1","#,
                r#""order_id":"order-lifecycle-remaining","#,
                r#""account_id":"{account}","#,
                r#""market_id":"perp:BTC","#,
                r#""side":"buy","#,
                r#""lifecycle":"{lifecycle}","#,
                r#""limit_price":"{limit_price}","#,
                r#""accepted_quantity":"{accepted}","#,
                r#""filled_quantity":"{filled}","#,
                r#""remaining_quantity":"{remaining}","#,
                r#""accepted_event_id":"accepted-event","#,
                r#""last_event_id":"last-event","#,
                r#""last_block_height":100}}"#
            ),
            account = Address::from_bytes(ACCOUNT_BYTES).to_api_string(),
            lifecycle = lifecycle_wire(lifecycle),
            limit_price = price("65000"),
            accepted = accepted,
            filled = filled,
            remaining = remaining,
        )
        .into_bytes()
    }

    fn pin(lifecycle: OrderLifecycleV1) {
        match lifecycle {
            OrderLifecycleV1::Filled => {
                let rejected = OrderCurrentRecordV1::decode(&current_bytes(
                    lifecycle,
                    quantity("2"),
                    quantity("1"),
                    quantity("1"),
                ));
                assert!(
                    matches!(rejected, Err(OrderStateError::InvalidRecord)),
                    "filled still fail-closes nonzero remaining: {rejected:?}"
                );
                let admitted = OrderCurrentRecordV1::decode(&current_bytes(
                    lifecycle,
                    quantity("1"),
                    quantity("1"),
                    quantity("0"),
                ))
                .expect("filled still admits zero remaining");
                assert_eq!(admitted.lifecycle(), OrderLifecycleV1::Filled);
                assert_eq!(admitted.remaining_quantity().raw(), 0);
            }
            OrderLifecycleV1::Accepted
            | OrderLifecycleV1::Rested
            | OrderLifecycleV1::Modified
            | OrderLifecycleV1::PartiallyFilled
            | OrderLifecycleV1::Cancelled => {
                let admitted = OrderCurrentRecordV1::decode(&current_bytes(
                    lifecycle,
                    quantity("2"),
                    quantity("1"),
                    quantity("1"),
                ))
                .unwrap_or_else(|error| {
                    panic!("{lifecycle:?} still skips the filled remaining-zero gate: {error:?}")
                });
                assert_eq!(admitted.lifecycle(), lifecycle);
                assert_ne!(admitted.remaining_quantity().raw(), 0);
            }
        }
    }

    pin(OrderLifecycleV1::Accepted);
    pin(OrderLifecycleV1::Rested);
    pin(OrderLifecycleV1::Modified);
    pin(OrderLifecycleV1::PartiallyFilled);
    pin(OrderLifecycleV1::Filled);
    pin(OrderLifecycleV1::Cancelled);
}

fn assert_reducer_failure(error: canonical_ledger::LedgerError, expected: &str) {
    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(error.reducer_reason_code(), Some(expected));
}

fn namespace_count(ledger: &CanonicalLedger<CanonicalOrderReducerV1>, namespace: &str) -> usize {
    ledger
        .state_image()
        .entries()
        .keys()
        .filter(|key| key.namespace() == namespace)
        .count()
}

fn ledger(first_height: u64) -> CanonicalLedger<CanonicalOrderReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalOrderReducerV1,
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

fn accepted_event(
    height: u64,
    event_index: u32,
    order_id: &OrderId,
    market: &MarketId,
    account: Address,
    quantity: Quantity,
) -> CanonicalEventEnvelope {
    order_event(
        height,
        event_index,
        EventPayload::OrderAccepted(OrderAccepted {
            order_id: order_id.clone(),
            account_id: account,
            market_id: market.clone(),
            side: OrderSide::Buy,
            limit_price: price("65000"),
            quantity,
        }),
        vec![market.clone()],
        vec![account],
        "1.0.0",
    )
}

fn order_event(
    height: u64,
    event_index: u32,
    payload: EventPayload,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    schema: &str,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: schema.to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}")).unwrap(),
        transaction_index: 0,
        canonical_event_index: event_index,
        market_ids,
        account_ids,
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

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 6).unwrap()
}

fn quantity(value: &str) -> Quantity {
    Quantity::parse_at_scale(value, 8).unwrap()
}
