use async_trait::async_trait;
use bytes::Bytes;
use canonical_events::{
    BlockEnvelope, CommittedNodeV1MappingContext, ConfirmationClass, MappingError,
    map_committed_node_v1_block,
};
use domain_types::{BlockHeight, ChainId, SourceId};
use hl_protocol::node::v1::{NodeStreamKind, parse_node_record};
use hl_protocol::{ObservationClass, SourceAdmission, SourceObservation, SourceTrust};

use crate::coordinator::CaptureCoordinator;
use crate::{
    BlockCandidate, CandidateError, CanonicalSequencer, SequencerConfig, SequencerDecision,
    SequencerError,
};

#[async_trait]
pub trait CanonicalBlockCommitter: Send + Sync {
    async fn commit(&self, block: &BlockEnvelope) -> Result<(), &'static str>;
}

#[async_trait]
impl CanonicalBlockCommitter for CaptureCoordinator {
    async fn commit(&self, block: &BlockEnvelope) -> Result<(), &'static str> {
        self.process_block(block)
            .await
            .map(|_| ())
            .map_err(|error| error.reason_code())
    }
}

#[derive(Debug, Clone)]
pub struct CommittedNodePipelineConfig {
    chain_id: ChainId,
    source_id: SourceId,
    source_version: String,
    admission: SourceAdmission,
    sequencer: SequencerConfig,
}

impl CommittedNodePipelineConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        chain_id: ChainId,
        source_id: SourceId,
        source_version: impl Into<String>,
        admission: SourceAdmission,
        first_height: BlockHeight,
        max_pending_blocks: usize,
        retained_committed_blocks: usize,
    ) -> Result<Self, PipelineError> {
        let source_version = source_version.into();
        if source_version.is_empty()
            || source_version.trim() != source_version
            || source_version.chars().any(char::is_control)
            || admission.observation_class() != ObservationClass::CommittedBlock
            || !matches!(
                admission.trust(),
                SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted
            )
        {
            return Err(PipelineError::InvalidConfig);
        }
        let sequencer = SequencerConfig::try_new(
            chain_id.clone(),
            first_height,
            max_pending_blocks,
            retained_committed_blocks,
        )
        .map_err(PipelineError::Sequencer)?;
        Ok(Self {
            chain_id,
            source_id,
            source_version,
            admission,
            sequencer,
        })
    }
}

pub struct CommittedNodePipeline<'a, C: CanonicalBlockCommitter + ?Sized> {
    config: CommittedNodePipelineConfig,
    sequencer: CanonicalSequencer,
    committer: &'a C,
}

impl<C: CanonicalBlockCommitter + ?Sized> std::fmt::Debug for CommittedNodePipeline<'_, C> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CommittedNodePipeline")
            .field("config", &self.config)
            .field("sequencer", &self.sequencer)
            .finish_non_exhaustive()
    }
}

impl<'a, C: CanonicalBlockCommitter + ?Sized> CommittedNodePipeline<'a, C> {
    #[must_use]
    pub fn new(config: CommittedNodePipelineConfig, committer: &'a C) -> Self {
        let sequencer = CanonicalSequencer::new(config.sequencer.clone());
        Self {
            config,
            sequencer,
            committer,
        }
    }

    pub async fn process_spooled(
        &mut self,
        observation: &SourceObservation,
    ) -> Result<PipelineOutcome, PipelineError> {
        if observation.source_id() != &self.config.source_id
            || observation.source_version() != self.config.source_version
            || observation.observation_class() != ObservationClass::CommittedBlock
        {
            return Err(PipelineError::ObservationMismatch);
        }
        let record = parse_node_record(
            NodeStreamKind::TransactionBlocks,
            Bytes::copy_from_slice(observation.payload()),
        )
        .map_err(|_| PipelineError::SourceParse)?;
        let confirmation_class = match self.config.admission.trust() {
            SourceTrust::LocallyVerifiedCommitted => ConfirmationClass::CommittedPrimary,
            SourceTrust::IndependentCommitted => ConfirmationClass::CommittedIndependent,
            _ => return Err(PipelineError::InvalidConfig),
        };
        let block = map_committed_node_v1_block(
            &record,
            &CommittedNodeV1MappingContext {
                chain_id: self.config.chain_id.clone(),
                source_id: self.config.source_id.clone(),
                source_version: self.config.source_version.clone(),
                source_offset: observation.cursor().offset().to_string(),
                expected_height: BlockHeight::new(observation.cursor().offset()),
                confirmation_class,
            },
        )
        .map_err(PipelineError::Mapping)?;
        let candidate =
            BlockCandidate::try_new(self.config.source_id.clone(), self.config.admission, block)
                .map_err(PipelineError::Candidate)?;
        let decisions = self
            .sequencer
            .observe(candidate)
            .map_err(PipelineError::Sequencer)?;
        self.execute(decisions).await
    }

    async fn execute(
        &self,
        decisions: Vec<SequencerDecision>,
    ) -> Result<PipelineOutcome, PipelineError> {
        let mut committed = None;
        let mut duplicate = None;
        let mut gap = None;
        for decision in decisions {
            match decision {
                SequencerDecision::Commit(block) => {
                    self.committer
                        .commit(&block)
                        .await
                        .map_err(PipelineError::Commit)?;
                    committed = Some(block.block_height());
                }
                SequencerDecision::RecordDuplicate { block_height, .. } => {
                    duplicate = Some(block_height);
                }
                SequencerDecision::RequestGap {
                    incident_id,
                    start,
                    end_inclusive,
                } => {
                    gap = Some((incident_id, start, end_inclusive));
                }
                SequencerDecision::AwaitMoreEvidence => {}
                SequencerDecision::Quarantine(record) => {
                    return Err(PipelineError::Quarantined {
                        reason_code: record.reason().reason_code(),
                    });
                }
                SequencerDecision::VerifyHistoricalBlock { .. } => {
                    return Err(PipelineError::HistoricalVerificationUnavailable);
                }
                SequencerDecision::AwaitOperatorResolution { .. } => {
                    return Err(PipelineError::OperatorResolutionRequired);
                }
                SequencerDecision::PublishProvisional(_) => {
                    return Err(PipelineError::UnexpectedProvisional);
                }
            }
        }
        if let Some(block_height) = committed {
            Ok(PipelineOutcome::Committed { block_height })
        } else if let Some((incident_id, start, end_inclusive)) = gap {
            Ok(PipelineOutcome::Gap {
                incident_id,
                start,
                end_inclusive,
            })
        } else if let Some(block_height) = duplicate {
            Ok(PipelineOutcome::Duplicate { block_height })
        } else {
            Ok(PipelineOutcome::AwaitingEvidence)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipelineOutcome {
    Committed {
        block_height: BlockHeight,
    },
    Duplicate {
        block_height: BlockHeight,
    },
    Gap {
        incident_id: String,
        start: BlockHeight,
        end_inclusive: BlockHeight,
    },
    AwaitingEvidence,
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("committed node pipeline configuration is invalid")]
    InvalidConfig,
    #[error("spooled observation does not match the committed node pipeline")]
    ObservationMismatch,
    #[error("spooled node observation cannot be parsed")]
    SourceParse,
    #[error("spooled node observation cannot be mapped: {0}")]
    Mapping(#[source] MappingError),
    #[error("mapped block is not an admissible sequencer candidate: {0}")]
    Candidate(#[source] CandidateError),
    #[error("canonical sequencing failed: {0}")]
    Sequencer(#[source] SequencerError),
    #[error("canonical commit failed")]
    Commit(&'static str),
    #[error("canonical evidence was quarantined")]
    Quarantined { reason_code: &'static str },
    #[error("historical verification is not yet connected")]
    HistoricalVerificationUnavailable,
    #[error("operator resolution is required")]
    OperatorResolutionRequired,
    #[error("a committed pipeline produced a provisional decision")]
    UnexpectedProvisional,
}

impl PipelineError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "capture_pipeline.invalid_config",
            Self::ObservationMismatch => "capture_pipeline.observation_mismatch",
            Self::SourceParse => "capture_pipeline.source_parse",
            Self::Mapping(error) => error.reason_code(),
            Self::Candidate(error) => error.reason_code(),
            Self::Sequencer(error) => error.reason_code(),
            Self::Commit(reason_code) | Self::Quarantined { reason_code } => reason_code,
            Self::HistoricalVerificationUnavailable => {
                "capture_pipeline.historical_verification_unavailable"
            }
            Self::OperatorResolutionRequired => "capture_pipeline.operator_resolution_required",
            Self::UnexpectedProvisional => "capture_pipeline.unexpected_provisional",
        }
    }
}
