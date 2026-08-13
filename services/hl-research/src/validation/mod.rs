#![forbid(unsafe_code)]

mod evaluate;
mod holdout;
mod purge;
mod runner;
mod split;

use domain_types::{BlockHeight, BlockRange, Decimal, ExperimentId};
use serde::{Deserialize, Serialize};

use crate::error::ResearchError;
use crate::experiment::ExperimentManifest;

pub use evaluate::{FoldEstimatorReport, run_evaluate_folds_bytes};
pub use holdout::{
    HoldoutIsolationReport, HoldoutState, refuse_leaked_holdout_batch, run_holdout_isolation_bytes,
};
pub use runner::{WalkForwardReport, run_walk_forward_bytes};
pub use split::{FoldAssignment, ValidationFold};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValidationPolicy {
    pub label_horizon_blocks: u64,
    pub embargo_blocks: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabeledRow {
    pub id: String,
    pub feature_height: BlockHeight,
    pub label_start: BlockHeight,
    pub label_end: BlockHeight,
    pub payload: String,
    #[serde(default)]
    pub features: Vec<String>,
    #[serde(default)]
    pub outcome: Option<String>,
}

impl LabeledRow {
    pub fn label_range(&self) -> Result<BlockRange, ResearchError> {
        BlockRange::new(self.label_start, self.label_end)
            .map_err(|_| ResearchError::SplitInvalid { field: "label" })
    }

    pub fn observation(&self) -> Result<(Vec<Decimal>, Decimal), ResearchError> {
        let outcome = self
            .outcome
            .as_deref()
            .ok_or(ResearchError::MissingObservation { field: "outcome" })?;
        let outcome = Decimal::parse_at_scale(outcome, 8)
            .map_err(|_| ResearchError::MissingObservation { field: "outcome" })?;
        if self.features.is_empty() {
            return Err(ResearchError::MissingObservation { field: "features" });
        }
        let features = self
            .features
            .iter()
            .map(|value| {
                Decimal::parse_at_scale(value, 8)
                    .map_err(|_| ResearchError::MissingObservation { field: "features" })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok((features, outcome))
    }

    pub fn outcome_value(&self) -> Result<Decimal, ResearchError> {
        let outcome = self
            .outcome
            .as_deref()
            .ok_or(ResearchError::MissingObservation { field: "outcome" })?;
        Decimal::parse_at_scale(outcome, 8)
            .map_err(|_| ResearchError::MissingObservation { field: "outcome" })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetAccess {
    Discovery,
    WalkForward,
    HoldoutIsolation,
    LockedHoldoutPass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchDataset {
    policy: ValidationPolicy,
    training_range: BlockRange,
    validation_ranges: Vec<BlockRange>,
    holdout_range: BlockRange,
    experiment_id: ExperimentId,
    random_seed: u64,
    rows: Vec<LabeledRow>,
}

impl ResearchDataset {
    pub fn from_parts(
        policy: ValidationPolicy,
        manifest: &ExperimentManifest,
        rows: Vec<LabeledRow>,
    ) -> Result<Self, ResearchError> {
        if let Some(field) = manifest.missing_field() {
            return Err(ResearchError::IncompleteManifest { field });
        }
        validate_ordered_splits(
            manifest.training_range,
            &manifest.validation_ranges,
            manifest.holdout_range,
        )?;
        for row in &rows {
            let _ = row.label_range()?;
        }
        let dataset = Self {
            policy,
            training_range: manifest.training_range,
            validation_ranges: manifest.validation_ranges.clone(),
            holdout_range: manifest.holdout_range,
            experiment_id: manifest.experiment_id()?,
            random_seed: manifest.random_seed,
            rows,
        };
        dataset.assert_no_holdout_label_leak()?;
        Ok(dataset)
    }

    #[must_use]
    pub const fn policy(&self) -> ValidationPolicy {
        self.policy
    }

    #[must_use]
    pub const fn training_range(&self) -> BlockRange {
        self.training_range
    }

    #[must_use]
    pub fn validation_ranges(&self) -> &[BlockRange] {
        &self.validation_ranges
    }

    #[must_use]
    pub const fn holdout_range(&self) -> BlockRange {
        self.holdout_range
    }

    #[must_use]
    pub fn experiment_id(&self) -> &ExperimentId {
        &self.experiment_id
    }

    #[must_use]
    pub const fn random_seed(&self) -> u64 {
        self.random_seed
    }

    pub fn rows_by_ids(
        &self,
        ids: &[String],
        access: DatasetAccess,
    ) -> Result<Vec<&LabeledRow>, ResearchError> {
        let view = self.rows_for(access)?;
        let mut rows = Vec::with_capacity(ids.len());
        for id in ids {
            let row =
                view.iter()
                    .find(|row| row.id == *id)
                    .ok_or(ResearchError::HoldoutLeakage {
                        field: "row.missing_or_holdout",
                    })?;
            rows.push(*row);
        }
        Ok(rows)
    }

    pub fn folds(&self) -> Result<Vec<ValidationFold>, ResearchError> {
        split::generate_folds(self.training_range, &self.validation_ranges, self.policy)
    }

    pub fn assign_fold(&self, fold: &ValidationFold) -> Result<FoldAssignment, ResearchError> {
        split::assign_fold(self, fold)
    }

    pub fn rows_for(&self, access: DatasetAccess) -> Result<Vec<&LabeledRow>, ResearchError> {
        match access {
            DatasetAccess::LockedHoldoutPass => Err(ResearchError::HoldoutNotImplemented),
            DatasetAccess::Discovery | DatasetAccess::WalkForward => Ok(self
                .rows
                .iter()
                .filter(|row| !contains(self.holdout_range, row.feature_height))
                .collect()),
            DatasetAccess::HoldoutIsolation => {
                let view: Vec<&LabeledRow> = self
                    .rows
                    .iter()
                    .filter(|row| contains(self.holdout_range, row.feature_height))
                    .collect();
                if view
                    .iter()
                    .any(|row| self.in_training_or_validation(row.feature_height))
                {
                    return Err(ResearchError::HoldoutLeakage {
                        field: "holdout.training_row",
                    });
                }
                Ok(view)
            }
        }
    }

    pub fn holdout_bytes_hash(&self, access: DatasetAccess) -> Result<[u8; 32], ResearchError> {
        match access {
            DatasetAccess::Discovery | DatasetAccess::WalkForward => {
                Err(ResearchError::HoldoutLeakage {
                    field: "holdout_bytes",
                })
            }
            DatasetAccess::LockedHoldoutPass => Err(ResearchError::HoldoutNotImplemented),
            DatasetAccess::HoldoutIsolation => {
                let rows = self.rows_for(DatasetAccess::HoldoutIsolation)?;
                Ok(hash_rows(&rows))
            }
        }
    }

    pub fn lock_for_pass(&self) -> Result<(), ResearchError> {
        Err(ResearchError::HoldoutNotImplemented)
    }

    pub fn in_holdout(&self, height: BlockHeight) -> bool {
        contains(self.holdout_range, height)
    }

    pub fn in_training_or_validation(&self, height: BlockHeight) -> bool {
        contains(self.training_range, height)
            || self
                .validation_ranges
                .iter()
                .any(|range| contains(*range, height))
    }

    fn assert_no_holdout_label_leak(&self) -> Result<(), ResearchError> {
        for row in &self.rows {
            if contains(self.holdout_range, row.feature_height) {
                continue;
            }
            let label = row.label_range()?;
            if overlaps(label, self.holdout_range) {
                return Err(ResearchError::HoldoutLeakage {
                    field: "label.holdout",
                });
            }
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize)]
pub struct ValidationFixture {
    pub policy: ValidationPolicy,
    pub experiment: ExperimentManifest,
    pub rows: Vec<LabeledRow>,
}

pub fn dataset_from_bytes(bytes: &[u8]) -> Result<ResearchDataset, ResearchError> {
    let fixture: ValidationFixture =
        serde_json::from_slice(bytes).map_err(|_| ResearchError::InvalidFixture)?;
    ResearchDataset::from_parts(fixture.policy, &fixture.experiment, fixture.rows)
}

pub(crate) fn contains(range: BlockRange, height: BlockHeight) -> bool {
    height >= range.start_inclusive && height <= range.end_inclusive
}

pub(crate) fn overlaps(left: BlockRange, right: BlockRange) -> bool {
    left.start_inclusive <= right.end_inclusive && right.start_inclusive <= left.end_inclusive
}

pub(crate) fn strictly_before(left: BlockRange, right: BlockRange) -> bool {
    left.end_inclusive < right.start_inclusive
}

pub(crate) fn intersect(left: BlockRange, right: BlockRange) -> Option<BlockRange> {
    let start = if left.start_inclusive > right.start_inclusive {
        left.start_inclusive
    } else {
        right.start_inclusive
    };
    let end = if left.end_inclusive < right.end_inclusive {
        left.end_inclusive
    } else {
        right.end_inclusive
    };
    if start <= end {
        BlockRange::new(start, end).ok()
    } else {
        None
    }
}

pub(crate) fn height_sub(height: BlockHeight, amount: u64) -> Result<BlockHeight, ResearchError> {
    height
        .get()
        .checked_sub(amount)
        .map(BlockHeight::new)
        .ok_or(ResearchError::SplitInvalid {
            field: "window_underflow",
        })
}

pub(crate) fn validate_ordered_splits(
    training: BlockRange,
    validation: &[BlockRange],
    holdout: BlockRange,
) -> Result<(), ResearchError> {
    if validation.is_empty() {
        return Err(ResearchError::SplitInvalid {
            field: "validation_ranges",
        });
    }
    let mut previous = training;
    for range in validation {
        if !strictly_before(previous, *range) {
            return Err(ResearchError::SplitInvalid {
                field: "validation_ranges",
            });
        }
        previous = *range;
    }
    if !strictly_before(previous, holdout) {
        return Err(ResearchError::SplitInvalid {
            field: "holdout_range",
        });
    }
    Ok(())
}

pub(crate) fn hash_rows(rows: &[&LabeledRow]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hl.research.rows.v1");
    let mut ordered = rows.to_vec();
    ordered.sort_by_key(|row| (row.feature_height.get(), row.id.as_str()));
    for row in ordered {
        hasher.update(row.id.as_bytes());
        hasher.update(&row.feature_height.get().to_le_bytes());
        hasher.update(&row.label_start.get().to_le_bytes());
        hasher.update(&row.label_end.get().to_le_bytes());
        hasher.update(row.payload.as_bytes());
        for feature in &row.features {
            hasher.update(feature.as_bytes());
        }
        if let Some(outcome) = &row.outcome {
            hasher.update(outcome.as_bytes());
        }
    }
    *hasher.finalize().as_bytes()
}
