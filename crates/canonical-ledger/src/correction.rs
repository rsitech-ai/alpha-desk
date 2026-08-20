use canonical_events::ConfirmationClass;

use crate::LedgerError;

/// V1 correction policy: explicit correction blocks are typed and default-deny.
///
/// `docs/contracts/deterministic-state-v1.md` accepts only committed primary or
/// independent blocks. Correction application is unimplemented; a `Corrected`
/// block must not mutate canonical state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationAdmission {
    Committed,
    NonCommitted,
    CorrectionUnimplemented,
}

#[must_use]
pub fn admit_confirmation(class: ConfirmationClass) -> ConfirmationAdmission {
    match class {
        ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => {
            ConfirmationAdmission::Committed
        }
        ConfirmationClass::Corrected => ConfirmationAdmission::CorrectionUnimplemented,
        ConfirmationClass::ProvisionalSource
        | ConfirmationClass::ReconciledSnapshot
        | ConfirmationClass::Expired => ConfirmationAdmission::NonCommitted,
    }
}

pub fn require_committed_confirmation(class: ConfirmationClass) -> Result<(), LedgerError> {
    match admit_confirmation(class) {
        ConfirmationAdmission::Committed => Ok(()),
        ConfirmationAdmission::NonCommitted => Err(LedgerError::NonCommittedBlock),
        ConfirmationAdmission::CorrectionUnimplemented => Err(LedgerError::CorrectionUnimplemented),
    }
}
