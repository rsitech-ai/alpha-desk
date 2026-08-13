use domain_types::Decimal;
use serde::Serialize;

use crate::error::ResearchError;
use crate::estimator::{EstimatorClass, FittedEstimator, LinearModel, fit};
use crate::ledger::VariantLedger;
use crate::metrics::{
    BootstrapReport, DEFAULT_REPLICATES, MultipleTestingReport, diagnose_family, score_predictions,
    stationary_block_bootstrap,
};

use super::purge::assert_train_labels_do_not_overlap_validation;
use super::runner::run_walk_forward;
use super::{DatasetAccess, ResearchDataset, dataset_from_bytes};

const APPROVED_CLASSES: [EstimatorClass; 2] = [
    EstimatorClass::MeanOutcome,
    EstimatorClass::UnivariateLinear,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoldFit {
    pub fold_index: usize,
    pub estimator: FittedEstimator,
    pub train_n: usize,
    pub validation_n: usize,
    pub metrics: crate::metrics::PerformanceMetrics,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoldEstimatorReport {
    pub schema_version: &'static str,
    pub mode: &'static str,
    pub experiment_id: String,
    pub fold_hash: String,
    pub significance: &'static str,
    pub alpha_quality_claimed: bool,
    pub stage_pass_claimed: bool,
    pub evaluations: Vec<FoldFit>,
    pub ledger: VariantLedger,
    pub bootstrap: BootstrapReport,
    pub multiple_testing: MultipleTestingReport,
}

pub fn evaluate_folds(dataset: &ResearchDataset) -> Result<FoldEstimatorReport, ResearchError> {
    let walk = run_walk_forward(dataset)?;
    let mut ledger = VariantLedger::new();
    let family = dataset.experiment_id().to_string();
    for class in APPROVED_CLASSES {
        ledger.register(&family, class)?;
    }

    let mut evaluations = Vec::new();
    let mut pooled_outcomes: Vec<Decimal> = Vec::new();
    let mut pooled_by_class: Vec<(EstimatorClass, Vec<Decimal>, Vec<Decimal>)> = APPROVED_CLASSES
        .iter()
        .map(|class| (*class, Vec::new(), Vec::new()))
        .collect();

    for (fold_index, assignment) in walk.folds.iter().enumerate() {
        let train = observations(dataset, &assignment.train_ids)?;
        let validation = observations(dataset, &assignment.validation_ids)?;
        let train_rows: Vec<_> = dataset
            .rows_by_ids(&assignment.train_ids, DatasetAccess::WalkForward)?
            .into_iter()
            .collect();
        assert_train_labels_do_not_overlap_validation(&train_rows, &assignment.fold)?;
        if train.outcomes.len() < 2 || validation.outcomes.is_empty() {
            return Err(ResearchError::InsufficientTrain {
                field: "fold_estimator",
            });
        }
        for (class_index, class) in APPROVED_CLASSES.iter().enumerate() {
            let model = fit(*class, &train.features, &train.outcomes)?;
            let predicted = predict_all(&model, &validation.features)?;
            let metrics = score_predictions(&predicted, &validation.outcomes)?;
            pooled_by_class[class_index].1.extend(predicted);
            pooled_by_class[class_index]
                .2
                .extend(validation.outcomes.iter().copied());
            evaluations.push(FoldFit {
                fold_index,
                estimator: model.report(),
                train_n: train.outcomes.len(),
                validation_n: validation.outcomes.len(),
                metrics,
            });
        }
        pooled_outcomes.extend(validation.outcomes);
    }

    for (class, predictions, outcomes) in &pooled_by_class {
        let variant_id = crate::ledger::variant_identity(&family, *class);
        let metrics = score_predictions(predictions, outcomes)?;
        ledger.record_metrics(&variant_id, metrics)?;
        ledger.mark_research_only(&variant_id)?;
    }

    let bootstrap = stationary_block_bootstrap(
        &pooled_outcomes,
        2,
        DEFAULT_REPLICATES,
        dataset.random_seed(),
    )?;
    let multiple_testing = diagnose_family(&ledger);
    Ok(FoldEstimatorReport {
        schema_version: "hl.research.fold-estimators.v1",
        mode: "synthetic_fold_estimators",
        experiment_id: family,
        fold_hash: walk.fold_hash,
        significance: "not_claimed",
        alpha_quality_claimed: false,
        stage_pass_claimed: false,
        evaluations,
        ledger,
        bootstrap,
        multiple_testing,
    })
}

pub fn run_evaluate_folds_bytes(bytes: &[u8]) -> Result<FoldEstimatorReport, ResearchError> {
    let dataset = dataset_from_bytes(bytes)?;
    evaluate_folds(&dataset)
}

struct ObservationBatch {
    features: Vec<Vec<Decimal>>,
    outcomes: Vec<Decimal>,
}

fn observations(
    dataset: &ResearchDataset,
    ids: &[String],
) -> Result<ObservationBatch, ResearchError> {
    let rows = dataset.rows_by_ids(ids, DatasetAccess::WalkForward)?;
    let mut features = Vec::with_capacity(rows.len());
    let mut outcomes = Vec::with_capacity(rows.len());
    for row in rows {
        let (row_features, outcome) = row.observation()?;
        features.push(row_features);
        outcomes.push(outcome);
    }
    Ok(ObservationBatch { features, outcomes })
}

fn predict_all(
    model: &LinearModel,
    features: &[Vec<Decimal>],
) -> Result<Vec<Decimal>, ResearchError> {
    features.iter().map(|row| model.predict(row)).collect()
}
