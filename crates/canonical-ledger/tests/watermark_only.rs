use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    ApplyOutcome, CanonicalLedger, EventReducer, LedgerLimits, WatermarkOnlyReducerV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};

#[test]
fn production_baseline_advances_only_empty_committed_blocks() {
    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(100),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("ledger");

    for (height, confirmation) in [
        (100, ConfirmationClass::CommittedPrimary),
        (101, ConfirmationClass::CommittedIndependent),
    ] {
        let outcome = ledger
            .apply_block(&empty_block(height, confirmation))
            .expect("empty committed block");
        assert!(matches!(outcome, ApplyOutcome::Applied(_)));
    }

    assert_eq!(
        ledger.checkpoint().expect("checkpoint").block_height(),
        BlockHeight::new(101)
    );
    assert!(ledger.state_image().entries().is_empty());
    assert_eq!(
        ledger.state_image().reducer_set_version(),
        WatermarkOnlyReducerV1::VERSION
    );
}

#[test]
fn production_baseline_quarantines_action_bearing_blocks_without_state_effects() {
    let reducer = WatermarkOnlyReducerV1;
    let event = trade_event(200);
    assert!(!reducer.supports(&event));

    let mut ledger = CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(200),
        reducer,
        LedgerLimits::production(),
    )
    .expect("ledger");
    let before = ledger.state_image().canonical_bytes();
    let block = BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(200),
        event.block_time(),
        ConfirmationClass::CommittedPrimary,
        vec![event],
        source_hashes(200),
    )
    .expect("block");

    let error = ledger
        .apply_block(&block)
        .expect_err("unqualified trade semantics");

    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

fn empty_block(height: u64, confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        confirmation,
        Vec::new(),
        source_hashes(height),
    )
    .expect("empty block")
}

fn trade_event(height: u64) -> CanonicalEventEnvelope {
    let block_time = ProtocolTime::from_unix_micros(height as i64).expect("time");
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).expect("price"),
        Quantity::parse_at_scale("0.01", 8).expect("quantity"),
        1,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec().expect("payload bytes")).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC").expect("market")],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![
            SourceEvidence::try_new_indexed(
                SourceId::new("test-primary").expect("source"),
                "v1",
                height.to_string(),
                payload_hash,
                0,
            )
            .expect("evidence"),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        ingested_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .expect("event")
}

fn source_hashes(height: u64) -> BTreeMap<SourceId, [u8; 32]> {
    BTreeMap::from([(
        SourceId::new("test-primary").expect("source"),
        *blake3::hash(&height.to_be_bytes()).as_bytes(),
    )])
}
