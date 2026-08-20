use serde::Serialize;

use crate::claims::{serialize_denied_true, serialize_unclaimed};
use crate::error::ResearchError;

use super::purge::assert_train_labels_do_not_overlap_validation;
use super::{DatasetAccess, FoldAssignment, ResearchDataset, dataset_from_bytes};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WalkForwardReport {
    pub schema_version: &'static str,
    pub mode: &'static str,
    pub walk_forward: &'static str,
    pub holdout: &'static str,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_quality_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_qualified: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub significance_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub stage_pass_claimed: bool,
    #[serde(serialize_with = "serialize_denied_true")]
    pub live_corpus: bool,
    #[serde(serialize_with = "serialize_denied_true")]
    pub replica_cmds_used: bool,
    pub fold_count: usize,
    pub fold_hash: String,
    pub folds: Vec<FoldAssignment>,
}

pub fn run_walk_forward(dataset: &ResearchDataset) -> Result<WalkForwardReport, ResearchError> {
    let _ = dataset.rows_for(DatasetAccess::WalkForward)?;
    if dataset
        .holdout_bytes_hash(DatasetAccess::WalkForward)
        .is_ok()
    {
        return Err(ResearchError::HoldoutLeakage {
            field: "walk_forward.holdout_bytes",
        });
    }

    let folds = dataset.folds()?;
    let mut assignments = Vec::with_capacity(folds.len());
    for fold in &folds {
        let assignment = dataset.assign_fold(fold)?;
        let train_rows: Vec<_> = dataset
            .rows_for(DatasetAccess::WalkForward)?
            .into_iter()
            .filter(|row| assignment.train_ids.iter().any(|id| id == &row.id))
            .collect();
        assert_train_labels_do_not_overlap_validation(&train_rows, fold)?;
        if assignment
            .train_ids
            .iter()
            .any(|id| assignment.validation_ids.iter().any(|other| other == id))
        {
            return Err(ResearchError::HoldoutLeakage {
                field: "walk_forward.row_reuse",
            });
        }
        assignments.push(assignment);
    }

    let fold_hash = hash_assignments(&assignments);
    Ok(WalkForwardReport {
        schema_version: "hl.research.walk-forward.v1",
        mode: "synthetic_walk_forward",
        walk_forward: "synthetic_folds",
        holdout: "sealed",
        alpha_quality_claimed: false,
        alpha_qualified: false,
        significance_claimed: false,
        stage_pass_claimed: false,
        live_corpus: false,
        replica_cmds_used: false,
        fold_count: assignments.len(),
        fold_hash: hex::encode(fold_hash),
        folds: assignments,
    })
}

pub fn run_walk_forward_bytes(bytes: &[u8]) -> Result<WalkForwardReport, ResearchError> {
    let dataset = dataset_from_bytes(bytes)?;
    run_walk_forward(&dataset)
}

fn hash_assignments(assignments: &[FoldAssignment]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hl.research.walk-forward.v1");
    for assignment in assignments {
        hasher.update(&assignment.content_hash());
    }
    *hasher.finalize().as_bytes()
}
