use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId};

use crate::{CanonicalLedger, LedgerError};

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

/// Typed correction record. This is not an account action and has no applicator.
///
/// Construction only succeeds for `ConfirmationClass::Corrected`. Apply, ingest,
/// and inspect paths must fail closed with [`LedgerError::CorrectionUnimplemented`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorrectionRecord {
    chain_id: ChainId,
    block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
}

impl CorrectionRecord {
    /// Bind a correction record from a corrected block. Other confirmation
    /// classes are not correction records.
    #[must_use]
    pub fn try_from_block(block: &BlockEnvelope) -> Option<Self> {
        match admit_confirmation(block.confirmation_class()) {
            ConfirmationAdmission::CorrectionUnimplemented => Some(Self {
                chain_id: block.chain_id().clone(),
                block_height: block.block_height(),
                canonical_block_hash: block.canonical_block_hash(),
            }),
            ConfirmationAdmission::Committed | ConfirmationAdmission::NonCommitted => None,
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }
}

/// Inspection of a correction record. Inspection never applies mutations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionInspectReport {
    admitted: bool,
    applied: bool,
    reason_code: &'static str,
}

impl CorrectionInspectReport {
    #[must_use]
    pub const fn admitted(self) -> bool {
        self.admitted
    }

    #[must_use]
    pub const fn applied(self) -> bool {
        self.applied
    }

    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        self.reason_code
    }
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

/// Default-deny correction applicator. This is not a real correction engine:
/// it never mutates ledger state and cannot invent account actions.
pub fn apply_correction<R>(
    _ledger: &CanonicalLedger<R>,
    _record: &CorrectionRecord,
) -> Result<(), LedgerError> {
    Err(LedgerError::CorrectionUnimplemented)
}

/// Inspect a correction record without applying it. The report is always deny.
#[must_use]
pub fn inspect_correction(_record: &CorrectionRecord) -> CorrectionInspectReport {
    CorrectionInspectReport {
        admitted: false,
        applied: false,
        reason_code: LedgerError::CorrectionUnimplemented.reason_code(),
    }
}
