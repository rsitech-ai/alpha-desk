use std::collections::BTreeMap;
use std::str::FromStr;

use api_contracts::{
    WireVaultCreated, WireVaultDistribution, encode_vault_created, encode_vault_distribution,
};
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, SourceEvidence,
};
use canonical_ledger::{
    AccountQuoteFlowCurrentRecordV1, AccountQuoteFlowScopeV1, AccountVaultRelationCurrentRecordV1,
    CanonicalLedger, CanonicalStateReducerV1, LedgerLimits, StateImage, StateImageLimits,
    VaultCurrentRecordV1, VaultPrincipalFlowCurrentRecordV1,
};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ProtocolTime, QuoteAmount, SourceId, TransactionId,
    VaultId,
};

const ACCOUNT_A: Address = Address::from_bytes([0x11; 20]);
const ACCOUNT_B: Address = Address::from_bytes([0x22; 20]);

#[test]
fn vault_create_and_distribution_update_principal_and_optional_user_quote() {
    let vault = VaultId::new("vault-a").unwrap();
    let mut ledger = ledger(10);
    ledger
        .apply_block(&block(
            10,
            vec![vault_created(
                10,
                0,
                "vault-a",
                "1.00",
                "0.10",
                vec![ACCOUNT_A],
            )],
        ))
        .unwrap();

    let current = vault_current(&ledger, &vault);
    assert_eq!(current.creation_amount(), quote("1.00"));
    assert_eq!(current.creation_fee(), quote("0.10"));
    assert_eq!(vault_principal(&ledger, &vault).deposits(), quote("1.00"));
    assert_eq!(
        user_vault_quote(&ledger, ACCOUNT_A, &vault).debits(),
        quote("1.10")
    );
    let relation_key = AccountVaultRelationCurrentRecordV1::state_key(&ACCOUNT_A, &vault).unwrap();
    assert!(ledger.state_image().entries().contains_key(&relation_key));

    ledger
        .apply_block(&block(
            11,
            vec![vault_distribution(
                11,
                0,
                "vault-a",
                "0.40",
                vec![ACCOUNT_A],
            )],
        ))
        .unwrap();
    assert_eq!(
        vault_principal(&ledger, &vault).withdrawals(),
        quote("0.40")
    );
    assert_eq!(
        user_vault_quote(&ledger, ACCOUNT_A, &vault).credits(),
        quote("0.40")
    );

    let restored = CanonicalLedger::try_from_state_image(
        StateImage::decode_canonical(
            &ledger.state_image().canonical_bytes(),
            StateImageLimits::production(),
        )
        .unwrap(),
        CanonicalStateReducerV1::try_new().unwrap(),
        LedgerLimits::production(),
    )
    .unwrap();
    assert_eq!(
        restored.state_image().canonical_bytes(),
        ledger.state_image().canonical_bytes()
    );
    assert_eq!(
        vault_current(&restored, &vault).creation_amount(),
        quote("1.00")
    );
}

#[test]
fn vault_create_without_users_is_vault_only_and_collisions_fail_closed() {
    let vault = VaultId::new("vault-b").unwrap();
    let mut ledger = ledger(20);
    ledger
        .apply_block(&block(
            20,
            vec![vault_created(20, 0, "vault-b", "2.00", "0.00", Vec::new())],
        ))
        .unwrap();
    assert_eq!(
        vault_current(&ledger, &vault).creation_amount(),
        quote("2.00")
    );
    assert_eq!(vault_principal(&ledger, &vault).deposits(), quote("2.00"));
    let key = AccountQuoteFlowCurrentRecordV1::state_key(
        &ACCOUNT_A,
        &AccountQuoteFlowScopeV1::VaultPrincipal {
            vault_id: vault.clone(),
        },
    )
    .unwrap();
    assert!(!ledger.state_image().entries().contains_key(&key));

    let before = ledger.state_image().canonical_bytes();
    let error = ledger
        .apply_block(&block(
            21,
            vec![vault_created(21, 0, "vault-b", "3.00", "0.00", Vec::new())],
        ))
        .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.reducer_failed");
    assert_eq!(
        error.reducer_reason_code(),
        Some("vault_state.vault_id_collision")
    );
    assert_eq!(ledger.state_image().canonical_bytes(), before);
}

#[test]
fn vault_create_rejects_empty_amount() {
    let mut ledger = ledger(30);
    let empty = vault_created(30, 0, "vault-c", "", "0.00", vec![ACCOUNT_A]);
    let error = ledger.apply_block(&block(30, vec![empty])).unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("canonical_state.empty_amount")
    );
}

#[test]
fn vault_create_rejects_ambiguous_accounts() {
    let mut ledger = ledger(31);
    let ambiguous = vault_created(31, 0, "vault-d", "1.00", "0.00", vec![ACCOUNT_A, ACCOUNT_B]);
    let error = ledger.apply_block(&block(31, vec![ambiguous])).unwrap_err();
    assert_eq!(
        error.reducer_reason_code(),
        Some("vault_state.ambiguous_accounts")
    );
}

#[test]
fn vault_leader_commission_paid_stays_catalog_only() {
    let payload = EventPayload::fixtures()
        .unwrap()
        .into_iter()
        .find(|payload| payload.kind() == EventKind::VaultLeaderCommissionPaid)
        .unwrap();
    let mut ledger = ledger(40);
    let error = ledger
        .apply_block(&block(40, vec![raw_event(40, 0, payload, Vec::new())]))
        .unwrap_err();
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
}

fn vault_current(
    ledger: &CanonicalLedger<CanonicalStateReducerV1>,
    vault_id: &VaultId,
) -> VaultCurrentRecordV1 {
    let key = VaultCurrentRecordV1::state_key(vault_id).unwrap();
    VaultCurrentRecordV1::decode_at(&key, ledger.state_image().entries().get(&key).unwrap())
        .unwrap()
}

fn vault_principal(
    ledger: &CanonicalLedger<CanonicalStateReducerV1>,
    vault_id: &VaultId,
) -> VaultPrincipalFlowCurrentRecordV1 {
    let key = VaultPrincipalFlowCurrentRecordV1::state_key(vault_id).unwrap();
    VaultPrincipalFlowCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn user_vault_quote(
    ledger: &CanonicalLedger<CanonicalStateReducerV1>,
    account: Address,
    vault_id: &VaultId,
) -> AccountQuoteFlowCurrentRecordV1 {
    let key = AccountQuoteFlowCurrentRecordV1::state_key(
        &account,
        &AccountQuoteFlowScopeV1::VaultPrincipal {
            vault_id: vault_id.clone(),
        },
    )
    .unwrap();
    AccountQuoteFlowCurrentRecordV1::decode_at(
        &key,
        ledger.state_image().entries().get(&key).unwrap(),
    )
    .unwrap()
}

fn vault_created(
    height: u64,
    index: u32,
    vault_id: &str,
    amount: &str,
    fee: &str,
    accounts: Vec<Address>,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        index,
        EventPayload::decode(
            EventKind::VaultCreated,
            &encode_vault_created(&WireVaultCreated {
                vault_id: vault_id.to_owned(),
                amount: amount.to_owned(),
                fee: fee.to_owned(),
            })
            .unwrap(),
        )
        .unwrap(),
        accounts,
    )
}

fn vault_distribution(
    height: u64,
    index: u32,
    vault_id: &str,
    amount: &str,
    accounts: Vec<Address>,
) -> CanonicalEventEnvelope {
    raw_event(
        height,
        index,
        EventPayload::decode(
            EventKind::VaultDistribution,
            &encode_vault_distribution(&WireVaultDistribution {
                vault_id: vault_id.to_owned(),
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

fn quote(value: &str) -> QuoteAmount {
    QuoteAmount::from_str(value).unwrap()
}
