use canonical_events::{
    CanonicalEventEnvelope, ConfirmationClass, EventPayload, OrderAccepted, OrderFilled,
    OrderModified, OrderPartiallyFilled,
};
use domain_types::{
    Address, BlockHeight, EventId, MarketId, OrderId, OrderSide, Price, ProtocolTime, Quantity,
    TradeId, TransactionId, UsdAmount,
};
use wallet_intelligence::{
    ActionSide, IntelligenceError, ObservedFill, observed_fills_from_order_events,
    slippage_from_fills, slippage_from_order_events,
};

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 6).unwrap()
}

fn quantity(value: &str) -> Quantity {
    Quantity::parse_at_scale(value, 8).unwrap()
}

fn account() -> Address {
    Address::from_bytes([0x11; 20])
}

fn market() -> MarketId {
    MarketId::new("perp:BTC").unwrap()
}

fn order_id() -> OrderId {
    OrderId::new("order-1").unwrap()
}

fn envelope(height: u64, event_index: u32, payload: EventPayload) -> CanonicalEventEnvelope {
    envelope_at(height, event_index, i64::try_from(height).unwrap(), payload)
}

fn envelope_at(
    height: u64,
    event_index: u32,
    time_micros: i64,
    payload: EventPayload,
) -> CanonicalEventEnvelope {
    CanonicalEventEnvelope::try_new(
        "1.0.0",
        "hyperliquid-mainnet",
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(time_micros).unwrap(),
        TransactionId::new(format!("tx-{height}")).unwrap(),
        0,
        event_index,
        EventId::new(format!("event-{height}-{event_index}")).unwrap(),
        vec![market()],
        vec![account()],
        ConfirmationClass::CommittedPrimary,
        payload,
        "parser-v1",
    )
    .unwrap()
}

fn accepted(limit: &str, qty: &str) -> EventPayload {
    EventPayload::OrderAccepted(OrderAccepted {
        order_id: order_id(),
        account_id: account(),
        market_id: market(),
        side: OrderSide::Buy,
        limit_price: price(limit),
        quantity: quantity(qty),
    })
}

fn modified(previous: &str, new_price: &str) -> EventPayload {
    EventPayload::OrderModified(OrderModified {
        order_id: order_id(),
        previous_price: price(previous),
        new_price: price(new_price),
        previous_quantity: quantity("1"),
        new_quantity: quantity("1"),
    })
}

fn filled(fill_price: &str, fill_quantity: &str) -> EventPayload {
    EventPayload::OrderFilled(OrderFilled {
        order_id: order_id(),
        trade_id: TradeId::new("trade-1").unwrap(),
        fill_price: price(fill_price),
        fill_quantity: quantity(fill_quantity),
    })
}

fn partially_filled(fill_price: &str, fill_quantity: &str, remaining: &str) -> EventPayload {
    EventPayload::OrderPartiallyFilled(OrderPartiallyFilled {
        order_id: order_id(),
        trade_id: TradeId::new("trade-partial").unwrap(),
        fill_price: price(fill_price),
        fill_quantity: quantity(fill_quantity),
        remaining_quantity: quantity(remaining),
    })
}

fn explicit_fill(fill_price: &str, reference: Option<&str>, notional: UsdAmount) -> ObservedFill {
    ObservedFill::try_new(
        price(fill_price),
        reference.map(price),
        ActionSide::Buy,
        notional,
    )
    .unwrap()
}

#[test]
fn fill_joins_in_force_limit_from_accept_or_modify_before_fill() {
    let from_accept = observed_fills_from_order_events(&[
        envelope(100, 0, accepted("100", "1")),
        envelope(100, 1, filled("101", "1")),
    ])
    .unwrap();
    assert_eq!(from_accept.len(), 1);
    assert_eq!(from_accept[0].fill_price, price("101"));
    assert_eq!(from_accept[0].observed_reference_price, Some(price("100")));
    assert_eq!(from_accept[0].side, ActionSide::Buy);

    let from_modify = observed_fills_from_order_events(&[
        envelope(100, 0, accepted("100", "1")),
        envelope(100, 1, modified("100", "150")),
        envelope(100, 2, filled("151", "1")),
    ])
    .unwrap();
    assert_eq!(from_modify[0].observed_reference_price, Some(price("150")));
    assert_ne!(from_modify[0].observed_reference_price, Some(price("100")));

    let partial_then_fill = slippage_from_order_events(&[
        envelope(100, 0, accepted("100", "1")),
        envelope(100, 1, partially_filled("101", "0.4", "0.6")),
        envelope(101, 0, filled("101", "0.6")),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(partial_then_fill.observed_fill_count, 2);
    assert_eq!(partial_then_fill.withheld_missing_reference_count, 0);
    assert_eq!(
        partial_then_fill.notional_weighted_slippage_bps.raw(),
        10_000
    );
}

#[test]
fn later_order_modified_does_not_rewrite_earlier_fill_slippage() {
    let events = [
        envelope(100, 0, accepted("100", "1")),
        envelope(100, 1, partially_filled("101", "0.4", "0.6")),
        envelope(101, 0, modified("100", "200")),
        envelope(101, 1, filled("202", "0.6")),
    ];
    let joined = observed_fills_from_order_events(&events).unwrap();
    assert_eq!(joined.len(), 2);
    assert_eq!(joined[0].observed_reference_price, Some(price("100")));
    assert_eq!(joined[1].observed_reference_price, Some(price("200")));
    assert_ne!(joined[0].observed_reference_price, Some(price("200")));

    let earlier_only =
        slippage_from_fills(&[explicit_fill("101", Some("100"), joined[0].notional)])
            .unwrap()
            .unwrap();
    assert_eq!(earlier_only.notional_weighted_slippage_bps.raw(), 10_000);

    let rewritten_earlier =
        slippage_from_fills(&[explicit_fill("101", Some("200"), joined[0].notional)])
            .unwrap()
            .unwrap();
    assert_ne!(
        earlier_only.notional_weighted_slippage_bps,
        rewritten_earlier.notional_weighted_slippage_bps
    );

    let joined_slippage = slippage_from_order_events(&events).unwrap().unwrap();
    let expected = slippage_from_fills(&joined).unwrap().unwrap();
    assert_eq!(joined_slippage, expected);
}

#[test]
fn fill_without_in_force_limit_withholds() {
    let missing = slippage_from_order_events(&[envelope(100, 0, filled("101", "1"))]).unwrap();
    assert!(missing.is_none());

    let mixed = slippage_from_order_events(&[
        envelope(
            100,
            0,
            EventPayload::OrderFilled(OrderFilled {
                order_id: OrderId::new("orphan").unwrap(),
                trade_id: TradeId::new("trade-orphan").unwrap(),
                fill_price: price("50"),
                fill_quantity: quantity("1"),
            }),
        ),
        envelope(101, 0, accepted("100", "1")),
        envelope(101, 1, filled("101", "1")),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(mixed.observed_fill_count, 1);
    assert_eq!(mixed.withheld_missing_reference_count, 0);
    assert_eq!(mixed.notional_weighted_slippage_bps.raw(), 10_000);
    assert_ne!(mixed.notional_weighted_slippage_bps.raw(), 0);
}

#[test]
fn explicit_reference_slippage_from_fills_is_unchanged() {
    let no_reference = explicit_fill("101", None, UsdAmount::parse_at_scale("1000", 8).unwrap());
    assert!(slippage_from_fills(&[no_reference]).unwrap().is_none());

    let paid = slippage_from_fills(&[explicit_fill(
        "101",
        Some("100"),
        UsdAmount::parse_at_scale("1000", 8).unwrap(),
    )])
    .unwrap()
    .unwrap();
    assert_eq!(paid.observed_fill_count, 1);
    assert_eq!(paid.withheld_missing_reference_count, 0);
    assert_eq!(paid.notional_weighted_slippage_bps.raw(), 10_000);
    assert_eq!(
        paid.signed_slippage,
        UsdAmount::parse_at_scale("10", 8).unwrap()
    );
}

#[test]
fn inverted_times_and_unknown_event_order_fail_closed() {
    let inverted = observed_fills_from_order_events(&[
        envelope_at(100, 0, 200, accepted("100", "1")),
        envelope_at(101, 0, 100, filled("101", "1")),
    ])
    .unwrap_err();
    assert!(matches!(
        inverted,
        IntelligenceError::Malformed {
            what: "order_event",
            reason: "inverted times"
        }
    ));

    let unknown_order = observed_fills_from_order_events(&[
        envelope(100, 1, accepted("100", "1")),
        envelope(100, 0, filled("101", "1")),
    ])
    .unwrap_err();
    assert!(matches!(
        unknown_order,
        IntelligenceError::Malformed {
            what: "order_event",
            reason: "unknown event order"
        }
    ));

    let malformed_price = observed_fills_from_order_events(&[
        envelope(100, 0, accepted("100", "1")),
        envelope(
            100,
            1,
            EventPayload::OrderFilled(OrderFilled {
                order_id: order_id(),
                trade_id: TradeId::new("trade-zero").unwrap(),
                fill_price: Price::from_raw(0, 6).unwrap(),
                fill_quantity: quantity("1"),
            }),
        ),
    ])
    .unwrap_err();
    assert!(matches!(
        malformed_price,
        IntelligenceError::Malformed {
            what: "observed_fill",
            reason: "prices must be positive"
        }
    ));
}
