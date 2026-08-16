use domain_types::ProtocolTime;
use wallet_intelligence::{BehaviorSample, ChangePointDetector, ChangeReason, IntelligenceError};

fn sample(
    seconds: i64,
    maker_ratio_ppm: u32,
    dormant: bool,
    leverage_milli: u32,
) -> BehaviorSample {
    labeled_sample(
        seconds,
        maker_ratio_ppm,
        100_000,
        dormant,
        leverage_milli,
        false,
        1,
        false,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
fn labeled_sample(
    seconds: i64,
    maker_ratio_ppm: u32,
    turnover_ppm: u32,
    dormant: bool,
    leverage_milli: u32,
    capital_activated: bool,
    market_count: u32,
    linked_account_migration: bool,
    risk_escalation: bool,
) -> BehaviorSample {
    BehaviorSample {
        protocol_time: ProtocolTime::from_unix_micros(seconds * 1_000_000).unwrap(),
        maker_ratio_ppm,
        turnover_ppm,
        leverage_milli,
        skill_bps: 20,
        dormant,
        capital_activated,
        market_count,
        linked_account_migration,
        risk_escalation,
    }
}

#[test]
fn change_points_are_append_only_and_within_tolerance() {
    let mut detector = ChangePointDetector::try_new(50_000, 1_000, 3).unwrap();
    for second in 1..=8 {
        detector
            .observe(sample(second, 800_000, false, 1_000))
            .unwrap();
    }
    let before = detector.regimes().to_vec();
    let mut detected_at = None;
    for second in 9..=16 {
        if let Some(regime) = detector
            .observe(sample(second, 100_000, false, 5_000))
            .unwrap()
        {
            detected_at = Some(regime.started_at.unix_micros());
            assert!(regime.reasons.contains(&ChangeReason::MakerRatioShift));
        }
    }
    let detected_at = detected_at.expect("maker-to-taker regime");
    assert!((9_000_000..=16_000_000).contains(&detected_at));
    assert_eq!(before[0].started_at, detector.regimes()[0].started_at);
    assert!(detector.regimes()[0].ended_at.is_some());
    assert!(detector.regimes().len() >= 2);
}

#[test]
fn dormant_reactivation_and_leverage_are_reason_coded() {
    let mut detector = ChangePointDetector::try_new(1, 0, 1).unwrap();
    detector.observe(sample(1, 500_000, false, 1_000)).unwrap();
    let regime = detector
        .observe(sample(2, 100_000, true, 9_000))
        .unwrap()
        .unwrap();
    assert!(regime.reasons.contains(&ChangeReason::DormantReactivation));
    assert!(regime.reasons.contains(&ChangeReason::LeverageEscalation));
}

#[test]
fn invalid_detector_config_keeps_existing_error() {
    let error = ChangePointDetector::try_new(0, 0, 1).unwrap_err();
    assert!(matches!(
        error,
        IntelligenceError::Malformed {
            what: "change_point",
            reason: "threshold, drift, and min_evidence must be valid"
        }
    ));
}

#[test]
fn zero_market_count_fails_closed() {
    let mut detector = ChangePointDetector::try_new(1, 0, 1).unwrap();
    let error = detector
        .observe(labeled_sample(
            1, 500_000, 100_000, false, 1_000, false, 0, false, false,
        ))
        .unwrap_err();
    assert!(matches!(
        error,
        IntelligenceError::Malformed {
            what: "change_point",
            reason: "market_count must be positive"
        }
    ));
}

#[test]
fn capital_specialization_migration_and_risk_reasons_are_emitted() {
    let mut detector = ChangePointDetector::try_new(1, 0, 1).unwrap();
    detector
        .observe(labeled_sample(
            1, 500_000, 100_000, false, 1_000, false, 4, false, false,
        ))
        .unwrap();
    let regime = detector
        .observe(labeled_sample(
            2, 100_000, 800_000, false, 1_000, true, 1, true, true,
        ))
        .unwrap()
        .unwrap();
    assert!(regime.reasons.contains(&ChangeReason::CapitalActivation));
    assert!(regime.reasons.contains(&ChangeReason::TurnoverShift));
    assert!(regime.reasons.contains(&ChangeReason::MarketSpecialization));
    assert!(
        regime
            .reasons
            .contains(&ChangeReason::LinkedAccountMigration)
    );
    assert!(regime.reasons.contains(&ChangeReason::RiskEscalation));
    assert!(regime.reasons.contains(&ChangeReason::MakerRatioShift));
}

#[test]
fn missing_reason_flags_do_not_invent_capital_or_migration() {
    let mut detector = ChangePointDetector::try_new(1, 0, 1).unwrap();
    detector.observe(sample(1, 500_000, false, 1_000)).unwrap();
    let regime = detector
        .observe(sample(2, 100_000, false, 1_000))
        .unwrap()
        .unwrap();
    assert!(!regime.reasons.contains(&ChangeReason::CapitalActivation));
    assert!(
        !regime
            .reasons
            .contains(&ChangeReason::LinkedAccountMigration)
    );
    assert!(!regime.reasons.contains(&ChangeReason::RiskEscalation));
    assert!(!regime.reasons.contains(&ChangeReason::MarketSpecialization));
}
