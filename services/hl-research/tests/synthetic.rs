use std::collections::BTreeMap;
use std::path::Path;

use domain_types::{BlockHeight, BlockRange, ExperimentId, FeatureSetVersion, LabelDefinitionId};
use ed25519_dalek::SigningKey;
use hl_research::{ExperimentManifest, ExperimentRegistry, ResearchError, run_synthetic_bytes};
use model_runtime::sign_files;

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

#[test]
fn live_named_bundle_dir_is_refused_even_when_files_exist() {
    let root = tempfile::tempdir().unwrap();
    let bundle_dir = root.path().join("live").join("signed-bundle");
    let approved_key = write_signed_linear_bundle(&bundle_dir);
    let error = run_synthetic_bytes(
        &synthetic_experiment_with_features(),
        Some(&bundle_dir),
        Some(approved_key),
    )
    .unwrap_err();
    assert_eq!(error, ResearchError::LiveCorpusForbidden);
}

#[test]
fn locked_holdout_named_bundle_dir_is_refused_even_when_files_exist() {
    let root = tempfile::tempdir().unwrap();
    let bundle_dir = root.path().join("locked-holdout").join("signed-bundle");
    let approved_key = write_signed_linear_bundle(&bundle_dir);
    let error = run_synthetic_bytes(
        &synthetic_experiment_with_features(),
        Some(&bundle_dir),
        Some(approved_key),
    )
    .unwrap_err();
    assert_eq!(error, ResearchError::LockedCorpusForbidden);
}

#[test]
fn synthetic_bundle_dir_still_loads_when_path_is_admitted() {
    let root = tempfile::tempdir().unwrap();
    let bundle_dir = root.path().join("synthetic-bundle");
    let approved_key = write_signed_linear_bundle(&bundle_dir);
    let report = run_synthetic_bytes(
        &synthetic_experiment_with_features(),
        Some(&bundle_dir),
        Some(approved_key),
    )
    .unwrap();
    assert_eq!(report.mode, "synthetic");
    assert_eq!(report.model_score.as_deref(), Some("2.10000000"));
    assert!(!report.live_corpus);
    assert!(!report.replica_cmds_used);
    let encoded = serde_json::to_value(&report).unwrap();
    assert_eq!(encoded["live_corpus"], false);
    assert_eq!(encoded["replica_cmds_used"], false);
}

fn synthetic_experiment_bytes() -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/research/synthetic-experiment-v1.json"),
    )
    .unwrap()
}

fn synthetic_experiment_with_features() -> Vec<u8> {
    let mut fixture: serde_json::Value =
        serde_json::from_slice(&synthetic_experiment_bytes()).unwrap();
    fixture["model_features"] = serde_json::json!({
        "names": ["flow", "crowding"],
        "values": ["2", "4"]
    });
    serde_json::to_vec(&fixture).unwrap()
}

fn write_signed_linear_bundle(dir: &Path) -> [u8; 32] {
    let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
    let mut files = linear_bundle_files();
    let signature = sign_files(&files, &signing_key);
    files.insert("signature.ed25519".to_owned(), signature.to_vec());
    std::fs::create_dir_all(dir).unwrap();
    for (name, bytes) in &files {
        std::fs::write(dir.join(name), bytes).unwrap();
    }
    signing_key.verifying_key().to_bytes()
}

fn linear_bundle_files() -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    files.insert(
        "manifest.toml".to_owned(),
        br#"model_id = "linear-synthetic-v1"
semantic_version = "0.1.0"
feature_set_version = "features-v1"
artifact_kind = "deterministic-linear-v1"
review_expires_unix_micros = 4102444800000000
approved_use = ["synthetic-research"]
prohibited_use = ["production-inference", "live-trading"]
"#
        .to_vec(),
    );
    files.insert(
        "feature-schema.json".to_owned(),
        br#"{"ordered_features":["flow","crowding"]}"#.to_vec(),
    );
    files.insert("preprocessing.json".to_owned(), b"{}".to_vec());
    files.insert("calibration.json".to_owned(), b"{}".to_vec());
    files.insert("evaluation.json".to_owned(), b"{}".to_vec());
    files.insert("training-data-manifest.json".to_owned(), b"{}".to_vec());
    files.insert(
        "model-card.md".to_owned(),
        b"synthetic linear fixture".to_vec(),
    );
    files.insert(
        "model.linear-v1.json".to_owned(),
        br#"{"kind":"deterministic-linear-v1","weights":["0.5","0.25"],"intercept":"0.1"}"#
            .to_vec(),
    );
    files
}
