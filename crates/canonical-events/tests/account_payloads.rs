use api_contracts::{
    MAX_CANONICAL_ACCOUNT_PAYLOAD_BYTES, WireAccountModeChanged, WireBuilderFeeCharged,
    WireCanonicalEventEnvelope, WireDepositCredited, WireFeeCharged, WireFundingPaid,
    WireFundingReceived, WireLeverageChanged, WireMarginModeChanged, WirePerpTransfer,
    WireReferralReward, WireSpotTransfer, WireSubaccountTransfer, WireVaultDeposit,
    WireVaultWithdrawal, WireWithdrawalDebited, decode_deposit_credited,
    encode_account_mode_changed, encode_builder_fee_charged, encode_default_event_payload,
    encode_deposit_credited, encode_fee_charged, encode_funding_paid, encode_funding_received,
    encode_leverage_changed, encode_margin_mode_changed, encode_perp_transfer,
    encode_referral_reward, encode_spot_transfer, encode_subaccount_transfer, encode_vault_deposit,
    encode_vault_withdrawal, encode_withdrawal_debited,
};
use canonical_events::{
    AccountModeChanged, BuilderFeeCharged, CanonicalEventEnvelope, CanonicalEventInput,
    ConfirmationClass, DepositCredited, EventKind, EventPayload, FeeCharged, FundingPaid,
    FundingReceived, LeverageChanged, MarginModeChanged, PerpTransfer, ReferralReward,
    SpotTransfer, SubaccountTransfer, VaultDeposit, VaultWithdrawal, WithdrawalDebited,
};
use domain_types::{
    AccountAbstractionModeV1, Address, AssetId, BlockHeight, ChainId, FeeRate, FeeTypeV1,
    FundingRate, KnownTime, Leverage, MarginModeV1, MarketId, ProtocolTime, Quantity, QuoteAmount,
    SourceId, TransactionId, VaultId,
};

const ACCOUNT_PAYLOAD_LIMIT: usize = MAX_CANONICAL_ACCOUNT_PAYLOAD_BYTES;

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

fn append_varint(bytes: &mut Vec<u8>, mut value: usize) {
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

fn read_varint(bytes: &[u8], cursor: &mut usize) -> usize {
    let mut value = 0_usize;
    for shift in (0..usize::BITS).step_by(7) {
        let byte = bytes[*cursor];
        *cursor += 1;
        value |= usize::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return value;
        }
    }
    panic!("test fixture varint exceeds usize");
}

fn mutate_inner_message(encoded: &[u8], mutate: impl FnOnce(Vec<u8>) -> Vec<u8>) -> Vec<u8> {
    let mut cursor = 0;
    assert_eq!(encoded[cursor], 0x0a, "expected event_kind field");
    cursor += 1;
    let event_kind_len = read_varint(encoded, &mut cursor);
    cursor += event_kind_len;
    assert_eq!(encoded[cursor], 0x12, "expected message field");
    let message_tag_end = cursor + 1;
    cursor = message_tag_end;
    let message_len = read_varint(encoded, &mut cursor);
    let message_start = cursor;
    let message_end = message_start + message_len;
    let message = mutate(encoded[message_start..message_end].to_vec());

    let mut result = encoded[..message_tag_end].to_vec();
    append_varint(&mut result, message.len());
    result.extend_from_slice(&message);
    result.extend_from_slice(&encoded[message_end..]);
    result
}

fn append_inner_padding(encoded: &[u8], padding_bytes: usize) -> Vec<u8> {
    mutate_inner_message(encoded, |mut message| {
        append_varint(&mut message, usize::from(100_u16) << 3 | 2);
        append_varint(&mut message, padding_bytes);
        message.resize(message.len() + padding_bytes, 0);
        message
    })
}

fn pad_inner_to_exact_size(encoded: &[u8], target_bytes: usize) -> Vec<u8> {
    let mut padding_bytes = target_bytes
        .checked_sub(encoded.len() + 8)
        .expect("target must leave room for the unknown field");
    for _ in 0..8 {
        let candidate = append_inner_padding(encoded, padding_bytes);
        match candidate.len().cmp(&target_bytes) {
            std::cmp::Ordering::Equal => return candidate,
            std::cmp::Ordering::Less => padding_bytes += target_bytes - candidate.len(),
            std::cmp::Ordering::Greater => padding_bytes -= candidate.len() - target_bytes,
        }
    }
    panic!("could not construct exact-size payload fixture");
}

fn valid_deposit_bytes() -> Vec<u8> {
    encode_deposit_credited(&WireDepositCredited {
        account_id: account(0x11).to_api_string(),
        asset_id: "USDC".to_owned(),
        amount: "1".to_owned(),
        deposit_reference: "deposit-42".to_owned(),
    })
    .unwrap()
}

fn deposit_envelope() -> CanonicalEventEnvelope {
    let account = account(0x11);
    let time = ProtocolTime::from_unix_micros(1_721_779_200_000_042).unwrap();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
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
        payload: EventPayload::DepositCredited(DepositCredited {
            account_id: account,
            asset_id: AssetId::new("USDC").unwrap(),
            amount: Quantity::from_raw(1, 0).unwrap(),
            deposit_reference: "deposit-42".to_owned(),
        }),
    })
    .unwrap()
}

fn valid_account_payload_bytes() -> Vec<(EventKind, Vec<u8>)> {
    let from = account(0x11).to_api_string();
    let to = account(0x22).to_api_string();
    vec![
        (
            EventKind::DepositCredited,
            encode_deposit_credited(&WireDepositCredited {
                account_id: from.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                deposit_reference: "deposit-42".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::WithdrawalDebited,
            encode_withdrawal_debited(&WireWithdrawalDebited {
                account_id: from.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                withdrawal_reference: "withdrawal-42".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::SpotTransfer,
            encode_spot_transfer(&WireSpotTransfer {
                from_account_id: from.clone(),
                to_account_id: to.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::PerpTransfer,
            encode_perp_transfer(&WirePerpTransfer {
                from_account_id: from.clone(),
                to_account_id: to.clone(),
                quote_amount: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::SubaccountTransfer,
            encode_subaccount_transfer(&WireSubaccountTransfer {
                master_account_id: account(0x33).to_api_string(),
                from_account_id: from.clone(),
                to_account_id: to.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::VaultDeposit,
            encode_vault_deposit(&WireVaultDeposit {
                vault_id: "vault-alpha".to_owned(),
                account_id: from.clone(),
                amount: "1".to_owned(),
                shares_issued: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::VaultWithdrawal,
            encode_vault_withdrawal(&WireVaultWithdrawal {
                vault_id: "vault-alpha".to_owned(),
                account_id: from.clone(),
                amount: "1".to_owned(),
                shares_redeemed: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::FeeCharged,
            encode_fee_charged(&WireFeeCharged {
                account_id: from.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                fee_rate: "-0.0001".to_owned(),
                fee_type: "maker_rebate".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::BuilderFeeCharged,
            encode_builder_fee_charged(&WireBuilderFeeCharged {
                account_id: from.clone(),
                builder_account_id: to.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::FundingPaid,
            encode_funding_paid(&WireFundingPaid {
                account_id: from.clone(),
                market_id: "perp:BTC".to_owned(),
                amount: "1".to_owned(),
                funding_rate: "-0.0001".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::FundingReceived,
            encode_funding_received(&WireFundingReceived {
                account_id: from.clone(),
                market_id: "perp:BTC".to_owned(),
                amount: "1".to_owned(),
                funding_rate: "0.0001".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::ReferralReward,
            encode_referral_reward(&WireReferralReward {
                account_id: from.clone(),
                referrer_account_id: to,
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::AccountModeChanged,
            encode_account_mode_changed(&WireAccountModeChanged {
                account_id: from.clone(),
                previous_mode: "standard".to_owned(),
                new_mode: "unified".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::MarginModeChanged,
            encode_margin_mode_changed(&WireMarginModeChanged {
                account_id: from.clone(),
                market_id: "perp:BTC".to_owned(),
                previous_mode: "cross".to_owned(),
                new_mode: "isolated".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::LeverageChanged,
            encode_leverage_changed(&WireLeverageChanged {
                account_id: from,
                market_id: "perp:BTC".to_owned(),
                previous_leverage: "3".to_owned(),
                new_leverage: "5".to_owned(),
            })
            .unwrap(),
        ),
    ]
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
fn all_fee_funding_reward_and_mode_payloads_round_trip_exact_domain_values() {
    let account_id = account(0x11);
    let other_account_id = account(0x22);
    let asset_id = AssetId::new("USDC").unwrap();
    let market_id = MarketId::new("perp:BTC").unwrap();

    let cases = [
        (
            EventKind::FeeCharged,
            encode_fee_charged(&WireFeeCharged {
                account_id: account_id.to_api_string(),
                asset_id: asset_id.to_string(),
                amount: "0.250000".to_owned(),
                fee_rate: "-0.000100".to_owned(),
                fee_type: "maker_rebate".to_owned(),
            })
            .unwrap(),
            EventPayload::FeeCharged(FeeCharged {
                account_id,
                asset_id: asset_id.clone(),
                amount: Quantity::parse_at_scale("0.25", 6).unwrap(),
                fee_rate: FeeRate::parse_at_scale("-0.0001", 6).unwrap(),
                fee_type: FeeTypeV1::MakerRebate,
            }),
        ),
        (
            EventKind::BuilderFeeCharged,
            encode_builder_fee_charged(&WireBuilderFeeCharged {
                account_id: account_id.to_api_string(),
                builder_account_id: other_account_id.to_api_string(),
                asset_id: asset_id.to_string(),
                amount: "0.100000".to_owned(),
            })
            .unwrap(),
            EventPayload::BuilderFeeCharged(BuilderFeeCharged {
                account_id,
                builder_account_id: other_account_id,
                asset_id: asset_id.clone(),
                amount: Quantity::parse_at_scale("0.1", 6).unwrap(),
            }),
        ),
        (
            EventKind::FundingPaid,
            encode_funding_paid(&WireFundingPaid {
                account_id: account_id.to_api_string(),
                market_id: market_id.to_string(),
                amount: "3.500000".to_owned(),
                funding_rate: "-0.000125".to_owned(),
            })
            .unwrap(),
            EventPayload::FundingPaid(FundingPaid {
                account_id,
                market_id: market_id.clone(),
                amount: QuoteAmount::parse_at_scale("3.5", 6).unwrap(),
                funding_rate: FundingRate::parse_at_scale("-0.000125", 6).unwrap(),
            }),
        ),
        (
            EventKind::FundingReceived,
            encode_funding_received(&WireFundingReceived {
                account_id: account_id.to_api_string(),
                market_id: market_id.to_string(),
                amount: "2.250000".to_owned(),
                funding_rate: "0.000075".to_owned(),
            })
            .unwrap(),
            EventPayload::FundingReceived(FundingReceived {
                account_id,
                market_id: market_id.clone(),
                amount: QuoteAmount::parse_at_scale("2.25", 6).unwrap(),
                funding_rate: FundingRate::parse_at_scale("0.000075", 6).unwrap(),
            }),
        ),
        (
            EventKind::ReferralReward,
            encode_referral_reward(&WireReferralReward {
                account_id: account_id.to_api_string(),
                referrer_account_id: other_account_id.to_api_string(),
                asset_id: asset_id.to_string(),
                amount: "0.500000".to_owned(),
            })
            .unwrap(),
            EventPayload::ReferralReward(ReferralReward {
                account_id,
                referrer_account_id: other_account_id,
                asset_id: asset_id.clone(),
                amount: Quantity::parse_at_scale("0.5", 6).unwrap(),
            }),
        ),
        (
            EventKind::AccountModeChanged,
            encode_account_mode_changed(&WireAccountModeChanged {
                account_id: account_id.to_api_string(),
                previous_mode: "standard".to_owned(),
                new_mode: "unified".to_owned(),
            })
            .unwrap(),
            EventPayload::AccountModeChanged(AccountModeChanged {
                account_id,
                previous_mode: AccountAbstractionModeV1::Standard,
                new_mode: AccountAbstractionModeV1::Unified,
            }),
        ),
        (
            EventKind::MarginModeChanged,
            encode_margin_mode_changed(&WireMarginModeChanged {
                account_id: account_id.to_api_string(),
                market_id: market_id.to_string(),
                previous_mode: "cross".to_owned(),
                new_mode: "strict_isolated".to_owned(),
            })
            .unwrap(),
            EventPayload::MarginModeChanged(MarginModeChanged {
                account_id,
                market_id: market_id.clone(),
                previous_mode: MarginModeV1::Cross,
                new_mode: MarginModeV1::StrictIsolated,
            }),
        ),
        (
            EventKind::LeverageChanged,
            encode_leverage_changed(&WireLeverageChanged {
                account_id: account_id.to_api_string(),
                market_id: market_id.to_string(),
                previous_leverage: "3".to_owned(),
                new_leverage: "5".to_owned(),
            })
            .unwrap(),
            EventPayload::LeverageChanged(LeverageChanged {
                account_id,
                market_id,
                previous_leverage: Leverage::from_raw(3, 0).unwrap(),
                new_leverage: Leverage::from_raw(5, 0).unwrap(),
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
fn fee_funding_reward_and_mode_payloads_reject_financially_invalid_values() {
    let account_id = account(0x11).to_api_string();
    let other_account_id = account(0x22).to_api_string();

    for (fee_type, fee_rate) in [
        ("maker_rebate", "0"),
        ("maker_rebate", "0.0001"),
        ("maker", "0"),
        ("maker", "-0.0001"),
        ("taker", "-0.0001"),
        ("referral_discount", "0"),
        ("protocol", "-0.0001"),
    ] {
        let encoded = encode_fee_charged(&WireFeeCharged {
            account_id: account_id.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
            fee_rate: fee_rate.to_owned(),
            fee_type: fee_type.to_owned(),
        });
        assert!(encoded.is_err(), "{fee_type} accepted fee rate {fee_rate}");
    }

    for invalid in [
        "0".to_owned(),
        "-1".to_owned(),
        "not-a-decimal".to_owned(),
        format!("0.{}", "1".repeat(39)),
    ] {
        for (kind, bytes, field) in [
            (
                EventKind::FundingPaid,
                encode_funding_paid(&WireFundingPaid {
                    account_id: account_id.clone(),
                    market_id: "perp:BTC".to_owned(),
                    amount: invalid.clone(),
                    funding_rate: "-0.0001".to_owned(),
                }),
                "FundingPaid.amount",
            ),
            (
                EventKind::FundingReceived,
                encode_funding_received(&WireFundingReceived {
                    account_id: account_id.clone(),
                    market_id: "perp:BTC".to_owned(),
                    amount: invalid.clone(),
                    funding_rate: "0.0001".to_owned(),
                }),
                "FundingReceived.amount",
            ),
            (
                EventKind::LeverageChanged,
                encode_leverage_changed(&WireLeverageChanged {
                    account_id: account_id.clone(),
                    market_id: "perp:BTC".to_owned(),
                    previous_leverage: "3".to_owned(),
                    new_leverage: invalid.clone(),
                }),
                "LeverageChanged.new_leverage",
            ),
        ] {
            if let Ok(bytes) = bytes {
                assert!(
                    EventPayload::decode(kind, &bytes).is_err(),
                    "{field} accepted {invalid}"
                );
            }
        }
    }

    assert!(
        encode_builder_fee_charged(&WireBuilderFeeCharged {
            account_id: account_id.clone(),
            builder_account_id: account_id.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_referral_reward(&WireReferralReward {
            account_id: account_id.clone(),
            referrer_account_id: account_id.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_account_mode_changed(&WireAccountModeChanged {
            account_id: account_id.clone(),
            previous_mode: "standard".to_owned(),
            new_mode: "standard".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_margin_mode_changed(&WireMarginModeChanged {
            account_id: account_id.clone(),
            market_id: "perp:BTC".to_owned(),
            previous_mode: "cross".to_owned(),
            new_mode: "cross".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_leverage_changed(&WireLeverageChanged {
            account_id,
            market_id: "perp:BTC".to_owned(),
            previous_leverage: "5".to_owned(),
            new_leverage: "5".to_owned(),
        })
        .is_err()
    );

    let valid = encode_referral_reward(&WireReferralReward {
        account_id: account(0x11).to_api_string(),
        referrer_account_id: other_account_id,
        asset_id: "USDC".to_owned(),
        amount: "1".to_owned(),
    })
    .unwrap();
    let noncanonical = mutate_inner_message(&valid, |message| append_varint_field(message, 100, 1));
    assert!(EventPayload::decode(EventKind::ReferralReward, &noncanonical).is_err());
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
fn every_cash_flow_numeric_field_rejects_zero_negative_malformed_and_overprecision() {
    let from = account(0x11).to_api_string();
    let to = account(0x22).to_api_string();
    for invalid in [
        "0".to_owned(),
        "-1".to_owned(),
        "not-a-decimal".to_owned(),
        format!("0.{}", "1".repeat(39)),
    ] {
        let cases = [
            (
                EventKind::DepositCredited,
                encode_deposit_credited(&WireDepositCredited {
                    account_id: from.clone(),
                    asset_id: "USDC".to_owned(),
                    amount: invalid.clone(),
                    deposit_reference: "deposit-42".to_owned(),
                })
                .unwrap(),
                "DepositCredited.amount",
            ),
            (
                EventKind::WithdrawalDebited,
                encode_withdrawal_debited(&WireWithdrawalDebited {
                    account_id: from.clone(),
                    asset_id: "USDC".to_owned(),
                    amount: invalid.clone(),
                    withdrawal_reference: "withdrawal-42".to_owned(),
                })
                .unwrap(),
                "WithdrawalDebited.amount",
            ),
            (
                EventKind::SpotTransfer,
                encode_spot_transfer(&WireSpotTransfer {
                    from_account_id: from.clone(),
                    to_account_id: to.clone(),
                    asset_id: "USDC".to_owned(),
                    amount: invalid.clone(),
                })
                .unwrap(),
                "SpotTransfer.amount",
            ),
            (
                EventKind::PerpTransfer,
                encode_perp_transfer(&WirePerpTransfer {
                    from_account_id: from.clone(),
                    to_account_id: to.clone(),
                    quote_amount: invalid.clone(),
                })
                .unwrap(),
                "PerpTransfer.quote_amount",
            ),
            (
                EventKind::SubaccountTransfer,
                encode_subaccount_transfer(&WireSubaccountTransfer {
                    master_account_id: account(0x33).to_api_string(),
                    from_account_id: from.clone(),
                    to_account_id: to.clone(),
                    asset_id: "USDC".to_owned(),
                    amount: invalid.clone(),
                })
                .unwrap(),
                "SubaccountTransfer.amount",
            ),
            (
                EventKind::VaultDeposit,
                encode_vault_deposit(&WireVaultDeposit {
                    vault_id: "vault-alpha".to_owned(),
                    account_id: from.clone(),
                    amount: invalid.clone(),
                    shares_issued: "1".to_owned(),
                })
                .unwrap(),
                "VaultDeposit.amount",
            ),
            (
                EventKind::VaultDeposit,
                encode_vault_deposit(&WireVaultDeposit {
                    vault_id: "vault-alpha".to_owned(),
                    account_id: from.clone(),
                    amount: "1".to_owned(),
                    shares_issued: invalid.clone(),
                })
                .unwrap(),
                "VaultDeposit.shares_issued",
            ),
            (
                EventKind::VaultWithdrawal,
                encode_vault_withdrawal(&WireVaultWithdrawal {
                    vault_id: "vault-alpha".to_owned(),
                    account_id: from.clone(),
                    amount: invalid.clone(),
                    shares_redeemed: "1".to_owned(),
                })
                .unwrap(),
                "VaultWithdrawal.amount",
            ),
            (
                EventKind::VaultWithdrawal,
                encode_vault_withdrawal(&WireVaultWithdrawal {
                    vault_id: "vault-alpha".to_owned(),
                    account_id: from.clone(),
                    amount: "1".to_owned(),
                    shares_redeemed: invalid.clone(),
                })
                .unwrap(),
                "VaultWithdrawal.shares_redeemed",
            ),
        ];

        for (kind, bytes, field) in cases {
            assert!(
                EventPayload::decode(kind, &bytes).is_err(),
                "{field} accepted {invalid}"
            );
        }
    }
}

#[test]
fn every_transfer_rejects_equal_endpoints_and_subaccount_master_may_equal_either() {
    let from = account(0x11).to_api_string();
    let to = account(0x22).to_api_string();

    assert!(
        encode_spot_transfer(&WireSpotTransfer {
            from_account_id: from.clone(),
            to_account_id: from.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .is_err()
    );
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
            master_account_id: account(0x33).to_api_string(),
            from_account_id: from.clone(),
            to_account_id: from.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .is_err()
    );

    for master_account_id in [from.clone(), to.clone()] {
        let bytes = encode_subaccount_transfer(&WireSubaccountTransfer {
            master_account_id,
            from_account_id: from.clone(),
            to_account_id: to.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .unwrap();
        assert!(EventPayload::decode(EventKind::SubaccountTransfer, &bytes).is_ok());
    }
}

#[test]
fn every_asset_vault_and_reference_path_rejects_blank_padded_or_control_values() {
    let from = account(0x11).to_api_string();
    let to = account(0x22).to_api_string();

    for invalid_asset in ["", " USDC", "USDC "] {
        assert!(
            encode_deposit_credited(&WireDepositCredited {
                account_id: from.clone(),
                asset_id: invalid_asset.to_owned(),
                amount: "1".to_owned(),
                deposit_reference: "deposit-42".to_owned(),
            })
            .is_err()
        );
        assert!(
            encode_withdrawal_debited(&WireWithdrawalDebited {
                account_id: from.clone(),
                asset_id: invalid_asset.to_owned(),
                amount: "1".to_owned(),
                withdrawal_reference: "withdrawal-42".to_owned(),
            })
            .is_err()
        );
        assert!(
            encode_spot_transfer(&WireSpotTransfer {
                from_account_id: from.clone(),
                to_account_id: to.clone(),
                asset_id: invalid_asset.to_owned(),
                amount: "1".to_owned(),
            })
            .is_err()
        );
        assert!(
            encode_subaccount_transfer(&WireSubaccountTransfer {
                master_account_id: account(0x33).to_api_string(),
                from_account_id: from.clone(),
                to_account_id: to.clone(),
                asset_id: invalid_asset.to_owned(),
                amount: "1".to_owned(),
            })
            .is_err()
        );
    }

    for invalid_vault in ["", " vault-alpha", "vault-alpha "] {
        assert!(
            encode_vault_deposit(&WireVaultDeposit {
                vault_id: invalid_vault.to_owned(),
                account_id: from.clone(),
                amount: "1".to_owned(),
                shares_issued: "1".to_owned(),
            })
            .is_err()
        );
        assert!(
            encode_vault_withdrawal(&WireVaultWithdrawal {
                vault_id: invalid_vault.to_owned(),
                account_id: from.clone(),
                amount: "1".to_owned(),
                shares_redeemed: "1".to_owned(),
            })
            .is_err()
        );
    }

    for invalid_reference in ["", " reference", "reference ", "reference\n"] {
        assert!(
            encode_deposit_credited(&WireDepositCredited {
                account_id: from.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                deposit_reference: invalid_reference.to_owned(),
            })
            .is_err()
        );
        assert!(
            encode_withdrawal_debited(&WireWithdrawalDebited {
                account_id: from.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                withdrawal_reference: invalid_reference.to_owned(),
            })
            .is_err()
        );
    }

    let boundary_reference = "r".repeat(256);
    for (kind, bytes) in [
        (
            EventKind::DepositCredited,
            encode_deposit_credited(&WireDepositCredited {
                account_id: from.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                deposit_reference: boundary_reference.clone(),
            })
            .unwrap(),
        ),
        (
            EventKind::WithdrawalDebited,
            encode_withdrawal_debited(&WireWithdrawalDebited {
                account_id: from.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                withdrawal_reference: boundary_reference,
            })
            .unwrap(),
        ),
    ] {
        assert!(
            EventPayload::decode(kind, &bytes).is_ok(),
            "{kind:?} rejected the exact reference byte boundary"
        );
    }

    let oversized_reference = "r".repeat(257);
    assert!(
        encode_deposit_credited(&WireDepositCredited {
            account_id: from.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
            deposit_reference: oversized_reference.clone(),
        })
        .is_err()
    );
    assert!(
        encode_withdrawal_debited(&WireWithdrawalDebited {
            account_id: from,
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
            withdrawal_reference: oversized_reference,
        })
        .is_err()
    );
}

#[test]
fn direct_decode_rejects_kind_mismatch_and_noncanonical_payload_bytes() {
    let fixtures = valid_account_payload_bytes();
    for (index, (_, bytes)) in fixtures.iter().enumerate() {
        let wrong_kind = fixtures[(index + 1) % fixtures.len()].0;
        assert!(
            EventPayload::decode(wrong_kind, bytes).is_err(),
            "payload {index} accepted mismatched {wrong_kind:?}"
        );
    }

    let bytes = valid_deposit_bytes();
    let noncanonical = append_varint_field(bytes, 100, 1);
    assert!(EventPayload::decode(EventKind::DepositCredited, &noncanonical).is_err());

    let inner_noncanonical = mutate_inner_message(&valid_deposit_bytes(), |message| {
        append_varint_field(message, 100, 1)
    });
    assert!(EventPayload::decode(EventKind::DepositCredited, &inner_noncanonical).is_err());
}

#[test]
fn strict_account_default_payloads_decode_to_canonical_variants() {
    for kind in [
        EventKind::DepositCredited,
        EventKind::WithdrawalDebited,
        EventKind::SpotTransfer,
        EventKind::PerpTransfer,
        EventKind::SubaccountTransfer,
        EventKind::VaultDeposit,
        EventKind::VaultWithdrawal,
        EventKind::FeeCharged,
        EventKind::BuilderFeeCharged,
        EventKind::FundingPaid,
        EventKind::FundingReceived,
        EventKind::ReferralReward,
        EventKind::AccountModeChanged,
        EventKind::MarginModeChanged,
        EventKind::LeverageChanged,
    ] {
        let bytes = encode_default_event_payload(kind.as_wire_name()).unwrap();
        let payload = EventPayload::decode(kind, &bytes)
            .unwrap_or_else(|error| panic!("{kind:?} default must decode: {error}"));
        assert_eq!(payload.kind(), kind);
    }
}

#[test]
fn inner_unknown_fields_preserve_exact_bound_and_one_over_fails_closed() {
    for (kind, valid) in valid_account_payload_bytes() {
        let exact = pad_inner_to_exact_size(&valid, ACCOUNT_PAYLOAD_LIMIT);
        assert_eq!(exact.len(), ACCOUNT_PAYLOAD_LIMIT);
        let exact_error = EventPayload::decode(kind, &exact).unwrap_err();
        assert!(
            !exact_error
                .to_string()
                .contains("exceeds the 16384-byte limit"),
            "{kind:?} exact-bound payload failed the inclusive size preflight"
        );

        let one_over = pad_inner_to_exact_size(&valid, ACCOUNT_PAYLOAD_LIMIT + 1);
        assert_eq!(one_over.len(), ACCOUNT_PAYLOAD_LIMIT + 1);
        let one_over_error = EventPayload::decode(kind, &one_over).unwrap_err();
        assert!(
            one_over_error
                .to_string()
                .contains("exceeds the 16384-byte limit"),
            "{kind:?} did not fail one-over at the size preflight: {one_over_error}"
        );

        let malformed = vec![0xff; ACCOUNT_PAYLOAD_LIMIT + 1];
        let malformed_error = EventPayload::decode(kind, &malformed).unwrap_err();
        assert!(
            malformed_error
                .to_string()
                .contains("exceeds the 16384-byte limit"),
            "{kind:?} unwrapped malformed bytes before the size preflight: {malformed_error}"
        );
    }

    let exact = pad_inner_to_exact_size(&valid_deposit_bytes(), ACCOUNT_PAYLOAD_LIMIT);
    assert!(decode_deposit_credited(&exact).is_ok());
    let mut exact_wire =
        WireCanonicalEventEnvelope::decode(&deposit_envelope().encode_to_vec().unwrap()).unwrap();
    exact_wire.payload = exact.clone();
    exact_wire.payload_hash = blake3::hash(&exact).as_bytes().to_vec();
    let decoded = CanonicalEventEnvelope::decode(&exact_wire.encode_to_vec()).unwrap();
    let reencoded = WireCanonicalEventEnvelope::decode(&decoded.encode_to_vec().unwrap()).unwrap();
    assert_eq!(reencoded.payload, exact);

    let one_over = pad_inner_to_exact_size(&valid_deposit_bytes(), ACCOUNT_PAYLOAD_LIMIT + 1);
    assert!(decode_deposit_credited(&one_over).is_err());
    let mut one_over_wire =
        WireCanonicalEventEnvelope::decode(&deposit_envelope().encode_to_vec().unwrap()).unwrap();
    one_over_wire.payload_hash = blake3::hash(&one_over).as_bytes().to_vec();
    one_over_wire.payload = one_over;
    assert!(CanonicalEventEnvelope::decode(&one_over_wire.encode_to_vec()).is_err());
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

#[test]
fn enclosing_fee_event_preserves_inner_unknown_fields_without_changing_rebate_sign() {
    let account_id = account(0x11);
    let time = ProtocolTime::from_unix_micros(1_721_779_200_000_043).unwrap();
    let envelope = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").unwrap(),
        block_height: BlockHeight::new(43),
        block_time: time,
        transaction_id: TransactionId::new("fee-tx-43").unwrap(),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: vec![account_id],
        source_evidence: vec![
            canonical_events::SourceEvidence::try_new(
                SourceId::new("fee-test").unwrap(),
                "v1",
                "43",
                [0x43; 32],
            )
            .unwrap(),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        ingested_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        canonicalized_at: KnownTime::from_unix_micros(time.unix_micros()).unwrap(),
        parser_version: "fee-test-v1".to_owned(),
        payload: EventPayload::FeeCharged(FeeCharged {
            account_id,
            asset_id: AssetId::new("USDC").unwrap(),
            amount: Quantity::parse_at_scale("0.25", 6).unwrap(),
            fee_rate: FeeRate::parse_at_scale("-0.0001", 6).unwrap(),
            fee_type: FeeTypeV1::MakerRebate,
        }),
    })
    .unwrap();
    let mut wire = WireCanonicalEventEnvelope::decode(&envelope.encode_to_vec().unwrap()).unwrap();
    wire.payload = mutate_inner_message(&wire.payload, |message| {
        append_varint_field(message, 100, 1)
    });
    wire.payload_hash = blake3::hash(&wire.payload).as_bytes().to_vec();

    let decoded = CanonicalEventEnvelope::decode(&wire.encode_to_vec()).unwrap();
    assert!(matches!(
        decoded.payload(),
        EventPayload::FeeCharged(FeeCharged {
            fee_rate,
            fee_type: FeeTypeV1::MakerRebate,
            ..
        }) if fee_rate.raw() < 0
    ));
    let reencoded = WireCanonicalEventEnvelope::decode(&decoded.encode_to_vec().unwrap()).unwrap();
    assert_eq!(reencoded.payload, wire.payload);
}
