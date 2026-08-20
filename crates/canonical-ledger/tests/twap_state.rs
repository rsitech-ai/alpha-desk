use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TwapCompleted, TwapSliceFilled, TwapStarted,
};
use canonical_ledger::{
    ApplyOutcome, CanonicalLedger, CanonicalTwapReducerV1, LedgerLimits, TwapCurrentRecordV1,
    TwapFactRecordV1, TwapLifecycleV1, TwapTransitionRecordV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, OrderId, Price, ProtocolTime, Quantity,
    SourceId, TransactionId,
};

const ACCOUNT_BYTES: [u8; 20] = [0x11; 20];

#[test]
fn start_slice_and_complete_keep_hash_linked_facts_without_inventing_side() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("twap-full").unwrap();
    let events = [
        started_event(100, 0, &order_id, &market, account, quantity("1")),
        slice_event(100, 1, &order_id, &market, account, 0, quantity("0.25")),
        slice_event(100, 2, &order_id, &market, account, 1, quantity("0.25")),
        completed_event(
            100,
            3,
            &order_id,
            &market,
            account,
            quantity("0.50"),
            price("65000"),
        ),
    ];
    let mut ledger = ledger(100);
    let ApplyOutcome::Applied(delta) = ledger.apply_block(&block(100, events.to_vec())).unwrap()
    else {
        panic!("new block must apply");
    };
    assert_eq!(delta.mutations().len(), 12);
    assert_eq!(namespace_count(&ledger, "twap-fact.v1"), 4);
    assert_eq!(namespace_count(&ledger, "twap-current.v1"), 1);
    assert_eq!(namespace_count(&ledger, "twap-transition.v1"), 4);
    assert_eq!(namespace_count(&ledger, "order-current.v1"), 0);
    assert_eq!(namespace_count(&ledger, "position-quantity-current.v1"), 0);

    let current_key = TwapCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let current = TwapCurrentRecordV1::decode_at(
        &current_key,
        ledger.state_image().entries().get(&current_key).unwrap(),
    )
    .unwrap();
    assert_eq!(current.lifecycle(), TwapLifecycleV1::Completed);
    assert_eq!(current.total_quantity(), quantity("1"));
    assert_eq!(current.filled_quantity(), quantity("0.50"));
    assert_eq!(current.remaining_quantity(), quantity("0.50"));
    assert_eq!(current.last_slice_index(), Some(1));
    assert_eq!(current.completed_average_price(), Some(price("65000")));
    assert_eq!(current.last_event_id(), events[3].event_id());

    let mut previous_result = None;
    for event in &events {
        let fact_key =
            TwapFactRecordV1::state_key_for(&market, &order_id, event.event_id()).unwrap();
        let fact = TwapFactRecordV1::decode_at(
            &fact_key,
            ledger.state_image().entries().get(&fact_key).unwrap(),
        )
        .unwrap();
        assert_eq!(fact.event_id(), event.event_id());
        assert_eq!(fact.payload_hash(), event.payload_hash());

        let transition_key =
            TwapTransitionRecordV1::state_key_for(&market, &order_id, event.event_id()).unwrap();
        let transition = TwapTransitionRecordV1::decode_at(
            &transition_key,
            ledger.state_image().entries().get(&transition_key).unwrap(),
        )
        .unwrap();
        assert_eq!(transition.prior_state_hash(), previous_result);
        assert_eq!(
            transition.rule_version(),
            "hyperliquid-alpha-desk-canonical-twap@1.0.0"
        );
        previous_result = Some(transition.result_state_hash());
    }
}

#[test]
fn overfill_nonincreasing_slice_and_filled_mismatch_fail_closed() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:ETH").unwrap();
    let order_id = OrderId::new("twap-fail").unwrap();
    let mut ledger = ledger(200);
    ledger
        .apply_block(&block(
            200,
            vec![started_event(
                200,
                0,
                &order_id,
                &market,
                account,
                quantity("1"),
            )],
        ))
        .unwrap();

    assert_reducer_failure(
        ledger
            .apply_block(&block(
                201,
                vec![slice_event(
                    201,
                    0,
                    &order_id,
                    &market,
                    account,
                    0,
                    quantity("2"),
                )],
            ))
            .unwrap_err(),
        "twap_state.overfill",
    );

    ledger
        .apply_block(&block(
            201,
            vec![slice_event(
                201,
                0,
                &order_id,
                &market,
                account,
                3,
                quantity("0.25"),
            )],
        ))
        .unwrap();
    assert_reducer_failure(
        ledger
            .apply_block(&block(
                202,
                vec![slice_event(
                    202,
                    0,
                    &order_id,
                    &market,
                    account,
                    3,
                    quantity("0.25"),
                )],
            ))
            .unwrap_err(),
        "twap_state.slice_index_not_increasing",
    );

    assert_reducer_failure(
        ledger
            .apply_block(&block(
                202,
                vec![completed_event(
                    202,
                    0,
                    &order_id,
                    &market,
                    account,
                    quantity("1"),
                    price("1"),
                )],
            ))
            .unwrap_err(),
        "twap_state.filled_mismatch",
    );
}

#[test]
fn exact_fill_does_not_auto_complete_and_zero_fill_completion_is_terminal() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let filled_id = OrderId::new("twap-exact").unwrap();
    let mut ledger = ledger(300);
    ledger
        .apply_block(&block(
            300,
            vec![
                started_event(300, 0, &filled_id, &market, account, quantity("1")),
                slice_event(300, 1, &filled_id, &market, account, 0, quantity("1")),
            ],
        ))
        .unwrap();
    let current_key = TwapCurrentRecordV1::state_key(&market, &filled_id).unwrap();
    let current = TwapCurrentRecordV1::decode_at(
        &current_key,
        ledger.state_image().entries().get(&current_key).unwrap(),
    )
    .unwrap();
    assert_eq!(current.lifecycle(), TwapLifecycleV1::Active);
    assert_eq!(current.remaining_quantity(), quantity("0"));

    let zero_id = OrderId::new("twap-zero").unwrap();
    ledger
        .apply_block(&block(
            301,
            vec![
                started_event(301, 0, &zero_id, &market, account, quantity("2")),
                completed_event(
                    301,
                    1,
                    &zero_id,
                    &market,
                    account,
                    quantity("0"),
                    price("0"),
                ),
            ],
        ))
        .unwrap();
    let zero_key = TwapCurrentRecordV1::state_key(&market, &zero_id).unwrap();
    let zero = TwapCurrentRecordV1::decode_at(
        &zero_key,
        ledger.state_image().entries().get(&zero_key).unwrap(),
    )
    .unwrap();
    assert_eq!(zero.lifecycle(), TwapLifecycleV1::Completed);
    assert_eq!(zero.filled_quantity(), quantity("0"));
    assert_eq!(zero.remaining_quantity(), quantity("2"));
    assert_eq!(zero.completed_average_price(), Some(price("0")));

    assert_reducer_failure(
        ledger
            .apply_block(&block(
                302,
                vec![slice_event(
                    302,
                    0,
                    &zero_id,
                    &market,
                    account,
                    0,
                    quantity("1"),
                )],
            ))
            .unwrap_err(),
        "twap_state.terminal_twap",
    );
}

#[test]
fn reducer_owns_only_exact_schema_and_records_are_canonical_and_key_bound() {
    use canonical_ledger::EventReducer;

    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("twap-codec").unwrap();
    let supported = started_event(600, 0, &order_id, &market, account, quantity("1"));
    let unsupported = twap_event(
        600,
        0,
        supported.payload().clone(),
        vec![market.clone()],
        vec![account],
        "1.1.0",
    );
    assert!(CanonicalTwapReducerV1.supports(&supported));
    assert!(!CanonicalTwapReducerV1.supports(&unsupported));

    let mut ledger = ledger(600);
    let error = ledger
        .apply_block(&block(600, vec![unsupported]))
        .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.unsupported_event");

    ledger
        .apply_block(&block(600, vec![supported.clone()]))
        .unwrap();
    let key = TwapCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let encoded = ledger.state_image().entries().get(&key).unwrap();
    for corrupt in [
        encoded[..encoded.len() - 1].to_vec(),
        [encoded.as_slice(), b" "].concat(),
        b"{}".to_vec(),
    ] {
        assert!(TwapCurrentRecordV1::decode(&corrupt).is_err());
    }
    let wrong_key =
        TwapCurrentRecordV1::state_key(&market, &OrderId::new("other").unwrap()).unwrap();
    assert!(TwapCurrentRecordV1::decode_at(&wrong_key, encoded).is_err());

    let oversized = OrderId::new("x".repeat(70_000)).unwrap();
    assert!(TwapCurrentRecordV1::state_key(&market, &oversized).is_err());
}

fn assert_reducer_failure(error: canonical_ledger::LedgerError, expected: &str) {
    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(error.reducer_reason_code(), Some(expected));
}

fn namespace_count(ledger: &CanonicalLedger<CanonicalTwapReducerV1>, namespace: &str) -> usize {
    ledger
        .state_image()
        .entries()
        .keys()
        .filter(|key| key.namespace() == namespace)
        .count()
}

fn ledger(first_height: u64) -> CanonicalLedger<CanonicalTwapReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalTwapReducerV1,
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

fn started_event(
    height: u64,
    event_index: u32,
    order_id: &OrderId,
    market: &MarketId,
    account: Address,
    total: Quantity,
) -> CanonicalEventEnvelope {
    twap_event(
        height,
        event_index,
        EventPayload::TwapStarted(TwapStarted {
            order_id: order_id.clone(),
            account_id: account,
            market_id: market.clone(),
            total_quantity: total,
            end_time: ProtocolTime::from_unix_micros(1_700_000_000_000_000).unwrap(),
        }),
        vec![market.clone()],
        vec![account],
        "1.0.0",
    )
}

fn slice_event(
    height: u64,
    event_index: u32,
    order_id: &OrderId,
    market: &MarketId,
    account: Address,
    slice_index: u32,
    fill_quantity: Quantity,
) -> CanonicalEventEnvelope {
    twap_event(
        height,
        event_index,
        EventPayload::TwapSliceFilled(TwapSliceFilled {
            order_id: order_id.clone(),
            slice_index,
            fill_price: price("65000"),
            fill_quantity,
        }),
        vec![market.clone()],
        vec![account],
        "1.0.0",
    )
}

fn completed_event(
    height: u64,
    event_index: u32,
    order_id: &OrderId,
    market: &MarketId,
    account: Address,
    filled_quantity: Quantity,
    average_price: Price,
) -> CanonicalEventEnvelope {
    twap_event(
        height,
        event_index,
        EventPayload::TwapCompleted(TwapCompleted {
            order_id: order_id.clone(),
            filled_quantity,
            average_price,
        }),
        vec![market.clone()],
        vec![account],
        "1.0.0",
    )
}

fn twap_event(
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
