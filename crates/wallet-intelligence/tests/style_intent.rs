use domain_types::{EvidenceId, KnownTime, ProbabilityPpm, ProtocolTime};
use feature_core::{EvidenceKind, EvidenceRef};
use wallet_intelligence::{
    HedgeEvidence, IntentClass, IntentFeatures, StyleClass, StyleFeatures, assess_hedge,
    classify_intent, classify_style,
};

fn time(label: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(label).unwrap()
}

#[test]
fn style_probabilities_sum_to_one_million_ppm() {
    let snapshot = classify_style(
        &StyleFeatures {
            maker_ratio_ppm: Some(850_000),
            turnover_ppm: Some(200_000),
            hold_period_micros: Some(2_000_000),
            inventory_reversion_ppm: Some(800_000),
            directional_beta_milli: Some(10),
            funding_sensitivity_ppm: Some(10_000),
            spot_perp_offset_ppm: Some(0),
            sync_activity_ppm: Some(0),
            response_lag_micros: Some(10_000_000),
            liquidation_flag: Some(false),
            vault_flag: Some(false),
        },
        time(500),
    )
    .unwrap();
    let total: u32 = snapshot
        .probabilities
        .iter()
        .map(|(_, probability)| probability.ppm())
        .sum();
    assert_eq!(total, 1_000_000);
    let maker = snapshot
        .probabilities
        .iter()
        .find(|(class, _)| *class == StyleClass::MarketMaking)
        .unwrap();
    assert!(maker.1.ppm() > 100_000);
}

#[test]
fn missing_style_inputs_raise_unclassified_mass() {
    let complete = classify_style(
        &StyleFeatures {
            maker_ratio_ppm: Some(200_000),
            turnover_ppm: Some(200_000),
            hold_period_micros: None,
            inventory_reversion_ppm: None,
            directional_beta_milli: None,
            funding_sensitivity_ppm: None,
            spot_perp_offset_ppm: None,
            sync_activity_ppm: None,
            response_lag_micros: None,
            liquidation_flag: None,
            vault_flag: None,
        },
        time(1),
    )
    .unwrap();
    let missing = classify_style(
        &StyleFeatures {
            maker_ratio_ppm: None,
            turnover_ppm: None,
            hold_period_micros: None,
            inventory_reversion_ppm: None,
            directional_beta_milli: None,
            funding_sensitivity_ppm: None,
            spot_perp_offset_ppm: None,
            sync_activity_ppm: None,
            response_lag_micros: None,
            liquidation_flag: None,
            vault_flag: None,
        },
        time(1),
    )
    .unwrap();
    let unclassified = |snapshot: &wallet_intelligence::StyleSnapshot| {
        snapshot
            .probabilities
            .iter()
            .find(|(class, _)| *class == StyleClass::UnclassifiedMixed)
            .map(|(_, probability)| probability.ppm())
            .unwrap()
    };
    assert!(unclassified(&missing) > unclassified(&complete));
    assert!(missing.missing_critical_inputs);
}

#[test]
fn style_change_does_not_rewrite_earlier_snapshot() {
    let early = classify_style(
        &StyleFeatures {
            maker_ratio_ppm: Some(850_000),
            turnover_ppm: Some(100_000),
            hold_period_micros: Some(10_000_000),
            inventory_reversion_ppm: Some(800_000),
            directional_beta_milli: Some(0),
            funding_sensitivity_ppm: Some(0),
            spot_perp_offset_ppm: Some(0),
            sync_activity_ppm: Some(0),
            response_lag_micros: Some(10_000_000),
            liquidation_flag: Some(false),
            vault_flag: Some(false),
        },
        time(499),
    )
    .unwrap();
    let later = classify_style(
        &StyleFeatures {
            maker_ratio_ppm: Some(100_000),
            turnover_ppm: Some(900_000),
            hold_period_micros: Some(1_000_000),
            inventory_reversion_ppm: Some(10_000),
            directional_beta_milli: Some(900),
            funding_sensitivity_ppm: Some(0),
            spot_perp_offset_ppm: Some(0),
            sync_activity_ppm: Some(0),
            response_lag_micros: Some(10_000_000),
            liquidation_flag: Some(false),
            vault_flag: Some(false),
        },
        time(500),
    )
    .unwrap();
    assert_eq!(early.effective_at.unix_micros(), 499);
    assert_ne!(early.probabilities, later.probabilities);
}

#[test]
fn intent_unknown_increases_when_inputs_are_missing() {
    let known = classify_intent(
        &IntentFeatures {
            position_was_flat: Some(true),
            size_increased: Some(true),
            size_decreased: Some(false),
            leverage_decreased: Some(false),
            maker_inventory: Some(false),
            carry_or_basis: Some(false),
            liquidation: Some(false),
            transfer: Some(false),
            hedge_evidence: Some(false),
        },
        time(1),
    )
    .unwrap();
    let missing = classify_intent(
        &IntentFeatures {
            position_was_flat: None,
            size_increased: None,
            size_decreased: None,
            leverage_decreased: None,
            maker_inventory: None,
            carry_or_basis: None,
            liquidation: None,
            transfer: None,
            hedge_evidence: None,
        },
        time(1),
    )
    .unwrap();
    let unknown = |snapshot: &wallet_intelligence::IntentSnapshot| {
        snapshot
            .probabilities
            .iter()
            .find(|(class, _)| *class == IntentClass::Unknown)
            .map(|(_, probability)| probability.ppm())
            .unwrap()
    };
    assert!(unknown(&missing) > unknown(&known));
    let total: u32 = missing
        .probabilities
        .iter()
        .map(|(_, probability)| probability.ppm())
        .sum();
    assert_eq!(total, 1_000_000);
}

#[test]
fn hedge_assessment_exposes_external_uncertainty() {
    let digest = [2_u8; 32];
    let evidence = EvidenceRef::try_new(
        EvidenceKind::StateSnapshot,
        EvidenceId::new("hedge-1").unwrap(),
        digest,
        time(1),
        KnownTime::from_unix_micros(1).unwrap(),
    )
    .unwrap();
    let assessment = assess_hedge(
        HedgeEvidence {
            opposing_spot_perp: true,
            correlated_opposite_positions: true,
            synchronized_changes: false,
            funding_sensitivity: true,
            low_net_beta_high_turnover: false,
            market_maker_inventory_reversion: false,
            linked_account_activity: false,
        },
        vec![evidence],
    )
    .unwrap();
    assert!(assessment.on_platform_hedge_probability > ProbabilityPpm::ZERO);
    assert!(assessment.external_hedge_uncertainty > ProbabilityPpm::ZERO);
    assert!(
        assessment
            .limitations
            .iter()
            .any(|limitation| limitation == "off_platform_hedges_unobservable")
    );
}
