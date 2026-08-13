use domain_types::BlockRange;
use serde::{Deserialize, Serialize};

use crate::error::ResearchError;

use super::{
    LabeledRow, ResearchDataset, ValidationPolicy, contains, height_sub, intersect, overlaps,
    strictly_before,
};

struct FoldBuckets {
    train_ids: Vec<String>,
    validation_ids: Vec<String>,
    purged_ids: Vec<String>,
    embargoed_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationFold {
    pub train: BlockRange,
    pub validation: BlockRange,
    pub purge: Vec<BlockRange>,
    pub embargo: Vec<BlockRange>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoldAssignment {
    pub fold: ValidationFold,
    pub train_ids: Vec<String>,
    pub validation_ids: Vec<String>,
    pub purged_ids: Vec<String>,
    pub embargoed_ids: Vec<String>,
}

impl FoldAssignment {
    #[must_use]
    pub fn content_hash(&self) -> [u8; 32] {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"hl.research.fold.v1");
        hasher.update(&encoded);
        *hasher.finalize().as_bytes()
    }
}

pub fn generate_folds(
    training: BlockRange,
    validation_ranges: &[BlockRange],
    policy: ValidationPolicy,
) -> Result<Vec<ValidationFold>, ResearchError> {
    let mut folds = Vec::with_capacity(validation_ranges.len());
    for (index, validation) in validation_ranges.iter().enumerate() {
        let train = if index == 0 {
            training
        } else {
            BlockRange::new(
                training.start_inclusive,
                validation_ranges[index - 1].end_inclusive,
            )
            .map_err(|_| ResearchError::SplitInvalid {
                field: "expanding_train",
            })?
        };
        if !strictly_before(train, *validation) {
            return Err(ResearchError::SplitInvalid {
                field: "fold_order",
            });
        }
        let purge = purge_windows(train, *validation, policy.label_horizon_blocks)?;
        let embargo = embargo_windows(train, *validation, &purge, policy.embargo_blocks)?;
        folds.push(ValidationFold {
            train,
            validation: *validation,
            purge,
            embargo,
        });
    }
    Ok(folds)
}

pub fn assign_fold(
    dataset: &ResearchDataset,
    fold: &ValidationFold,
) -> Result<FoldAssignment, ResearchError> {
    let mut buckets = FoldBuckets {
        train_ids: Vec::new(),
        validation_ids: Vec::new(),
        purged_ids: Vec::new(),
        embargoed_ids: Vec::new(),
    };

    for row in &dataset.rows {
        classify_row(dataset, fold, row, &mut buckets)?;
    }

    buckets.train_ids.sort();
    buckets.validation_ids.sort();
    buckets.purged_ids.sort();
    buckets.embargoed_ids.sort();

    Ok(FoldAssignment {
        fold: fold.clone(),
        train_ids: buckets.train_ids,
        validation_ids: buckets.validation_ids,
        purged_ids: buckets.purged_ids,
        embargoed_ids: buckets.embargoed_ids,
    })
}

fn classify_row(
    dataset: &ResearchDataset,
    fold: &ValidationFold,
    row: &LabeledRow,
    buckets: &mut FoldBuckets,
) -> Result<(), ResearchError> {
    if contains(dataset.holdout_range(), row.feature_height) {
        return Ok(());
    }
    if contains(fold.validation, row.feature_height) {
        buckets.validation_ids.push(row.id.clone());
        return Ok(());
    }
    if !contains(fold.train, row.feature_height) {
        return Ok(());
    }
    if in_windows(&fold.embargo, row.feature_height) {
        buckets.embargoed_ids.push(row.id.clone());
        return Ok(());
    }
    if in_windows(&fold.purge, row.feature_height) || overlaps(row.label_range()?, fold.validation)
    {
        buckets.purged_ids.push(row.id.clone());
        return Ok(());
    }
    if row.feature_height >= fold.validation.start_inclusive {
        return Err(ResearchError::HoldoutLeakage {
            field: "train.future_feature",
        });
    }
    buckets.train_ids.push(row.id.clone());
    Ok(())
}

fn in_windows(windows: &[BlockRange], height: domain_types::BlockHeight) -> bool {
    windows.iter().any(|range| contains(*range, height))
}

fn purge_windows(
    train: BlockRange,
    validation: BlockRange,
    horizon: u64,
) -> Result<Vec<BlockRange>, ResearchError> {
    if horizon == 0 {
        return Ok(Vec::new());
    }
    let end = height_sub(validation.start_inclusive, 1)?;
    let start = height_sub(validation.start_inclusive, horizon)?;
    let window =
        BlockRange::new(start, end).map_err(|_| ResearchError::SplitInvalid { field: "purge" })?;
    Ok(intersect(window, train).into_iter().collect())
}

fn embargo_windows(
    train: BlockRange,
    validation: BlockRange,
    purge: &[BlockRange],
    embargo: u64,
) -> Result<Vec<BlockRange>, ResearchError> {
    if embargo == 0 {
        return Ok(Vec::new());
    }
    let end_exclusive = if let Some(first_purge) = purge.first() {
        first_purge.start_inclusive
    } else {
        validation.start_inclusive
    };
    let end = height_sub(end_exclusive, 1)?;
    let start = height_sub(end_exclusive, embargo)?;
    let window = BlockRange::new(start, end)
        .map_err(|_| ResearchError::SplitInvalid { field: "embargo" })?;
    Ok(intersect(window, train).into_iter().collect())
}
