use hl_research::{EstimatorClass, ResearchError, VariantLedger, VariantStatus, score_predictions};

fn metrics() -> hl_research::PerformanceMetrics {
    score_predictions(
        &[domain_types::Decimal::parse_at_scale("1.00000000", 8).unwrap()],
        &[domain_types::Decimal::parse_at_scale("1.00000000", 8).unwrap()],
    )
    .unwrap()
}

#[test]
fn variant_identity_is_stable_and_family_scoped() {
    let mut ledger = VariantLedger::new();
    let first = ledger
        .register("family-a", EstimatorClass::UnivariateLinear)
        .unwrap();
    let again = ledger
        .register("family-a", EstimatorClass::UnivariateLinear)
        .unwrap();
    let other = ledger
        .register("family-b", EstimatorClass::UnivariateLinear)
        .unwrap();
    assert_eq!(first, again);
    assert_ne!(first, other);
    assert_eq!(ledger.len(), 2);
}

#[test]
fn metrics_cannot_be_rewritten_and_significance_is_refused() {
    let mut ledger = VariantLedger::new();
    let id = ledger
        .register("family-a", EstimatorClass::MeanOutcome)
        .unwrap();
    ledger.record_metrics(&id, metrics()).unwrap();
    let error = ledger.record_metrics(&id, metrics()).unwrap_err();
    assert_eq!(error, ResearchError::ImmutableVariant);
    ledger.mark_research_only(&id).unwrap();
    assert_eq!(ledger.records()[0].status, VariantStatus::ResearchOnly);
    assert_eq!(
        ledger.claim_significance().unwrap_err(),
        ResearchError::SignificanceNotClaimed
    );
    assert_eq!(
        ledger.accept_holdout(&id).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}
