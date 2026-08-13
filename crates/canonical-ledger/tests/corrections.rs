use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    CanonicalLedger, ConfirmationAdmission, CorrectionRecord, LedgerLimits, WatermarkOnlyReducerV1,
    admit_confirmation, apply_correction, inspect_correction,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};

#[test]
fn confirmation_admission_is_exhaustive_and_corrections_are_unimplemented() {
    assert_eq!(
        admit_confirmation(ConfirmationClass::CommittedPrimary),
        ConfirmationAdmission::Committed
    );
    assert_eq!(
        admit_confirmation(ConfirmationClass::CommittedIndependent),
        ConfirmationAdmission::Committed
    );
    assert_eq!(
        admit_confirmation(ConfirmationClass::Corrected),
        ConfirmationAdmission::CorrectionUnimplemented
    );
    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Expired,
    ] {
        assert_eq!(
            admit_confirmation(class),
            ConfirmationAdmission::NonCommitted
        );
    }
}

#[test]
fn corrected_blocks_are_refused_without_mutating_state_and_redelivery_is_idempotent() {
    let mut ledger = ledger(10);
    let before = ledger.state_image().canonical_bytes();
    let corrected = classified_block(10, ConfirmationClass::Corrected);

    let first = ledger
        .apply_block(&corrected)
        .expect_err("correction must be denied");
    assert_eq!(first.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());

    let second = ledger
        .apply_block(&corrected)
        .expect_err("redelivered correction must stay denied");
    assert_eq!(second.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn correction_records_fail_closed_on_apply_and_inspect_without_account_actions() {
    let ledger = ledger(11);
    let before = ledger.state_image().canonical_bytes();
    let corrected = classified_block(11, ConfirmationClass::Corrected);
    let record = CorrectionRecord::try_from_block(&corrected).expect("typed correction");

    assert!(
        CorrectionRecord::try_from_block(&classified_block(
            11,
            ConfirmationClass::CommittedPrimary
        ))
        .is_none()
    );
    assert!(
        CorrectionRecord::try_from_block(&classified_block(
            11,
            ConfirmationClass::ProvisionalSource
        ))
        .is_none()
    );

    let applied = apply_correction(&ledger, &record).expect_err("correction apply denied");
    assert_eq!(applied.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());

    let inspection = inspect_correction(&record);
    assert!(!inspection.admitted());
    assert!(!inspection.applied());
    assert_eq!(inspection.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn corrected_blocks_with_trade_shaped_payloads_still_do_not_mutate_state() {
    let mut ledger = ledger(12);
    let before = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&trade_shaped_correction(12))
        .expect_err("payload-bearing correction must be denied");
    assert_eq!(error.reason_code(), "ledger.correction_unimplemented");
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn non_committed_non_correction_classes_stay_fail_closed() {
    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Expired,
    ] {
        let mut ledger = ledger(20);
        let before = ledger.state_image().canonical_bytes();
        let error = ledger
            .apply_block(&classified_block(20, class))
            .expect_err("non-committed block");
        assert_eq!(error.reason_code(), "ledger.non_committed_block");
        assert_eq!(ledger.state_image().canonical_bytes(), before);
    }
}

fn ledger(height: u64) -> CanonicalLedger<WatermarkOnlyReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("ledger")
}

fn classified_block(height: u64, confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        confirmation,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("correction-test").expect("source"),
            [height as u8; 32],
        )]),
    )
    .expect("block")
}

fn trade_shaped_correction(height: u64) -> BlockEnvelope {
    let block_time = ProtocolTime::from_unix_micros(height as i64).expect("time");
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).expect("price"),
        Quantity::parse_at_scale("0.01", 8).expect("quantity"),
        1,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec().expect("payload bytes")).as_bytes();
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
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
                SourceId::new("correction-test").expect("source"),
                "v1",
                format!("{height}:0"),
                payload_hash,
                0,
            )
            .expect("evidence"),
        ],
        confirmation_class: ConfirmationClass::Corrected,
        observed_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        ingested_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .expect("event");
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        block_time,
        ConfirmationClass::Corrected,
        vec![event],
        BTreeMap::from([(
            SourceId::new("correction-test").expect("source"),
            [height as u8; 32],
        )]),
    )
    .expect("corrected trade-shaped block")
}
