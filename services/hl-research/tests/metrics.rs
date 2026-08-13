use domain_types::Decimal;
use hl_research::{
    EstimatorClass, ResearchError, VariantLedger, claim_discovery, fit, run_evaluate_folds_bytes,
    run_walk_forward_bytes, score_predictions, stationary_block_bootstrap,
};

fn dec(value: &str) -> Decimal {
    Decimal::parse_at_scale(value, 8).unwrap()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/research")
            .join(name),
    )
    .unwrap()
}

#[test]
fn univariate_linear_recovers_exact_synthetic_line() {
    let xs = [1, 2, 3, 4, 5].map(|value| vec![dec(&format!("{value}.00000000"))]);
    let ys = [3, 5, 7, 9, 11].map(|value| dec(&format!("{value}.00000000")));
    let model = fit(EstimatorClass::UnivariateLinear, &xs, &ys).unwrap();
    let report = model.report();
    assert_eq!(report.intercept, "1.00000000");
    assert_eq!(report.weights, vec!["2.00000000"]);
    assert_eq!(
        model.predict(&[dec("10.00000000")]).unwrap(),
        dec("21.00000000")
    );
}

#[test]
fn constant_feature_fails_closed() {
    let xs = [vec![dec("1.00000000")], vec![dec("1.00000000")]];
    let ys = [dec("3.00000000"), dec("5.00000000")];
    let error = fit(EstimatorClass::UnivariateLinear, &xs, &ys).unwrap_err();
    assert_eq!(
        error,
        ResearchError::UnmodeledVariance {
            field: "univariate_linear",
        }
    );
}

#[test]
fn fold_estimators_fit_without_holdout_and_do_not_claim_significance() {
    let report = run_evaluate_folds_bytes(&fixture("fold-estimator-v1.json")).unwrap();
    assert_eq!(report.mode, "synthetic_fold_estimators");
    assert_eq!(report.significance, "not_claimed");
    assert!(!report.alpha_quality_claimed);
    assert!(!report.stage_pass_claimed);
    assert_eq!(report.ledger.len(), 2);
    assert_eq!(report.multiple_testing.significance, "not_claimed");
    assert_eq!(report.multiple_testing.withheld_reason, "no_locked_holdout");
    assert_eq!(report.bootstrap.significance, "not_claimed");
    assert!(report.bootstrap.lower_bound.is_none());

    let linear = report
        .evaluations
        .iter()
        .find(|evaluation| evaluation.estimator.class == EstimatorClass::UnivariateLinear)
        .unwrap();
    assert_eq!(linear.estimator.intercept, "1.00000000");
    assert_eq!(linear.estimator.weights, vec!["2.00000000"]);
    assert_eq!(linear.metrics.mean_abs_error, "0.00000000");
    assert_eq!(linear.metrics.sharpe, "not_claimed");
    assert_eq!(linear.metrics.significance, "not_claimed");
}

#[test]
fn evaluate_folds_is_deterministic() {
    let first = run_evaluate_folds_bytes(&fixture("fold-estimator-v1.json")).unwrap();
    let second = run_evaluate_folds_bytes(&fixture("fold-estimator-v1.json")).unwrap();
    assert_eq!(first.fold_hash, second.fold_hash);
    assert_eq!(first.evaluations, second.evaluations);
    assert_eq!(first.bootstrap, second.bootstrap);
}

#[test]
fn walk_forward_without_observations_cannot_fit() {
    let error = run_evaluate_folds_bytes(&fixture("walk-forward-v1.json")).unwrap_err();
    assert_eq!(
        error,
        ResearchError::MissingObservation { field: "outcome" }
    );
    let walk = run_walk_forward_bytes(&fixture("walk-forward-v1.json")).unwrap();
    assert_eq!(walk.walk_forward, "synthetic_folds");
}

#[test]
fn bootstrap_withholds_bound_and_never_claims_significance() {
    let small: Vec<Decimal> = (0..12).map(|i| dec(&format!("{i}.00000000"))).collect();
    let small_report = stationary_block_bootstrap(&small, 2, 200, 7).unwrap();
    assert_eq!(small_report.significance, "not_claimed");
    assert!(small_report.lower_bound.is_none());
    assert_eq!(
        small_report.withheld_reason,
        Some("insufficient_independent_outcomes")
    );

    let large: Vec<Decimal> = (0..40).map(|i| dec(&format!("{i}.00000000"))).collect();
    let large_report = stationary_block_bootstrap(&large, 4, 200, 7).unwrap();
    assert!(large_report.lower_bound.is_some());
    assert_eq!(large_report.significance, "not_claimed");
    assert_eq!(large_report.withheld_reason, Some("no_locked_holdout"));
    let again = stationary_block_bootstrap(&large, 4, 200, 7).unwrap();
    assert_eq!(large_report, again);
}

#[test]
fn score_predictions_hit_rate_is_exact() {
    let metrics = score_predictions(
        &[dec("1.00000000"), dec("-1.00000000")],
        &[dec("2.00000000"), dec("-3.00000000")],
    )
    .unwrap();
    assert_eq!(metrics.n, 2);
    assert_eq!(metrics.hit_rate, "1.00000000");
    assert_eq!(metrics.significance, "not_claimed");
}

#[test]
fn claim_discovery_fails_closed() {
    let ledger = VariantLedger::new();
    assert_eq!(
        claim_discovery(&ledger).unwrap_err(),
        ResearchError::SignificanceNotClaimed
    );
}
