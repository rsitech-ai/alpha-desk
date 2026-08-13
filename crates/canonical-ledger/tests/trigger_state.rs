use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TriggerOrderActivated,
};
use canonical_ledger::{
    ApplyOutcome, CanonicalLedger, CanonicalTriggerReducerV1, LedgerLimits, TriggerCurrentRecordV1,
    TriggerFactRecordV1, TriggerTransitionRecordV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, OrderId, Price, ProtocolTime, SourceId,
    TransactionId,
};

const ACCOUNT_BYTES: [u8; 20] = [0x11; 20];

#[test]
fn activation_creates_immutable_fact_current_state_and_hash_linked_transition() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("trigger-1").unwrap();
    let event = trigger_event(100, 0, &order_id, &market, account, "1.0.0");
    let mut ledger = ledger(100);

    let ApplyOutcome::Applied(delta) = ledger
        .apply_block(&block(100, vec![event.clone()]))
        .unwrap()
    else {
        panic!("new block must apply");
    };
    assert_eq!(delta.mutations().len(), 3);
    assert_eq!(namespace_count(&ledger, "trigger-fact.v1"), 1);
    assert_eq!(namespace_count(&ledger, "trigger-current.v1"), 1);
    assert_eq!(namespace_count(&ledger, "trigger-transition.v1"), 1);
    assert_eq!(namespace_count(&ledger, "order-current.v1"), 0);
    assert_eq!(namespace_count(&ledger, "position-quantity-current.v1"), 0);

    let current_key = TriggerCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let current = TriggerCurrentRecordV1::decode_at(
        &current_key,
        ledger.state_image().entries().get(&current_key).unwrap(),
    )
    .unwrap();
    assert_eq!(current.order_id(), &order_id);
    assert_eq!(current.account_id(), account);
    assert_eq!(current.market_id(), &market);
    assert_eq!(current.trigger_price(), price("64000"));
    assert_eq!(current.oracle_price(), price("63990"));
    assert_eq!(current.last_event_id(), event.event_id());

    let fact_key =
        TriggerFactRecordV1::state_key_for(&market, &order_id, event.event_id()).unwrap();
    let fact = TriggerFactRecordV1::decode_at(
        &fact_key,
        ledger.state_image().entries().get(&fact_key).unwrap(),
    )
    .unwrap();
    assert_eq!(fact.event_id(), event.event_id());
    assert_eq!(fact.payload_hash(), event.payload_hash());
    assert_eq!(fact.account_id(), account);

    let transition_key =
        TriggerTransitionRecordV1::state_key_for(&market, &order_id, event.event_id()).unwrap();
    let transition = TriggerTransitionRecordV1::decode_at(
        &transition_key,
        ledger.state_image().entries().get(&transition_key).unwrap(),
    )
    .unwrap();
    assert!(transition.prior_state_hash().is_none());
    assert_eq!(
        transition.rule_version(),
        "hyperliquid-alpha-desk-canonical-trigger@1.0.0"
    );
}

#[test]
fn duplicate_activation_and_identity_mismatch_fail_closed() {
    let account = Address::from_bytes(ACCOUNT_BYTES);
    let other = Address::from_bytes([0x22; 20]);
    let market = MarketId::new("perp:ETH").unwrap();
    let order_id = OrderId::new("trigger-dup").unwrap();
    let mut ledger = ledger(200);
    ledger
        .apply_block(&block(
            200,
            vec![trigger_event(200, 0, &order_id, &market, account, "1.0.0")],
        ))
        .unwrap();

    assert_reducer_failure(
        ledger
            .apply_block(&block(
                201,
                vec![trigger_event(201, 0, &order_id, &market, account, "1.0.0")],
            ))
            .unwrap_err(),
        "trigger_state.order_id_collision",
    );

    let mismatched = trigger_event_with_accounts(
        201,
        0,
        &OrderId::new("trigger-mismatch").unwrap(),
        &market,
        vec![market.clone()],
        vec![account, other],
        "1.0.0",
    );
    assert_reducer_failure(
        ledger
            .apply_block(&block(201, vec![mismatched]))
            .unwrap_err(),
        "trigger_state.identity_mismatch",
    );
}

#[test]
fn reducer_owns_only_exact_schema_and_records_are_canonical_and_key_bound() {
    use canonical_ledger::EventReducer;

    let account = Address::from_bytes(ACCOUNT_BYTES);
    let market = MarketId::new("perp:BTC").unwrap();
    let order_id = OrderId::new("trigger-codec").unwrap();
    let supported = trigger_event(600, 0, &order_id, &market, account, "1.0.0");
    let unsupported = trigger_event(600, 0, &order_id, &market, account, "1.1.0");
    assert!(CanonicalTriggerReducerV1.supports(&supported));
    assert!(!CanonicalTriggerReducerV1.supports(&unsupported));

    let mut ledger = ledger(600);
    let error = ledger
        .apply_block(&block(600, vec![unsupported]))
        .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.unsupported_event");

    ledger
        .apply_block(&block(600, vec![supported.clone()]))
        .unwrap();
    let key = TriggerCurrentRecordV1::state_key(&market, &order_id).unwrap();
    let encoded = ledger.state_image().entries().get(&key).unwrap();
    for corrupt in [
        encoded[..encoded.len() - 1].to_vec(),
        [encoded.as_slice(), b" "].concat(),
        b"{}".to_vec(),
    ] {
        assert!(TriggerCurrentRecordV1::decode(&corrupt).is_err());
    }
    let wrong_key =
        TriggerCurrentRecordV1::state_key(&market, &OrderId::new("other").unwrap()).unwrap();
    assert!(TriggerCurrentRecordV1::decode_at(&wrong_key, encoded).is_err());

    let oversized = OrderId::new("x".repeat(70_000)).unwrap();
    assert!(TriggerCurrentRecordV1::state_key(&market, &oversized).is_err());
}

fn assert_reducer_failure(error: canonical_ledger::LedgerError, expected: &str) {
    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(error.reducer_reason_code(), Some(expected));
}

fn namespace_count(ledger: &CanonicalLedger<CanonicalTriggerReducerV1>, namespace: &str) -> usize {
    ledger
        .state_image()
        .entries()
        .keys()
        .filter(|key| key.namespace() == namespace)
        .count()
}

fn ledger(first_height: u64) -> CanonicalLedger<CanonicalTriggerReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalTriggerReducerV1,
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

fn trigger_event(
    height: u64,
    event_index: u32,
    order_id: &OrderId,
    market: &MarketId,
    account: Address,
    schema: &str,
) -> CanonicalEventEnvelope {
    trigger_event_with_accounts(
        height,
        event_index,
        order_id,
        market,
        vec![market.clone()],
        vec![account],
        schema,
    )
}

fn trigger_event_with_accounts(
    height: u64,
    event_index: u32,
    order_id: &OrderId,
    _market: &MarketId,
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    schema: &str,
) -> CanonicalEventEnvelope {
    let payload = EventPayload::TriggerOrderActivated(TriggerOrderActivated {
        order_id: order_id.clone(),
        trigger_price: price("64000"),
        oracle_price: price("63990"),
    });
    envelope(
        height,
        event_index,
        payload,
        market_ids,
        account_ids,
        schema,
    )
}

fn envelope(
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
