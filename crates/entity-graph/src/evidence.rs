use domain_types::{EvidenceId, KnownTime, ProbabilityPpm, ProtocolTime};
use feature_core::{Bitemporal, EvidenceRef};
use serde::{Deserialize, Serialize};

use crate::{GraphError, GraphNodeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LinkKind {
    ProtocolSubaccount,
    ProtocolVaultMembership,
    ProtocolVaultManager,
    ApprovedOperatorAnnotation,
    FundingPath,
    CoordinatedExecution,
    SizePriceFingerprint,
    LeaderFollower,
    CounterpartyInventoryHandoff,
    StrategyMigration,
}

impl LinkKind {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::ProtocolSubaccount => "protocol_subaccount",
            Self::ProtocolVaultMembership => "protocol_vault_membership",
            Self::ProtocolVaultManager => "protocol_vault_manager",
            Self::ApprovedOperatorAnnotation => "approved_operator_annotation",
            Self::FundingPath => "funding_path",
            Self::CoordinatedExecution => "coordinated_execution",
            Self::SizePriceFingerprint => "size_price_fingerprint",
            Self::LeaderFollower => "leader_follower",
            Self::CounterpartyInventoryHandoff => "counterparty_inventory_handoff",
            Self::StrategyMigration => "strategy_migration",
        }
    }

    #[must_use]
    pub const fn is_hard(self) -> bool {
        match self {
            Self::ProtocolSubaccount
            | Self::ProtocolVaultMembership
            | Self::ProtocolVaultManager
            | Self::ApprovedOperatorAnnotation => true,
            Self::FundingPath
            | Self::CoordinatedExecution
            | Self::SizePriceFingerprint
            | Self::LeaderFollower
            | Self::CounterpartyInventoryHandoff
            | Self::StrategyMigration => false,
        }
    }

    /// Identity-bearing hard links that may form an administrative group.
    ///
    /// Vault membership is a hard protocol fact (a depositor is certainly in
    /// that vault) but does not collapse distinct depositors into one entity.
    /// Soft evidence never forms a group.
    #[must_use]
    pub const fn forms_administrative_group(self) -> bool {
        match self {
            Self::ProtocolSubaccount
            | Self::ProtocolVaultManager
            | Self::ApprovedOperatorAnnotation => true,
            Self::ProtocolVaultMembership
            | Self::FundingPath
            | Self::CoordinatedExecution
            | Self::SizePriceFingerprint
            | Self::LeaderFollower
            | Self::CounterpartyInventoryHandoff
            | Self::StrategyMigration => false,
        }
    }

    #[must_use]
    pub const fn evidence_family(self) -> EvidenceFamily {
        match self {
            Self::ProtocolSubaccount
            | Self::ProtocolVaultMembership
            | Self::ProtocolVaultManager
            | Self::ApprovedOperatorAnnotation => EvidenceFamily::HardProtocol,
            Self::FundingPath => EvidenceFamily::FundingPath,
            Self::CoordinatedExecution => EvidenceFamily::Synchronization,
            Self::SizePriceFingerprint => EvidenceFamily::Fingerprint,
            Self::LeaderFollower => EvidenceFamily::FollowerLag,
            Self::CounterpartyInventoryHandoff => EvidenceFamily::InventoryHandoff,
            Self::StrategyMigration => EvidenceFamily::Migration,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EvidenceFamily {
    HardProtocol,
    FundingPath,
    Synchronization,
    Fingerprint,
    FollowerLag,
    InventoryHandoff,
    Migration,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinkEvidence {
    pub evidence_id: EvidenceId,
    pub left: GraphNodeId,
    pub right: GraphNodeId,
    pub kind: LinkKind,
    pub probability: ProbabilityPpm,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub superseded_at: Option<KnownTime>,
    pub source_refs: Vec<EvidenceRef>,
    pub reviewer: Option<String>,
    pub revision: u32,
}

impl LinkEvidence {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        evidence_id: EvidenceId,
        left: GraphNodeId,
        right: GraphNodeId,
        kind: LinkKind,
        probability: ProbabilityPpm,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        source_refs: Vec<EvidenceRef>,
        reviewer: Option<String>,
    ) -> Result<Self, GraphError> {
        if left == right {
            return Err(GraphError::Malformed {
                what: "link_evidence",
                reason: "self-link",
            });
        }
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(GraphError::TemporalInversion);
        }
        if kind.is_hard() && probability != ProbabilityPpm::ONE {
            return Err(GraphError::Malformed {
                what: "link_evidence",
                reason: "hard links must be certainty",
            });
        }
        if kind == LinkKind::ApprovedOperatorAnnotation
            && reviewer
                .as_ref()
                .is_none_or(|value| value.trim().is_empty())
        {
            return Err(GraphError::Malformed {
                what: "link_evidence",
                reason: "annotation requires reviewer",
            });
        }
        Ok(Self {
            evidence_id,
            left,
            right,
            kind,
            probability,
            effective_at,
            known_at,
            superseded_at: None,
            source_refs,
            reviewer,
            revision: 1,
        })
    }
}

impl Bitemporal for LinkEvidence {
    fn effective_at(&self) -> ProtocolTime {
        self.effective_at
    }

    fn known_at(&self) -> KnownTime {
        self.known_at
    }

    fn superseded_at(&self) -> Option<KnownTime> {
        self.superseded_at
    }

    fn revision(&self) -> u32 {
        self.revision
    }
}
