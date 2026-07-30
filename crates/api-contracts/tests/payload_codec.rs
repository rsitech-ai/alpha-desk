use api_contracts::{
    MAX_CANONICAL_ACCOUNT_PAYLOAD_BYTES, MAX_CANONICAL_TRADE_PAYLOAD_BYTES, PayloadCodecError,
    WireAccountModeChanged, WireAssetContextUpdated, WireBackstopLiquidation,
    WireBuilderFeeCharged, WireDepositCredited, WireDexCreated, WireFeeCharged, WireFundingPaid,
    WireFundingRateUpdated, WireFundingReceived, WireLeverageChanged, WireLiquidationFill,
    WireLiquidationStarted, WireMarginModeChanged, WireMarginTableChanged, WireMarketCreated,
    WireMarketHalted, WireMarketMetadataChanged, WireMarketResumed, WireOpenInterestCapChanged,
    WireOracleUpdated, WireOrderAccepted, WireOrderCancelled, WireOrderFilled, WireOrderModified,
    WireOrderPartiallyFilled, WireOrderRejected, WireOrderRested, WireOutcomeCreated,
    WireOutcomeResolved, WirePerpTransfer, WirePositionSettled, WireReferralReward,
    WireSpotTransfer, WireSubaccountTransfer, WireTradeMatched, WireTradeParticipantV1,
    WireVaultDeposit, WireVaultWithdrawal, WireWithdrawalDebited, decode_account_mode_changed,
    decode_asset_context_updated, decode_backstop_liquidation, decode_builder_fee_charged,
    decode_deposit_credited, decode_dex_created, decode_fee_charged, decode_funding_paid,
    decode_funding_rate_updated, decode_funding_received, decode_leverage_changed,
    decode_liquidation_fill, decode_liquidation_started, decode_margin_mode_changed,
    decode_margin_table_changed, decode_market_created, decode_market_halted,
    decode_market_metadata_changed, decode_market_resumed, decode_open_interest_cap_changed,
    decode_oracle_updated, decode_order_accepted, decode_order_cancelled, decode_order_filled,
    decode_order_modified, decode_order_partially_filled, decode_order_rejected,
    decode_order_rested, decode_outcome_created, decode_outcome_resolved, decode_perp_transfer,
    decode_position_settled, decode_referral_reward, decode_spot_transfer,
    decode_subaccount_transfer, decode_trade_matched, decode_vault_deposit,
    decode_vault_withdrawal, decode_withdrawal_debited, encode_account_mode_changed,
    encode_asset_context_updated, encode_backstop_liquidation, encode_builder_fee_charged,
    encode_default_event_payload, encode_deposit_credited, encode_dex_created, encode_fee_charged,
    encode_funding_paid, encode_funding_rate_updated, encode_funding_received,
    encode_leverage_changed, encode_liquidation_fill, encode_liquidation_started,
    encode_margin_mode_changed, encode_margin_table_changed, encode_market_created,
    encode_market_halted, encode_market_metadata_changed, encode_market_resumed,
    encode_open_interest_cap_changed, encode_oracle_updated, encode_order_accepted,
    encode_order_cancelled, encode_order_filled, encode_order_modified,
    encode_order_partially_filled, encode_order_rejected, encode_order_rested,
    encode_outcome_created, encode_outcome_resolved, encode_perp_transfer, encode_position_settled,
    encode_referral_reward, encode_spot_transfer, encode_subaccount_transfer, encode_trade_matched,
    encode_vault_deposit, encode_vault_withdrawal, encode_withdrawal_debited,
    validate_event_payload,
};
use prost::Message;

const ACCOUNT_PAYLOAD_LIMIT: usize = MAX_CANONICAL_ACCOUNT_PAYLOAD_BYTES;
const ACCOUNT_PAYLOAD_SIZE_REASON: &str = "canonical account payload exceeds the 16384-byte limit";

fn trade() -> WireTradeMatched {
    WireTradeMatched {
        trade_id: None,
        market_id: None,
        maker_order_id: None,
        taker_order_id: None,
        price: "65000".to_owned(),
        quantity: "0.01".to_owned(),
        deterministic_seed: 7,
        participants: None,
    }
}

fn trade_participants() -> [WireTradeParticipantV1; 2] {
    [
        WireTradeParticipantV1 {
            role: "buyer".to_owned(),
            account_id: primary_account(),
            start_position: "996.67".to_owned(),
            order_id: "12212201265".to_owned(),
            twap_id: Some(91),
            client_order_id: Some("0x11111111111111111111111111111111".to_owned()),
        },
        WireTradeParticipantV1 {
            role: "seller".to_owned(),
            account_id: secondary_account(),
            start_position: "-996.7".to_owned(),
            order_id: "12212198275".to_owned(),
            twap_id: None,
            client_order_id: None,
        },
    ]
}

#[derive(Clone, PartialEq, Message)]
struct TestTypedPayloadEnvelope {
    #[prost(string, tag = "1")]
    event_kind: String,
    #[prost(bytes = "vec", tag = "2")]
    message: Vec<u8>,
}

#[derive(Clone, PartialEq, Message)]
struct TestDecimalValue {
    #[prost(string, tag = "1")]
    value: String,
}

#[derive(Clone, PartialEq, Message)]
struct TestTradeParticipantV1 {
    #[prost(int32, tag = "1")]
    role: i32,
    #[prost(string, tag = "2")]
    account_id: String,
    #[prost(string, tag = "3")]
    start_position: String,
    #[prost(string, tag = "4")]
    order_id: String,
    #[prost(uint64, optional, tag = "5")]
    twap_id: Option<u64>,
    #[prost(string, tag = "6")]
    client_order_id: String,
}

#[derive(Clone, PartialEq, Message)]
struct TestTradeMatched {
    #[prost(string, tag = "1")]
    trade_id: String,
    #[prost(string, tag = "2")]
    market_id: String,
    #[prost(string, tag = "3")]
    maker_order_id: String,
    #[prost(string, tag = "4")]
    taker_order_id: String,
    #[prost(message, optional, tag = "5")]
    price: Option<TestDecimalValue>,
    #[prost(message, optional, tag = "6")]
    quantity: Option<TestDecimalValue>,
    #[prost(uint64, tag = "7")]
    deterministic_seed: u64,
    #[prost(message, repeated, tag = "8")]
    participants: Vec<TestTradeParticipantV1>,
}

fn rewrite_first_trade_cloid(encoded: &[u8], cloid: &str) -> Vec<u8> {
    let mut envelope = TestTypedPayloadEnvelope::decode(encoded).unwrap();
    let mut trade = TestTradeMatched::decode(envelope.message.as_slice()).unwrap();
    trade.participants[0].client_order_id = cloid.to_owned();
    envelope.message = trade.encode_to_vec();
    envelope.encode_to_vec()
}

fn deposit_with_asset_bytes(asset_bytes: usize) -> WireDepositCredited {
    WireDepositCredited {
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        asset_id: "A".repeat(asset_bytes),
        amount: "1".to_owned(),
        deposit_reference: "deposit-42".to_owned(),
    }
}

fn primary_account() -> String {
    "0x1111111111111111111111111111111111111111".to_owned()
}

fn secondary_account() -> String {
    "0x2222222222222222222222222222222222222222".to_owned()
}

fn encode_fee_with_asset_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_fee_charged(&WireFeeCharged {
        account_id: primary_account(),
        asset_id: "A".repeat(bytes),
        amount: "1".to_owned(),
        fee_rate: "-0.0001".to_owned(),
        fee_type: "maker_rebate".to_owned(),
    })
}

fn encode_builder_fee_with_asset_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_builder_fee_charged(&WireBuilderFeeCharged {
        account_id: primary_account(),
        builder_account_id: secondary_account(),
        asset_id: "A".repeat(bytes),
        amount: "1".to_owned(),
    })
}

fn encode_funding_paid_with_market_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_funding_paid(&WireFundingPaid {
        account_id: primary_account(),
        market_id: "M".repeat(bytes),
        amount: "1".to_owned(),
        funding_rate: "-0.0001".to_owned(),
    })
}

fn encode_funding_received_with_market_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_funding_received(&WireFundingReceived {
        account_id: primary_account(),
        market_id: "M".repeat(bytes),
        amount: "1".to_owned(),
        funding_rate: "0.0001".to_owned(),
    })
}

fn encode_referral_reward_with_asset_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_referral_reward(&WireReferralReward {
        account_id: primary_account(),
        referrer_account_id: secondary_account(),
        asset_id: "A".repeat(bytes),
        amount: "1".to_owned(),
    })
}

fn encode_margin_mode_with_market_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_margin_mode_changed(&WireMarginModeChanged {
        account_id: primary_account(),
        market_id: "M".repeat(bytes),
        previous_mode: "cross".to_owned(),
        new_mode: "isolated".to_owned(),
    })
}

fn encode_leverage_with_market_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_leverage_changed(&WireLeverageChanged {
        account_id: primary_account(),
        market_id: "M".repeat(bytes),
        previous_leverage: "3".to_owned(),
        new_leverage: "5".to_owned(),
    })
}

fn encode_liquidation_started_with_id_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_liquidation_started(&WireLiquidationStarted {
        account_id: primary_account(),
        liquidation_id: "L".repeat(bytes),
        margin_value: "9.000000".to_owned(),
        maintenance_requirement: "10.000000".to_owned(),
    })
}

fn encode_liquidation_fill_with_market_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_liquidation_fill(&WireLiquidationFill {
        liquidation_id: "liquidation-42".to_owned(),
        account_id: primary_account(),
        market_id: "M".repeat(bytes),
        price: "65000.000000".to_owned(),
        quantity: "0.01000000".to_owned(),
    })
}

fn encode_backstop_liquidation_with_market_bytes(
    bytes: usize,
) -> Result<Vec<u8>, PayloadCodecError> {
    encode_backstop_liquidation(&WireBackstopLiquidation {
        liquidation_id: "liquidation-42".to_owned(),
        account_id: primary_account(),
        backstop_account_id: secondary_account(),
        market_id: "M".repeat(bytes),
        quantity: "0.01000000".to_owned(),
    })
}

fn encode_position_settled_with_market_bytes(bytes: usize) -> Result<Vec<u8>, PayloadCodecError> {
    encode_position_settled(&WirePositionSettled {
        account_id: primary_account(),
        market_id: "M".repeat(bytes),
        settlement_price: "0.000000".to_owned(),
        settled_quantity: "1.00000000".to_owned(),
        realized_pnl: "-5.000000".to_owned(),
    })
}

fn assert_account_payload_size_error(error: PayloadCodecError, kind: &str) {
    assert!(matches!(
        error,
        PayloadCodecError::Invalid {
            kind: actual_kind,
            reason,
        } if actual_kind == kind && reason == ACCOUNT_PAYLOAD_SIZE_REASON
    ));
}

fn assert_encoder_exact_one_over_and_70k(
    kind: &str,
    encoder: fn(usize) -> Result<Vec<u8>, PayloadCodecError>,
) {
    let seed_field_bytes = ACCOUNT_PAYLOAD_LIMIT - 1_024;
    let seed = encoder(seed_field_bytes).unwrap();
    assert!(seed.len() < ACCOUNT_PAYLOAD_LIMIT);

    let exact_field_bytes = seed_field_bytes + ACCOUNT_PAYLOAD_LIMIT - seed.len();
    let exact = encoder(exact_field_bytes).unwrap();
    assert_eq!(
        exact.len(),
        ACCOUNT_PAYLOAD_LIMIT,
        "{kind} encoder rejected the inclusive boundary"
    );

    assert_account_payload_size_result(encoder(exact_field_bytes + 1), kind);
    let outcome = std::panic::catch_unwind(|| encoder(70_000));
    let error = outcome
        .unwrap_or_else(|_| panic!("{kind} public encoder panicked on a 70k probe"))
        .unwrap_err();
    assert_account_payload_size_error(error, kind);
}

fn assert_account_payload_size_result(result: Result<Vec<u8>, PayloadCodecError>, kind: &str) {
    match result {
        Err(error) => assert_account_payload_size_error(error, kind),
        Ok(bytes) => panic!(
            "{kind} oversized encoder unexpectedly returned {} bytes",
            bytes.len()
        ),
    }
}

fn append_test_varint(bytes: &mut Vec<u8>, mut value: usize) {
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

fn append_outer_unknown_padding(encoded: &[u8], padding_bytes: usize) -> Vec<u8> {
    let mut padded = encoded.to_vec();
    padded.push(0x1a);
    append_test_varint(&mut padded, padding_bytes);
    padded.resize(padded.len() + padding_bytes, 0);
    padded
}

fn pad_outer_unknown_to_exact_size(encoded: &[u8], target_bytes: usize) -> Vec<u8> {
    let mut padding_bytes = target_bytes
        .checked_sub(encoded.len() + 4)
        .expect("target must leave room for the unknown field");
    for _ in 0..8 {
        let candidate = append_outer_unknown_padding(encoded, padding_bytes);
        match candidate.len().cmp(&target_bytes) {
            std::cmp::Ordering::Equal => return candidate,
            std::cmp::Ordering::Less => padding_bytes += target_bytes - candidate.len(),
            std::cmp::Ordering::Greater => padding_bytes -= candidate.len() - target_bytes,
        }
    }
    panic!("could not construct exact-size outer payload fixture");
}

#[test]
fn trade_identities_round_trip_when_full_partial_or_absent() {
    let absent = trade();
    assert_eq!(
        decode_trade_matched(&encode_trade_matched(&absent).unwrap()).unwrap(),
        absent
    );

    let mut full = trade();
    full.trade_id = Some("trade-42".to_owned());
    full.market_id = Some("perp:BTC".to_owned());
    full.maker_order_id = Some("maker-7".to_owned());
    full.taker_order_id = Some("taker-9".to_owned());
    assert_eq!(
        decode_trade_matched(&encode_trade_matched(&full).unwrap()).unwrap(),
        full
    );

    let mut partial = trade();
    partial.market_id = Some("spot:BTC/USDC".to_owned());
    partial.taker_order_id = Some("taker-10".to_owned());
    assert_eq!(
        decode_trade_matched(&encode_trade_matched(&partial).unwrap()).unwrap(),
        partial
    );
}

#[test]
fn trade_participants_round_trip_in_canonical_buyer_seller_order() {
    let value = WireTradeMatched {
        participants: Some(trade_participants()),
        ..trade()
    };
    let first = encode_trade_matched(&value).unwrap();
    let second = encode_trade_matched(&value).unwrap();

    assert_eq!(first, second);
    assert_eq!(decode_trade_matched(&first).unwrap(), value);
}

#[test]
fn trade_participant_cloids_require_exact_lowercase_128_bit_hex_on_encode_and_decode() {
    let invalid = [
        "11111111111111111111111111111111",
        "0x1111111111111111111111111111111",
        "0x111111111111111111111111111111111",
        "0xA1111111111111111111111111111111",
        "0xg1111111111111111111111111111111",
        "",
    ];

    for cloid in invalid {
        let mut participants = trade_participants();
        participants[0].client_order_id = Some(cloid.to_owned());
        assert!(
            matches!(
                encode_trade_matched(&WireTradeMatched {
                    participants: Some(participants),
                    ..trade()
                }),
                Err(PayloadCodecError::Invalid { .. })
            ),
            "encoder admitted non-canonical trade cloid {cloid:?}"
        );
    }

    let valid = WireTradeMatched {
        participants: Some(trade_participants()),
        ..trade()
    };
    let encoded = encode_trade_matched(&valid).unwrap();
    for cloid in &invalid[..invalid.len() - 1] {
        assert!(
            matches!(
                decode_trade_matched(&rewrite_first_trade_cloid(&encoded, cloid)),
                Err(PayloadCodecError::Invalid { .. })
            ),
            "decoder admitted non-canonical trade cloid {cloid:?}"
        );
    }

    assert_eq!(decode_trade_matched(&encoded).unwrap(), valid);
    let null = WireTradeMatched {
        participants: Some([
            WireTradeParticipantV1 {
                client_order_id: None,
                ..trade_participants()[0].clone()
            },
            trade_participants()[1].clone(),
        ]),
        ..trade()
    };
    assert_eq!(
        decode_trade_matched(&encode_trade_matched(&null).unwrap()).unwrap(),
        null
    );
}

#[test]
fn trade_participants_reject_wrong_roles_duplicate_accounts_and_invalid_values() {
    let mut wrong_order = trade_participants();
    wrong_order.swap(0, 1);
    let mut duplicate = trade_participants();
    duplicate[1].account_id = duplicate[0].account_id.clone();
    let mut missing_order = trade_participants();
    missing_order[0].order_id.clear();
    let mut invalid_start = trade_participants();
    invalid_start[0].start_position = "--1".to_owned();

    for participants in [wrong_order, duplicate, missing_order, invalid_start] {
        assert!(matches!(
            encode_trade_matched(&WireTradeMatched {
                participants: Some(participants),
                ..trade()
            }),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }

    for (price, quantity) in [("0", "0.01"), ("65000", "0"), ("-1", "0.01")] {
        assert!(matches!(
            encode_trade_matched(&WireTradeMatched {
                price: price.to_owned(),
                quantity: quantity.to_owned(),
                ..trade()
            }),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }
}

#[test]
fn trade_payloads_apply_the_16_kib_preflight_on_encode_and_decode() {
    let oversized = WireTradeMatched {
        maker_order_id: Some("x".repeat(MAX_CANONICAL_TRADE_PAYLOAD_BYTES)),
        ..trade()
    };
    assert!(matches!(
        encode_trade_matched(&oversized),
        Err(PayloadCodecError::Invalid { .. })
    ));

    let encoded = encode_trade_matched(&trade()).unwrap();
    let exact = pad_outer_unknown_to_exact_size(&encoded, MAX_CANONICAL_TRADE_PAYLOAD_BYTES);
    assert!(decode_trade_matched(&exact).is_ok());
    let oversized = append_outer_unknown_padding(&exact, 1);
    assert!(matches!(
        decode_trade_matched(&oversized),
        Err(PayloadCodecError::Invalid { .. })
    ));
}

#[test]
fn empty_or_surrounding_whitespace_identities_are_rejected() {
    for invalid in ["", " trade-42", "trade-42 ", " "] {
        let mut value = trade();
        value.trade_id = Some(invalid.to_owned());
        assert!(matches!(
            encode_trade_matched(&value),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }

    let mut encoded = encode_trade_matched(&WireTradeMatched {
        trade_id: Some("x".to_owned()),
        ..trade()
    })
    .unwrap();
    let identity = encoded
        .windows(3)
        .position(|window| window == [0x0a, 0x01, b'x'])
        .expect("encoded trade identity");
    encoded[identity + 2] = b' ';

    assert!(matches!(
        decode_trade_matched(&encoded),
        Err(PayloadCodecError::Invalid { .. })
    ));
}

#[test]
fn account_cash_flow_wire_payloads_round_trip_deterministically() {
    let deposit = WireDepositCredited {
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        asset_id: "USDC".to_owned(),
        amount: "125.500000".to_owned(),
        deposit_reference: "deposit-42".to_owned(),
    };
    let withdrawal = WireWithdrawalDebited {
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        asset_id: "USDC".to_owned(),
        amount: "25.250000".to_owned(),
        withdrawal_reference: "withdrawal-43".to_owned(),
    };
    let spot = WireSpotTransfer {
        from_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        to_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
        asset_id: "USDC".to_owned(),
        amount: "10.125000".to_owned(),
    };
    let perp = WirePerpTransfer {
        from_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        to_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
        quote_amount: "2000.125000".to_owned(),
    };
    let subaccount = WireSubaccountTransfer {
        master_account_id: "0x3333333333333333333333333333333333333333".to_owned(),
        from_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        to_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
        asset_id: "USDC".to_owned(),
        amount: "4.500000".to_owned(),
    };
    let vault_deposit = WireVaultDeposit {
        vault_id: "vault-alpha".to_owned(),
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        amount: "1000.000000".to_owned(),
        shares_issued: "10.50000000".to_owned(),
    };
    let vault_withdrawal = WireVaultWithdrawal {
        vault_id: "vault-alpha".to_owned(),
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        amount: "750.000000".to_owned(),
        shares_redeemed: "7.50000000".to_owned(),
    };

    assert_eq!(
        decode_deposit_credited(&encode_deposit_credited(&deposit).unwrap()).unwrap(),
        deposit
    );
    assert_eq!(
        decode_withdrawal_debited(&encode_withdrawal_debited(&withdrawal).unwrap()).unwrap(),
        withdrawal
    );
    assert_eq!(
        decode_spot_transfer(&encode_spot_transfer(&spot).unwrap()).unwrap(),
        spot
    );
    assert_eq!(
        decode_perp_transfer(&encode_perp_transfer(&perp).unwrap()).unwrap(),
        perp
    );
    assert_eq!(
        decode_subaccount_transfer(&encode_subaccount_transfer(&subaccount).unwrap()).unwrap(),
        subaccount
    );
    assert_eq!(
        decode_vault_deposit(&encode_vault_deposit(&vault_deposit).unwrap()).unwrap(),
        vault_deposit
    );
    assert_eq!(
        decode_vault_withdrawal(&encode_vault_withdrawal(&vault_withdrawal).unwrap()).unwrap(),
        vault_withdrawal
    );
    assert_eq!(
        encode_deposit_credited(&deposit).unwrap(),
        encode_deposit_credited(&deposit).unwrap()
    );

    let boundary_reference = WireDepositCredited {
        deposit_reference: "x".repeat(256),
        ..deposit
    };
    assert_eq!(
        decode_deposit_credited(&encode_deposit_credited(&boundary_reference).unwrap()).unwrap(),
        boundary_reference
    );

    let master_is_from = WireSubaccountTransfer {
        master_account_id: subaccount.from_account_id.clone(),
        ..subaccount
    };
    assert_eq!(
        decode_subaccount_transfer(&encode_subaccount_transfer(&master_is_from).unwrap()).unwrap(),
        master_is_from
    );
}

#[test]
fn account_fee_funding_reward_and_mode_wire_payloads_round_trip_exactly() {
    let account = "0x1111111111111111111111111111111111111111".to_owned();
    let other = "0x2222222222222222222222222222222222222222".to_owned();

    let fee = WireFeeCharged {
        account_id: account.clone(),
        asset_id: "USDC".to_owned(),
        amount: "1.250000".to_owned(),
        fee_rate: "0.000500".to_owned(),
        fee_type: "taker".to_owned(),
    };
    assert_eq!(
        decode_fee_charged(&encode_fee_charged(&fee).unwrap()).unwrap(),
        fee
    );

    let rebate = WireFeeCharged {
        account_id: account.clone(),
        asset_id: "USDC".to_owned(),
        amount: "0.250000".to_owned(),
        fee_rate: "-0.000100".to_owned(),
        fee_type: "maker_rebate".to_owned(),
    };
    assert_eq!(
        decode_fee_charged(&encode_fee_charged(&rebate).unwrap()).unwrap(),
        rebate
    );

    let builder = WireBuilderFeeCharged {
        account_id: account.clone(),
        builder_account_id: other.clone(),
        asset_id: "USDC".to_owned(),
        amount: "0.100000".to_owned(),
    };
    assert_eq!(
        decode_builder_fee_charged(&encode_builder_fee_charged(&builder).unwrap()).unwrap(),
        builder
    );

    let funding_paid = WireFundingPaid {
        account_id: account.clone(),
        market_id: "perp:BTC".to_owned(),
        amount: "3.500000".to_owned(),
        funding_rate: "-0.000125".to_owned(),
    };
    assert_eq!(
        decode_funding_paid(&encode_funding_paid(&funding_paid).unwrap()).unwrap(),
        funding_paid
    );

    let funding_received = WireFundingReceived {
        account_id: account.clone(),
        market_id: "perp:ETH".to_owned(),
        amount: "2.250000".to_owned(),
        funding_rate: "0.000075".to_owned(),
    };
    assert_eq!(
        decode_funding_received(&encode_funding_received(&funding_received).unwrap()).unwrap(),
        funding_received
    );

    let referral = WireReferralReward {
        account_id: account.clone(),
        referrer_account_id: other,
        asset_id: "USDC".to_owned(),
        amount: "0.500000".to_owned(),
    };
    assert_eq!(
        decode_referral_reward(&encode_referral_reward(&referral).unwrap()).unwrap(),
        referral
    );

    let account_mode = WireAccountModeChanged {
        account_id: account.clone(),
        previous_mode: "standard".to_owned(),
        new_mode: "unified".to_owned(),
    };
    assert_eq!(
        decode_account_mode_changed(&encode_account_mode_changed(&account_mode).unwrap()).unwrap(),
        account_mode
    );

    let margin_mode = WireMarginModeChanged {
        account_id: account.clone(),
        market_id: "perp:BTC".to_owned(),
        previous_mode: "cross".to_owned(),
        new_mode: "strict_isolated".to_owned(),
    };
    assert_eq!(
        decode_margin_mode_changed(&encode_margin_mode_changed(&margin_mode).unwrap()).unwrap(),
        margin_mode
    );

    let leverage = WireLeverageChanged {
        account_id: account,
        market_id: "perp:BTC".to_owned(),
        previous_leverage: "3".to_owned(),
        new_leverage: "5".to_owned(),
    };
    assert_eq!(
        decode_leverage_changed(&encode_leverage_changed(&leverage).unwrap()).unwrap(),
        leverage
    );
}

#[test]
fn liquidation_and_settlement_wire_payloads_round_trip_exactly() {
    let account = primary_account();
    let backstop = secondary_account();

    let started = WireLiquidationStarted {
        account_id: account.clone(),
        liquidation_id: "liquidation-42".to_owned(),
        margin_value: "99.000000".to_owned(),
        maintenance_requirement: "100.000000".to_owned(),
    };
    assert_eq!(
        decode_liquidation_started(&encode_liquidation_started(&started).unwrap()).unwrap(),
        started
    );

    let fill = WireLiquidationFill {
        liquidation_id: "liquidation-42".to_owned(),
        account_id: account.clone(),
        market_id: "perp:BTC".to_owned(),
        price: "65000.000000".to_owned(),
        quantity: "0.01000000".to_owned(),
    };
    assert_eq!(
        decode_liquidation_fill(&encode_liquidation_fill(&fill).unwrap()).unwrap(),
        fill
    );

    let backstop_fill = WireBackstopLiquidation {
        liquidation_id: "liquidation-42".to_owned(),
        account_id: account.clone(),
        backstop_account_id: backstop,
        market_id: "perp:BTC".to_owned(),
        quantity: "0.00500000".to_owned(),
    };
    assert_eq!(
        decode_backstop_liquidation(&encode_backstop_liquidation(&backstop_fill).unwrap()).unwrap(),
        backstop_fill
    );

    for settlement_price in ["0.000000", "63000.000000"] {
        for realized_pnl in ["-125.500000", "0.000000", "125.500000"] {
            let settled = WirePositionSettled {
                account_id: account.clone(),
                market_id: "perp:BTC".to_owned(),
                settlement_price: settlement_price.to_owned(),
                settled_quantity: "0.01000000".to_owned(),
                realized_pnl: realized_pnl.to_owned(),
            };
            assert_eq!(
                decode_position_settled(&encode_position_settled(&settled).unwrap()).unwrap(),
                settled
            );
        }
    }
}

#[test]
fn liquidation_and_settlement_wire_payloads_reject_invalid_boundaries() {
    let account = primary_account();
    let backstop = secondary_account();

    for invalid_id in ["", " liquidation-42", "liquidation-42 "] {
        assert!(
            encode_liquidation_started(&WireLiquidationStarted {
                account_id: account.clone(),
                liquidation_id: invalid_id.to_owned(),
                margin_value: "9.000000".to_owned(),
                maintenance_requirement: "10.000000".to_owned(),
            })
            .is_err(),
            "LiquidationStarted accepted liquidation_id {invalid_id:?}"
        );
    }

    for (margin_value, maintenance_requirement) in [
        ("-1.000000", "10.000000"),
        ("9.00000", "10.000000"),
        ("10.000000", "10.000000"),
        ("11.000000", "10.000000"),
        ("invalid", "10.000000"),
    ] {
        assert!(
            encode_liquidation_started(&WireLiquidationStarted {
                account_id: account.clone(),
                liquidation_id: "liquidation-42".to_owned(),
                margin_value: margin_value.to_owned(),
                maintenance_requirement: maintenance_requirement.to_owned(),
            })
            .is_err(),
            "LiquidationStarted accepted margin {margin_value:?} and maintenance {maintenance_requirement:?}"
        );
    }

    assert!(
        encode_backstop_liquidation(&WireBackstopLiquidation {
            liquidation_id: "liquidation-42".to_owned(),
            account_id: account.clone(),
            backstop_account_id: account.clone(),
            market_id: "perp:BTC".to_owned(),
            quantity: "1".to_owned(),
        })
        .is_err()
    );

    for invalid_account in [
        "",
        "0xAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        "0x111111111111111111111111111111111111111",
    ] {
        assert!(
            encode_position_settled(&WirePositionSettled {
                account_id: invalid_account.to_owned(),
                market_id: "perp:BTC".to_owned(),
                settlement_price: "0".to_owned(),
                settled_quantity: "1".to_owned(),
                realized_pnl: "-1".to_owned(),
            })
            .is_err()
        );
    }

    let valid = WireBackstopLiquidation {
        liquidation_id: "liquidation-42".to_owned(),
        account_id: account,
        backstop_account_id: backstop,
        market_id: "perp:BTC".to_owned(),
        quantity: "1".to_owned(),
    };
    let encoded = encode_backstop_liquidation(&valid).unwrap();
    assert!(matches!(
        decode_liquidation_fill(&encoded),
        Err(PayloadCodecError::KindMismatch { .. })
    ));
}

#[test]
fn account_fee_reward_and_mode_wire_payloads_reject_invalid_boundaries() {
    let account = "0x1111111111111111111111111111111111111111".to_owned();
    let other = "0x2222222222222222222222222222222222222222".to_owned();

    for invalid in ["Maker", " maker", "maker ", "unknown"] {
        assert!(
            encode_fee_charged(&WireFeeCharged {
                account_id: account.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                fee_rate: "0.001".to_owned(),
                fee_type: invalid.to_owned(),
            })
            .is_err()
        );
    }
    for (fee_type, fee_rate) in [
        ("maker_rebate", "0"),
        ("maker_rebate", "0.001"),
        ("maker", "0"),
        ("maker", "-0.001"),
        ("taker", "-0.001"),
        ("referral_discount", "0"),
        ("protocol", "-0.001"),
    ] {
        assert!(
            encode_fee_charged(&WireFeeCharged {
                account_id: account.clone(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                fee_rate: fee_rate.to_owned(),
                fee_type: fee_type.to_owned(),
            })
            .is_err(),
            "{fee_type} accepted fee rate {fee_rate}"
        );
    }

    assert!(
        encode_builder_fee_charged(&WireBuilderFeeCharged {
            account_id: account.clone(),
            builder_account_id: account.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_referral_reward(&WireReferralReward {
            account_id: account.clone(),
            referrer_account_id: account.clone(),
            asset_id: "USDC".to_owned(),
            amount: "1".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_account_mode_changed(&WireAccountModeChanged {
            account_id: account.clone(),
            previous_mode: "standard".to_owned(),
            new_mode: "standard".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_margin_mode_changed(&WireMarginModeChanged {
            account_id: account.clone(),
            market_id: "perp:BTC".to_owned(),
            previous_mode: "cross".to_owned(),
            new_mode: "cross".to_owned(),
        })
        .is_err()
    );
    assert!(
        encode_leverage_changed(&WireLeverageChanged {
            account_id: account,
            market_id: "perp:BTC".to_owned(),
            previous_leverage: "5".to_owned(),
            new_leverage: "5".to_owned(),
        })
        .is_err()
    );

    let valid = WireReferralReward {
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        referrer_account_id: other,
        asset_id: "USDC".to_owned(),
        amount: "1".to_owned(),
    };
    let encoded = encode_referral_reward(&valid).unwrap();
    assert!(matches!(
        decode_fee_charged(&encoded),
        Err(PayloadCodecError::KindMismatch { .. })
    ));
}

#[test]
fn account_cash_flow_wire_payloads_reject_missing_padded_and_unsafe_fields() {
    let valid = WireDepositCredited {
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        asset_id: "USDC".to_owned(),
        amount: "125.500000".to_owned(),
        deposit_reference: "deposit-42".to_owned(),
    };
    for invalid in [
        WireDepositCredited {
            account_id: String::new(),
            ..valid.clone()
        },
        WireDepositCredited {
            asset_id: " USDC".to_owned(),
            ..valid.clone()
        },
        WireDepositCredited {
            amount: "125.5 ".to_owned(),
            ..valid.clone()
        },
        WireDepositCredited {
            deposit_reference: String::new(),
            ..valid.clone()
        },
        WireDepositCredited {
            deposit_reference: "deposit\n42".to_owned(),
            ..valid.clone()
        },
        WireDepositCredited {
            deposit_reference: "x".repeat(257),
            ..valid.clone()
        },
    ] {
        assert!(matches!(
            encode_deposit_credited(&invalid),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }

    let encoded = encode_deposit_credited(&valid).unwrap();
    assert!(matches!(
        decode_withdrawal_debited(&encoded),
        Err(PayloadCodecError::KindMismatch { .. })
    ));
}

#[test]
fn account_cash_flow_payload_bound_is_inclusive_and_stable() {
    let seed_asset_bytes = ACCOUNT_PAYLOAD_LIMIT - 1_024;
    let seed = encode_deposit_credited(&deposit_with_asset_bytes(seed_asset_bytes)).unwrap();
    assert!(seed.len() < ACCOUNT_PAYLOAD_LIMIT);

    let exact_asset_bytes = seed_asset_bytes + ACCOUNT_PAYLOAD_LIMIT - seed.len();
    let exact_value = deposit_with_asset_bytes(exact_asset_bytes);
    let exact = encode_deposit_credited(&exact_value).unwrap();
    assert_eq!(exact.len(), ACCOUNT_PAYLOAD_LIMIT);
    assert_eq!(decode_deposit_credited(&exact).unwrap(), exact_value);

    assert_account_payload_size_result(
        encode_deposit_credited(&deposit_with_asset_bytes(exact_asset_bytes + 1)),
        "DepositCredited",
    );

    let huge_probe =
        std::panic::catch_unwind(|| encode_deposit_credited(&deposit_with_asset_bytes(70_000)));
    let huge_probe_error = huge_probe
        .expect("the public encoder must return an error rather than panic")
        .unwrap_err();
    assert_account_payload_size_error(huge_probe_error, "DepositCredited");

    let oversized_malformed = vec![0xff; ACCOUNT_PAYLOAD_LIMIT + 1];
    let decode_error = decode_deposit_credited(&oversized_malformed).unwrap_err();
    assert_account_payload_size_error(decode_error, "DepositCredited");
}

#[test]
fn every_task1_account_codec_enforces_the_shared_payload_bound() {
    let account = "0x1111111111111111111111111111111111111111".to_owned();
    let other_account = "0x2222222222222222222222222222222222222222".to_owned();
    let oversized = "X".repeat(70_000);

    let encoder_results = [
        (
            "DepositCredited",
            encode_deposit_credited(&WireDepositCredited {
                account_id: account.clone(),
                asset_id: oversized.clone(),
                amount: "1".to_owned(),
                deposit_reference: "deposit-42".to_owned(),
            }),
        ),
        (
            "WithdrawalDebited",
            encode_withdrawal_debited(&WireWithdrawalDebited {
                account_id: account.clone(),
                asset_id: oversized.clone(),
                amount: "1".to_owned(),
                withdrawal_reference: "withdrawal-42".to_owned(),
            }),
        ),
        (
            "SpotTransfer",
            encode_spot_transfer(&WireSpotTransfer {
                from_account_id: account.clone(),
                to_account_id: other_account.clone(),
                asset_id: oversized.clone(),
                amount: "1".to_owned(),
            }),
        ),
        (
            "PerpTransfer",
            encode_perp_transfer(&WirePerpTransfer {
                from_account_id: account.clone(),
                to_account_id: other_account.clone(),
                quote_amount: oversized.clone(),
            }),
        ),
        (
            "SubaccountTransfer",
            encode_subaccount_transfer(&WireSubaccountTransfer {
                master_account_id: account.clone(),
                from_account_id: account.clone(),
                to_account_id: other_account.clone(),
                asset_id: oversized.clone(),
                amount: "1".to_owned(),
            }),
        ),
        (
            "VaultDeposit",
            encode_vault_deposit(&WireVaultDeposit {
                vault_id: oversized.clone(),
                account_id: account.clone(),
                amount: "1".to_owned(),
                shares_issued: "1".to_owned(),
            }),
        ),
        (
            "VaultWithdrawal",
            encode_vault_withdrawal(&WireVaultWithdrawal {
                vault_id: oversized,
                account_id: account,
                amount: "1".to_owned(),
                shares_redeemed: "1".to_owned(),
            }),
        ),
    ];
    for (kind, result) in encoder_results {
        assert_account_payload_size_result(result, kind);
    }

    let oversized_malformed = vec![0xff; ACCOUNT_PAYLOAD_LIMIT + 1];
    let decoder_results = [
        (
            "DepositCredited",
            decode_deposit_credited(&oversized_malformed).map(|_| ()),
        ),
        (
            "WithdrawalDebited",
            decode_withdrawal_debited(&oversized_malformed).map(|_| ()),
        ),
        (
            "SpotTransfer",
            decode_spot_transfer(&oversized_malformed).map(|_| ()),
        ),
        (
            "PerpTransfer",
            decode_perp_transfer(&oversized_malformed).map(|_| ()),
        ),
        (
            "SubaccountTransfer",
            decode_subaccount_transfer(&oversized_malformed).map(|_| ()),
        ),
        (
            "VaultDeposit",
            decode_vault_deposit(&oversized_malformed).map(|_| ()),
        ),
        (
            "VaultWithdrawal",
            decode_vault_withdrawal(&oversized_malformed).map(|_| ()),
        ),
    ];
    for (kind, result) in decoder_results {
        match result {
            Err(error) => assert_account_payload_size_error(error, kind),
            Ok(()) => panic!("{kind} oversized decoder unexpectedly succeeded"),
        }
    }
}

#[test]
fn every_task2_public_encoder_and_direct_decoder_enforces_the_shared_payload_bound() {
    for (kind, encoder) in [
        (
            "FeeCharged",
            encode_fee_with_asset_bytes as fn(usize) -> Result<Vec<u8>, PayloadCodecError>,
        ),
        ("BuilderFeeCharged", encode_builder_fee_with_asset_bytes),
        ("FundingPaid", encode_funding_paid_with_market_bytes),
        ("FundingReceived", encode_funding_received_with_market_bytes),
        ("ReferralReward", encode_referral_reward_with_asset_bytes),
        ("MarginModeChanged", encode_margin_mode_with_market_bytes),
        ("LeverageChanged", encode_leverage_with_market_bytes),
    ] {
        assert_encoder_exact_one_over_and_70k(kind, encoder);
    }

    let account_modes = ["standard", "unified", "portfolio", "dex_abstraction"];
    // AccountModeChanged is structurally bounded: the address validator admits
    // exactly 42 bytes and both modes come from this frozen four-value set.
    // Exhausting every valid transition therefore covers every encodable size.
    for previous_mode in account_modes {
        for new_mode in account_modes {
            if previous_mode == new_mode {
                continue;
            }
            let value = WireAccountModeChanged {
                account_id: primary_account(),
                previous_mode: previous_mode.to_owned(),
                new_mode: new_mode.to_owned(),
            };
            let encoded = encode_account_mode_changed(&value).unwrap();
            assert!(
                encoded.len() < ACCOUNT_PAYLOAD_LIMIT,
                "the fixed-width AccountModeChanged encoder exceeded the account bound"
            );
            assert_eq!(decode_account_mode_changed(&encoded).unwrap(), value);
        }
    }
    for probe_bytes in [ACCOUNT_PAYLOAD_LIMIT, ACCOUNT_PAYLOAD_LIMIT + 1, 70_000] {
        let outcome = std::panic::catch_unwind(|| {
            encode_account_mode_changed(&WireAccountModeChanged {
                account_id: primary_account(),
                previous_mode: "standard".to_owned(),
                new_mode: "X".repeat(probe_bytes),
            })
        });
        assert!(matches!(
            outcome.unwrap_or_else(|_| {
                panic!("AccountModeChanged public encoder panicked on malformed input")
            }),
            Err(PayloadCodecError::Invalid { kind, .. }) if kind == "AccountModeChanged"
        ));
    }

    type Decoder = fn(&[u8]) -> Result<(), PayloadCodecError>;
    let decoders: [(&str, Decoder); 8] = [
        ("FeeCharged", |bytes| decode_fee_charged(bytes).map(|_| ())),
        ("BuilderFeeCharged", |bytes| {
            decode_builder_fee_charged(bytes).map(|_| ())
        }),
        ("FundingPaid", |bytes| {
            decode_funding_paid(bytes).map(|_| ())
        }),
        ("FundingReceived", |bytes| {
            decode_funding_received(bytes).map(|_| ())
        }),
        ("ReferralReward", |bytes| {
            decode_referral_reward(bytes).map(|_| ())
        }),
        ("AccountModeChanged", |bytes| {
            decode_account_mode_changed(bytes).map(|_| ())
        }),
        ("MarginModeChanged", |bytes| {
            decode_margin_mode_changed(bytes).map(|_| ())
        }),
        ("LeverageChanged", |bytes| {
            decode_leverage_changed(bytes).map(|_| ())
        }),
    ];
    for (kind, decode) in decoders {
        let default = encode_default_event_payload(kind).unwrap();

        let exact = pad_outer_unknown_to_exact_size(&default, ACCOUNT_PAYLOAD_LIMIT);
        assert_eq!(exact.len(), ACCOUNT_PAYLOAD_LIMIT);
        decode(&exact).unwrap_or_else(|error| {
            panic!("{kind} direct decoder rejected the inclusive boundary: {error}")
        });

        let exact_malformed = vec![0xff; ACCOUNT_PAYLOAD_LIMIT];
        assert!(matches!(
            decode(&exact_malformed),
            Err(PayloadCodecError::Decode {
                kind: decode_kind,
                ..
            }) if decode_kind == "TypedPayloadEnvelope"
        ));

        for probe in [
            pad_outer_unknown_to_exact_size(&default, ACCOUNT_PAYLOAD_LIMIT + 1),
            vec![0xff; ACCOUNT_PAYLOAD_LIMIT + 1],
            pad_outer_unknown_to_exact_size(&default, 70_000),
            vec![0xff; 70_000],
        ] {
            let outcome = std::panic::catch_unwind(|| decode(&probe));
            let error = outcome
                .unwrap_or_else(|_| panic!("{kind} public decoder panicked on oversized input"))
                .unwrap_err();
            assert_account_payload_size_error(error, kind);
        }
    }
}

#[test]
fn every_task3_public_encoder_and_direct_decoder_enforces_the_shared_payload_bound() {
    for (kind, encoder) in [
        (
            "LiquidationStarted",
            encode_liquidation_started_with_id_bytes
                as fn(usize) -> Result<Vec<u8>, PayloadCodecError>,
        ),
        ("LiquidationFill", encode_liquidation_fill_with_market_bytes),
        (
            "BackstopLiquidation",
            encode_backstop_liquidation_with_market_bytes,
        ),
        ("PositionSettled", encode_position_settled_with_market_bytes),
    ] {
        assert_encoder_exact_one_over_and_70k(kind, encoder);
    }

    type Decoder = fn(&[u8]) -> Result<(), PayloadCodecError>;
    let decoders: [(&str, Decoder); 4] = [
        ("LiquidationStarted", |bytes| {
            decode_liquidation_started(bytes).map(|_| ())
        }),
        ("LiquidationFill", |bytes| {
            decode_liquidation_fill(bytes).map(|_| ())
        }),
        ("BackstopLiquidation", |bytes| {
            decode_backstop_liquidation(bytes).map(|_| ())
        }),
        ("PositionSettled", |bytes| {
            decode_position_settled(bytes).map(|_| ())
        }),
    ];
    for (kind, decode) in decoders {
        let default = encode_default_event_payload(kind).unwrap();

        let exact = pad_outer_unknown_to_exact_size(&default, ACCOUNT_PAYLOAD_LIMIT);
        assert_eq!(exact.len(), ACCOUNT_PAYLOAD_LIMIT);
        decode(&exact).unwrap_or_else(|error| {
            panic!("{kind} direct decoder rejected the inclusive boundary: {error}")
        });

        let exact_malformed = vec![0xff; ACCOUNT_PAYLOAD_LIMIT];
        assert!(matches!(
            decode(&exact_malformed),
            Err(PayloadCodecError::Decode {
                kind: decode_kind,
                ..
            }) if decode_kind == "TypedPayloadEnvelope"
        ));

        for probe in [
            pad_outer_unknown_to_exact_size(&default, ACCOUNT_PAYLOAD_LIMIT + 1),
            vec![0xff; ACCOUNT_PAYLOAD_LIMIT + 1],
            pad_outer_unknown_to_exact_size(&default, 70_000),
            vec![0xff; 70_000],
        ] {
            let outcome = std::panic::catch_unwind(|| decode(&probe));
            let error = outcome
                .unwrap_or_else(|_| panic!("{kind} public decoder panicked on oversized input"))
                .unwrap_err();
            assert_account_payload_size_error(error, kind);
        }
    }
}

#[test]
fn generic_validator_preflights_every_account_payload_size() {
    for kind in [
        "DepositCredited",
        "WithdrawalDebited",
        "SpotTransfer",
        "PerpTransfer",
        "SubaccountTransfer",
        "VaultDeposit",
        "VaultWithdrawal",
        "FeeCharged",
        "BuilderFeeCharged",
        "FundingPaid",
        "FundingReceived",
        "ReferralReward",
        "AccountModeChanged",
        "MarginModeChanged",
        "LeverageChanged",
        "LiquidationStarted",
        "LiquidationFill",
        "BackstopLiquidation",
        "PositionSettled",
    ] {
        let default = encode_default_event_payload(kind).unwrap();

        let exact = pad_outer_unknown_to_exact_size(&default, ACCOUNT_PAYLOAD_LIMIT);
        assert_eq!(exact.len(), ACCOUNT_PAYLOAD_LIMIT);
        validate_event_payload(kind, &exact)
            .unwrap_or_else(|error| panic!("{kind} exact-bound payload failed: {error}"));

        let one_over_well_formed =
            pad_outer_unknown_to_exact_size(&default, ACCOUNT_PAYLOAD_LIMIT + 1);
        assert_eq!(one_over_well_formed.len(), ACCOUNT_PAYLOAD_LIMIT + 1);
        let error = validate_event_payload(kind, &one_over_well_formed).unwrap_err();
        assert_account_payload_size_error(error, kind);

        let one_over_malformed = vec![0xff; ACCOUNT_PAYLOAD_LIMIT + 1];
        let error = validate_event_payload(kind, &one_over_malformed).unwrap_err();
        assert_account_payload_size_error(error, kind);

        for probe in [
            pad_outer_unknown_to_exact_size(&default, 70_000),
            vec![0xff; 70_000],
        ] {
            let outcome =
                std::panic::catch_unwind(|| validate_event_payload(kind, probe.as_slice()));
            let error = outcome
                .unwrap_or_else(|_| panic!("{kind} generic validator panicked on a 70k probe"))
                .unwrap_err();
            assert_account_payload_size_error(error, kind);
        }
    }
}

#[test]
fn generic_validator_preserves_unrelated_kind_behavior() {
    let kind = "TriggerOrderActivated";
    let valid = encode_default_event_payload(kind).unwrap();
    validate_event_payload(kind, &valid).unwrap();

    let oversized_malformed = vec![0xff; ACCOUNT_PAYLOAD_LIMIT + 1];
    assert!(matches!(
        validate_event_payload(kind, &oversized_malformed),
        Err(PayloadCodecError::Decode {
            kind: decode_kind,
            ..
        }) if decode_kind == "TypedPayloadEnvelope"
    ));
}

#[test]
fn strict_account_default_payloads_validate_deterministically() {
    for kind in [
        "DepositCredited",
        "WithdrawalDebited",
        "SpotTransfer",
        "PerpTransfer",
        "SubaccountTransfer",
        "VaultDeposit",
        "VaultWithdrawal",
        "FeeCharged",
        "BuilderFeeCharged",
        "FundingPaid",
        "FundingReceived",
        "ReferralReward",
        "AccountModeChanged",
        "MarginModeChanged",
        "LeverageChanged",
        "LiquidationStarted",
        "LiquidationFill",
        "BackstopLiquidation",
        "PositionSettled",
    ] {
        let first = encode_default_event_payload(kind).unwrap();
        let second = encode_default_event_payload(kind).unwrap();
        assert_eq!(
            first, second,
            "{kind} default payload must be deterministic"
        );
        validate_event_payload(kind, &first)
            .unwrap_or_else(|error| panic!("{kind} default payload must validate: {error}"));
    }
}

#[test]
fn order_admission_wire_payloads_round_trip_exactly() {
    let accepted = WireOrderAccepted {
        order_id: "order-17".to_owned(),
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        market_id: "perp:BTC".to_owned(),
        side: "buy".to_owned(),
        limit_price: "65000.125000".to_owned(),
        quantity: "0.75000000".to_owned(),
    };
    assert_eq!(
        decode_order_accepted(&encode_order_accepted(&accepted).unwrap()).unwrap(),
        accepted
    );

    let rested = WireOrderRested {
        order_id: "order-17".to_owned(),
        market_id: "perp:BTC".to_owned(),
        remaining_quantity: "0.75000000".to_owned(),
        limit_price: "65000.125000".to_owned(),
    };
    assert_eq!(
        decode_order_rested(&encode_order_rested(&rested).unwrap()).unwrap(),
        rested
    );

    let modified = WireOrderModified {
        order_id: "order-17".to_owned(),
        previous_price: "65000.125000".to_owned(),
        new_price: "65001.000000".to_owned(),
        previous_quantity: "0.75000000".to_owned(),
        new_quantity: "0.50000000".to_owned(),
    };
    assert_eq!(
        decode_order_modified(&encode_order_modified(&modified).unwrap()).unwrap(),
        modified
    );
}

#[test]
fn order_wire_payloads_reject_missing_padded_and_wrong_kind_fields() {
    let accepted = WireOrderAccepted {
        order_id: "order-17".to_owned(),
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        market_id: "perp:BTC".to_owned(),
        side: "buy".to_owned(),
        limit_price: "65000.125000".to_owned(),
        quantity: "0.75000000".to_owned(),
    };
    for mutate in [
        |value: &mut WireOrderAccepted| value.order_id.clear(),
        |value: &mut WireOrderAccepted| value.account_id = " account".to_owned(),
        |value: &mut WireOrderAccepted| value.market_id = "perp:BTC ".to_owned(),
        |value: &mut WireOrderAccepted| value.side.clear(),
        |value: &mut WireOrderAccepted| value.limit_price.clear(),
        |value: &mut WireOrderAccepted| value.quantity.clear(),
    ] {
        let mut invalid = accepted.clone();
        mutate(&mut invalid);
        assert!(matches!(
            encode_order_accepted(&invalid),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }
    let encoded = encode_order_accepted(&accepted).unwrap();
    assert!(matches!(
        decode_order_rested(&encoded),
        Err(PayloadCodecError::KindMismatch { .. })
    ));
}

#[test]
fn order_outcome_wire_payloads_round_trip_exactly() {
    let partial = WireOrderPartiallyFilled {
        order_id: "order-17".to_owned(),
        trade_id: "trade-18".to_owned(),
        fill_price: "65000.125000".to_owned(),
        fill_quantity: "0.25000000".to_owned(),
        remaining_quantity: "0.50000000".to_owned(),
    };
    assert_eq!(
        decode_order_partially_filled(&encode_order_partially_filled(&partial).unwrap()).unwrap(),
        partial
    );

    let filled = WireOrderFilled {
        order_id: "order-17".to_owned(),
        trade_id: "trade-19".to_owned(),
        fill_price: "65001.000000".to_owned(),
        fill_quantity: "0.50000000".to_owned(),
    };
    assert_eq!(
        decode_order_filled(&encode_order_filled(&filled).unwrap()).unwrap(),
        filled
    );

    let cancelled = WireOrderCancelled {
        order_id: "order-20".to_owned(),
        reason: "operator_requested".to_owned(),
        remaining_quantity: "0.12500000".to_owned(),
    };
    assert_eq!(
        decode_order_cancelled(&encode_order_cancelled(&cancelled).unwrap()).unwrap(),
        cancelled
    );

    let rejected = WireOrderRejected {
        client_order_id: "client-21".to_owned(),
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        reason_code: "invalid_tick".to_owned(),
        reason: "limit price is not aligned to the active tick".to_owned(),
    };
    assert_eq!(
        decode_order_rejected(&encode_order_rejected(&rejected).unwrap()).unwrap(),
        rejected
    );
}

#[test]
fn order_outcome_wire_payloads_reject_unsafe_reasons_and_missing_identity() {
    let invalid_rejections = [
        WireOrderRejected {
            client_order_id: String::new(),
            account_id: "0x1111111111111111111111111111111111111111".to_owned(),
            reason_code: "invalid_tick".to_owned(),
            reason: "invalid tick".to_owned(),
        },
        WireOrderRejected {
            client_order_id: "client-21".to_owned(),
            account_id: " account".to_owned(),
            reason_code: "invalid_tick".to_owned(),
            reason: "invalid tick".to_owned(),
        },
        WireOrderRejected {
            client_order_id: "client-21".to_owned(),
            account_id: "0x1111111111111111111111111111111111111111".to_owned(),
            reason_code: "invalid\ncode".to_owned(),
            reason: "invalid tick".to_owned(),
        },
        WireOrderRejected {
            client_order_id: "client-21".to_owned(),
            account_id: "0x1111111111111111111111111111111111111111".to_owned(),
            reason_code: "x".repeat(129),
            reason: "invalid tick".to_owned(),
        },
        WireOrderRejected {
            client_order_id: "client-21".to_owned(),
            account_id: "0x1111111111111111111111111111111111111111".to_owned(),
            reason_code: "invalid_tick".to_owned(),
            reason: "x".repeat(1_025),
        },
    ];
    for invalid in invalid_rejections {
        assert!(matches!(
            encode_order_rejected(&invalid),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }
}

#[test]
fn market_metadata_wire_payloads_round_trip_exactly() {
    let dex = WireDexCreated {
        dex_id: "validator".to_owned(),
        name: "Hyperliquid Validator Perpetuals".to_owned(),
        operator_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
    };
    assert_eq!(
        decode_dex_created(&encode_dex_created(&dex).unwrap()).unwrap(),
        dex
    );

    let asset = WireAssetContextUpdated {
        asset_id: "USDC".to_owned(),
        context_version: "asset-context-7".to_owned(),
        context_hash: vec![0x22; 32],
    };
    assert_eq!(
        decode_asset_context_updated(&encode_asset_context_updated(&asset).unwrap()).unwrap(),
        asset
    );

    let market = WireMarketCreated {
        market_id: "perp:BTC".to_owned(),
        dex_id: "validator".to_owned(),
        base_asset_id: "BTC".to_owned(),
        quote_asset_id: "USDC".to_owned(),
        tick_size: "0.100000".to_owned(),
        lot_size: "0.00001000".to_owned(),
    };
    assert_eq!(
        decode_market_created(&encode_market_created(&market).unwrap()).unwrap(),
        market
    );

    let changed = WireMarketMetadataChanged {
        market_id: "perp:BTC".to_owned(),
        metadata_version: "market-metadata-8".to_owned(),
        metadata_hash: vec![0x33; 32],
    };
    assert_eq!(
        decode_market_metadata_changed(&encode_market_metadata_changed(&changed).unwrap()).unwrap(),
        changed
    );
}

#[test]
fn market_metadata_wire_payloads_reject_ambiguous_or_unbounded_fields() {
    let invalid_dexes = [
        WireDexCreated {
            dex_id: String::new(),
            name: "Validator".to_owned(),
            operator_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        },
        WireDexCreated {
            dex_id: "validator".to_owned(),
            name: "Validator\nPerpetuals".to_owned(),
            operator_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        },
        WireDexCreated {
            dex_id: "validator".to_owned(),
            name: "x".repeat(257),
            operator_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        },
    ];
    for invalid in invalid_dexes {
        assert!(matches!(
            encode_dex_created(&invalid),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }

    let invalid_assets = [
        WireAssetContextUpdated {
            asset_id: " USDC".to_owned(),
            context_version: "asset-context-7".to_owned(),
            context_hash: vec![0x22; 32],
        },
        WireAssetContextUpdated {
            asset_id: "USDC".to_owned(),
            context_version: "asset\ncontext".to_owned(),
            context_hash: vec![0x22; 32],
        },
        WireAssetContextUpdated {
            asset_id: "USDC".to_owned(),
            context_version: "asset-context-7".to_owned(),
            context_hash: vec![0x22; 31],
        },
    ];
    for invalid in invalid_assets {
        assert!(matches!(
            encode_asset_context_updated(&invalid),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }

    let invalid_market = WireMarketCreated {
        market_id: "perp:BTC".to_owned(),
        dex_id: "validator".to_owned(),
        base_asset_id: "BTC".to_owned(),
        quote_asset_id: "USDC".to_owned(),
        tick_size: String::new(),
        lot_size: "0.00001000".to_owned(),
    };
    assert!(matches!(
        encode_market_created(&invalid_market),
        Err(PayloadCodecError::Invalid { .. })
    ));

    let invalid_change = WireMarketMetadataChanged {
        market_id: "perp:BTC".to_owned(),
        metadata_version: "x".repeat(129),
        metadata_hash: vec![0x33; 32],
    };
    assert!(matches!(
        encode_market_metadata_changed(&invalid_change),
        Err(PayloadCodecError::Invalid { .. })
    ));
}

#[test]
fn market_metadata_wire_payloads_reject_wrong_kind() {
    let encoded = encode_dex_created(&WireDexCreated {
        dex_id: "validator".to_owned(),
        name: "Validator".to_owned(),
        operator_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
    })
    .unwrap();
    assert!(matches!(
        decode_market_created(&encoded),
        Err(PayloadCodecError::KindMismatch { .. })
    ));
}

#[test]
fn market_status_wire_payloads_round_trip_and_reject_ambiguous_transitions() {
    let halted = WireMarketHalted {
        market_id: "perp:BTC".to_owned(),
        reason: "scheduled_protocol_upgrade".to_owned(),
    };
    assert_eq!(
        decode_market_halted(&encode_market_halted(&halted).unwrap()).unwrap(),
        halted
    );

    let resumed = WireMarketResumed {
        market_id: "perp:BTC".to_owned(),
        reason: "upgrade_complete".to_owned(),
    };
    assert_eq!(
        decode_market_resumed(&encode_market_resumed(&resumed).unwrap()).unwrap(),
        resumed
    );

    let cap = WireOpenInterestCapChanged {
        market_id: "perp:BTC".to_owned(),
        previous_cap: "100000000".to_owned(),
        new_cap: "125000000".to_owned(),
    };
    assert_eq!(
        decode_open_interest_cap_changed(&encode_open_interest_cap_changed(&cap).unwrap()).unwrap(),
        cap
    );

    let margin = WireMarginTableChanged {
        market_id: "perp:BTC".to_owned(),
        previous_table_hash: "margin-table-v7".to_owned(),
        new_table_hash: "margin-table-v8".to_owned(),
    };
    assert_eq!(
        decode_margin_table_changed(&encode_margin_table_changed(&margin).unwrap()).unwrap(),
        margin
    );

    for invalid in [
        WireMarketHalted {
            market_id: String::new(),
            reason: "scheduled".to_owned(),
        },
        WireMarketHalted {
            market_id: "perp:BTC".to_owned(),
            reason: "unsafe\nreason".to_owned(),
        },
        WireMarketHalted {
            market_id: "perp:BTC".to_owned(),
            reason: "x".repeat(1_025),
        },
    ] {
        assert!(matches!(
            encode_market_halted(&invalid),
            Err(PayloadCodecError::Invalid { .. })
        ));
    }

    assert!(matches!(
        encode_open_interest_cap_changed(&WireOpenInterestCapChanged {
            market_id: "perp:BTC".to_owned(),
            previous_cap: "100".to_owned(),
            new_cap: "100".to_owned(),
        }),
        Err(PayloadCodecError::Invalid { .. })
    ));
    assert!(matches!(
        encode_margin_table_changed(&WireMarginTableChanged {
            market_id: "perp:BTC".to_owned(),
            previous_table_hash: "same".to_owned(),
            new_table_hash: "same".to_owned(),
        }),
        Err(PayloadCodecError::Invalid { .. })
    ));
}

#[test]
fn market_valuation_and_outcome_wire_payloads_round_trip_and_bound_time() {
    let oracle = WireOracleUpdated {
        market_id: "perp:BTC".to_owned(),
        oracle_price: "65000.125000".to_owned(),
        source: "hyperliquid-validator-oracle".to_owned(),
        effective_at_micros: 1_721_779_200_000_042,
    };
    assert_eq!(
        decode_oracle_updated(&encode_oracle_updated(&oracle).unwrap()).unwrap(),
        oracle
    );

    let funding = WireFundingRateUpdated {
        market_id: "perp:BTC".to_owned(),
        funding_rate: "-0.00001250".to_owned(),
        effective_at_micros: 1_721_779_200_000_043,
    };
    assert_eq!(
        decode_funding_rate_updated(&encode_funding_rate_updated(&funding).unwrap()).unwrap(),
        funding
    );

    let created = WireOutcomeCreated {
        market_id: "outcome:presidential-election".to_owned(),
        outcome_id: "candidate-a".to_owned(),
        description: "Candidate A wins the election".to_owned(),
    };
    assert_eq!(
        decode_outcome_created(&encode_outcome_created(&created).unwrap()).unwrap(),
        created
    );

    let resolved = WireOutcomeResolved {
        market_id: "outcome:presidential-election".to_owned(),
        outcome_id: "candidate-a".to_owned(),
        settlement_value: "1.000000".to_owned(),
        resolved_at_micros: 1_730_000_000_000_000,
    };
    assert_eq!(
        decode_outcome_resolved(&encode_outcome_resolved(&resolved).unwrap()).unwrap(),
        resolved
    );

    assert!(matches!(
        encode_oracle_updated(&WireOracleUpdated {
            effective_at_micros: -1,
            ..oracle
        }),
        Err(PayloadCodecError::Invalid { .. })
    ));
    assert!(matches!(
        encode_funding_rate_updated(&WireFundingRateUpdated {
            effective_at_micros: -1,
            ..funding
        }),
        Err(PayloadCodecError::Invalid { .. })
    ));
    assert!(matches!(
        encode_outcome_created(&WireOutcomeCreated {
            description: "unsafe\noutcome".to_owned(),
            ..created
        }),
        Err(PayloadCodecError::Invalid { .. })
    ));
    assert!(matches!(
        encode_outcome_resolved(&WireOutcomeResolved {
            resolved_at_micros: -1,
            ..resolved
        }),
        Err(PayloadCodecError::Invalid { .. })
    ));
}
