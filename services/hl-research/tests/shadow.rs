use domain_types::{KnownTime, ProtocolTime};
use hl_research::{
    ResearchError, ShadowCapture, ShadowDecision, ShadowOutcome, run_shadow_capture_bytes,
};

fn decision() -> ShadowDecision {
    ShadowDecision {
        id: "d1".to_owned(),
        decided_at: ProtocolTime::from_unix_micros(1000).unwrap(),
        known_at: KnownTime::from_unix_micros(1100).unwrap(),
        prediction: "long".to_owned(),
        expected_cost: "0.01000000".to_owned(),
        model_hash: "aa".to_owned(),
        feature_hash: "bb".to_owned(),
    }
}

#[test]
fn shadow_capture_records_decision_then_later_outcome() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/research/shadow-capture-v1.json");
    let report = run_shadow_capture_bytes(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report.mode, "shadow_capture");
    assert_eq!(report.shadow_live, "capture_only");
    assert!(!report.live_trading);
    assert!(!report.signer_attached);
    assert!(!report.alpha_quality_claimed);
    assert!(!report.alpha_qualified);
    assert!(!report.significance_claimed);
    assert!(!report.stage_pass_claimed);
    assert!(!report.live_corpus);
    assert!(!report.replica_cmds_used);
    assert_eq!(report.decisions, 1);
    assert_eq!(report.outcomes, 1);
}

#[test]
fn outcome_before_horizon_is_refused() {
    let mut capture = ShadowCapture::new();
    capture.record_decision(decision()).unwrap();
    let error = capture
        .record_outcome(
            ShadowOutcome {
                id: "d1".to_owned(),
                observed_at: ProtocolTime::from_unix_micros(1200).unwrap(),
                known_at: KnownTime::from_unix_micros(1300).unwrap(),
                realized_net: "0.01".to_owned(),
            },
            KnownTime::from_unix_micros(10000).unwrap(),
            1000,
        )
        .unwrap_err();
    assert_eq!(
        error,
        ResearchError::ShadowLeakage {
            field: "outcome.before_horizon",
        }
    );
}

#[test]
fn future_outcome_past_cutoff_is_refused() {
    let mut capture = ShadowCapture::new();
    capture.record_decision(decision()).unwrap();
    let error = capture
        .record_outcome(
            ShadowOutcome {
                id: "d1".to_owned(),
                observed_at: ProtocolTime::from_unix_micros(20000).unwrap(),
                known_at: KnownTime::from_unix_micros(21000).unwrap(),
                realized_net: "0.01".to_owned(),
            },
            KnownTime::from_unix_micros(10000).unwrap(),
            1000,
        )
        .unwrap_err();
    assert_eq!(
        error,
        ResearchError::FutureData {
            field: "outcome.known_at",
        }
    );
}

#[test]
fn shadow_capture_cannot_attach_a_trading_signer_or_go_live() {
    let capture = ShadowCapture::new();
    assert_eq!(
        capture.attach_trading_signer(&[1, 2, 3]).unwrap_err(),
        ResearchError::TradingSignerForbidden
    );
    assert_eq!(
        capture.promote_to_live_trading().unwrap_err(),
        ResearchError::TradingSignerForbidden
    );
    assert_eq!(
        capture.promote_to_shadow_registry().unwrap_err(),
        ResearchError::ShadowLiveNotImplemented
    );
}
