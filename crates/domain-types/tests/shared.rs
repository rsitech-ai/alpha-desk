use domain_types::{
    AccountAbstractionModeV1, BlockHeight, BlockRange, ClosedInterval, FeeTypeV1, KnownTime,
    LatencyDistribution, MarginModeV1, OrderSide, ProtocolTime, ValueError,
};
use proptest::prelude::*;

#[test]
fn latency_distribution_requires_monotonic_percentiles() {
    assert!(LatencyDistribution::new(10, 20, 30, 40).is_ok());
    assert_eq!(
        LatencyDistribution::new(10, 9, 30, 40),
        Err(ValueError::OutOfRange)
    );
}

#[test]
fn block_range_requires_non_decreasing_heights() {
    assert!(BlockRange::new(BlockHeight::new(9), BlockHeight::new(9)).is_ok());
    assert_eq!(
        BlockRange::new(BlockHeight::new(10), BlockHeight::new(9)),
        Err(ValueError::OutOfRange)
    );
}

#[test]
fn closed_interval_requires_ordered_bounds() {
    assert!(ClosedInterval::new(2_u64, 2_u64).is_ok());
    assert_eq!(
        ClosedInterval::new(3_u64, 2_u64),
        Err(ValueError::OutOfRange)
    );
}

#[test]
fn protocol_timestamps_reject_negative_values_and_serde_input() {
    assert_eq!(
        ProtocolTime::from_unix_micros(-1),
        Err(ValueError::OutOfRange)
    );
    assert_eq!(KnownTime::from_unix_micros(-1), Err(ValueError::OutOfRange));
    assert!(serde_json::from_str::<ProtocolTime>("-1").is_err());
    assert!(serde_json::from_str::<KnownTime>("-1").is_err());
}

#[test]
fn protocol_timestamp_serde_round_trips() {
    let time = ProtocolTime::from_unix_micros(123_456).unwrap();
    assert_eq!(
        serde_json::from_str::<ProtocolTime>(&serde_json::to_string(&time).unwrap()).unwrap(),
        time
    );
}

#[test]
fn order_side_has_an_exact_wire_contract_distinct_from_position_direction() {
    assert_eq!(OrderSide::Buy.as_wire_name(), "buy");
    assert_eq!(OrderSide::Sell.as_wire_name(), "sell");
    assert_eq!(OrderSide::parse_wire("buy"), Ok(OrderSide::Buy));
    assert_eq!(OrderSide::parse_wire("sell"), Ok(OrderSide::Sell));
    for invalid in ["Buy", "SELL", "long", "short", " buy", "sell ", ""] {
        assert_eq!(OrderSide::parse_wire(invalid), Err(ValueError::Invalid));
    }
}

#[test]
fn canonical_account_enums_have_frozen_case_sensitive_wire_contracts() {
    let fee_types = [
        ("maker", FeeTypeV1::Maker),
        ("taker", FeeTypeV1::Taker),
        ("maker_rebate", FeeTypeV1::MakerRebate),
        ("referral_discount", FeeTypeV1::ReferralDiscount),
        ("protocol", FeeTypeV1::Protocol),
    ];
    for (wire, value) in fee_types {
        assert_eq!(value.as_wire_name(), wire);
        assert_eq!(FeeTypeV1::parse_wire(wire), Ok(value));
    }

    let account_modes = [
        ("standard", AccountAbstractionModeV1::Standard),
        ("unified", AccountAbstractionModeV1::Unified),
        ("portfolio", AccountAbstractionModeV1::Portfolio),
        ("dex_abstraction", AccountAbstractionModeV1::DexAbstraction),
    ];
    for (wire, value) in account_modes {
        assert_eq!(value.as_wire_name(), wire);
        assert_eq!(AccountAbstractionModeV1::parse_wire(wire), Ok(value));
    }

    let margin_modes = [
        ("cross", MarginModeV1::Cross),
        ("isolated", MarginModeV1::Isolated),
        ("strict_isolated", MarginModeV1::StrictIsolated),
    ];
    for (wire, value) in margin_modes {
        assert_eq!(value.as_wire_name(), wire);
        assert_eq!(MarginModeV1::parse_wire(wire), Ok(value));
    }

    for invalid in ["", "Maker", " maker", "maker ", "maker-rebate", "unknown"] {
        assert_eq!(FeeTypeV1::parse_wire(invalid), Err(ValueError::Invalid));
    }
    for invalid in [
        "",
        "Standard",
        " standard",
        "standard ",
        "dex-abstraction",
        "unknown",
    ] {
        assert_eq!(
            AccountAbstractionModeV1::parse_wire(invalid),
            Err(ValueError::Invalid)
        );
    }
    for invalid in [
        "",
        "Cross",
        " cross",
        "cross ",
        "strict-isolated",
        "unknown",
    ] {
        assert_eq!(MarginModeV1::parse_wire(invalid), Err(ValueError::Invalid));
    }
}

proptest! {
    #[test]
    fn ordered_intervals_are_constructible(lower in any::<u64>(), width in any::<u64>()) {
        let upper = lower.saturating_add(width);
        prop_assert!(ClosedInterval::new(lower, upper).is_ok());
    }

    #[test]
    fn monotonic_latency_percentiles_are_constructible(p10 in any::<u16>(), d50 in any::<u16>(), d90 in any::<u16>(), d99 in any::<u16>()) {
        let p50 = p10.saturating_add(d50);
        let p90 = p50.saturating_add(d90);
        let p99 = p90.saturating_add(d99);
        prop_assert!(LatencyDistribution::new(u64::from(p10), u64::from(p50), u64::from(p90), u64::from(p99)).is_ok());
    }

    #[test]
    fn nonnegative_timestamps_round_trip(value in any::<u64>()) {
        let value = value.min(i64::MAX as u64) as i64;
        let time = ProtocolTime::from_unix_micros(value).unwrap();
        prop_assert_eq!(time.unix_micros(), value);
    }
}
