use std::sync::Arc;

use async_trait::async_trait;
use domain_types::{ChainId, ManifestId, SourceId};
use hl_protocol::{ObservationClass, SourceObservation};
use storage_ports::{
    ArchiveError, CursorPolicy, LocalRecordSequence, RawObservationArchive, RawObservationBatch,
};

use crate::spool::{CloseReceipt, SpoolError, SpoolRead, SpoolReader};

const MICROS_PER_HOUR: i64 = 3_600_000_000;
const MAX_ARCHIVE_BATCH_RECORDS: usize = 1_000_000;
const MAX_ARCHIVE_BATCH_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RawBatchIdentity {
    source_id: SourceId,
    source_version: String,
    observation_class: ObservationClass,
    cursor_epoch: String,
    parser_schema_version: String,
    received_hour: i64,
}

impl RawBatchIdentity {
    fn from_observation(observation: &SourceObservation) -> Self {
        Self {
            source_id: observation.source_id().clone(),
            source_version: observation.source_version().to_owned(),
            observation_class: observation.observation_class(),
            cursor_epoch: observation.cursor().epoch().to_owned(),
            parser_schema_version: observation.parser_schema_version().to_owned(),
            received_hour: observation.received().wall_micros() / MICROS_PER_HOUR,
        }
    }
}

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

    async fn verify_archived_segment(
        &self,
        verification: &RawSegmentArchiveVerification,
    ) -> Result<(), RawSegmentArchiveError>;

    async fn contains_archived_epoch(
        &self,
        chain_id: &ChainId,
        source_id: &SourceId,
        cursor_epoch: &str,
    ) -> Result<bool, RawSegmentArchiveError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSpoolArchiveEvidence {
    manifest_blake3: [u8; 32],
    segment_blake3: [u8; 32],
    first_local_sequence: LocalRecordSequence,
    last_cursor: hl_protocol::SourceCursor,
    last_local_sequence: LocalRecordSequence,
    record_count: u64,
}

impl RawSpoolArchiveEvidence {
    pub fn try_new(
        manifest_blake3: [u8; 32],
        segment_blake3: [u8; 32],
        first_local_sequence: LocalRecordSequence,
        last_cursor: hl_protocol::SourceCursor,
        last_local_sequence: LocalRecordSequence,
        record_count: u64,
    ) -> Result<Self, RawSegmentArchiveError> {
        if record_count == 0
            || first_local_sequence.get().checked_add(record_count - 1)
                != Some(last_local_sequence.get())
        {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        Ok(Self {
            manifest_blake3,
            segment_blake3,
            first_local_sequence,
            last_cursor,
            last_local_sequence,
            record_count,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSegmentArchiveVerification {
    chain_id: ChainId,
    source_id: SourceId,
    spool: RawSpoolArchiveEvidence,
    manifest_ids: Vec<ManifestId>,
}

impl RawSegmentArchiveVerification {
    #[must_use]
    pub fn new(
        chain_id: ChainId,
        source_id: SourceId,
        spool: RawSpoolArchiveEvidence,
        manifest_ids: Vec<ManifestId>,
    ) -> Self {
        Self {
            chain_id,
            source_id,
            spool,
            manifest_ids,
        }
    }
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

    async fn verify_archived_segment(
        &self,
        verification: &RawSegmentArchiveVerification,
    ) -> Result<(), RawSegmentArchiveError> {
        let archive = Arc::clone(&self.archive);
        let verification = verification.clone();
        tokio::task::spawn_blocking(move || {
            verify_archived_segment(archive.as_ref(), &verification)
        })
        .await
        .map_err(|_| RawSegmentArchiveError::BlockingTask)?
    }

    async fn contains_archived_epoch(
        &self,
        chain_id: &ChainId,
        source_id: &SourceId,
        cursor_epoch: &str,
    ) -> Result<bool, RawSegmentArchiveError> {
        let archive = Arc::clone(&self.archive);
        let chain_id = chain_id.clone();
        let source_id = source_id.clone();
        let cursor_epoch = cursor_epoch.to_owned();
        tokio::task::spawn_blocking(move || {
            archive
                .contains_raw_cursor_epoch(&chain_id, &source_id, &cursor_epoch)
                .map_err(RawSegmentArchiveError::Archive)
        })
        .await
        .map_err(|_| RawSegmentArchiveError::BlockingTask)?
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_archived_segment(
    archive: &dyn RawObservationArchive,
    verification: &RawSegmentArchiveVerification,
) -> Result<(), RawSegmentArchiveError> {
    if verification.manifest_ids.is_empty() {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    let mut previous_last_sequence = None;
    let mut verified_record_count = 0_u64;
    let mut verified_last_cursor = None;
    let mut verified_last_sequence = None;
    for manifest_id in &verification.manifest_ids {
        let verified = archive
            .verify_raw_manifest(manifest_id)
            .map_err(RawSegmentArchiveError::Archive)?;
        if verified.object().chain_id() != &verification.chain_id
            || verified.object().source_id() != &verification.source_id
            || verified.spool_manifest_blake3() != verification.spool.manifest_blake3
            || verified.spool_segment_blake3() != verification.spool.segment_blake3
            || verified.cursor_policy() != CursorPolicy::MonotonicByteOffset
        {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        let range = verified
            .local_sequence_range()
            .ok_or(RawSegmentArchiveError::VerificationMismatch)?;
        if previous_last_sequence.is_none()
            && range.start() != verification.spool.first_local_sequence
        {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        if previous_last_sequence.is_some_and(|previous: LocalRecordSequence| {
            previous.checked_next().ok() != Some(range.start())
        }) {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        previous_last_sequence = Some(range.end());
        verified_record_count = verified_record_count
            .checked_add(
                range
                    .end()
                    .get()
                    .checked_sub(range.start().get())
                    .and_then(|delta| delta.checked_add(1))
                    .ok_or(RawSegmentArchiveError::VerificationMismatch)?,
            )
            .ok_or(RawSegmentArchiveError::VerificationMismatch)?;
        verified_last_sequence = Some(range.end());
        verified_last_cursor = Some(
            hl_protocol::SourceCursor::new(
                verified.object().cursor_range().epoch().to_owned(),
                verified.object().cursor_range().end_offset(),
            )
            .map_err(|_| RawSegmentArchiveError::VerificationMismatch)?,
        );
    }
    if verified_last_sequence != Some(verification.spool.last_local_sequence)
        || verified_last_cursor.as_ref() != Some(&verification.spool.last_cursor)
        || verified_record_count != verification.spool.record_count
    {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    Ok(())
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
    let mut batch_identity = None;
    let cursor_policy = segment
        .manifest()
        .cursor_policy()
        .unwrap_or(CursorPolicy::ContiguousNativeOffset);
    let first_local_sequence = match cursor_policy {
        CursorPolicy::ContiguousNativeOffset => None,
        CursorPolicy::MonotonicByteOffset => Some(
            segment
                .manifest()
                .first_local_sequence()
                .ok_or(RawSegmentArchiveError::VerificationMismatch)?,
        ),
    };
    let mut last_local_sequence: Option<LocalRecordSequence> = None;
    let mut batch_first_local_sequence = None;
    let mut observation_count = 0_u64;
    let mut batch_count = 0_u64;
    let mut manifest_ids = Vec::new();
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
        let identity = RawBatchIdentity::from_observation(&observation);
        let payload_bytes = u64::try_from(observation.payload().len())
            .map_err(|_| RawSegmentArchiveError::SizeOverflow)?;
        let next_batch_bytes = batch_bytes
            .checked_add(payload_bytes)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
        let must_flush = !batch.is_empty()
            && (batch_identity.as_ref() != Some(&identity)
                || batch.len() >= config.max_batch_records()
                || next_batch_bytes > config.max_batch_bytes());
        if must_flush {
            manifest_ids.push(archive_batch(
                archive,
                &chain_id,
                &source_id,
                segment,
                std::mem::take(&mut batch),
                batch_first_local_sequence,
            )?);
            batch_count = batch_count
                .checked_add(1)
                .ok_or(RawSegmentArchiveError::SizeOverflow)?;
            batch_bytes = 0;
        }
        let record_local_sequence = match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => None,
            CursorPolicy::MonotonicByteOffset => Some(match last_local_sequence {
                Some(previous) => previous
                    .checked_next()
                    .map_err(RawSegmentArchiveError::Archive)?,
                None => first_local_sequence.ok_or(RawSegmentArchiveError::VerificationMismatch)?,
            }),
        };
        if batch.is_empty() {
            batch_identity = Some(identity);
            batch_first_local_sequence = record_local_sequence;
        }
        batch_bytes = batch_bytes
            .checked_add(payload_bytes)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
        batch.push(observation);
        last_local_sequence = record_local_sequence;
        observation_count = observation_count
            .checked_add(1)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
    }
    if !batch.is_empty() {
        manifest_ids.push(archive_batch(
            archive,
            &chain_id,
            &source_id,
            segment,
            batch,
            batch_first_local_sequence,
        )?);
        batch_count = batch_count
            .checked_add(1)
            .ok_or(RawSegmentArchiveError::SizeOverflow)?;
    }
    if observation_count != segment.manifest().record_count() {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    match cursor_policy {
        CursorPolicy::MonotonicByteOffset => {
            if last_local_sequence != segment.manifest().last_local_sequence() {
                return Err(RawSegmentArchiveError::VerificationMismatch);
            }
        }
        CursorPolicy::ContiguousNativeOffset => {}
    }
    segment
        .verify_current()
        .map_err(RawSegmentArchiveError::Spool)?;
    Ok(RawSegmentArchiveSummary {
        observation_count,
        batch_count,
        manifest_ids,
    })
}

fn archive_batch(
    archive: &dyn RawObservationArchive,
    chain_id: &ChainId,
    source_id: &domain_types::SourceId,
    segment: &CloseReceipt,
    observations: Vec<SourceObservation>,
    first_local_sequence: Option<LocalRecordSequence>,
) -> Result<ManifestId, RawSegmentArchiveError> {
    let batch = match segment
        .manifest()
        .cursor_policy()
        .unwrap_or(CursorPolicy::ContiguousNativeOffset)
    {
        CursorPolicy::ContiguousNativeOffset => RawObservationBatch::try_new(
            chain_id.clone(),
            observations,
            segment.manifest_hash(),
            segment.manifest().segment_blake3(),
        ),
        CursorPolicy::MonotonicByteOffset => RawObservationBatch::try_new_byte_offsets(
            chain_id.clone(),
            observations,
            segment.manifest_hash(),
            segment.manifest().segment_blake3(),
            first_local_sequence.ok_or(RawSegmentArchiveError::VerificationMismatch)?,
        ),
    }
    .map_err(RawSegmentArchiveError::Archive)?;
    let expected_policy = batch.cursor_policy();
    let expected_sequence_range = batch.local_sequence_range();
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
        || verified.cursor_policy() != expected_policy
        || verified.local_sequence_range() != expected_sequence_range
    {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    Ok(receipt.manifest_id().clone())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSegmentArchiveSummary {
    observation_count: u64,
    batch_count: u64,
    manifest_ids: Vec<ManifestId>,
}

impl RawSegmentArchiveSummary {
    #[must_use]
    pub const fn observation_count(&self) -> u64 {
        self.observation_count
    }

    #[must_use]
    pub const fn batch_count(&self) -> u64 {
        self.batch_count
    }

    #[must_use]
    pub fn manifest_ids(&self) -> &[ManifestId] {
        &self.manifest_ids
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
