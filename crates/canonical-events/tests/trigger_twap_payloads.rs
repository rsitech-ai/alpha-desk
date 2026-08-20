use std::str::FromStr;

use api_contracts::{
    WireTriggerOrderActivated, WireTwapCompleted, WireTwapSliceFilled, WireTwapStarted,
    encode_trigger_order_activated, encode_twap_completed, encode_twap_slice_filled,
    encode_twap_started,
};
use canonical_events::{
    ContractError, EventKind, EventPayload, TriggerOrderActivated, TwapCompleted, TwapSliceFilled,
    TwapStarted,
};
use domain_types::{Address, MarketId, OrderId, Price, ProtocolTime, Quantity};

#[test]
fn trigger_and_twap_payloads_decode_to_exact_domain_values() {
    let account = Address::from_bytes([0x11; 20]);
    let trigger_bytes = encode_trigger_order_activated(&WireTriggerOrderActivated {
        order_id: "order-17".to_owned(),
        trigger_price: "65000.125000".to_owned(),
        oracle_price: "64990.000000".to_owned(),
    })
    .unwrap();
    assert_eq!(
        EventPayload::decode(EventKind::TriggerOrderActivated, &trigger_bytes).unwrap(),
        EventPayload::TriggerOrderActivated(TriggerOrderActivated {
            order_id: OrderId::new("order-17").unwrap(),
            trigger_price: Price::parse_at_scale("65000.125", 6).unwrap(),
            oracle_price: Price::parse_at_scale("64990", 6).unwrap(),
        })
    );

    let started_bytes = encode_twap_started(&WireTwapStarted {
        order_id: "twap-9".to_owned(),
        account_id: account.to_api_string(),
        market_id: "perp:BTC".to_owned(),
        total_quantity: "1.25000000".to_owned(),
        end_time_micros: 1_700_000_000_000_000,
    })
    .unwrap();
    assert_eq!(
        EventPayload::decode(EventKind::TwapStarted, &started_bytes).unwrap(),
        EventPayload::TwapStarted(TwapStarted {
            order_id: OrderId::new("twap-9").unwrap(),
            account_id: account,
            market_id: MarketId::new("perp:BTC").unwrap(),
            total_quantity: Quantity::parse_at_scale("1.25", 8).unwrap(),
            end_time: ProtocolTime::from_unix_micros(1_700_000_000_000_000).unwrap(),
        })
    );

    let slice_bytes = encode_twap_slice_filled(&WireTwapSliceFilled {
        order_id: "twap-9".to_owned(),
        slice_index: 3,
        fill_price: "65001.000000".to_owned(),
        fill_quantity: "0.25000000".to_owned(),
    })
    .unwrap();
    assert_eq!(
        EventPayload::decode(EventKind::TwapSliceFilled, &slice_bytes).unwrap(),
        EventPayload::TwapSliceFilled(TwapSliceFilled {
            order_id: OrderId::new("twap-9").unwrap(),
            slice_index: 3,
            fill_price: Price::parse_at_scale("65001", 6).unwrap(),
            fill_quantity: Quantity::parse_at_scale("0.25", 8).unwrap(),
        })
    );

    let completed_bytes = encode_twap_completed(&WireTwapCompleted {
        order_id: "twap-9".to_owned(),
        filled_quantity: "1.25000000".to_owned(),
        average_price: "65000.500000".to_owned(),
    })
    .unwrap();
    assert_eq!(
        EventPayload::decode(EventKind::TwapCompleted, &completed_bytes).unwrap(),
        EventPayload::TwapCompleted(TwapCompleted {
            order_id: OrderId::new("twap-9").unwrap(),
            filled_quantity: Quantity::parse_at_scale("1.25", 8).unwrap(),
            average_price: Price::parse_at_scale("65000.5", 6).unwrap(),
        })
    );
}

#[test]
fn trigger_and_twap_payloads_reject_invalid_semantics() {
    for (kind, bytes) in [
        (
            EventKind::TriggerOrderActivated,
            encode_trigger_order_activated(&WireTriggerOrderActivated {
                order_id: "order-17".to_owned(),
                trigger_price: "0".to_owned(),
                oracle_price: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::TriggerOrderActivated,
            encode_trigger_order_activated(&WireTriggerOrderActivated {
                order_id: "order-17".to_owned(),
                trigger_price: "1".to_owned(),
                oracle_price: "-1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::TwapStarted,
            encode_twap_started(&WireTwapStarted {
                order_id: "twap-9".to_owned(),
                account_id: Address::from_bytes([0x11; 20]).to_api_string(),
                market_id: "perp:BTC".to_owned(),
                total_quantity: "0".to_owned(),
                end_time_micros: 1,
            })
            .unwrap(),
        ),
        (
            EventKind::TwapSliceFilled,
            encode_twap_slice_filled(&WireTwapSliceFilled {
                order_id: "twap-9".to_owned(),
                slice_index: 0,
                fill_price: "1".to_owned(),
                fill_quantity: "0".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::TwapCompleted,
            encode_twap_completed(&WireTwapCompleted {
                order_id: "twap-9".to_owned(),
                filled_quantity: "-1".to_owned(),
                average_price: "1".to_owned(),
            })
            .unwrap(),
        ),
        (
            EventKind::TwapCompleted,
            encode_twap_completed(&WireTwapCompleted {
                order_id: "twap-9".to_owned(),
                filled_quantity: "0".to_owned(),
                average_price: "1".to_owned(),
            })
            .unwrap(),
        ),
    ] {
        assert!(
            matches!(
                EventPayload::decode(kind, &bytes),
                Err(ContractError::Invalid {
                    field: "payload",
                    ..
                })
            ),
            "{kind:?} must reject invalid semantics"
        );
    }
}

#[test]
fn zero_fill_twap_completion_requires_zero_average_price() {
    let bytes = encode_twap_completed(&WireTwapCompleted {
        order_id: "twap-9".to_owned(),
        filled_quantity: "0".to_owned(),
        average_price: "0".to_owned(),
    })
    .unwrap();
    let payload = EventPayload::decode(EventKind::TwapCompleted, &bytes).unwrap();
    let EventPayload::TwapCompleted(completed) = payload else {
        panic!("zero-fill completion must remain TwapCompleted");
    };
    assert_eq!(completed.filled_quantity, Quantity::from_str("0").unwrap());
    assert_eq!(completed.average_price, Price::from_str("0").unwrap());
}
