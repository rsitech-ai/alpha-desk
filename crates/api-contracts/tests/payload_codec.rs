use api_contracts::{
    PayloadCodecError, WireTradeMatched, decode_trade_matched, encode_trade_matched,
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
