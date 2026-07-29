use std::sync::Arc;

use async_trait::async_trait;
use domain_types::ChainId;
use hl_protocol::SourceObservation;
use storage_ports::{ArchiveError, RawObservationArchive, RawObservationBatch};

use crate::spool::{CloseReceipt, SpoolError, SpoolRead, SpoolReader};

const MICROS_PER_HOUR: i64 = 3_600_000_000;
const MAX_ARCHIVE_BATCH_RECORDS: usize = 1_000_000;
const MAX_ARCHIVE_BATCH_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RawSegmentArchiveConfig {
    max_payload_bytes: usize,
    max_batch_records: usize,
    max_batch_bytes: u64,
}

impl RawSegmentArchiveConfig {
    pub fn try_new(
        max_payload_bytes: usize,
        max_batch_records: usize,
        max_batch_bytes: u64,
    ) -> Result<Self, RawSegmentArchiveError> {
        if max_payload_bytes == 0
            || !(1..=MAX_ARCHIVE_BATCH_RECORDS).contains(&max_batch_records)
            || max_batch_bytes == 0
            || max_batch_bytes > MAX_ARCHIVE_BATCH_BYTES
            || u64::try_from(max_payload_bytes).map_err(|_| RawSegmentArchiveError::SizeOverflow)?
                > max_batch_bytes
        {
            return Err(RawSegmentArchiveError::InvalidConfig);
        }
        Ok(Self {
            max_payload_bytes,
            max_batch_records,
            max_batch_bytes,
        })
    }

    #[must_use]
    pub const fn max_payload_bytes(self) -> usize {
        self.max_payload_bytes
    }

    #[must_use]
    pub const fn max_batch_records(self) -> usize {
        self.max_batch_records
    }

    #[must_use]
    pub const fn max_batch_bytes(self) -> u64 {
        self.max_batch_bytes
    }
}

#[async_trait]
pub trait RawSegmentArchive: Send + Sync {
    async fn archive_segment(
        &self,
        chain_id: &ChainId,
        segment: &CloseReceipt,
        config: RawSegmentArchiveConfig,
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
        config: RawSegmentArchiveConfig,
    ) -> Result<RawSegmentArchiveSummary, RawSegmentArchiveError> {
        let archive = Arc::clone(&self.archive);
        let chain_id = chain_id.clone();
        let segment = segment.clone();
        tokio::task::spawn_blocking(move || {
            archive_segment(archive.as_ref(), chain_id, &segment, config)
        })
        .await
        .map_err(|_| RawSegmentArchiveError::BlockingTask)?
    }
}

fn archive_segment(
    archive: &dyn RawObservationArchive,
    chain_id: ChainId,
    segment: &CloseReceipt,
    config: RawSegmentArchiveConfig,
) -> Result<RawSegmentArchiveSummary, RawSegmentArchiveError> {
    segment
        .verify_current()
        .map_err(RawSegmentArchiveError::Spool)?;
    let reader =
        SpoolReader::open(segment.segment_path()).map_err(RawSegmentArchiveError::Spool)?;
    let source_id = reader.header().source_id().clone();
    let source_version = reader.header().source_version().to_owned();
    let mut records = reader.stream().map_err(RawSegmentArchiveError::Spool)?;
    let mut batch = Vec::<SourceObservation>::with_capacity(config.max_batch_records());
    let mut batch_bytes = 0_u64;
    let mut batch_hour = None;
    let mut observation_count = 0_u64;
    let mut batch_count = 0_u64;
    loop {
        let record = match records
            .next_record()
            .map_err(RawSegmentArchiveError::Spool)?
        {
            SpoolRead::Record(record) => record,
            SpoolRead::EndOfFile => break,
            SpoolRead::IncompleteTail { record_offset } => {
                return Err(RawSegmentArchiveError::Spool(SpoolError::IncompleteTail {
                    record_offset,
                }));
            }
        };
        let observation = record
            .into_observation(
                source_id.clone(),
                source_version.clone(),
                config.max_payload_bytes(),
            )
            .map_err(|_| RawSegmentArchiveError::Observation)?;
        let hour = observation.received().wall_micros() / MICROS_PER_HOUR;
        let payload_bytes = u64::try_from(observation.payload().len())
            .map_err(|_| RawSegmentArchiveError::SizeOverflow)?;
        let next_batch_bytes = batch_bytes
            .checked_add(payload_bytes)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
        let must_flush = !batch.is_empty()
            && (batch_hour != Some(hour)
                || batch.len() >= config.max_batch_records()
                || next_batch_bytes > config.max_batch_bytes());
        if must_flush {
            archive_batch(
                archive,
                &chain_id,
                &source_id,
                segment,
                std::mem::take(&mut batch),
            )?;
            batch_count = batch_count
                .checked_add(1)
                .ok_or(RawSegmentArchiveError::SizeOverflow)?;
            batch_bytes = 0;
        }
        if batch.is_empty() {
            batch_hour = Some(hour);
        }
        batch_bytes = batch_bytes
            .checked_add(payload_bytes)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
        batch.push(observation);
        observation_count = observation_count
            .checked_add(1)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
    }
    if !batch.is_empty() {
        archive_batch(archive, &chain_id, &source_id, segment, batch)?;
        batch_count = batch_count
            .checked_add(1)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
    }
    if observation_count != segment.manifest().record_count() {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    segment
        .verify_current()
        .map_err(RawSegmentArchiveError::Spool)?;
    Ok(RawSegmentArchiveSummary {
        observation_count,
        batch_count,
    })
}

fn archive_batch(
    archive: &dyn RawObservationArchive,
    chain_id: &ChainId,
    source_id: &domain_types::SourceId,
    segment: &CloseReceipt,
    observations: Vec<SourceObservation>,
) -> Result<(), RawSegmentArchiveError> {
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
        || verified.object().chain_id() != chain_id
        || verified.object().source_id() != source_id
    {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    Ok(())
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
