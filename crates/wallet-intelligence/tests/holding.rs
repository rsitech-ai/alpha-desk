use domain_types::ProtocolTime;
use wallet_intelligence::{IntelligenceError, ObservedHoldInterval, holding_time_distribution};

fn time(seconds: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(seconds * 1_000_000).unwrap()
}

fn hold(open: i64, close: i64) -> ObservedHoldInterval {
    ObservedHoldInterval::try_new(time(open), time(close)).unwrap()
}

#[test]
fn holding_time_uses_observed_open_and_close_only() {
    let distribution = holding_time_distribution(&[hold(1, 2), hold(3, 6), hold(7, 8)]).unwrap();
    assert_eq!(distribution.sample_count, 3);
    assert_eq!(distribution.min_micros, 1_000_000);
    assert_eq!(distribution.max_micros, 3_000_000);
    assert_eq!(distribution.median_micros, 1_000_000);
    assert_eq!(distribution.total_micros, 5_000_000);
}

#[test]
fn even_sample_median_is_an_observed_duration() {
    let distribution = holding_time_distribution(&[hold(1, 2), hold(3, 6)]).unwrap();
    assert_eq!(distribution.median_micros, 1_000_000);
}

#[test]
fn empty_or_inverted_intervals_fail_closed() {
    let empty = holding_time_distribution(&[]).unwrap_err();
    assert!(matches!(
        empty,
        IntelligenceError::InsufficientHistory {
            what: "holding_time"
        }
    ));
    let inverted = ObservedHoldInterval::try_new(time(5), time(4)).unwrap_err();
    assert!(matches!(
        inverted,
        IntelligenceError::Malformed {
            what: "holding_time",
            reason: "close before open"
        }
    ));
    let bypass = ObservedHoldInterval {
        opened_at: time(5),
        closed_at: time(4),
    };
    let inverted_distribution = holding_time_distribution(&[bypass]).unwrap_err();
    assert!(matches!(
        inverted_distribution,
        IntelligenceError::Malformed {
            what: "holding_time",
            reason: "close before open"
        }
    ));
}
