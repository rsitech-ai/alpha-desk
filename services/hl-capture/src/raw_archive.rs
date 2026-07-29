use std::sync::Arc;

use async_trait::async_trait;
use domain_types::ChainId;
use hl_protocol::SourceObservation;
use storage_ports::{ArchiveError, RawObservationArchive, RawObservationBatch};

use crate::spool::{CloseReceipt, SpoolError, SpoolReader};

const MICROS_PER_HOUR: i64 = 3_600_000_000;

#[async_trait]
pub trait RawSegmentArchive: Send + Sync {
    async fn archive_segment(
        &self,
        chain_id: &ChainId,
        segment: &CloseReceipt,
        max_payload_bytes: usize,
    ) -> Result<RawSegmentArchiveSummary, RawSegmentArchiveError>;
}

#[derive(Clone)]
pub struct BlockingRawSegmentArchive {
    archive: Arc<dyn RawObservationArchive>,
}

impl std::fmt::Debug for BlockingRawSegmentArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BlockingRawSegmentArchive")
            .finish_non_exhaustive()
    }
}

impl BlockingRawSegmentArchive {
    #[must_use]
    pub fn new(archive: Arc<dyn RawObservationArchive>) -> Self {
        Self { archive }
    }
}

#[async_trait]
impl RawSegmentArchive for BlockingRawSegmentArchive {
    async fn archive_segment(
        &self,
        chain_id: &ChainId,
        segment: &CloseReceipt,
        max_payload_bytes: usize,
    ) -> Result<RawSegmentArchiveSummary, RawSegmentArchiveError> {
        let archive = Arc::clone(&self.archive);
        let chain_id = chain_id.clone();
        let segment = segment.clone();
        tokio::task::spawn_blocking(move || {
            archive_segment(archive.as_ref(), chain_id, &segment, max_payload_bytes)
        })
        .await
        .map_err(|_| RawSegmentArchiveError::BlockingTask)?
    }
}

fn archive_segment(
    archive: &dyn RawObservationArchive,
    chain_id: ChainId,
    segment: &CloseReceipt,
    max_payload_bytes: usize,
) -> Result<RawSegmentArchiveSummary, RawSegmentArchiveError> {
    if max_payload_bytes == 0 {
        return Err(RawSegmentArchiveError::InvalidConfig);
    }
    segment
        .verify_current()
        .map_err(RawSegmentArchiveError::Spool)?;
    let reader =
        SpoolReader::open(segment.segment_path()).map_err(RawSegmentArchiveError::Spool)?;
    let source_id = reader.header().source_id().clone();
    let source_version = reader.header().source_version().to_owned();
    let records = reader.read_all().map_err(RawSegmentArchiveError::Spool)?;
    let expected_count = usize::try_from(segment.manifest().record_count())
        .map_err(|_| RawSegmentArchiveError::SizeOverflow)?;
    if records.len() != expected_count {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    let observations = records
        .into_iter()
        .map(|record| {
            record
                .into_observation(source_id.clone(), source_version.clone(), max_payload_bytes)
                .map_err(|_| RawSegmentArchiveError::Observation)
        })
        .collect::<Result<Vec<_>, _>>()?;
    segment
        .verify_current()
        .map_err(RawSegmentArchiveError::Spool)?;
    let mut batches = Vec::<Vec<SourceObservation>>::new();
    for observation in observations {
        let hour = observation.received().wall_micros() / MICROS_PER_HOUR;
        if batches.last().is_none_or(|batch| {
            batch
                .last()
                .is_none_or(|last| last.received().wall_micros() / MICROS_PER_HOUR != hour)
        }) {
            batches.push(Vec::new());
        }
        batches
            .last_mut()
            .ok_or(RawSegmentArchiveError::SizeOverflow)?
            .push(observation);
    }

    let batch_count =
        u64::try_from(batches.len()).map_err(|_| RawSegmentArchiveError::SizeOverflow)?;
    let mut observation_count = 0_u64;
    for observations in batches {
        let count =
            u64::try_from(observations.len()).map_err(|_| RawSegmentArchiveError::SizeOverflow)?;
        let batch = RawObservationBatch::try_new(
            chain_id.clone(),
            observations,
            segment.manifest_hash(),
            segment.manifest().segment_blake3(),
        )
        .map_err(RawSegmentArchiveError::Archive)?;
        let receipt = archive
            .append_batch(&batch)
            .map_err(RawSegmentArchiveError::Archive)?;
        let verified = archive
            .verify_raw_manifest(receipt.manifest_id())
            .map_err(RawSegmentArchiveError::Archive)?;
        if verified.spool_manifest_blake3() != segment.manifest_hash()
            || verified.spool_segment_blake3() != segment.manifest().segment_blake3()
            || verified.object().chain_id() != &chain_id
            || verified.object().source_id() != &source_id
        {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        observation_count = observation_count
            .checked_add(count)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
    }
    Ok(RawSegmentArchiveSummary {
        observation_count,
        batch_count,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawSegmentArchiveSummary {
    observation_count: u64,
    batch_count: u64,
}

impl RawSegmentArchiveSummary {
    #[must_use]
    pub const fn observation_count(self) -> u64 {
        self.observation_count
    }

    #[must_use]
    pub const fn batch_count(self) -> u64 {
        self.batch_count
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RawSegmentArchiveError {
    #[error("raw segment archive configuration is invalid")]
    InvalidConfig,
    #[error("raw segment spool evidence is invalid: {0}")]
    Spool(#[source] SpoolError),
    #[error("raw segment record cannot be reconstructed")]
    Observation,
    #[error("raw segment archive operation failed: {0}")]
    Archive(#[source] ArchiveError),
    #[error("raw segment archive verification did not match the spool evidence")]
    VerificationMismatch,
    #[error("raw segment archive size calculation overflowed")]
    SizeOverflow,
    #[error("raw segment archive blocking task failed")]
    BlockingTask,
}

impl RawSegmentArchiveError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidConfig => "capture_raw_archive.invalid_config",
            Self::Spool(error) => error.reason_code(),
            Self::Observation => "capture_raw_archive.observation",
            Self::Archive(error) => error.reason_code(),
            Self::VerificationMismatch => "capture_raw_archive.verification_mismatch",
            Self::SizeOverflow => "capture_raw_archive.size_overflow",
            Self::BlockingTask => "capture_raw_archive.blocking_task",
        }
    }
}
