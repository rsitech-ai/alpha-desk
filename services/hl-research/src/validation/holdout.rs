use serde::Serialize;

use crate::error::ResearchError;

use super::{DatasetAccess, ResearchDataset, dataset_from_bytes, hash_rows};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldoutState {
    Sealed,
    OpenedForEvaluation,
    Closed,
    Invalidated,
}

impl HoldoutState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sealed => "sealed",
            Self::OpenedForEvaluation => "opened_for_evaluation",
            Self::Closed => "closed",
            Self::Invalidated => "invalidated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct HoldoutIsolationReport {
    pub schema_version: &'static str,
    pub mode: &'static str,
    pub state: HoldoutState,
    pub holdout: &'static str,
    pub locked: bool,
    pub holdout_passed: bool,
    pub alpha_quality_claimed: bool,
    pub stage_pass_claimed: bool,
    pub training_rows_visible: usize,
    pub holdout_rows: usize,
    pub holdout_hash: String,
}

pub fn evaluate_holdout_isolation(
    dataset: &ResearchDataset,
) -> Result<HoldoutIsolationReport, ResearchError> {
    let discovery = dataset.rows_for(DatasetAccess::Discovery)?;
    if discovery
        .iter()
        .any(|row| dataset.in_holdout(row.feature_height))
    {
        return Err(ResearchError::HoldoutLeakage {
            field: "discovery.holdout_row",
        });
    }
    if dataset.holdout_bytes_hash(DatasetAccess::Discovery).is_ok() {
        return Err(ResearchError::HoldoutLeakage {
            field: "discovery.holdout_bytes",
        });
    }

    let holdout = dataset.rows_for(DatasetAccess::HoldoutIsolation)?;
    if holdout
        .iter()
        .any(|row| dataset.in_training_or_validation(row.feature_height))
    {
        return Err(ResearchError::HoldoutLeakage {
            field: "holdout.training_row",
        });
    }
    refuse_training_rows_in_holdout(&holdout, dataset)?;

    let holdout_hash = hex::encode(dataset.holdout_bytes_hash(DatasetAccess::HoldoutIsolation)?);
    let _ = hash_rows(&discovery);

    match dataset.lock_for_pass() {
        Err(ResearchError::HoldoutNotImplemented) => {}
        Ok(()) => {
            return Err(ResearchError::HoldoutLeakage {
                field: "holdout.unlocked_pass",
            });
        }
        Err(error) => return Err(error),
    }

    Ok(HoldoutIsolationReport {
        schema_version: "hl.research.holdout-isolation.v1",
        mode: "holdout_isolation",
        state: HoldoutState::Closed,
        holdout: "isolation_only",
        locked: false,
        holdout_passed: false,
        alpha_quality_claimed: false,
        stage_pass_claimed: false,
        training_rows_visible: 0,
        holdout_rows: holdout.len(),
        holdout_hash,
    })
}

pub fn refuse_leaked_holdout_batch(
    dataset: &ResearchDataset,
    rows: &[&super::LabeledRow],
) -> Result<(), ResearchError> {
    if rows
        .iter()
        .any(|row| !dataset.in_holdout(row.feature_height))
    {
        return Err(ResearchError::HoldoutLeakage {
            field: "holdout.foreign_row",
        });
    }
    if rows
        .iter()
        .any(|row| dataset.in_training_or_validation(row.feature_height))
    {
        return Err(ResearchError::HoldoutLeakage {
            field: "holdout.training_row",
        });
    }
    Ok(())
}

pub fn run_holdout_isolation_bytes(bytes: &[u8]) -> Result<HoldoutIsolationReport, ResearchError> {
    let dataset = dataset_from_bytes(bytes)?;
    evaluate_holdout_isolation(&dataset)
}

fn refuse_training_rows_in_holdout(
    holdout: &[&super::LabeledRow],
    dataset: &ResearchDataset,
) -> Result<(), ResearchError> {
    let discovery_ids: Vec<&str> = dataset
        .rows_for(DatasetAccess::Discovery)?
        .into_iter()
        .map(|row| row.id.as_str())
        .collect();
    if holdout
        .iter()
        .any(|row| discovery_ids.iter().any(|id| *id == row.id))
    {
        return Err(ResearchError::HoldoutLeakage {
            field: "holdout.training_row",
        });
    }
    Ok(())
}
