use domain_types::{EventId, Horizon, KnownTime, MarketId, Price, ProtocolTime, UsdAmount};
use wallet_intelligence::{
    ActionSide, IntelligenceError, LiquidityRole, MarkoutKind, MarkoutPoint, evaluate_markouts,
};

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, 8).unwrap()
}

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 2).unwrap()
}

fn point(kind: MarkoutKind, entry: &str, later: &str) -> MarkoutPoint {
    MarkoutPoint {
        action_id: EventId::new("action-1").unwrap(),
        market_id: MarketId::new("BTC").unwrap(),
        kind,
        side: ActionSide::Buy,
        role: LiquidityRole::Taker,
        entry_at: ProtocolTime::from_unix_micros(1_000_000).unwrap(),
        entry_price: price(entry),
        horizon: Horizon::MS_250,
        price_at_horizon: price(later),
        price_known_at: KnownTime::from_unix_micros(1_250_000).unwrap(),
        fee: usd("0"),
        funding: usd("0"),
        notional: usd("1000"),
    }
}

#[test]
fn empty_markouts_withhold_and_observed_prices_evaluate() {
    assert!(
        evaluate_markouts(&[], KnownTime::from_unix_micros(1_250_000).unwrap())
            .unwrap()
            .is_none()
    );
    let known = KnownTime::from_unix_micros(1_250_000).unwrap();
    let results = evaluate_markouts(
        &[
            point(MarkoutKind::Entry, "100", "101"),
            point(MarkoutKind::Exit, "100", "99"),
        ],
        known,
    )
    .unwrap()
    .unwrap();
    assert_eq!(results.len(), 2);
    assert!(results[0].complete);
    assert_eq!(results[0].kind, MarkoutKind::Entry);
    assert_eq!(results[0].net_markout_bps.raw(), 10_000);
    assert_eq!(results[1].kind, MarkoutKind::Exit);
    assert_eq!(results[1].net_markout_bps.raw(), -10_000);
}

#[test]
fn markout_fails_closed_without_inventing_prices() {
    let known_early = KnownTime::from_unix_micros(1_000_000).unwrap();
    let incomplete = point(MarkoutKind::Entry, "100", "101")
        .evaluate(known_early)
        .unwrap();
    assert!(!incomplete.complete);
    assert_eq!(incomplete.net_markout_bps.raw(), 0);

    let mut too_soon = point(MarkoutKind::Entry, "100", "101");
    too_soon.price_known_at = KnownTime::from_unix_micros(1_100_000).unwrap();
    let premature = too_soon
        .evaluate(KnownTime::from_unix_micros(2_000_000).unwrap())
        .unwrap_err();
    assert!(matches!(
        premature,
        IntelligenceError::Malformed {
            what: "markout",
            reason: "price known before horizon elapsed"
        }
    ));

    let mut zero = point(MarkoutKind::Entry, "100", "101");
    zero.entry_price = Price::from_raw(0, 2).unwrap();
    let error = zero
        .evaluate(KnownTime::from_unix_micros(1_250_000).unwrap())
        .unwrap_err();
    assert!(matches!(error, IntelligenceError::DivisionByZero));
}
