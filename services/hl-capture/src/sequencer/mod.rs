//! Deterministic in-memory continuity decisions for canonical block candidates.
//!
//! This module performs no I/O. A `Commit` decision and the logical committed
//! watermark are not durable until later orchestration archives the block and
//! persists its archive-bound cursor atomically.

mod divergence;
mod gap;
mod watermark;

use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, BlockError, ConfirmationClass, EvidenceMergeError};
use domain_types::{BlockHeight, ChainId, EventId, SourceId};
use hl_protocol::{PublicationLane, SourceAdmission, SourceTrust};

use divergence::{
    canonical_block_divergence, event_source_evidence_divergence, source_block_hash_divergence,
};
use gap::gap_incident_id;
pub use gap::GapRange;
use watermark::Watermark;

use crate::QuarantineRecord;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequencerConfig {
    chain_id: ChainId,
    first_height: BlockHeight,
    max_pending_blocks: usize,
    retained_committed_blocks: usize,
}

impl SequencerConfig {
    pub fn try_new(
        chain_id: ChainId,
        first_height: BlockHeight,
        max_pending_blocks: usize,
        retained_committed_blocks: usize,
    ) -> Result<Self, SequencerError> {
        if max_pending_blocks == 0 {
            return Err(SequencerError::InvalidCapacity {
                field: "max_pending_blocks",
            });
        }
        if retained_committed_blocks == 0 {
            return Err(SequencerError::InvalidCapacity {
                field: "retained_committed_blocks",
            });
        }
        Ok(Self {
            chain_id,
            first_height,
            max_pending_blocks,
            retained_committed_blocks,
        })
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn first_height(&self) -> BlockHeight {
        self.first_height
    }

    #[must_use]
    pub const fn max_pending_blocks(&self) -> usize {
        self.max_pending_blocks
    }

    #[must_use]
    pub const fn retained_committed_blocks(&self) -> usize {
        self.retained_committed_blocks
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockCandidate {
    source_id: SourceId,
    admission: SourceAdmission,
    block: BlockEnvelope,
}

impl BlockCandidate {
    pub fn try_new(
        source_id: SourceId,
        admission: SourceAdmission,
        block: BlockEnvelope,
    ) -> Result<Self, CandidateError> {
        if block.source_block_hashes().len() != 1
            || !block.source_block_hashes().contains_key(&source_id)
        {
            return Err(CandidateError::MissingSourceBlockHash);
        }
        if block.events().iter().any(|event| {
            event
                .source_evidence()
                .iter()
                .any(|evidence| evidence.source_id() != &source_id)
        }) {
            return Err(CandidateError::UnexpectedEventSourceEvidence);
        }
        validate_confirmation(admission, block.confirmation_class())?;
        Ok(Self {
            source_id,
            admission,
            block,
        })
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn admission(&self) -> SourceAdmission {
        self.admission
    }

    #[must_use]
    pub const fn block(&self) -> &BlockEnvelope {
        &self.block
    }

    #[must_use]
    pub fn into_parts(self) -> (SourceId, SourceAdmission, BlockEnvelope) {
        (self.source_id, self.admission, self.block)
    }
}

fn validate_confirmation(
    admission: SourceAdmission,
    confirmation: ConfirmationClass,
) -> Result<(), CandidateError> {
    let expected = match admission.publication_lane() {
        PublicationLane::CommittedCandidate => match admission.trust() {
            SourceTrust::LocallyVerifiedCommitted => ConfirmationClass::CommittedPrimary,
            SourceTrust::IndependentCommitted => ConfirmationClass::CommittedIndependent,
            SourceTrust::ReconciledSnapshot
            | SourceTrust::RecoveryOnly
            | SourceTrust::ThirdPartyProvisional
            | SourceTrust::MempoolProvisional => {
                return Err(CandidateError::UnsupportedPublicationLane);
            }
        },
        PublicationLane::Provisional => match admission.trust() {
            SourceTrust::ThirdPartyProvisional => ConfirmationClass::ProvisionalSource,
            SourceTrust::LocallyVerifiedCommitted
            | SourceTrust::IndependentCommitted
            | SourceTrust::ReconciledSnapshot
            | SourceTrust::RecoveryOnly
            | SourceTrust::MempoolProvisional => {
                return Err(CandidateError::UnsupportedPublicationLane);
            }
        },
        PublicationLane::Reconciliation | PublicationLane::Recovery | PublicationLane::Mempool => {
            return Err(CandidateError::UnsupportedPublicationLane);
        }
    };
    if confirmation == expected {
        Ok(())
    } else {
        Err(CandidateError::ConfirmationMismatch)
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum CandidateError {
    #[error("candidate block must contain exactly its source block hash")]
    MissingSourceBlockHash,
    #[error("source admission and canonical confirmation class disagree")]
    ConfirmationMismatch,
    #[error("candidate event evidence names a different source")]
    UnexpectedEventSourceEvidence,
    #[error("publication lane is not accepted by the canonical sequencer")]
    UnsupportedPublicationLane,
}

impl CandidateError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::MissingSourceBlockHash => "sequencer.missing_source_block_hash",
            Self::ConfirmationMismatch => "sequencer.confirmation_mismatch",
            Self::UnexpectedEventSourceEvidence => "sequencer.unexpected_event_source_evidence",
            Self::UnsupportedPublicationLane => "sequencer.unsupported_publication_lane",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequencerDecision {
    /// The block is the next contiguous logical commit candidate.
    ///
    /// Callers must archive it before advancing a durable cursor.
    Commit(BlockEnvelope),
    RecordDuplicate {
        block_height: BlockHeight,
        source_id: SourceId,
    },
    RequestGap {
        incident_id: String,
        start: BlockHeight,
        end_inclusive: BlockHeight,
    },
    Quarantine(QuarantineRecord),
    PublishProvisional(BlockEnvelope),
    /// The in-memory retained window cannot prove whether this old block
    /// matches; verify it against the immutable archive.
    VerifyHistoricalBlock {
        block_height: BlockHeight,
        source_id: SourceId,
        canonical_block_hash: [u8; 32],
    },
    AwaitOperatorResolution {
        incident_id: String,
    },
    AwaitMoreEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SequencerHealth {
    Green,
    RedGap {
        incident_id: String,
        start: BlockHeight,
        end_inclusive: BlockHeight,
    },
    Red {
        incident_id: String,
    },
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SequencerError {
    #[error("sequencer capacity {field} must be greater than zero")]
    InvalidCapacity { field: &'static str },
    #[error("candidate belongs to chain {actual}, expected {expected}")]
    ChainMismatch { expected: ChainId, actual: ChainId },
    #[error("pending canonical block capacity {limit} is exhausted")]
    PendingCapacityExceeded { limit: usize },
    #[error("canonical block evidence merge failed: {0}")]
    InvalidMergedBlock(BlockError),
    #[error("canonical event evidence merge failed: {reason}")]
    InvalidMergedEvidence {
        event_id: Option<EventId>,
        reason: &'static str,
    },
    #[error(transparent)]
    Candidate(CandidateError),
}

impl SequencerError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidCapacity { .. } => "sequencer.invalid_capacity",
            Self::ChainMismatch { .. } => "sequencer.chain_mismatch",
            Self::PendingCapacityExceeded { .. } => "sequencer.pending_capacity_exceeded",
            Self::InvalidMergedBlock(_) => "sequencer.invalid_merged_block",
            Self::InvalidMergedEvidence { .. } => "sequencer.invalid_merged_evidence",
            Self::Candidate(error) => error.reason_code(),
        }
    }
}

#[derive(Debug)]
struct PendingBlock {
    block: BlockEnvelope,
}

impl PendingBlock {
    fn new(block: BlockEnvelope) -> Self {
        Self { block }
    }
}

#[derive(Debug)]
pub struct CanonicalSequencer {
    config: SequencerConfig,
    committed: Watermark,
    provisional: Watermark,
    pending: BTreeMap<BlockHeight, PendingBlock>,
    outstanding_gap: Option<GapRange>,
    red_incident_id: Option<String>,
    quarantines: Vec<QuarantineRecord>,
}

impl CanonicalSequencer {
    #[must_use]
    pub fn new(config: SequencerConfig) -> Self {
        Self {
            committed: Watermark::new(config.retained_committed_blocks),
            provisional: Watermark::new(config.retained_committed_blocks),
            config,
            pending: BTreeMap::new(),
            outstanding_gap: None,
            red_incident_id: None,
            quarantines: Vec::new(),
        }
    }

    pub fn observe(
        &mut self,
        candidate: BlockCandidate,
    ) -> Result<Vec<SequencerDecision>, SequencerError> {
        if candidate.block.chain_id() != &self.config.chain_id {
            return Err(SequencerError::ChainMismatch {
                expected: self.config.chain_id.clone(),
                actual: candidate.block.chain_id().clone(),
            });
        }

        match candidate.admission.publication_lane() {
            PublicationLane::CommittedCandidate => self.observe_committed(candidate),
            PublicationLane::Provisional => self.observe_provisional(candidate),
            PublicationLane::Reconciliation
            | PublicationLane::Recovery
            | PublicationLane::Mempool => Err(SequencerError::Candidate(
                CandidateError::UnsupportedPublicationLane,
            )),
        }
    }

    #[must_use]
    /// Returns the contiguous logical watermark, not a persisted cursor.
    pub const fn committed_watermark(&self) -> Option<BlockHeight> {
        self.committed.current()
    }

    #[must_use]
    pub const fn provisional_watermark(&self) -> Option<BlockHeight> {
        self.provisional.current()
    }

    #[must_use]
    pub const fn outstanding_gap(&self) -> Option<GapRange> {
        self.outstanding_gap
    }

    #[must_use]
    pub fn pending_block_count(&self) -> usize {
        self.pending.len()
    }

    #[must_use]
    pub fn quarantines(&self) -> &[QuarantineRecord] {
        &self.quarantines
    }

    #[must_use]
    pub fn health(&self) -> SequencerHealth {
        if let Some(incident_id) = &self.red_incident_id {
            SequencerHealth::Red {
                incident_id: incident_id.clone(),
            }
        } else if let Some(gap) = self.outstanding_gap {
            SequencerHealth::RedGap {
                incident_id: gap_incident_id(&self.config.chain_id, gap),
                start: gap.start(),
                end_inclusive: gap.end_inclusive(),
            }
        } else {
            SequencerHealth::Green
        }
    }

    fn observe_committed(
        &mut self,
        candidate: BlockCandidate,
    ) -> Result<Vec<SequencerDecision>, SequencerError> {
        if let Some(incident_id) = &self.red_incident_id {
            return Ok(vec![SequencerDecision::AwaitOperatorResolution {
                incident_id: incident_id.clone(),
            }]);
        }

        let height = candidate.block.block_height();
        if height < self.config.first_height
            || self
                .committed
                .current()
                .is_some_and(|watermark| height <= watermark)
        {
            return self.observe_retained(candidate, true);
        }

        if let Some(existing) = self.pending.get_mut(&height) {
            return match compare_and_merge(&mut existing.block, &candidate) {
                Ok(()) => Ok(vec![SequencerDecision::RecordDuplicate {
                    block_height: height,
                    source_id: candidate.source_id,
                }]),
                Err(MergeError::Divergence(record)) => Ok(vec![self.latch_divergence(*record)]),
                Err(MergeError::InvalidBlock(error)) => {
                    Err(SequencerError::InvalidMergedBlock(error))
                }
                Err(MergeError::InvalidEvidence { event_id, reason }) => {
                    Err(SequencerError::InvalidMergedEvidence { event_id, reason })
                }
            };
        }
        let next_height = self.next_committed_height();
        if self.pending.len() >= self.config.max_pending_blocks && Some(height) != next_height {
            return Err(SequencerError::PendingCapacityExceeded {
                limit: self.config.max_pending_blocks,
            });
        }
        self.pending
            .insert(height, PendingBlock::new(candidate.block));

        let mut decisions = self.drain_contiguous();
        self.refresh_gap(&mut decisions);
        if decisions.is_empty() {
            decisions.push(SequencerDecision::AwaitMoreEvidence);
        }
        Ok(decisions)
    }

    fn observe_provisional(
        &mut self,
        candidate: BlockCandidate,
    ) -> Result<Vec<SequencerDecision>, SequencerError> {
        let height = candidate.block.block_height();
        if self
            .provisional
            .current()
            .is_some_and(|watermark| height <= watermark)
        {
            return self.observe_retained(candidate, false);
        }

        self.provisional.advance(candidate.block.clone());
        Ok(vec![SequencerDecision::PublishProvisional(candidate.block)])
    }

    fn observe_retained(
        &mut self,
        candidate: BlockCandidate,
        committed: bool,
    ) -> Result<Vec<SequencerDecision>, SequencerError> {
        let watermark = if committed {
            &mut self.committed
        } else {
            &mut self.provisional
        };
        let height = candidate.block.block_height();
        let Some(existing) = watermark.retained_mut(height) else {
            return Ok(vec![SequencerDecision::VerifyHistoricalBlock {
                block_height: height,
                source_id: candidate.source_id,
                canonical_block_hash: candidate.block.canonical_block_hash(),
            }]);
        };

        let comparison = compare_and_merge(&mut existing.block, &candidate);
        match comparison {
            Ok(()) => Ok(vec![SequencerDecision::RecordDuplicate {
                block_height: height,
                source_id: candidate.source_id,
            }]),
            Err(MergeError::Divergence(record)) => Ok(vec![self.latch_divergence(*record)]),
            Err(MergeError::InvalidBlock(error)) => Err(SequencerError::InvalidMergedBlock(error)),
            Err(MergeError::InvalidEvidence { event_id, reason }) => {
                Err(SequencerError::InvalidMergedEvidence { event_id, reason })
            }
        }
    }

    fn drain_contiguous(&mut self) -> Vec<SequencerDecision> {
        let mut decisions = Vec::new();
        while let Some(next) = self.next_committed_height() {
            let Some(pending) = self.pending.remove(&next) else {
                break;
            };
            self.committed.advance(pending.block.clone());
            decisions.push(SequencerDecision::Commit(pending.block));
        }
        decisions
    }

    fn refresh_gap(&mut self, decisions: &mut Vec<SequencerDecision>) {
        let Some(next) = self.next_committed_height() else {
            self.outstanding_gap = None;
            return;
        };
        let next_pending = self.pending.keys().next().copied();
        let gap = next_pending
            .filter(|height| *height > next)
            .map(|height| GapRange::new(next, BlockHeight::new(height.get().saturating_sub(1))));
        if gap != self.outstanding_gap {
            self.outstanding_gap = gap;
            if let Some(gap) = gap {
                decisions.push(SequencerDecision::RequestGap {
                    incident_id: gap_incident_id(&self.config.chain_id, gap),
                    start: gap.start(),
                    end_inclusive: gap.end_inclusive(),
                });
            }
        }
    }

    fn next_committed_height(&self) -> Option<BlockHeight> {
        match self.committed.current() {
            Some(height) => height.get().checked_add(1).map(BlockHeight::new),
            None => Some(self.config.first_height),
        }
    }

    fn latch_divergence(&mut self, record: QuarantineRecord) -> SequencerDecision {
        if self.red_incident_id.is_none() {
            self.red_incident_id = Some(record.incident_id().to_owned());
            self.quarantines.push(record.clone());
        }
        SequencerDecision::Quarantine(record)
    }
}

fn compare_and_merge(
    existing: &mut BlockEnvelope,
    candidate: &BlockCandidate,
) -> Result<(), MergeError> {
    if existing.canonical_block_hash() != candidate.block.canonical_block_hash() {
        return Err(MergeError::Divergence(Box::new(
            canonical_block_divergence(
                existing.chain_id(),
                existing.block_height(),
                existing.source_block_hashes().keys().cloned().collect(),
                candidate.source_id.clone(),
                existing.canonical_block_hash(),
                candidate.block.canonical_block_hash(),
            ),
        )));
    }
    if let Some(record) = source_hash_conflict(
        existing.chain_id(),
        existing.block_height(),
        existing.source_block_hashes(),
        candidate,
    ) {
        return Err(MergeError::Divergence(Box::new(record)));
    }

    let mut source_hashes = existing.source_block_hashes().clone();
    source_hashes.extend(
        candidate
            .block
            .source_block_hashes()
            .iter()
            .map(|(source, hash)| (source.clone(), *hash)),
    );
    if existing.events().len() != candidate.block.events().len() {
        return Err(MergeError::InvalidEvidence {
            event_id: None,
            reason: "matching canonical block hashes contain different event counts",
        });
    }
    let mut merged_events = Vec::with_capacity(existing.events().len());
    for (existing_event, candidate_event) in existing.events().iter().zip(candidate.block.events())
    {
        match existing_event.merge_matching_source_evidence(candidate_event) {
            Ok(merged) => merged_events.push(merged),
            Err(EvidenceMergeError::SourceEvidenceConflict {
                source_id,
                existing_hash,
                conflicting_hash,
            }) => {
                return Err(MergeError::Divergence(Box::new(
                    event_source_evidence_divergence(
                        existing.chain_id(),
                        existing.block_height(),
                        existing_event.event_id().clone(),
                        source_id,
                        existing_hash,
                        conflicting_hash,
                    ),
                )));
            }
            Err(EvidenceMergeError::CanonicalContentMismatch) => {
                return Err(MergeError::InvalidEvidence {
                    event_id: Some(existing_event.event_id().clone()),
                    reason: "matching canonical block hash contains different event content",
                });
            }
        }
    }

    *existing = BlockEnvelope::try_new(
        existing.chain_id().clone(),
        existing.block_height(),
        existing.block_time(),
        existing.confirmation_class(),
        merged_events,
        source_hashes,
    )
    .map_err(MergeError::InvalidBlock)?;
    Ok(())
}

#[derive(Debug)]
enum MergeError {
    Divergence(Box<QuarantineRecord>),
    InvalidBlock(BlockError),
    InvalidEvidence {
        event_id: Option<EventId>,
        reason: &'static str,
    },
}

fn source_hash_conflict(
    chain_id: &ChainId,
    block_height: BlockHeight,
    existing: &BTreeMap<SourceId, [u8; 32]>,
    candidate: &BlockCandidate,
) -> Option<QuarantineRecord> {
    let candidate_hash = candidate.block.source_block_hashes()[&candidate.source_id];
    existing
        .get(&candidate.source_id)
        .filter(|existing_hash| **existing_hash != candidate_hash)
        .map(|existing_hash| {
            source_block_hash_divergence(
                chain_id,
                block_height,
                candidate.source_id.clone(),
                *existing_hash,
                candidate_hash,
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use canonical_events::{
        CanonicalEventEnvelope, CanonicalEventInput, EventPayload, SourceEvidence, TradeMatched,
    };
    use domain_types::{KnownTime, Price, ProtocolTime, Quantity, TransactionId};
    use hl_protocol::ObservationClass;

    fn known(micros: i64) -> KnownTime {
        KnownTime::from_unix_micros(micros).expect("known test time")
    }

    fn fixture_block(confirmation: ConfirmationClass) -> (SourceId, BlockEnvelope) {
        let source_id = SourceId::new("primary").expect("source ID");
        let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
            schema_version: "1.0.0".to_owned(),
            chain_id: ChainId::new("mainnet").expect("chain"),
            block_height: BlockHeight::new(100),
            block_time: ProtocolTime::from_unix_micros(100_000).expect("block time"),
            transaction_id: TransactionId::new("tx-100").expect("transaction"),
            transaction_index: 0,
            canonical_event_index: 0,
            market_ids: Vec::new(),
            account_ids: Vec::new(),
            source_evidence: vec![SourceEvidence::try_new(
                source_id.clone(),
                "node-v1",
                "block:100",
                [1; 32],
            )
            .expect("source evidence")],
            confirmation_class: confirmation,
            observed_at: known(2_000),
            ingested_at: known(3_000),
            canonicalized_at: known(4_000),
            parser_version: "canonical-parser-v1".to_owned(),
            payload: EventPayload::TradeMatched(TradeMatched::without_identities(
                Price::parse_at_scale("65000", 6).expect("price"),
                Quantity::parse_at_scale("0.01", 8).expect("quantity"),
                1,
            )),
        })
        .expect("canonical event");
        let block = BlockEnvelope::try_new(
            ChainId::new("mainnet").expect("chain"),
            BlockHeight::new(100),
            ProtocolTime::from_unix_micros(100_000).expect("block time"),
            confirmation,
            vec![event],
            BTreeMap::from([(source_id.clone(), [0x55; 32])]),
        )
        .expect("canonical block");
        (source_id, block)
    }

    fn sequencer() -> CanonicalSequencer {
        CanonicalSequencer::new(
            SequencerConfig::try_new(
                ChainId::new("mainnet").expect("chain"),
                BlockHeight::new(100),
                8,
                8,
            )
            .expect("sequencer config"),
        )
    }

    fn confirmation_for(trust: SourceTrust) -> ConfirmationClass {
        match trust {
            SourceTrust::LocallyVerifiedCommitted => ConfirmationClass::CommittedPrimary,
            SourceTrust::IndependentCommitted => ConfirmationClass::CommittedIndependent,
            SourceTrust::ThirdPartyProvisional => ConfirmationClass::ProvisionalSource,
            SourceTrust::ReconciledSnapshot
            | SourceTrust::RecoveryOnly
            | SourceTrust::MempoolProvisional => ConfirmationClass::CommittedPrimary,
        }
    }

    fn assert_observe_rejects_unsupported_lane(
        admission: SourceAdmission,
        confirmation: ConfirmationClass,
    ) {
        let (source_id, block) = fixture_block(confirmation);
        let candidate = BlockCandidate {
            source_id,
            admission,
            block,
        };
        let error = sequencer()
            .observe(candidate)
            .expect_err("unsupported lanes fail closed at observe");
        assert_eq!(
            error,
            SequencerError::Candidate(CandidateError::UnsupportedPublicationLane)
        );
        assert_eq!(
            error.reason_code(),
            "sequencer.unsupported_publication_lane"
        );
    }

    #[test]
    fn observe_returns_existing_unsupported_lane_error_when_try_new_is_bypassed() {
        let mut saw_reconciliation = false;
        let mut saw_recovery = false;
        let mut saw_mempool = false;

        for trust in SourceTrust::ALL {
            for class in ObservationClass::ALL {
                let Ok(admission) = SourceAdmission::new(trust, class) else {
                    continue;
                };
                let confirmation = confirmation_for(trust);
                match admission.publication_lane() {
                    PublicationLane::CommittedCandidate | PublicationLane::Provisional => {
                        let (source_id, block) = fixture_block(confirmation);
                        let candidate = BlockCandidate::try_new(source_id, admission, block)
                            .expect("admitted sequencer lanes still construct");
                        sequencer()
                            .observe(candidate)
                            .expect("admitted lanes keep the observe success contract");
                    }
                    PublicationLane::Reconciliation => {
                        saw_reconciliation = true;
                        assert_observe_rejects_unsupported_lane(admission, confirmation);
                    }
                    PublicationLane::Recovery => {
                        saw_recovery = true;
                        assert_observe_rejects_unsupported_lane(admission, confirmation);
                    }
                    PublicationLane::Mempool => {
                        saw_mempool = true;
                        assert_observe_rejects_unsupported_lane(admission, confirmation);
                    }
                }
            }
        }

        assert!(
            saw_reconciliation && saw_recovery && saw_mempool,
            "fixture must still construct reconciliation, recovery, and mempool admissions"
        );
    }
}
