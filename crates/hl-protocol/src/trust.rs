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
        let publication_lane = match trust {
            SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted => {
                match observation_class {
                    ObservationClass::CommittedBlock => PublicationLane::CommittedCandidate,
                    ObservationClass::AuxiliaryOrderStatus
                    | ObservationClass::AuxiliaryBookDiff
                    | ObservationClass::AuxiliaryLedger => PublicationLane::Reconciliation,
                    ObservationClass::Snapshot
                    | ObservationClass::HistoricalBlock
                    | ObservationClass::PublicMarketData
                    | ObservationClass::ProvisionalFeed
                    | ObservationClass::ProvisionalMempool => {
                        return Err(SourceTrustError::IncompatibleObservationClass);
                    }
                }
            }
            SourceTrust::ReconciledSnapshot => match observation_class {
                ObservationClass::Snapshot => PublicationLane::Reconciliation,
                ObservationClass::CommittedBlock
                | ObservationClass::AuxiliaryOrderStatus
                | ObservationClass::AuxiliaryBookDiff
                | ObservationClass::AuxiliaryLedger
                | ObservationClass::HistoricalBlock
                | ObservationClass::PublicMarketData
                | ObservationClass::ProvisionalFeed
                | ObservationClass::ProvisionalMempool => {
                    return Err(SourceTrustError::IncompatibleObservationClass);
                }
            },
            SourceTrust::RecoveryOnly => match observation_class {
                ObservationClass::HistoricalBlock => PublicationLane::Recovery,
                ObservationClass::CommittedBlock
                | ObservationClass::AuxiliaryOrderStatus
                | ObservationClass::AuxiliaryBookDiff
                | ObservationClass::AuxiliaryLedger
                | ObservationClass::Snapshot
                | ObservationClass::PublicMarketData
                | ObservationClass::ProvisionalFeed
                | ObservationClass::ProvisionalMempool => {
                    return Err(SourceTrustError::IncompatibleObservationClass);
                }
            },
            SourceTrust::ThirdPartyProvisional => match observation_class {
                ObservationClass::PublicMarketData | ObservationClass::ProvisionalFeed => {
                    PublicationLane::Provisional
                }
                ObservationClass::CommittedBlock
                | ObservationClass::AuxiliaryOrderStatus
                | ObservationClass::AuxiliaryBookDiff
                | ObservationClass::AuxiliaryLedger
                | ObservationClass::Snapshot
                | ObservationClass::HistoricalBlock
                | ObservationClass::ProvisionalMempool => {
                    return Err(SourceTrustError::IncompatibleObservationClass);
                }
            },
            SourceTrust::MempoolProvisional => match observation_class {
                ObservationClass::ProvisionalMempool => PublicationLane::Mempool,
                ObservationClass::CommittedBlock
                | ObservationClass::AuxiliaryOrderStatus
                | ObservationClass::AuxiliaryBookDiff
                | ObservationClass::AuxiliaryLedger
                | ObservationClass::Snapshot
                | ObservationClass::HistoricalBlock
                | ObservationClass::PublicMarketData
                | ObservationClass::ProvisionalFeed => {
                    return Err(SourceTrustError::IncompatibleObservationClass);
                }
            },
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
