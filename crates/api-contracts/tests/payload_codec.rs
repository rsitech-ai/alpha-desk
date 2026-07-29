use api_contracts::{
    PayloadCodecError, WireAssetContextUpdated, WireDexCreated, WireMarketCreated,
    WireMarketMetadataChanged, WireOrderAccepted, WireOrderCancelled, WireOrderFilled,
    WireOrderModified, WireOrderPartiallyFilled, WireOrderRejected, WireOrderRested,
    WireTradeMatched, decode_asset_context_updated, decode_dex_created, decode_market_created,
    decode_market_metadata_changed, decode_order_accepted, decode_order_cancelled,
    decode_order_filled, decode_order_modified, decode_order_partially_filled,
    decode_order_rejected, decode_order_rested, decode_trade_matched, encode_asset_context_updated,
    encode_dex_created, encode_market_created, encode_market_metadata_changed,
    encode_order_accepted, encode_order_cancelled, encode_order_filled, encode_order_modified,
    encode_order_partially_filled, encode_order_rejected, encode_order_rested,
    encode_trade_matched,
};

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
