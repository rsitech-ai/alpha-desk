use api_contracts::{
    MAX_CANONICAL_ACCOUNT_PAYLOAD_BYTES, PayloadCodecError, WireAssetContextUpdated,
    WireDepositCredited, WireDexCreated, WireFundingRateUpdated, WireMarginTableChanged,
    WireMarketCreated, WireMarketHalted, WireMarketMetadataChanged, WireMarketResumed,
    WireOpenInterestCapChanged, WireOracleUpdated, WireOrderAccepted, WireOrderCancelled,
    WireOrderFilled, WireOrderModified, WireOrderPartiallyFilled, WireOrderRejected,
    WireOrderRested, WireOutcomeCreated, WireOutcomeResolved, WirePerpTransfer, WireSpotTransfer,
    WireSubaccountTransfer, WireTradeMatched, WireVaultDeposit, WireVaultWithdrawal,
    WireWithdrawalDebited, decode_asset_context_updated, decode_deposit_credited,
    decode_dex_created, decode_funding_rate_updated, decode_margin_table_changed,
    decode_market_created, decode_market_halted, decode_market_metadata_changed,
    decode_market_resumed, decode_open_interest_cap_changed, decode_oracle_updated,
    decode_order_accepted, decode_order_cancelled, decode_order_filled, decode_order_modified,
    decode_order_partially_filled, decode_order_rejected, decode_order_rested,
    decode_outcome_created, decode_outcome_resolved, decode_perp_transfer, decode_spot_transfer,
    decode_subaccount_transfer, decode_trade_matched, decode_vault_deposit,
    decode_vault_withdrawal, decode_withdrawal_debited, encode_asset_context_updated,
    encode_default_event_payload, encode_deposit_credited, encode_dex_created,
    encode_funding_rate_updated, encode_margin_table_changed, encode_market_created,
    encode_market_halted, encode_market_metadata_changed, encode_market_resumed,
    encode_open_interest_cap_changed, encode_oracle_updated, encode_order_accepted,
    encode_order_cancelled, encode_order_filled, encode_order_modified,
    encode_order_partially_filled, encode_order_rejected, encode_order_rested,
    encode_outcome_created, encode_outcome_resolved, encode_perp_transfer, encode_spot_transfer,
    encode_subaccount_transfer, encode_trade_matched, encode_vault_deposit,
    encode_vault_withdrawal, encode_withdrawal_debited, validate_event_payload,
};

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
    }
}

fn deposit_with_asset_bytes(asset_bytes: usize) -> WireDepositCredited {
    WireDepositCredited {
        account_id: "0x1111111111111111111111111111111111111111".to_owned(),
        asset_id: "A".repeat(asset_bytes),
        amount: "1".to_owned(),
        deposit_reference: "deposit-42".to_owned(),
    }
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
fn every_account_cash_flow_codec_enforces_the_shared_payload_bound() {
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
fn generic_validator_preflights_every_account_payload_size() {
    for kind in [
        "DepositCredited",
        "WithdrawalDebited",
        "SpotTransfer",
        "PerpTransfer",
        "SubaccountTransfer",
        "VaultDeposit",
        "VaultWithdrawal",
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
    let kind = "FeeCharged";
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
