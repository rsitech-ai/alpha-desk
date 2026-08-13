use domain_types::Decimal;
use hl_research::{
    GateDecision, PromotionEvidence, ResearchError, calibrate_scores, evaluate_promotion, promote,
    run_promote_bytes, stamp_holdout_passed, stationary_block_bootstrap,
};

fn dec(value: &str) -> Decimal {
    Decimal::parse_at_scale(value, 8).unwrap()
}

#[test]
fn promotion_without_locked_holdout_is_withheld_and_cannot_stamp_pass() {
    let outcomes = [dec("1.00000000"); 12];
    let bootstrap = stationary_block_bootstrap(&outcomes, 2, 200, 7).unwrap();
    let calibration = calibrate_scores(&outcomes, &outcomes).unwrap();
    let report = evaluate_promotion(&PromotionEvidence {
        outcome_count: 12,
        holdout_locked: false,
        holdout_outcome_count: 0,
        calendar_days: None,
        bootstrap: &bootstrap,
        calibration: &calibration,
        metrics: None,
        shadow_live: false,
        episode_shares_ppm: &[500_000, 500_000],
    });
    assert_eq!(report.decision, "withheld");
    assert!(!report.promoted);
    assert!(!report.holdout_passed);
    assert!(!report.alpha_quality_claimed);
    assert!(!report.stage_pass_claimed);
    assert!(
        report
            .gates
            .iter()
            .all(|gate| gate.decision != GateDecision::Fail || gate.name != "locked_holdout")
    );
    let holdout = report
        .gates
        .iter()
        .find(|gate| gate.name == "locked_holdout")
        .unwrap();
    assert_eq!(holdout.decision, GateDecision::Withheld);
    assert_eq!(
        promote(&report).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
    assert_eq!(
        stamp_holdout_passed(&report).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}

#[test]
fn promotion_fails_independent_outcomes_but_still_does_not_promote() {
    let outcomes = [dec("1.00000000"); 8];
    let bootstrap = stationary_block_bootstrap(&outcomes, 2, 200, 7).unwrap();
    let calibration = calibrate_scores(
        &[dec("21.00000000"), dec("22.00000000")],
        &[dec("21.00000000"), dec("22.00000000")],
    )
    .unwrap();
    let report = evaluate_promotion(&PromotionEvidence {
        outcome_count: 8,
        holdout_locked: false,
        holdout_outcome_count: 0,
        calendar_days: None,
        bootstrap: &bootstrap,
        calibration: &calibration,
        metrics: None,
        shadow_live: false,
        episode_shares_ppm: &[300_000],
    });
    let outcomes_gate = report
        .gates
        .iter()
        .find(|gate| gate.name == "independent_outcomes")
        .unwrap();
    assert_eq!(outcomes_gate.decision, GateDecision::Fail);
    let concentration = report
        .gates
        .iter()
        .find(|gate| gate.name == "concentration")
        .unwrap();
    assert_eq!(concentration.decision, GateDecision::Fail);
    assert_eq!(report.decision, "withheld");
    assert!(!report.promoted);
}

#[test]
fn promote_cli_path_returns_withheld_report() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/research/fold-estimator-v1.json");
    let report = run_promote_bytes(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report.decision, "withheld");
    assert!(!report.promoted);
    assert!(!report.holdout_passed);
    assert_eq!(
        promote(&report).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}

#[test]
fn locked_flag_without_protocol_still_cannot_pass() {
    let outcomes = [dec("0.50000000"); 40];
    let bootstrap = stationary_block_bootstrap(&outcomes, 4, 200, 7).unwrap();
    let calibration = calibrate_scores(&outcomes, &outcomes).unwrap();
    let report = evaluate_promotion(&PromotionEvidence {
        outcome_count: 120,
        holdout_locked: true,
        holdout_outcome_count: 40,
        calendar_days: Some(120),
        bootstrap: &bootstrap,
        calibration: &calibration,
        metrics: None,
        shadow_live: false,
        episode_shares_ppm: &[10_000; 12],
    });
    let holdout = report
        .gates
        .iter()
        .find(|gate| gate.name == "locked_holdout")
        .unwrap();
    assert_eq!(holdout.decision, GateDecision::Fail);
    assert_eq!(holdout.reason, "holdout_pass_not_implemented");
    assert_eq!(report.decision, "withheld");
    assert!(!report.holdout_passed);
    assert!(!report.promoted);
}
