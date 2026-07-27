use domain_types::{
    BlockHeight, BlockRange, ClosedInterval, KnownTime, LatencyDistribution, ProtocolTime,
    ValueError,
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
