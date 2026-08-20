use super::{LabeledRow, ValidationFold, contains, overlaps};
use crate::error::ResearchError;

pub fn assert_train_labels_do_not_overlap_validation(
    rows: &[&LabeledRow],
    fold: &ValidationFold,
) -> Result<(), ResearchError> {
    for row in rows {
        if overlaps(row.label_range()?, fold.validation) {
            return Err(ResearchError::HoldoutLeakage {
                field: "train.label_overlap",
            });
        }
        if contains(fold.validation, row.feature_height) {
            return Err(ResearchError::HoldoutLeakage {
                field: "train.validation_feature",
            });
        }
        if row.feature_height >= fold.validation.start_inclusive {
            return Err(ResearchError::HoldoutLeakage {
                field: "train.future_feature",
            });
        }
    }
    Ok(())
}
