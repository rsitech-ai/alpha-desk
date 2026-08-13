use domain_types::{BlockHeight, BlockRange, ExperimentId, FeatureSetVersion, LabelDefinitionId};
use hl_research::{ExperimentManifest, ExperimentRegistry, ResearchError, run_synthetic_bytes};

fn complete_manifest() -> ExperimentManifest {
    ExperimentManifest {
        hypothesis: "synthetic edge".to_owned(),
        owner: "researcher".to_owned(),
        code_commit: "abc".to_owned(),
        rust_toolchain: "1.97.1".to_owned(),
        feature_set_version: FeatureSetVersion::new("features-v1").unwrap(),
        label_definition: LabelDefinitionId::new("executable-net-return-v1").unwrap(),
        market_universe_version: "synth".to_owned(),
        wallet_score_version: "none".to_owned(),
        cluster_version_policy: "none".to_owned(),
        training_range: BlockRange::new(BlockHeight::new(1), BlockHeight::new(10)).unwrap(),
        validation_ranges: vec![
            BlockRange::new(BlockHeight::new(11), BlockHeight::new(12)).unwrap(),
        ],
        holdout_range: BlockRange::new(BlockHeight::new(13), BlockHeight::new(14)).unwrap(),
        data_manifest_hash: "00".to_owned(),
        model_config_hash: "00".to_owned(),
        random_seed: 7,
        cost_model_version: "synthetic-cost-v1".to_owned(),
        execution_latency_assumptions: "fixed-zero".to_owned(),
        promotion_metrics: vec!["net_pnl".to_owned()],
        reviewers: vec!["fixture".to_owned()],
    }
}

#[test]
fn incomplete_manifest_cannot_register() {
    let mut manifest = complete_manifest();
    manifest.hypothesis.clear();
    let mut registry = ExperimentRegistry::new();
    let error = registry.submit(manifest).unwrap_err();
    assert_eq!(
        error,
        ResearchError::IncompleteManifest {
            field: "hypothesis",
        }
    );
}

#[test]
fn registered_manifest_is_immutable() {
    let mut registry = ExperimentRegistry::new();
    let record = registry.submit(complete_manifest()).unwrap();
    let mut changed = complete_manifest();
    changed.hypothesis = "mutated".to_owned();
    let error = registry
        .replace_registered(&record.experiment_id, changed)
        .unwrap_err();
    assert_eq!(error, ResearchError::ImmutableExperiment);
    let again = registry.submit(complete_manifest()).unwrap();
    assert_eq!(again.experiment_id, record.experiment_id);
}

#[test]
fn locked_holdout_pass_remains_unimplemented() {
    let mut registry = ExperimentRegistry::new();
    let record = registry.submit(complete_manifest()).unwrap();
    assert_eq!(
        registry.open_holdout(&record.experiment_id).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
    let _ = ExperimentId::new("unused");
}

#[test]
fn synthetic_experiment_runs_and_does_not_claim_stage_pass() {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures/research/synthetic-experiment-v1.json");
    let bytes = std::fs::read(path).unwrap();
    let report = run_synthetic_bytes(&bytes, None, None).unwrap();
    assert_eq!(report.mode, "synthetic");
    assert!(!report.alpha_quality_claimed);
    assert!(!report.alpha_qualified);
    assert!(!report.significance_claimed);
    assert!(!report.stage_pass_claimed);
    assert!(!report.live_corpus);
    assert!(!report.replica_cmds_used);
    assert_eq!(report.walk_forward, "not_evaluated");
    assert_eq!(report.holdout, "not_evaluated");
    assert_eq!(report.shadow_live, "not_evaluated");
    assert_eq!(report.filled_quantity, "0.40000000");
}

#[test]
fn future_data_in_a_synthetic_fixture_is_refused() {
    let mut fixture: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/research/synthetic-experiment-v1.json"),
        )
        .unwrap(),
    )
    .unwrap();
    fixture["simulation"]["evaluation_known_at"] = serde_json::json!(10);
    fixture["simulation"]["books"][1]["known_at"] = serde_json::json!(1_000_000);
    let encoded = serde_json::to_vec(&fixture).unwrap();
    let error = run_synthetic_bytes(&encoded, None, None).unwrap_err();
    assert_eq!(
        error,
        ResearchError::FutureData {
            field: "book.known_at",
        }
    );
}
