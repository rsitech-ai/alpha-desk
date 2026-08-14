use std::collections::BTreeMap;

use domain_types::{
    BlockHeight, ClosedInterval, Direction, EntityId, EvidenceId, FeatureSetVersion, Horizon,
    KnownTime, MarketId, ModelVersion, ProbabilityPpm, ProtocolTime, SignalId, UsdAmount,
};
use feature_core::{
    EvidenceKind, EvidenceRef, FeatureValue, HealthAssessment, HealthState, MissingReason,
};
use market_intelligence::{market_feature_key, AnalogueSet, MarketFeatureSnapshot, MemorySupport};
use signal_core::{
    append_event, fold_lifecycle, transition_allowed, EvidenceBundle, InvalidationRule, Signal,
    SignalActor, SignalConfirmationClass, SignalError, SignalLifecycleEvent, SignalLifecycleState,
    SignalType,
};

fn time(micros: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(micros).unwrap()
}

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).unwrap()
}

fn health(state: HealthState) -> HealthAssessment {
    HealthAssessment::try_new("signal", state, "synthetic").unwrap()
}

fn snapshot(state: HealthState, signed: i64) -> MarketFeatureSnapshot {
    let mut values = BTreeMap::new();
    let flow = market_feature_key("smart_flow_acceleration_milli").unwrap();
    let book = market_feature_key("book").unwrap();
    let fills = market_feature_key("fills").unwrap();
    let inventory = market_feature_key("inventory").unwrap();
    if state == HealthState::Red {
        values.insert(flow, FeatureValue::Missing(MissingReason::RedDataHealth));
        values.insert(book, FeatureValue::Missing(MissingReason::RedDataHealth));
        values.insert(fills, FeatureValue::Missing(MissingReason::RedDataHealth));
        values.insert(
            inventory,
            FeatureValue::Missing(MissingReason::RedDataHealth),
        );
    } else {
        values.insert(flow, FeatureValue::SignedInteger(signed));
        values.insert(
            book,
            FeatureValue::Decimal {
                raw: 20_000 * 100_000_000,
                scale: 8,
            },
        );
        values.insert(fills, FeatureValue::Boolean(true));
        values.insert(
            inventory,
            FeatureValue::Decimal {
                raw: 100 * 100_000_000,
                scale: 8,
            },
        );
    }
    MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        time(1_000_000),
        known(1_000_000),
        BlockHeight::new(9),
        values,
        health(state),
    )
    .unwrap()
}

fn analogues() -> AnalogueSet {
    AnalogueSet {
        matches: Vec::new(),
        independent_episode_count: 0,
        support: MemorySupport::OutsideSupport {
            reason: "synthetic_unqualified".to_owned(),
        },
        historical_support: ProbabilityPpm::ZERO,
        manifest_hash: [2_u8; 32],
    }
}

fn event_ref() -> EvidenceRef {
    EvidenceRef::try_new(
        EvidenceKind::CanonicalEvent,
        EvidenceId::new("ev-1").unwrap(),
        [3_u8; 32],
        time(1_000_000),
        known(1_000_000),
    )
    .unwrap()
}

fn complete_bundle() -> EvidenceBundle {
    EvidenceBundle::try_new(
        SignalId::new("sig-1").unwrap(),
        vec![event_ref()],
        vec![(EntityId::new("e1").unwrap(), ProbabilityPpm::ONE)],
        snapshot(HealthState::Green, 10),
        snapshot(HealthState::Green, 40),
        BlockHeight::new(12),
        ProbabilityPpm::ONE,
        [4_u8; 32],
        "cc9faa2".to_owned(),
        [5_u8; 32],
        analogues(),
        vec![InvalidationRule::DataHealthNotGreen],
        UsdAmount::from_raw(1_000_000_000, 8).unwrap(),
        Horizon::MINUTES_5,
        vec!["synthetic_unqualified".to_owned()],
    )
    .unwrap()
}

fn incomplete_bundle() -> EvidenceBundle {
    EvidenceBundle::try_new(
        SignalId::new("sig-1").unwrap(),
        Vec::new(),
        Vec::new(),
        snapshot(HealthState::Green, 10),
        snapshot(HealthState::Green, 40),
        BlockHeight::new(0),
        ProbabilityPpm::ZERO,
        [0_u8; 32],
        "cc9faa2".to_owned(),
        [0_u8; 32],
        analogues(),
        Vec::new(),
        UsdAmount::from_raw(0, 8).unwrap(),
        Horizon::MINUTES_5,
        Vec::new(),
    )
    .unwrap()
}

#[test]
fn candidate_to_live_is_invalid() {
    let bundle = complete_bundle();
    let error = transition_allowed(
        Some(SignalLifecycleState::Candidate),
        SignalLifecycleState::Live,
        &SignalType::IndependentSmartFlowAcceleration,
        &bundle,
        HealthState::Green,
        true,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        SignalError::InvalidTransition {
            from: SignalLifecycleState::Candidate,
            to: SignalLifecycleState::Live,
        }
    ));
}

#[test]
fn incomplete_evidence_cannot_validate_or_go_live() {
    let bundle = incomplete_bundle();
    let missing = bundle.missing_for_admission();
    assert!(missing.contains(&"canonical_event_refs".to_owned()));
    assert!(missing.contains(&"model_artifact_hash".to_owned()));
    assert!(missing.contains(&"cost_assumptions".to_owned()));
    assert!(missing.contains(&"invalidation_rules".to_owned()));
    assert!(missing.contains(&"data_watermark".to_owned()));
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Candidate),
            SignalLifecycleState::Validated,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::IncompleteEvidence(_))
    ));
}

#[test]
fn research_only_cannot_enter_live() {
    let bundle = complete_bundle();
    let research = SignalType::research_only("originator-accumulation").unwrap();
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Validated),
            SignalLifecycleState::Live,
            &research,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::ResearchOnlyCannotGoLive)
    ));
}

#[test]
fn red_health_blocks_validation() {
    let bundle = complete_bundle();
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Candidate),
            SignalLifecycleState::Validated,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Red,
            true,
        ),
        Err(SignalError::UnsupportedHealth)
    ));
}

#[test]
fn append_only_fold_rejects_candidate_to_live_and_accepts_validated() {
    let bundle = complete_bundle();
    let create = SignalLifecycleEvent::try_new(
        SignalId::new("sig-1").unwrap(),
        None,
        SignalLifecycleState::Candidate,
        time(1_000_000),
        known(1_000_000),
        "created".to_owned(),
        bundle.content_hash,
        "cc9faa2".to_owned(),
        SignalActor::System,
    )
    .unwrap();
    let events = append_event(
        &[],
        create,
        &SignalType::IndependentSmartFlowAcceleration,
        &bundle,
        HealthState::Green,
        false,
    )
    .unwrap();
    assert_eq!(
        fold_lifecycle(&events).unwrap(),
        SignalLifecycleState::Candidate
    );
    let validated = SignalLifecycleEvent::try_new(
        SignalId::new("sig-1").unwrap(),
        Some(SignalLifecycleState::Candidate),
        SignalLifecycleState::Validated,
        time(1_000_000),
        known(2_000_000),
        "gates_passed".to_owned(),
        bundle.content_hash,
        "cc9faa2".to_owned(),
        SignalActor::System,
    )
    .unwrap();
    let events = append_event(
        &events,
        validated,
        &SignalType::IndependentSmartFlowAcceleration,
        &bundle,
        HealthState::Green,
        true,
    )
    .unwrap();
    assert_eq!(
        fold_lifecycle(&events).unwrap(),
        SignalLifecycleState::Validated
    );
}

#[test]
fn missing_book_or_fills_cannot_validate_or_go_live() {
    let mut values = BTreeMap::new();
    values.insert(
        market_feature_key("smart_flow_acceleration_milli").unwrap(),
        FeatureValue::SignedInteger(10),
    );
    values.insert(
        market_feature_key("book").unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    values.insert(
        market_feature_key("fills").unwrap(),
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    values.insert(
        market_feature_key("inventory").unwrap(),
        FeatureValue::Decimal {
            raw: 100 * 100_000_000,
            scale: 8,
        },
    );
    let missing = MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        time(1_000_000),
        known(1_000_000),
        BlockHeight::new(9),
        values,
        health(HealthState::Green),
    )
    .unwrap();
    let bundle = EvidenceBundle::try_new(
        SignalId::new("sig-1").unwrap(),
        vec![event_ref()],
        vec![(EntityId::new("e1").unwrap(), ProbabilityPpm::ONE)],
        missing.clone(),
        missing,
        BlockHeight::new(12),
        ProbabilityPpm::ONE,
        [4_u8; 32],
        "cc9faa2".to_owned(),
        [5_u8; 32],
        analogues(),
        vec![InvalidationRule::DataHealthNotGreen],
        UsdAmount::from_raw(1_000_000_000, 8).unwrap(),
        Horizon::MINUTES_5,
        vec!["synthetic_unqualified".to_owned()],
    )
    .unwrap();
    let missing_inputs = bundle.missing_for_admission();
    assert!(missing_inputs.contains(&"book".to_owned()));
    assert!(missing_inputs.contains(&"fills".to_owned()));
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Candidate),
            SignalLifecycleState::Validated,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::IncompleteEvidence(_))
    ));
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Validated),
            SignalLifecycleState::Live,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::IncompleteEvidence(_))
    ));
}

#[test]
fn boolean_book_cannot_validate_or_go_live() {
    let mut values = BTreeMap::new();
    values.insert(
        market_feature_key("smart_flow_acceleration_milli").unwrap(),
        FeatureValue::SignedInteger(10),
    );
    values.insert(
        market_feature_key("book").unwrap(),
        FeatureValue::Boolean(true),
    );
    values.insert(
        market_feature_key("fills").unwrap(),
        FeatureValue::Boolean(true),
    );
    values.insert(
        market_feature_key("inventory").unwrap(),
        FeatureValue::Decimal {
            raw: 100 * 100_000_000,
            scale: 8,
        },
    );
    let minted = MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        time(1_000_000),
        known(1_000_000),
        BlockHeight::new(9),
        values,
        health(HealthState::Green),
    )
    .unwrap();
    let bundle = EvidenceBundle::try_new(
        SignalId::new("sig-1").unwrap(),
        vec![event_ref()],
        vec![(EntityId::new("e1").unwrap(), ProbabilityPpm::ONE)],
        minted.clone(),
        minted,
        BlockHeight::new(12),
        ProbabilityPpm::ONE,
        [4_u8; 32],
        "cc9faa2".to_owned(),
        [5_u8; 32],
        analogues(),
        vec![InvalidationRule::DataHealthNotGreen],
        UsdAmount::from_raw(1_000_000_000, 8).unwrap(),
        Horizon::MINUTES_5,
        vec!["synthetic_unqualified".to_owned()],
    )
    .unwrap();
    let missing_inputs = bundle.missing_for_admission();
    assert!(missing_inputs.contains(&"book".to_owned()));
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Candidate),
            SignalLifecycleState::Validated,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::IncompleteEvidence(_))
    ));
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Validated),
            SignalLifecycleState::Live,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::IncompleteEvidence(_))
    ));
}

#[test]
fn missing_inventory_cannot_validate_or_go_live() {
    let decimal_book = FeatureValue::Decimal {
        raw: 20_000 * 100_000_000,
        scale: 8,
    };
    let true_fills = FeatureValue::Boolean(true);
    let bundle = inventory_bundle(
        decimal_book,
        true_fills,
        FeatureValue::Missing(MissingReason::NotObserved),
    );
    let missing_inputs = bundle.missing_for_admission();
    assert!(missing_inputs.contains(&"inventory".to_owned()));
    assert!(!missing_inputs.contains(&"book".to_owned()));
    assert!(!missing_inputs.contains(&"fills".to_owned()));
    assert_eq!(bundle.malformed_for_admission(), None);
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Candidate),
            SignalLifecycleState::Validated,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::IncompleteEvidence(missing)) if missing.contains(&"inventory".to_owned())
    ));
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Validated),
            SignalLifecycleState::Live,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::IncompleteEvidence(missing)) if missing.contains(&"inventory".to_owned())
    ));
}

#[test]
fn boolean_inventory_cannot_validate_or_go_live() {
    let decimal_book = FeatureValue::Decimal {
        raw: 20_000 * 100_000_000,
        scale: 8,
    };
    let true_fills = FeatureValue::Boolean(true);
    let bundle = inventory_bundle(decimal_book, true_fills, FeatureValue::Boolean(true));
    let missing_inputs = bundle.missing_for_admission();
    assert!(!missing_inputs.contains(&"inventory".to_owned()));
    assert!(!missing_inputs.contains(&"book".to_owned()));
    assert!(!missing_inputs.contains(&"fills".to_owned()));
    assert_eq!(
        bundle.malformed_for_admission(),
        Some(("inventory", "boolean cannot mint decimal depth"))
    );
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Candidate),
            SignalLifecycleState::Validated,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
    assert!(matches!(
        transition_allowed(
            Some(SignalLifecycleState::Validated),
            SignalLifecycleState::Live,
            &SignalType::IndependentSmartFlowAcceleration,
            &bundle,
            HealthState::Green,
            true,
        ),
        Err(SignalError::Malformed {
            what: "inventory",
            reason: "boolean cannot mint decimal depth",
        })
    ));
}

fn inventory_bundle(
    book: FeatureValue,
    fills: FeatureValue,
    inventory: FeatureValue,
) -> EvidenceBundle {
    let mut values = BTreeMap::new();
    values.insert(
        market_feature_key("smart_flow_acceleration_milli").unwrap(),
        FeatureValue::SignedInteger(10),
    );
    values.insert(market_feature_key("book").unwrap(), book);
    values.insert(market_feature_key("fills").unwrap(), fills);
    values.insert(market_feature_key("inventory").unwrap(), inventory);
    let minted = MarketFeatureSnapshot::try_new(
        MarketId::new("BTC").unwrap(),
        Horizon::MINUTES_5,
        FeatureSetVersion::new("market-v1").unwrap(),
        time(1_000_000),
        known(1_000_000),
        BlockHeight::new(9),
        values,
        health(HealthState::Green),
    )
    .unwrap();
    EvidenceBundle::try_new(
        SignalId::new("sig-1").unwrap(),
        vec![event_ref()],
        vec![(EntityId::new("e1").unwrap(), ProbabilityPpm::ONE)],
        minted.clone(),
        minted,
        BlockHeight::new(12),
        ProbabilityPpm::ONE,
        [4_u8; 32],
        "cc9faa2".to_owned(),
        [5_u8; 32],
        analogues(),
        vec![InvalidationRule::DataHealthNotGreen],
        UsdAmount::from_raw(1_000_000_000, 8).unwrap(),
        Horizon::MINUTES_5,
        vec!["synthetic_unqualified".to_owned()],
    )
    .unwrap()
}

#[test]
fn confirmation_live_gate_covers_every_class() {
    for class in [
        SignalConfirmationClass::CommittedPrimary,
        SignalConfirmationClass::CommittedIndependent,
        SignalConfirmationClass::ProvisionalSource,
        SignalConfirmationClass::SyntheticUnqualified,
    ] {
        match class {
            SignalConfirmationClass::CommittedPrimary
            | SignalConfirmationClass::CommittedIndependent => {
                assert!(
                    class.can_enter_live(),
                    "{class:?} committed lanes stay live-capable without new qualification"
                );
            }
            SignalConfirmationClass::ProvisionalSource
            | SignalConfirmationClass::SyntheticUnqualified => {
                assert!(
                    !class.can_enter_live(),
                    "{class:?} must stay fail-closed for live entry"
                );
            }
        }
    }
}

#[test]
fn synthetic_confirmation_cannot_construct_live_signal() {
    let error = Signal::try_new(
        SignalId::new("sig-live").unwrap(),
        SignalType::IndependentSmartFlowAcceleration,
        MarketId::new("BTC").unwrap(),
        Direction::Long,
        known(1_000_000),
        time(1_000_000),
        BlockHeight::new(12),
        SignalConfirmationClass::SyntheticUnqualified,
        Horizon::MINUTES_5,
        domain_types::BasisPoints::from_raw(20, 0).unwrap(),
        domain_types::BasisPoints::from_raw(5, 0).unwrap(),
        ProbabilityPpm::ONE,
        ClosedInterval::new(
            domain_types::BasisPoints::from_raw(1, 0).unwrap(),
            domain_types::BasisPoints::from_raw(30, 0).unwrap(),
        )
        .unwrap(),
        UsdAmount::from_raw(1, 8).unwrap(),
        Horizon::MINUTES_5,
        ProbabilityPpm::from_ppm(100_000).unwrap(),
        domain_types::BasisPoints::from_raw(10, 0).unwrap(),
        health(HealthState::Green),
        ModelVersion::new("signals-v1").unwrap(),
        FeatureSetVersion::new("market-v1").unwrap(),
        [7_u8; 32],
        [8_u8; 32],
        SignalLifecycleState::Live,
    )
    .unwrap_err();
    assert!(matches!(error, SignalError::ContractViolation(_)));
}
