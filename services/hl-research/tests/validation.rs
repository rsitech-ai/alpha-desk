use domain_types::{BlockHeight, BlockRange, FeatureSetVersion, LabelDefinitionId};
use hl_research::{
    DatasetAccess, ExperimentManifest, ExperimentRegistry, HoldoutState, LabeledRow, ResearchError,
    ResearchStatus, ValidationPolicy, run_holdout_isolation_bytes, run_walk_forward_bytes,
};

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
        training_range: BlockRange::new(BlockHeight::new(1), BlockHeight::new(100)).unwrap(),
        validation_ranges: vec![
            BlockRange::new(BlockHeight::new(101), BlockHeight::new(110)).unwrap(),
            BlockRange::new(BlockHeight::new(111), BlockHeight::new(120)).unwrap(),
        ],
        holdout_range: BlockRange::new(BlockHeight::new(151), BlockHeight::new(160)).unwrap(),
        data_manifest_hash: "00".to_owned(),
        model_config_hash: "00".to_owned(),
        random_seed: 7,
        cost_model_version: "synthetic-cost-v1".to_owned(),
        execution_latency_assumptions: "fixed-zero".to_owned(),
        promotion_metrics: vec!["net_pnl".to_owned()],
        reviewers: vec!["fixture".to_owned()],
    }
}

fn fixture_bytes() -> Vec<u8> {
    std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/research/walk-forward-v1.json"),
    )
    .unwrap()
}

#[test]
fn overlapping_labels_are_purged_and_embargo_excludes_adjacent_rows() {
    let report = run_walk_forward_bytes(&fixture_bytes()).unwrap();
    assert_eq!(report.fold_count, 2);
    assert_eq!(report.walk_forward, "synthetic_folds");
    assert!(!report.alpha_quality_claimed);
    assert!(!report.stage_pass_claimed);

    let fold0 = &report.folds[0];
    assert_eq!(fold0.train_ids, vec!["train-safe"]);
    assert_eq!(fold0.purged_ids, vec!["train-purge-fold0"]);
    assert_eq!(fold0.embargoed_ids, vec!["train-embargo-fold0"]);
    assert!(fold0.validation_ids.contains(&"val-fold0".to_owned()));
    assert!(!fold0.train_ids.iter().any(|id| id == "holdout-secret"));
    assert!(!fold0.validation_ids.iter().any(|id| id == "holdout-secret"));

    let fold1 = &report.folds[1];
    assert!(fold1.train_ids.contains(&"train-safe".to_owned()));
    assert!(fold1.purged_ids.contains(&"train-purge-fold1".to_owned()));
    assert!(
        fold1
            .embargoed_ids
            .contains(&"train-embargo-fold1".to_owned())
    );
    assert_eq!(fold1.validation_ids, vec!["val-fold1"]);
    assert!(!fold1.train_ids.contains(&"val-fold1".to_owned()));
}

#[test]
fn walk_forward_is_deterministic() {
    let first = run_walk_forward_bytes(&fixture_bytes()).unwrap();
    let second = run_walk_forward_bytes(&fixture_bytes()).unwrap();
    assert_eq!(first.fold_hash, second.fold_hash);
    assert_eq!(first.folds, second.folds);
}

#[test]
fn discovery_cannot_read_holdout_bytes() {
    let dataset = hl_research::ResearchDataset::from_parts(
        ValidationPolicy {
            label_horizon_blocks: 5,
            embargo_blocks: 2,
        },
        &complete_manifest(),
        vec![
            LabeledRow {
                id: "train-safe".to_owned(),
                feature_height: BlockHeight::new(50),
                label_start: BlockHeight::new(50),
                label_end: BlockHeight::new(55),
                payload: "train-only".to_owned(),
                features: Vec::new(),
                outcome: None,
            },
            LabeledRow {
                id: "holdout-secret".to_owned(),
                feature_height: BlockHeight::new(155),
                label_start: BlockHeight::new(155),
                label_end: BlockHeight::new(160),
                payload: "must-not-enter-train".to_owned(),
                features: Vec::new(),
                outcome: None,
            },
        ],
    )
    .unwrap();

    let discovery = dataset.rows_for(DatasetAccess::Discovery).unwrap();
    assert!(
        discovery
            .iter()
            .all(|row| row.id != "holdout-secret" && !dataset.in_holdout(row.feature_height))
    );
    assert_eq!(
        dataset
            .holdout_bytes_hash(DatasetAccess::Discovery)
            .unwrap_err(),
        ResearchError::HoldoutLeakage {
            field: "holdout_bytes",
        }
    );
    assert_eq!(
        dataset
            .holdout_bytes_hash(DatasetAccess::WalkForward)
            .unwrap_err(),
        ResearchError::HoldoutLeakage {
            field: "holdout_bytes",
        }
    );
}

#[test]
fn holdout_isolation_cannot_see_training_rows_and_does_not_pass() {
    let report = run_holdout_isolation_bytes(&fixture_bytes()).unwrap();
    assert_eq!(report.holdout, "isolation_only");
    assert_eq!(report.state, HoldoutState::Closed);
    assert_eq!(report.state.as_str(), "closed");
    assert!(!report.locked);
    assert!(!report.holdout_passed);
    assert!(!report.alpha_quality_claimed);
    assert!(!report.stage_pass_claimed);
    assert_eq!(report.training_rows_visible, 0);
    assert_eq!(report.holdout_rows, 1);
}

#[test]
fn leaked_training_row_in_holdout_batch_is_refused() {
    let dataset = hl_research::ResearchDataset::from_parts(
        ValidationPolicy {
            label_horizon_blocks: 5,
            embargo_blocks: 2,
        },
        &complete_manifest(),
        vec![
            LabeledRow {
                id: "train-safe".to_owned(),
                feature_height: BlockHeight::new(50),
                label_start: BlockHeight::new(50),
                label_end: BlockHeight::new(55),
                payload: "train-only".to_owned(),
                features: Vec::new(),
                outcome: None,
            },
            LabeledRow {
                id: "holdout-secret".to_owned(),
                feature_height: BlockHeight::new(155),
                label_start: BlockHeight::new(155),
                label_end: BlockHeight::new(160),
                payload: "must-not-enter-train".to_owned(),
                features: Vec::new(),
                outcome: None,
            },
        ],
    )
    .unwrap();
    let mixed: Vec<&LabeledRow> = dataset
        .rows_for(DatasetAccess::Discovery)
        .unwrap()
        .into_iter()
        .chain(dataset.rows_for(DatasetAccess::HoldoutIsolation).unwrap())
        .collect();
    let error = hl_research::refuse_leaked_holdout_batch(&dataset, &mixed).unwrap_err();
    assert_eq!(
        error,
        ResearchError::HoldoutLeakage {
            field: "holdout.foreign_row",
        }
    );
}

#[test]
fn label_overlapping_holdout_is_refused_at_dataset_construction() {
    let error = hl_research::ResearchDataset::from_parts(
        ValidationPolicy {
            label_horizon_blocks: 5,
            embargo_blocks: 2,
        },
        &complete_manifest(),
        vec![LabeledRow {
            id: "leak".to_owned(),
            feature_height: BlockHeight::new(50),
            label_start: BlockHeight::new(50),
            label_end: BlockHeight::new(155),
            payload: "into-holdout".to_owned(),
            features: Vec::new(),
            outcome: None,
        }],
    )
    .unwrap_err();
    assert_eq!(
        error,
        ResearchError::HoldoutLeakage {
            field: "label.holdout",
        }
    );
}

#[test]
fn locked_holdout_pass_stays_unimplemented() {
    let mut registry = ExperimentRegistry::new();
    let record = registry.submit(complete_manifest()).unwrap();
    assert_eq!(
        registry.open_holdout(&record.experiment_id).unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
    assert_eq!(
        registry
            .open_locked_holdout(&record.experiment_id)
            .unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );

    let dataset = hl_research::ResearchDataset::from_parts(
        ValidationPolicy {
            label_horizon_blocks: 5,
            embargo_blocks: 2,
        },
        &complete_manifest(),
        Vec::new(),
    )
    .unwrap();
    assert_eq!(
        dataset.lock_for_pass().unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
    assert_eq!(
        dataset
            .rows_for(DatasetAccess::LockedHoldoutPass)
            .unwrap_err(),
        ResearchError::HoldoutNotImplemented
    );
}

#[test]
fn overlapping_splits_are_refused() {
    let mut manifest = complete_manifest();
    manifest.holdout_range = BlockRange::new(BlockHeight::new(105), BlockHeight::new(160)).unwrap();
    let error = hl_research::ResearchDataset::from_parts(
        ValidationPolicy {
            label_horizon_blocks: 5,
            embargo_blocks: 2,
        },
        &manifest,
        Vec::new(),
    )
    .unwrap_err();
    assert_eq!(
        error,
        ResearchError::SplitInvalid {
            field: "holdout_range",
        }
    );
}

#[test]
fn status_does_not_claim_locked_holdout_or_stage_pass() {
    let status = ResearchStatus::current();
    assert!(!status.walk_forward);
    assert!(!status.holdout);
    assert!(!status.shadow_live);
    assert!(status.synthetic_walk_forward);
    assert!(status.holdout_isolation);
    assert!(status.shadow_capture);
    assert!(status.synthetic_estimators);
    assert!(status.variant_ledger);
    assert!(!status.significance_claimed);
    assert!(!status.alpha_quality_claimed);
    assert!(!status.stage_pass_claimed);
    assert!(!status.trading_signer);
}
