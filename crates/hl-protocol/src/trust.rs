use serde::{Deserialize, Serialize};

use crate::ObservationClass;

/// Configured completeness and provenance contract for one source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceTrust {
    LocallyVerifiedCommitted,
    IndependentCommitted,
    ReconciledSnapshot,
    RecoveryOnly,
    ThirdPartyProvisional,
    MempoolProvisional,
}

impl SourceTrust {
    pub const ALL: [Self; 6] = [
        Self::LocallyVerifiedCommitted,
        Self::IndependentCommitted,
        Self::ReconciledSnapshot,
        Self::RecoveryOnly,
        Self::ThirdPartyProvisional,
        Self::MempoolProvisional,
    ];
}

/// Physically and logically distinct destination selected by source policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PublicationLane {
    CommittedCandidate,
    Reconciliation,
    Recovery,
    Provisional,
    Mempool,
}

/// Validated trust/class pairing. Callers cannot supply derived promotion flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceAdmission {
    trust: SourceTrust,
    observation_class: ObservationClass,
    publication_lane: PublicationLane,
}

impl SourceAdmission {
    pub fn new(
        trust: SourceTrust,
        observation_class: ObservationClass,
    ) -> Result<Self, SourceTrustError> {
        let publication_lane = match (trust, observation_class) {
            (
                SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted,
                ObservationClass::CommittedBlock,
            ) => PublicationLane::CommittedCandidate,
            (
                SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted,
                ObservationClass::AuxiliaryOrderStatus
                | ObservationClass::AuxiliaryBookDiff
                | ObservationClass::AuxiliaryLedger,
            )
            | (SourceTrust::ReconciledSnapshot, ObservationClass::Snapshot) => {
                PublicationLane::Reconciliation
            }
            (SourceTrust::RecoveryOnly, ObservationClass::HistoricalBlock) => {
                PublicationLane::Recovery
            }
            (
                SourceTrust::ThirdPartyProvisional,
                ObservationClass::PublicMarketData | ObservationClass::ProvisionalFeed,
            ) => PublicationLane::Provisional,
            (SourceTrust::MempoolProvisional, ObservationClass::ProvisionalMempool) => {
                PublicationLane::Mempool
            }
            _ => return Err(SourceTrustError::IncompatibleObservationClass),
        };
        Ok(Self {
            trust,
            observation_class,
            publication_lane,
        })
    }

    #[must_use]
    pub const fn trust(self) -> SourceTrust {
        self.trust
    }

    #[must_use]
    pub const fn observation_class(self) -> ObservationClass {
        self.observation_class
    }

    #[must_use]
    pub const fn publication_lane(self) -> PublicationLane {
        self.publication_lane
    }

    #[must_use]
    pub const fn can_advance_committed_watermark(&self) -> bool {
        matches!(self.publication_lane, PublicationLane::CommittedCandidate)
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum SourceTrustError {
    #[error("source trust is incompatible with the observation class")]
    IncompatibleObservationClass,
}
