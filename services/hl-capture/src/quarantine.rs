use domain_types::{BlockHeight, ChainId, EventId, SourceId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuarantineReason {
    ConflictingCanonicalBlock {
        existing_hash: [u8; 32],
        conflicting_hash: [u8; 32],
    },
    ConflictingSourceBlockHash {
        source_id: SourceId,
        existing_hash: [u8; 32],
        conflicting_hash: [u8; 32],
    },
    ConflictingEventSourceEvidence {
        event_id: EventId,
        source_id: SourceId,
        existing_hash: [u8; 32],
        conflicting_hash: [u8; 32],
    },
}

impl QuarantineReason {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::ConflictingCanonicalBlock { .. } => "sequencer.conflicting_canonical_block",
            Self::ConflictingSourceBlockHash { .. } => "sequencer.conflicting_source_block_hash",
            Self::ConflictingEventSourceEvidence { .. } => {
                "sequencer.conflicting_event_source_evidence"
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuarantineRecord {
    incident_id: String,
    chain_id: ChainId,
    block_height: BlockHeight,
    existing_source_ids: Vec<SourceId>,
    conflicting_source_id: SourceId,
    reason: QuarantineReason,
}

impl QuarantineRecord {
    pub(crate) fn new(
        incident_id: String,
        chain_id: ChainId,
        block_height: BlockHeight,
        existing_source_ids: Vec<SourceId>,
        conflicting_source_id: SourceId,
        reason: QuarantineReason,
    ) -> Self {
        Self {
            incident_id,
            chain_id,
            block_height,
            existing_source_ids,
            conflicting_source_id,
            reason,
        }
    }

    #[must_use]
    pub fn incident_id(&self) -> &str {
        &self.incident_id
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
    pub fn existing_source_ids(&self) -> &[SourceId] {
        &self.existing_source_ids
    }

    #[must_use]
    pub const fn conflicting_source_id(&self) -> &SourceId {
        &self.conflicting_source_id
    }

    #[must_use]
    pub const fn reason(&self) -> &QuarantineReason {
        &self.reason
    }
}
