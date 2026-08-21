use std::collections::BTreeMap;
use std::str::FromStr;

use api_contracts::{WireValidatorRewardPaid, encode_validator_reward_paid};
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, SourceEvidence,
};
use canonical_ledger::{
    CanonicalLedger, CanonicalStateReducerV1, LedgerLimits, ValidatorRewardCurrentRecordV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ProtocolTime, Quantity, SourceId, TransactionId,
};

const ACCOUNT: Address = Address::from_bytes([0x11; 20]);
const VALIDATOR: &str = "hl-validator-1";

#[test]
fn validator_rewards_key_by_validator_string() {
    let mut ledger = ledger(80);
    ledger
        .apply_block(&block(80, vec![reward(80, 0, "1.0", Vec::new())]))
        .unwrap();
    let current = current(&ledger);
    assert_eq!(current.validator(), VALIDATOR);
    assert_eq!(current.credits(), qty("1.0"));
    assert!(
        ledger
            .state_image()
            .entries()
            .keys()
            .all(|key| key.namespace() != "account-quote-flow-current.v1"
                && key.namespace() != "account-quantity-flow-current.v1"
                && key.namespace() != "account-fact.v1")
    );
}

#[test]
fn validator_rewards_refuse_account_inference() {
    let mut ledger = ledger(81);
    let error = ledger
        .apply_block(&block(81, vec![reward(81, 0, "1.0", vec![ACCOUNT])]))
        .unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("validator_state.account_inferred")
    );
}

fn current(ledger: &CanonicalLedger<CanonicalStateReducerV1>) -> ValidatorRewardCurrentRecordV1 {
    let key = ValidatorRewardCurrentRecordV1::state_key(VALIDATOR).unwrap();
    ValidatorRewardCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn reward(height: u64, index: u32, amount: &str, accounts: Vec<Address>) -> CanonicalEventEnvelope {
    raw_event(
        height,
        index,
        EventPayload::decode(
            EventKind::ValidatorRewardPaid,
            &encode_validator_reward_paid(&WireValidatorRewardPaid {
                validator: VALIDATOR.to_owned(),
                amount: amount.to_owned(),
            })
            .unwrap(),
        )
        .unwrap(),
        accounts,
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
