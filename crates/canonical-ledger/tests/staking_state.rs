use std::collections::BTreeMap;
use std::str::FromStr;

use api_contracts::{
    WireStakingDelegated, WireStakingDeposit, WireStakingUndelegated,
    WireStakingWithdrawalCompleted, WireStakingWithdrawalQueued, encode_staking_delegated,
    encode_staking_deposit, encode_staking_undelegated, encode_staking_withdrawal_completed,
    encode_staking_withdrawal_queued,
};
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, SourceEvidence,
};
use canonical_ledger::{
    CanonicalLedger, CanonicalStateReducerV1, LedgerLimits, StakingDelegationCurrentRecordV1,
    StakingDelegationRelationCurrentRecordV1, StakingLiquidCurrentRecordV1,
    StakingPendingCurrentRecordV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ProtocolTime, Quantity, SourceId, TransactionId,
};

const ACCOUNT: Address = Address::from_bytes([0x11; 20]);
const VALIDATOR: &str = "0x5555555555555555555555555555555555555555";

#[test]
fn staking_deposit_delegate_undelegate_and_withdrawal_queue_complete() {
    let mut ledger = ledger(50);
    ledger
        .apply_block(&block(50, vec![deposit(50, 0, "2.0")]))
        .unwrap();
    assert_eq!(liquid(&ledger).credits(), qty("2.0"));

    ledger
        .apply_block(&block(51, vec![delegate(51, 0, "1.0")]))
        .unwrap();
    assert_eq!(liquid(&ledger).debits(), qty("1.0"));
    assert_eq!(delegation(&ledger).credits(), qty("1.0"));
    let relation_key =
        StakingDelegationRelationCurrentRecordV1::state_key(&ACCOUNT, VALIDATOR).unwrap();
    assert!(ledger.state_image().entries().contains_key(&relation_key));

    ledger
        .apply_block(&block(52, vec![undelegate(52, 0, "1.0")]))
        .unwrap();
    assert_eq!(delegation(&ledger).debits(), qty("1.0"));

    ledger
        .apply_block(&block(53, vec![queue(53, 0, "1.0")]))
        .unwrap();
    assert_eq!(pending(&ledger).credits(), qty("1.0"));
    ledger
        .apply_block(&block(54, vec![complete(54, 0, "1.0")]))
        .unwrap();
    assert_eq!(pending(&ledger).debits(), qty("1.0"));
}

#[test]
fn staking_fails_closed_on_insufficient_liquid() {
    let mut ledger = ledger(60);
    ledger
        .apply_block(&block(60, vec![deposit(60, 0, "1.0")]))
        .unwrap();
    let before = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&block(61, vec![queue(61, 0, "2.0")]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("staking_state.insufficient_liquid")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn staking_fails_closed_on_missing_delegation() {
    let mut ledger = ledger(70);
    let error = ledger
        .apply_block(&block(70, vec![undelegate(70, 0, "1.0")]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("staking_state.missing_delegation")
    );
}

fn liquid(ledger: &CanonicalLedger<CanonicalStateReducerV1>) -> StakingLiquidCurrentRecordV1 {
    let key = StakingLiquidCurrentRecordV1::state_key(&ACCOUNT).unwrap();
    StakingLiquidCurrentRecordV1::decode_at(&key, ledger.state_image().entries().get(&key).unwrap())
        .unwrap()
}

fn pending(ledger: &CanonicalLedger<CanonicalStateReducerV1>) -> StakingPendingCurrentRecordV1 {
    let key = StakingPendingCurrentRecordV1::state_key(&ACCOUNT).unwrap();
    StakingPendingCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn delegation(
    ledger: &CanonicalLedger<CanonicalStateReducerV1>,
) -> StakingDelegationCurrentRecordV1 {
    let key = StakingDelegationCurrentRecordV1::state_key(&ACCOUNT, VALIDATOR).unwrap();
    StakingDelegationCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn deposit(height: u64, index: u32, amount: &str) -> CanonicalEventEnvelope {
    staking_event(
        height,
        index,
        EventKind::StakingDeposit,
        encode_staking_deposit(&WireStakingDeposit {
            account_id: ACCOUNT.to_api_string(),
            amount: amount.to_owned(),
        })
        .unwrap(),
    )
}

fn delegate(height: u64, index: u32, amount: &str) -> CanonicalEventEnvelope {
    staking_event(
        height,
        index,
        EventKind::StakingDelegated,
        encode_staking_delegated(&WireStakingDelegated {
            account_id: ACCOUNT.to_api_string(),
            validator: VALIDATOR.to_owned(),
            amount: amount.to_owned(),
        })
        .unwrap(),
    )
}

fn undelegate(height: u64, index: u32, amount: &str) -> CanonicalEventEnvelope {
    staking_event(
        height,
        index,
        EventKind::StakingUndelegated,
        encode_staking_undelegated(&WireStakingUndelegated {
            account_id: ACCOUNT.to_api_string(),
            validator: VALIDATOR.to_owned(),
            amount: amount.to_owned(),
        })
        .unwrap(),
    )
}

fn queue(height: u64, index: u32, amount: &str) -> CanonicalEventEnvelope {
    staking_event(
        height,
        index,
        EventKind::StakingWithdrawalQueued,
        encode_staking_withdrawal_queued(&WireStakingWithdrawalQueued {
            account_id: ACCOUNT.to_api_string(),
            amount: amount.to_owned(),
        })
        .unwrap(),
    )
}

fn complete(height: u64, index: u32, amount: &str) -> CanonicalEventEnvelope {
    staking_event(
        height,
        index,
        EventKind::StakingWithdrawalCompleted,
        encode_staking_withdrawal_completed(&WireStakingWithdrawalCompleted {
            account_id: ACCOUNT.to_api_string(),
            amount: amount.to_owned(),
        })
        .unwrap(),
    )
}

fn staking_event(
    height: u64,
    event_index: u32,
    kind: EventKind,
    bytes: Vec<u8>,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        event_index,
        EventPayload::decode(kind, &bytes).unwrap(),
        vec![ACCOUNT],
    )
}

fn ledger(first_height: u64) -> CanonicalLedger<CanonicalStateReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").unwrap(),
        BlockHeight::new(first_height),
        CanonicalStateReducerV1::try_new().unwrap(),
        LedgerLimits::production(),
    )
    .unwrap()
}

fn raw_event(
    height: u64,
    event_index: u32,
    payload: EventPayload,
    accounts: Vec<Address>,
) -> CanonicalEventEnvelope {
    let payload_hash = *blake3::hash(&payload.encode_to_vec().unwrap()).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(height),
        block_time: ProtocolTime::from_unix_micros(height as i64).unwrap(),
        transaction_id: TransactionId::new(format!("tx-{height}-{event_index}")).unwrap(),
        transaction_index: event_index,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: accounts,
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

fn qty(value: &str) -> Quantity {
    Quantity::from_str(value).unwrap()
}
