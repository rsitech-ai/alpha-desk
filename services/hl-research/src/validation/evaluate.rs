use domain_types::{Decimal, RoundingMode};
use serde::Serialize;

use crate::baselines::FOLD_ESTIMATOR_CLASSES;
use crate::claims::serialize_unclaimed;
use crate::error::ResearchError;
use crate::estimator::{EstimatorClass, FittedEstimator, LinearModel, METRIC_SCALE, fit, zero};
use crate::ledger::VariantLedger;
use crate::metrics::{
    BootstrapReport, CalibrationReport, DEFAULT_REPLICATES, MultipleTestingReport,
    calibrate_scores, diagnose_family, score_predictions, stationary_block_bootstrap,
};
use crate::promotion::{PromotionEvidence, PromotionReport, evaluate_promotion};

use super::purge::assert_train_labels_do_not_overlap_validation;
use super::runner::run_walk_forward;
use super::{DatasetAccess, ResearchDataset, dataset_from_bytes};

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
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_quality_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_qualified: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub significance_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub stage_pass_claimed: bool,
    pub evaluations: Vec<FoldFit>,
    pub ledger: VariantLedger,
    pub bootstrap: BootstrapReport,
    pub calibration: CalibrationReport,
    pub multiple_testing: MultipleTestingReport,
    pub promotion: PromotionReport,
}

pub fn evaluate_folds(dataset: &ResearchDataset) -> Result<FoldEstimatorReport, ResearchError> {
    let walk = run_walk_forward(dataset)?;
    let mut ledger = VariantLedger::new();
    let family = dataset.experiment_id().to_string();
    for class in FOLD_ESTIMATOR_CLASSES {
        ledger.register(&family, class)?;
    }

    let mut evaluations = Vec::new();
    let mut pooled_outcomes: Vec<Decimal> = Vec::new();
    let mut pooled_by_class: Vec<(EstimatorClass, Vec<Decimal>, Vec<Decimal>)> =
        FOLD_ESTIMATOR_CLASSES
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
        for (class_index, class) in FOLD_ESTIMATOR_CLASSES.iter().enumerate() {
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

    let linear = pooled_by_class
        .iter()
        .find(|(class, _, _)| *class == EstimatorClass::UnivariateLinear)
        .ok_or(ResearchError::UnsupportedEstimator)?;
    let calibration = calibrate_scores(&linear.1, &linear.2)?;
    let linear_metrics = score_predictions(&linear.1, &linear.2)?;
    let shares = episode_shares_ppm(&pooled_outcomes)?;
    let bootstrap = stationary_block_bootstrap(
        &pooled_outcomes,
        2,
        DEFAULT_REPLICATES,
        dataset.random_seed(),
    )?;
    let promotion = evaluate_promotion(&PromotionEvidence {
        outcome_count: pooled_outcomes.len(),
        holdout_lock: None,
        holdout_outcome_count: 0,
        calendar_days: None,
        bootstrap: &bootstrap,
        calibration: &calibration,
        metrics: Some(&linear_metrics),
        shadow_live: false,
        episode_shares_ppm: &shares,
    });
    let multiple_testing = diagnose_family(&ledger);
    Ok(FoldEstimatorReport {
        schema_version: "hl.research.fold-estimators.v1",
        mode: "synthetic_fold_estimators",
        experiment_id: family,
        fold_hash: walk.fold_hash,
        significance: "not_claimed",
        alpha_quality_claimed: false,
        alpha_qualified: false,
        significance_claimed: false,
        stage_pass_claimed: false,
        evaluations,
        ledger,
        bootstrap,
        calibration,
        multiple_testing,
        promotion,
    })
}

pub fn run_evaluate_folds_bytes(bytes: &[u8]) -> Result<FoldEstimatorReport, ResearchError> {
    let dataset = dataset_from_bytes(bytes)?;
    evaluate_folds(&dataset)
}

pub fn run_promote_bytes(bytes: &[u8]) -> Result<PromotionReport, ResearchError> {
    let report = run_evaluate_folds_bytes(bytes)?;
    Ok(report.promotion)
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

fn episode_shares_ppm(outcomes: &[Decimal]) -> Result<Vec<u32>, ResearchError> {
    let mut total = zero()?;
    let mut abs_values = Vec::with_capacity(outcomes.len());
    for outcome in outcomes {
        let aligned = outcome
            .rescale(METRIC_SCALE, RoundingMode::TowardZero)
            .map_err(|_| ResearchError::Metric {
                field: "concentration",
            })?;
        let abs = if aligned.raw() < 0 {
            zero()?
                .checked_sub(aligned)
                .map_err(|_| ResearchError::Metric {
                    field: "concentration",
                })?
        } else {
            aligned
        };
        total = total.checked_add(abs).map_err(|_| ResearchError::Metric {
            field: "concentration",
        })?;
        abs_values.push(abs);
    }
    if total.raw() == 0 {
        return Ok(Vec::new());
    }
    let million = Decimal::from_raw(1_000_000, 0).map_err(|_| ResearchError::Metric {
        field: "concentration",
    })?;
    abs_values
        .iter()
        .map(|value| {
            let share = value
                .checked_div(total, METRIC_SCALE, RoundingMode::TowardZero)
                .map_err(|_| ResearchError::Metric {
                    field: "concentration",
                })?;
            let ppm = share
                .checked_mul(million, 0, RoundingMode::TowardZero)
                .map_err(|_| ResearchError::Metric {
                    field: "concentration",
                })?;
            u32::try_from(ppm.raw()).map_err(|_| ResearchError::Metric {
                field: "concentration",
            })
        })
        .collect()
}
