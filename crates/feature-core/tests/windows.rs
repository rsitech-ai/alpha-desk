use domain_types::{Decimal, EventId, ProtocolTime};
use feature_core::{
    FeatureError, RollingWindow, WINDOW_PARAMETER_VERSION, WindowAlgorithm, WindowUpdate,
};

fn event(id: &str, seq: u64, value: i128) -> WindowUpdate {
    WindowUpdate {
        event_id: EventId::new(id).unwrap(),
        event_time: ProtocolTime::from_unix_micros(i64::try_from(seq).unwrap() * 1_000).unwrap(),
        sequence: seq,
        value: Decimal::from_raw(value, 2).unwrap(),
        paired_value: None,
    }
}

fn pair(id: &str, seq: u64, x: i128, y: i128) -> WindowUpdate {
    WindowUpdate {
        event_id: EventId::new(id).unwrap(),
        event_time: ProtocolTime::from_unix_micros(i64::try_from(seq).unwrap() * 1_000).unwrap(),
        sequence: seq,
        value: Decimal::from_raw(x, 2).unwrap(),
        paired_value: Some(Decimal::from_raw(y, 2).unwrap()),
    }
}

#[test]
fn duplicate_event_ids_are_ignored() {
    let mut window = RollingWindow::try_new(WindowAlgorithm::EventCount, 8, 0, 0, 0).unwrap();
    assert!(window.update(event("e1", 1, 100)).unwrap());
    assert!(!window.update(event("e1", 2, 999)).unwrap());
    let snapshot = window.snapshot().unwrap();
    assert_eq!(snapshot.count, 1);
    assert_eq!(snapshot.value.unwrap().raw(), 100);
}

#[test]
fn chunked_and_one_pass_event_count_windows_match() {
    let values = [10, 20, 30, 40, 50];
    let mut one_pass = RollingWindow::try_new(WindowAlgorithm::EventCount, 16, 0, 0, 0).unwrap();
    for (index, value) in values.iter().enumerate() {
        one_pass
            .update(event(
                &format!("e{index}"),
                u64::try_from(index + 1).unwrap(),
                *value,
            ))
            .unwrap();
    }
    let mut chunked = RollingWindow::try_new(WindowAlgorithm::EventCount, 16, 0, 0, 0).unwrap();
    for (index, value) in values.iter().take(2).enumerate() {
        chunked
            .update(event(
                &format!("e{index}"),
                u64::try_from(index + 1).unwrap(),
                *value,
            ))
            .unwrap();
    }
    for (index, value) in values.iter().enumerate().skip(2) {
        chunked
            .update(event(
                &format!("e{index}"),
                u64::try_from(index + 1).unwrap(),
                *value,
            ))
            .unwrap();
    }
    assert_eq!(one_pass.snapshot().unwrap(), chunked.snapshot().unwrap());
    assert_eq!(
        one_pass.snapshot().unwrap().parameter_version,
        WINDOW_PARAMETER_VERSION
    );
}

#[test]
fn protocol_time_window_drops_samples_outside_duration() {
    let mut window =
        RollingWindow::try_new(WindowAlgorithm::ProtocolTime, 16, 2_000, 0, 0).unwrap();
    window.update(event("e1", 1, 100)).unwrap();
    window.update(event("e2", 2, 200)).unwrap();
    window.update(event("e3", 5, 400)).unwrap();
    let snapshot = window.snapshot().unwrap();
    assert_eq!(snapshot.count, 1);
}

#[test]
fn covariance_and_quantile_are_deterministic() {
    let mut covariance = RollingWindow::try_new(WindowAlgorithm::Covariance, 16, 0, 0, 0).unwrap();
    covariance.update(pair("a", 1, 100, 200)).unwrap();
    covariance.update(pair("b", 2, 200, 400)).unwrap();
    covariance.update(pair("c", 3, 300, 600)).unwrap();
    let cov = covariance.snapshot().unwrap().value.unwrap();
    assert!(cov.raw() > 0);

    let mut quantile =
        RollingWindow::try_new(WindowAlgorithm::QuantileSketch, 16, 0, 0, 500_000).unwrap();
    for (index, value) in [10, 20, 30, 40, 50].iter().enumerate() {
        quantile
            .update(event(
                &format!("q{index}"),
                u64::try_from(index + 1).unwrap(),
                *value,
            ))
            .unwrap();
    }
    assert_eq!(quantile.snapshot().unwrap().value.unwrap().raw(), 30);
}

#[test]
fn unsupported_and_malformed_window_inputs_fail_closed() {
    assert!(RollingWindow::try_new(WindowAlgorithm::EventCount, 0, 0, 0, 0).is_err());
    let mut window = RollingWindow::try_new(WindowAlgorithm::Covariance, 4, 0, 0, 0).unwrap();
    let error = window.update(event("e1", 1, 1)).unwrap_err();
    assert!(matches!(error, FeatureError::Malformed { .. }));
}

#[test]
fn covariance_paired_value_admission_covers_every_constructible_window_algorithm() {
    fn window(algorithm: WindowAlgorithm) -> RollingWindow {
        let (decay_ppm, quantile_ppm) = match algorithm {
            WindowAlgorithm::ExponentiallyWeighted => (500_000, 0),
            WindowAlgorithm::QuantileSketch => (0, 500_000),
            WindowAlgorithm::EventCount
            | WindowAlgorithm::ProtocolTime
            | WindowAlgorithm::Covariance
            | WindowAlgorithm::RobustZScore => (0, 0),
        };
        RollingWindow::try_new(algorithm, 8, 0, decay_ppm, quantile_ppm).unwrap()
    }

    fn pin(algorithm: WindowAlgorithm) {
        match algorithm {
            WindowAlgorithm::Covariance => {
                let mut reject = window(algorithm);
                let error = reject.update(event("missing-pair", 1, 1)).unwrap_err();
                assert!(
                    matches!(
                        error,
                        FeatureError::Malformed {
                            what: "window_update",
                            reason: "covariance requires paired_value",
                        }
                    ),
                    "covariance still fail-closes a missing paired_value: {error:?}"
                );
                let mut admit = window(algorithm);
                assert!(
                    admit.update(pair("has-pair", 1, 100, 200)).unwrap(),
                    "covariance still admits a present paired_value"
                );
            }
            WindowAlgorithm::EventCount
            | WindowAlgorithm::ProtocolTime
            | WindowAlgorithm::ExponentiallyWeighted
            | WindowAlgorithm::QuantileSketch
            | WindowAlgorithm::RobustZScore => {
                let mut skip = window(algorithm);
                assert!(
                    skip.update(event("no-pair", 1, 100)).unwrap(),
                    "{algorithm:?} still skips the covariance paired_value gate"
                );
            }
        }
    }

    pin(WindowAlgorithm::EventCount);
    pin(WindowAlgorithm::ProtocolTime);
    pin(WindowAlgorithm::ExponentiallyWeighted);
    pin(WindowAlgorithm::QuantileSketch);
    pin(WindowAlgorithm::Covariance);
    pin(WindowAlgorithm::RobustZScore);
}

#[test]
fn robust_z_requires_history_and_nonzero_mad() {
    let mut window = RollingWindow::try_new(WindowAlgorithm::RobustZScore, 8, 0, 0, 0).unwrap();
    window.update(event("e1", 1, 100)).unwrap();
    window.update(event("e2", 2, 100)).unwrap();
    assert!(window.snapshot().is_err());
    window.update(event("e3", 3, 30)).unwrap();
    assert!(window.snapshot().is_err());
    window.update(event("e4", 4, 80)).unwrap();
    assert!(window.snapshot().unwrap().value.is_some());
}
