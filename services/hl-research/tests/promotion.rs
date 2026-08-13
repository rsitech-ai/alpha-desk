use std::path::{Path, PathBuf};

use domain_types::Decimal;
use hl_research::{
    GateDecision, HoldoutLock, PromotionEvidence, ResearchError, ResearchStatus, calibrate_scores,
    evaluate_promotion, lock_path_is_in_repo, promote, run_promote_bytes, stamp_holdout_passed,
    stationary_block_bootstrap,
};

fn dec(value: &str) -> Decimal {
    Decimal::parse_at_scale(value, 8).unwrap()
}

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/research")
}

fn withheld_evidence<'a>(
    bootstrap: &'a hl_research::BootstrapReport,
    calibration: &'a hl_research::CalibrationReport,
    episode_shares_ppm: &'a [u32],
    outcome_count: usize,
) -> PromotionEvidence<'a> {
    PromotionEvidence {
        outcome_count,
        holdout_lock: None,
        holdout_outcome_count: 0,
        calendar_days: None,
        bootstrap,
        calibration,
        metrics: None,
        shadow_live: false,
        episode_shares_ppm,
    }
}

fn assert_unclaimed(value: &serde_json::Value, field: &str) {
    assert_eq!(value[field], false, "{field} must serialize as false");
}

#[test]
fn promotion_without_locked_holdout_is_withheld_and_cannot_stamp_pass() {
    let outcomes = [dec("1.00000000"); 12];
    let bootstrap = stationary_block_bootstrap(&outcomes, 2, 200, 7).unwrap();
    let calibration = calibrate_scores(&outcomes, &outcomes).unwrap();
    let report = evaluate_promotion(&withheld_evidence(
        &bootstrap,
        &calibration,
        &[500_000, 500_000],
        12,
    ));
    assert_eq!(report.decision, "withheld");
    assert!(!report.promoted);
    assert!(!report.holdout_passed);
    assert!(!report.alpha_quality_claimed);
    assert!(!report.alpha_qualified);
    assert!(!report.significance_claimed);
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
    assert_eq!(holdout.reason, "no_locked_holdout");
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
    let report = evaluate_promotion(&withheld_evidence(&bootstrap, &calibration, &[300_000], 8));
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
    assert!(!report.alpha_qualified);
    assert!(!report.significance_claimed);
}

#[test]
fn promote_cli_path_returns_withheld_report() {
    let path = fixture_dir().join("fold-estimator-v1.json");
    assert!(lock_path_is_in_repo(&path));
    assert_eq!(
        HoldoutLock::open(&path).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
    let report = run_promote_bytes(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(report.decision, "withheld");
    assert!(!report.promoted);
    assert!(!report.holdout_passed);
    assert!(!report.alpha_qualified);
    assert!(!report.significance_claimed);
    assert_eq!(
        promote(&report).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}

#[test]
fn in_repo_fixtures_cannot_invent_a_holdout_lock() {
    let dir = fixture_dir();
    let mut saw_fixture = false;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if !path.is_file() {
            continue;
        }
        saw_fixture = true;
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            !name.contains("replica_cmds"),
            "research fixtures must not ship replica_cmds: {name}"
        );
        assert!(
            path.extension().and_then(|ext| ext.to_str()) != Some("lock"),
            "research fixtures must not ship a holdout lock file: {name}"
        );
        assert!(lock_path_is_in_repo(&path));
        assert_eq!(
            HoldoutLock::open(&path).unwrap_err(),
            ResearchError::HoldoutNotImplemented
        );
        let bytes = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        assert!(
            !text.contains("replica_cmds"),
            "{} must not reference replica_cmds",
            path.display()
        );
        assert!(
            !text.contains("HOLDOUT_PASSED"),
            "{} must not claim HOLDOUT_PASSED",
            path.display()
        );
        assert_eq!(
            HoldoutLock::from_bytes(&bytes).unwrap_err(),
            ResearchError::HoldoutNotImplemented
        );
    }
    assert!(saw_fixture);

    let invented = br#"{"locked":true,"holdout_passed":true,"alpha_qualified":true,"significance_claimed":true}"#;
    assert_eq!(
        HoldoutLock::from_bytes(invented).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}

#[test]
fn invented_lock_bytes_and_temp_files_still_cannot_pass_holdout() {
    let fake = std::env::temp_dir().join("alpha-desk-invented-holdout.lock");
    std::fs::write(
        &fake,
        br#"{"holdout_passed":true,"alpha_qualified":true,"significance_claimed":true}"#,
    )
    .unwrap();
    let error = HoldoutLock::open(&fake).unwrap_err();
    let _ = std::fs::remove_file(&fake);
    assert_eq!(error, ResearchError::HoldoutNotImplemented);

    let crate_src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(lock_path_is_in_repo(&crate_src));
    assert_eq!(
        HoldoutLock::open(&crate_src).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}

#[test]
fn mutated_promotion_report_cannot_serialize_alpha_or_holdout_pass() {
    let outcomes = [dec("0.50000000"); 40];
    let bootstrap = stationary_block_bootstrap(&outcomes, 4, 200, 7).unwrap();
    let calibration = calibrate_scores(&outcomes, &outcomes).unwrap();
    let mut report = evaluate_promotion(&PromotionEvidence {
        outcome_count: 120,
        holdout_lock: None,
        holdout_outcome_count: 40,
        calendar_days: Some(120),
        bootstrap: &bootstrap,
        calibration: &calibration,
        metrics: None,
        shadow_live: false,
        episode_shares_ppm: &[10_000; 12],
    });
    report.promoted = true;
    report.holdout_passed = true;
    report.alpha_quality_claimed = true;
    report.alpha_qualified = true;
    report.significance_claimed = true;
    report.stage_pass_claimed = true;
    let encoded = serde_json::to_value(&report).unwrap();
    assert_eq!(encoded["decision"], "withheld");
    assert_unclaimed(&encoded, "promoted");
    assert_unclaimed(&encoded, "holdout_passed");
    assert_unclaimed(&encoded, "alpha_quality_claimed");
    assert_unclaimed(&encoded, "alpha_qualified");
    assert_unclaimed(&encoded, "significance_claimed");
    assert_unclaimed(&encoded, "stage_pass_claimed");
    assert_eq!(
        stamp_holdout_passed(&report).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}

#[test]
fn status_json_cannot_claim_alpha_significance_or_locked_corpus() {
    let mut status = ResearchStatus::current();
    assert!(!status.alpha_qualified);
    assert!(!status.significance_claimed);
    assert!(!status.locked_corpus);
    status.alpha_qualified = true;
    status.significance_claimed = true;
    status.alpha_quality_claimed = true;
    status.stage_pass_claimed = true;
    status.locked_corpus = true;
    let encoded = serde_json::to_value(&status).unwrap();
    assert_unclaimed(&encoded, "alpha_qualified");
    assert_unclaimed(&encoded, "significance_claimed");
    assert_unclaimed(&encoded, "alpha_quality_claimed");
    assert_unclaimed(&encoded, "stage_pass_claimed");
    assert_unclaimed(&encoded, "locked_corpus");
}

#[test]
fn research_sources_do_not_parse_replica_cmds_or_a_live_corpus() {
    let src = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    for root in [src, fixture_dir()] {
        walk_text_files(&root, &mut |path, text| {
            assert!(
                !text.contains("replica_cmds"),
                "{} must not reference replica_cmds",
                path.display()
            );
        });
    }
}

fn walk_text_files(dir: &Path, visit: &mut impl FnMut(&Path, &str)) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk_text_files(&path, visit);
            continue;
        }
        let allowed = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| matches!(ext, "rs" | "json" | "toml"));
        if !allowed {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        visit(&path, &text);
    }
}
