use domain_types::{Price, UsdAmount};
use wallet_intelligence::{ActionSide, IntelligenceError, ObservedFill, slippage_from_fills};

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, 8).unwrap()
}

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 2).unwrap()
}

fn fill(
    fill_price: &str,
    reference: Option<&str>,
    side: ActionSide,
    notional: &str,
) -> ObservedFill {
    ObservedFill::try_new(price(fill_price), reference.map(price), side, usd(notional)).unwrap()
}

#[test]
fn missing_fills_or_references_withhold_slippage() {
    assert!(slippage_from_fills(&[]).unwrap().is_none());
    let no_reference = fill("101", None, ActionSide::Buy, "1000");
    assert!(slippage_from_fills(&[no_reference]).unwrap().is_none());
}

#[test]
fn observed_fill_versus_reference_is_notional_weighted() {
    let paid = slippage_from_fills(&[fill("101", Some("100"), ActionSide::Buy, "1000")])
        .unwrap()
        .unwrap();
    assert_eq!(paid.observed_fill_count, 1);
    assert_eq!(paid.withheld_missing_reference_count, 0);
    assert_eq!(paid.notional_weighted_slippage_bps.raw(), 10_000);
    assert_eq!(paid.signed_slippage, usd("10"));

    let earned_sell = slippage_from_fills(&[fill("101", Some("100"), ActionSide::Sell, "1000")])
        .unwrap()
        .unwrap();
    assert_eq!(earned_sell.notional_weighted_slippage_bps.raw(), -10_000);

    let mixed = slippage_from_fills(&[
        fill("101", Some("100"), ActionSide::Buy, "1000"),
        fill("100", None, ActionSide::Buy, "500"),
    ])
    .unwrap()
    .unwrap();
    assert_eq!(mixed.observed_fill_count, 1);
    assert_eq!(mixed.withheld_missing_reference_count, 1);
    assert_eq!(mixed.notional_weighted_slippage_bps.raw(), 10_000);
}

#[test]
fn invalid_observed_fills_fail_closed() {
    let zero_price = ObservedFill::try_new(
        Price::from_raw(0, 2).unwrap(),
        Some(price("100")),
        ActionSide::Buy,
        usd("1"),
    )
    .unwrap_err();
    assert!(matches!(
        zero_price,
        IntelligenceError::Malformed {
            what: "observed_fill",
            reason: "prices must be positive"
        }
    ));
    let zero_notional =
        ObservedFill::try_new(price("100"), Some(price("100")), ActionSide::Buy, usd("0"))
            .unwrap_err();
    assert!(matches!(
        zero_notional,
        IntelligenceError::Malformed {
            what: "observed_fill",
            reason: "notional must be positive"
        }
    ));
    let scale = ObservedFill::try_new(
        Price::parse_at_scale("100", 2).unwrap(),
        Some(Price::parse_at_scale("100", 4).unwrap()),
        ActionSide::Buy,
        usd("1"),
    )
    .unwrap_err();
    assert!(matches!(scale, IntelligenceError::ScaleMismatch));
}
