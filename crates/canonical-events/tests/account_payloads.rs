use api_contracts::{
    WireCanonicalEventEnvelope, WireDepositCredited, WirePerpTransfer, WireSpotTransfer,
    WireSubaccountTransfer, WireVaultDeposit, WireVaultWithdrawal, WireWithdrawalDebited,
    encode_deposit_credited, encode_perp_transfer, encode_spot_transfer,
    encode_subaccount_transfer, encode_vault_deposit, encode_vault_withdrawal,
    encode_withdrawal_debited,
};
use canonical_events::{
    CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, DepositCredited, EventKind,
    EventPayload, PerpTransfer, SpotTransfer, SubaccountTransfer, VaultDeposit, VaultWithdrawal,
    WithdrawalDebited,
};
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, KnownTime, ProtocolTime, Quantity, QuoteAmount,
    SourceId, TransactionId, VaultId,
};

fn account(byte: u8) -> Address {
    Address::from_bytes([byte; 20])
}

fn append_varint_field(mut bytes: Vec<u8>, field_number: u32, value: u64) -> Vec<u8> {
    fn append_varint(bytes: &mut Vec<u8>, mut value: u64) {
        loop {
            let byte = u8::try_from(value & 0x7f).unwrap();
            value >>= 7;
            if value == 0 {
                bytes.push(byte);
                return;
            }
            bytes.push(byte | 0x80);
        }
    }

    append_varint(&mut bytes, u64::from(field_number) << 3);
    append_varint(&mut bytes, value);
    bytes
}

#[test]
fn all_cash_flow_payloads_decode_to_exact_domain_values_and_round_trip() {
    let from = account(0x11);
    let to = account(0x22);
    let master = account(0x33);
    let asset = AssetId::new("USDC").unwrap();
    let vault = VaultId::new("vault-alpha").unwrap();

    let cases = [
        (
            EventKind::DepositCredited,
            encode_deposit_credited(&WireDepositCredited {
                account_id: from.to_api_string(),
                asset_id: asset.to_string(),
                amount: "125.500000".to_owned(),
                deposit_reference: "deposit-42".to_owned(),
            })
            .unwrap(),
            EventPayload::DepositCredited(DepositCredited {
                account_id: from,
                asset_id: asset.clone(),
                amount: Quantity::parse_at_scale("125.5", 6).unwrap(),
                deposit_reference: "deposit-42".to_owned(),
            }),
        ),
        (
            EventKind::WithdrawalDebited,
            encode_withdrawal_debited(&WireWithdrawalDebited {
                account_id: from.to_api_string(),
                asset_id: asset.to_string(),
                amount: "25.250000".to_owned(),
                withdrawal_reference: "withdrawal-43".to_owned(),
            })
            .unwrap(),
            EventPayload::WithdrawalDebited(WithdrawalDebited {
                account_id: from,
                asset_id: asset.clone(),
                amount: Quantity::parse_at_scale("25.25", 6).unwrap(),
                withdrawal_reference: "withdrawal-43".to_owned(),
            }),
        ),
        (
            EventKind::SpotTransfer,
            encode_spot_transfer(&WireSpotTransfer {
                from_account_id: from.to_api_string(),
                to_account_id: to.to_api_string(),
                asset_id: asset.to_string(),
                amount: "10.125000".to_owned(),
            })
            .unwrap(),
            EventPayload::SpotTransfer(SpotTransfer {
                from_account_id: from,
                to_account_id: to,
                asset_id: asset.clone(),
                amount: Quantity::parse_at_scale("10.125", 6).unwrap(),
            }),
        ),
        (
            EventKind::PerpTransfer,
            encode_perp_transfer(&WirePerpTransfer {
                from_account_id: from.to_api_string(),
                to_account_id: to.to_api_string(),
                quote_amount: "2000.125000".to_owned(),
            })
            .unwrap(),
            EventPayload::PerpTransfer(PerpTransfer {
                from_account_id: from,
                to_account_id: to,
                quote_amount: QuoteAmount::parse_at_scale("2000.125", 6).unwrap(),
            }),
        ),
        (
            EventKind::SubaccountTransfer,
            encode_subaccount_transfer(&WireSubaccountTransfer {
                master_account_id: master.to_api_string(),
                from_account_id: from.to_api_string(),
                to_account_id: to.to_api_string(),
                asset_id: asset.to_string(),
                amount: "4.500000".to_owned(),
            })
            .unwrap(),
            EventPayload::SubaccountTransfer(SubaccountTransfer {
                master_account_id: master,
                from_account_id: from,
                to_account_id: to,
                asset_id: asset.clone(),
                amount: Quantity::parse_at_scale("4.5", 6).unwrap(),
            }),
        ),
        (
            EventKind::VaultDeposit,
            encode_vault_deposit(&WireVaultDeposit {
                vault_id: vault.to_string(),
                account_id: from.to_api_string(),
                amount: "1000.000000".to_owned(),
                shares_issued: "10.50000000".to_owned(),
            })
            .unwrap(),
            EventPayload::VaultDeposit(VaultDeposit {
                vault_id: vault.clone(),
                account_id: from,
                amount: QuoteAmount::parse_at_scale("1000", 6).unwrap(),
                shares_issued: Quantity::parse_at_scale("10.5", 8).unwrap(),
            }),
        ),
        (
            EventKind::VaultWithdrawal,
            encode_vault_withdrawal(&WireVaultWithdrawal {
                vault_id: vault.to_string(),
                account_id: from.to_api_string(),
                amount: "750.000000".to_owned(),
                shares_redeemed: "7.50000000".to_owned(),
            })
            .unwrap(),
            EventPayload::VaultWithdrawal(VaultWithdrawal {
                vault_id: vault,
                account_id: from,
                amount: QuoteAmount::parse_at_scale("750", 6).unwrap(),
                shares_redeemed: Quantity::parse_at_scale("7.5", 8).unwrap(),
            }),
        ),
    ];

    for (kind, bytes, expected) in cases {
        let decoded = EventPayload::decode(kind, &bytes).unwrap();
        assert_eq!(decoded, expected);
        assert_eq!(decoded.kind(), kind);
        assert_eq!(decoded.encode_to_vec().unwrap(), bytes);
    }
}

#[test]
fn cash_flow_payloads_reject_invalid_ids_endpoints_amounts_and_references() {
    let from = account(0x11).to_api_string();
    let to = account(0x22).to_api_string();

    for invalid_account in [
        "0X1111111111111111111111111111111111111111",
        "0x111111111111111111111111111111111111111A",
        " 0x1111111111111111111111111111111111111111",
        "0x1111111111111111111111111111111111111111 ",
    ] {
        assert!(
            encode_deposit_credited(&WireDepositCredited {
                account_id: invalid_account.to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                deposit_reference: "deposit-42".to_owned(),
            })
            .is_err()
        );
    }

    for invalid_amount in [
        "0".to_owned(),
        "-1".to_owned(),
        "not-a-decimal".to_owned(),
        format!("0.{}", "1".repeat(39)),
    ] {
        let bytes = encode_spot_transfer(&WireSpotTransfer {
            from_account_id: from.clone(),
            to_account_id: to.clone(),
            asset_id: "USDC".to_owned(),
            amount: invalid_amount,
        })
        .unwrap();
        assert!(EventPayload::decode(EventKind::SpotTransfer, &bytes).is_err());
    }

    assert!(
        encode_perp_transfer(&WirePerpTransfer {
            from_account_id: from.clone(),
            to_account_id: from.clone(),
            quote_amount: "1".to_owned(),
        })
        .is_err()
    );

    assert!(
        encode_subaccount_transfer(&WireSubaccountTransfer {
            master_account_id: from.clone(),
            from_account_id: to.clone(),
            to_account_id: to.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .is_err()
    );

    for invalid_reference in [
        String::new(),
        " deposit-42".to_owned(),
        "deposit-42 ".to_owned(),
        "deposit\n42".to_owned(),
        "x".repeat(257),
    ] {
        let direct = EventPayload::DepositCredited(DepositCredited {
            account_id: account(0x11),
            asset_id: AssetId::new("USDC").unwrap(),
            amount: Quantity::from_raw(1, 0).unwrap(),
            deposit_reference: invalid_reference,
        });
        assert!(direct.encode_to_vec().is_err());
    }

    for invalid_id in ["", " USDC", "USDC ", " "] {
        let bytes = encode_withdrawal_debited(&WireWithdrawalDebited {
            account_id: from.clone(),
            asset_id: invalid_id.to_owned(),
            amount: "1".to_owned(),
            withdrawal_reference: "withdrawal-43".to_owned(),
        });
        assert!(bytes.is_err());
    }

    let zero_shares = encode_vault_deposit(&WireVaultDeposit {
        vault_id: "vault-alpha".to_owned(),
        account_id: from,
        amount: "1".to_owned(),
        shares_issued: "0".to_owned(),
    })
    .unwrap();
    assert!(EventPayload::decode(EventKind::VaultDeposit, &zero_shares).is_err());
}

#[test]
fn direct_decode_rejects_kind_mismatch_and_noncanonical_payload_bytes() {
    let bytes = encode_deposit_credited(&WireDepositCredited {
        account_id: account(0x11).to_api_string(),
        asset_id: "USDC".to_owned(),
        amount: "1.000000".to_owned(),
        deposit_reference: "deposit-42".to_owned(),
    })
    .unwrap();
    assert!(EventPayload::decode(EventKind::WithdrawalDebited, &bytes).is_err());

    let noncanonical = append_varint_field(bytes, 100, 1);
    assert!(EventPayload::decode(EventKind::DepositCredited, &noncanonical).is_err());
}

#[test]
fn enclosing_cash_flow_event_preserves_forward_compatible_payload_bytes() {
    let account = account(0x11);
    let payload = EventPayload::DepositCredited(DepositCredited {
        account_id: account,
        asset_id: AssetId::new("USDC").unwrap(),
        amount: Quantity::parse_at_scale("125.5", 6).unwrap(),
        deposit_reference: "deposit-42".to_owned(),
    });
    let time = ProtocolTime::from_unix_micros(1_721_779_200_000_042).unwrap();
    let envelope = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(42),
        block_time: time,
        transaction_id: TransactionId::new("cash-flow-tx-42").unwrap(),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: vec![account],
        source_evidence: vec![
            canonical_events::SourceEvidence::try_new(
                SourceId::new("cash-flow-test").unwrap(),
                "v1",
                "42",
                [0x42; 32],
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        ingested_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        parser_version: "cash-flow-test-v1".to_owned(),
        payload,
    })
    .unwrap();
    let mut wire = WireCanonicalEventEnvelope::decode(&envelope.encode_to_vec().unwrap()).unwrap();
    wire.payload = append_varint_field(wire.payload, 100, 1);
    wire.payload_hash = blake3::hash(&wire.payload).as_bytes().to_vec();

    let decoded = CanonicalEventEnvelope::decode(&wire.encode_to_vec()).unwrap();
    assert!(matches!(
        decoded.payload(),
        EventPayload::DepositCredited(DepositCredited {
            deposit_reference,
            ..
        }) if deposit_reference == "deposit-42"
    ));
    let reencoded = WireCanonicalEventEnvelope::decode(&decoded.encode_to_vec().unwrap()).unwrap();
    assert_eq!(reencoded.payload, wire.payload);
}
