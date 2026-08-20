use std::sync::Arc;

use async_trait::async_trait;
use canonical_archive::{LocalParquetArchive, RawArchiveCheckpoint, RawV3Archive};
use domain_types::{ChainId, ManifestId, SourceId};
use hl_protocol::{ObservationClass, SourceObservation};
use storage_ports::{
    ArchiveError, CursorPolicy, LocalRecordSequence, LocalRecordSequenceRange,
    RawArchiveCheckpointEntriesV2, RawArchiveCheckpointEntryV2, RawObservationArchive,
    RawObservationBatch, RawObservationIterator, RawObservationRange,
    SequencedRawObservationIterator, VerifiedRawManifest,
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

    async fn load_checkpoint_entries(
        &self,
        chain_id: &ChainId,
        source_id: &SourceId,
    ) -> Result<Option<RawArchiveCheckpointEntriesV2>, RawSegmentArchiveError>;
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
    checkpoint_entries: Option<RawArchiveCheckpointEntriesV2>,
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
            checkpoint_entries: None,
        }
    }

    #[must_use]
    pub fn with_checkpoint_entries(mut self, entries: RawArchiveCheckpointEntriesV2) -> Self {
        self.checkpoint_entries = Some(entries);
        self
    }
}

#[derive(Clone)]
pub(crate) struct CaptureRawObservationArchive {
    legacy: Arc<LocalParquetArchive>,
    v3: Option<Arc<RawV3Archive>>,
}

impl std::fmt::Debug for CaptureRawObservationArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CaptureRawObservationArchive")
            .field("v3_enabled", &self.v3.is_some())
            .finish_non_exhaustive()
    }
}

impl CaptureRawObservationArchive {
    pub(crate) fn new(legacy: Arc<LocalParquetArchive>, v3: Option<Arc<RawV3Archive>>) -> Self {
        Self { legacy, v3 }
    }

    fn byte_offset_archive(&self) -> &dyn RawObservationArchive {
        self.v3
            .as_ref()
            .map_or(self.legacy.as_ref() as &dyn RawObservationArchive, |v3| {
                v3.as_ref() as &dyn RawObservationArchive
            })
    }
}

impl RawObservationArchive for CaptureRawObservationArchive {
    fn append_batch(
        &self,
        batch: &RawObservationBatch,
    ) -> Result<storage_ports::RawObservationReceipt, ArchiveError> {
        match batch.cursor_policy() {
            CursorPolicy::ContiguousNativeOffset => self.legacy.append_batch(batch),
            CursorPolicy::MonotonicByteOffset => self.byte_offset_archive().append_batch(batch),
        }
    }

    fn read_observations(
        &self,
        chain: &ChainId,
        source: &SourceId,
        range: RawObservationRange,
    ) -> Result<RawObservationIterator, ArchiveError> {
        match self.v3.as_ref() {
            Some(v3) => match v3.read_observations(chain, source, range.clone()) {
                Ok(iterator) => Ok(iterator),
                Err(ArchiveError::RangeUnavailable) => {
                    self.legacy.read_observations(chain, source, range)
                }
                Err(error) => Err(error),
            },
            None => self.legacy.read_observations(chain, source, range),
        }
    }

    fn read_observations_by_sequence(
        &self,
        chain: &ChainId,
        source: &SourceId,
        range: LocalRecordSequenceRange,
    ) -> Result<SequencedRawObservationIterator, ArchiveError> {
        self.byte_offset_archive()
            .read_observations_by_sequence(chain, source, range)
    }

    fn verify_raw_manifest(
        &self,
        manifest: &ManifestId,
    ) -> Result<VerifiedRawManifest, ArchiveError> {
        match self.v3.as_ref() {
            Some(v3) => match v3.verify_raw_manifest(manifest) {
                Ok(verified) => Ok(verified),
                Err(ArchiveError::ReceiptIndexRebuildRequired) => {
                    self.legacy.verify_raw_manifest(manifest)
                }
                Err(error) => Err(error),
            },
            None => self.legacy.verify_raw_manifest(manifest),
        }
    }

    fn contains_raw_cursor_epoch(
        &self,
        chain: &ChainId,
        source: &SourceId,
        cursor_epoch: &str,
    ) -> Result<bool, ArchiveError> {
        if self
            .byte_offset_archive()
            .contains_raw_cursor_epoch(chain, source, cursor_epoch)?
        {
            return Ok(true);
        }
        if self.v3.is_some() {
            self.legacy
                .contains_raw_cursor_epoch(chain, source, cursor_epoch)
        } else {
            Ok(false)
        }
    }
}

#[derive(Clone)]
pub struct BlockingRawSegmentArchive {
    archive: Arc<dyn RawObservationArchive>,
    v3: Option<Arc<RawV3Archive>>,
}

impl std::fmt::Debug for BlockingRawSegmentArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("BlockingRawSegmentArchive")
            .field("v3_checkpoint", &self.v3.is_some())
            .finish_non_exhaustive()
    }
}

impl BlockingRawSegmentArchive {
    #[must_use]
    pub fn new(archive: Arc<dyn RawObservationArchive>) -> Self {
        Self { archive, v3: None }
    }

    #[must_use]
    pub fn with_v3(archive: Arc<dyn RawObservationArchive>, v3: Arc<RawV3Archive>) -> Self {
        Self {
            archive,
            v3: Some(v3),
        }
    }

    #[must_use]
    pub fn from_v3(v3: Arc<RawV3Archive>) -> Self {
        Self {
            archive: Arc::clone(&v3) as Arc<dyn RawObservationArchive>,
            v3: Some(v3),
        }
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
        let v3 = self.v3.clone();
        let chain_id = chain_id.clone();
        let segment = segment.clone();
        tokio::task::spawn_blocking(move || {
            archive_segment(archive.as_ref(), v3.as_deref(), chain_id, &segment, config)
        })
        .await
        .map_err(|_| RawSegmentArchiveError::BlockingTask)?
    }

    async fn verify_archived_segment(
        &self,
        verification: &RawSegmentArchiveVerification,
    ) -> Result<(), RawSegmentArchiveError> {
        let archive = Arc::clone(&self.archive);
        let v3 = self.v3.clone();
        let verification = verification.clone();
        tokio::task::spawn_blocking(move || {
            verify_archived_segment(archive.as_ref(), v3.as_deref(), &verification)
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

    async fn load_checkpoint_entries(
        &self,
        chain_id: &ChainId,
        source_id: &SourceId,
    ) -> Result<Option<RawArchiveCheckpointEntriesV2>, RawSegmentArchiveError> {
        let v3 = self.v3.clone();
        let chain_id = chain_id.clone();
        let source_id = source_id.clone();
        tokio::task::spawn_blocking(move || {
            load_v3_checkpoint_entries(v3.as_deref(), chain_id, source_id)
        })
        .await
        .map_err(|_| RawSegmentArchiveError::BlockingTask)?
    }
}

fn load_v3_checkpoint_entries(
    v3: Option<&RawV3Archive>,
    chain_id: ChainId,
    source_id: SourceId,
) -> Result<Option<RawArchiveCheckpointEntriesV2>, RawSegmentArchiveError> {
    let Some(v3) = v3 else {
        return Ok(None);
    };
    let has_v3_current = v3
        .maintenance_statistics(&chain_id, &source_id)
        .map_err(RawSegmentArchiveError::Archive)?
        .logical_manifest_count()
        > 0;
    match (
        has_v3_current,
        v3.load_checkpoint(&chain_id, &source_id)
            .map_err(RawSegmentArchiveError::Archive)?,
    ) {
        (false, None) => Ok(None),
        (true, Some(RawArchiveCheckpoint::V2(loaded))) => Ok(Some(loaded.entries().clone())),
        (false, Some(_)) | (true, None) | (true, Some(RawArchiveCheckpoint::V1(_))) => {
            Err(RawSegmentArchiveError::VerificationMismatch)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn verify_archived_segment(
    archive: &dyn RawObservationArchive,
    v3: Option<&RawV3Archive>,
    verification: &RawSegmentArchiveVerification,
) -> Result<(), RawSegmentArchiveError> {
    if verification.manifest_ids.is_empty() {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    if let Some(entries) = &verification.checkpoint_entries {
        if entries.entries().len() != verification.manifest_ids.len() {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        for (expected_id, entry) in verification.manifest_ids.iter().zip(entries.entries()) {
            if entry.manifest_id() != expected_id {
                return Err(RawSegmentArchiveError::VerificationMismatch);
            }
        }
    }
    let mut previous_last_sequence = None;
    let mut verified_record_count = 0_u64;
    let mut verified_last_cursor = None;
    let mut verified_last_sequence = None;
    for (index, manifest_id) in verification.manifest_ids.iter().enumerate() {
        let verified = match &verification.checkpoint_entries {
            Some(entries) => archive
                .verify_raw_manifest_at_sequence(
                    manifest_id,
                    entries.entries()[index].local_sequence_range(),
                )
                .map_err(RawSegmentArchiveError::Archive)?,
            None => archive
                .verify_raw_manifest(manifest_id)
                .map_err(RawSegmentArchiveError::Archive)?,
        };
        if let Some(entries) = &verification.checkpoint_entries
            && verified.manifest_sha256() != entries.entries()[index].manifest_sha256()
        {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        if verified.object().chain_id() != &verification.chain_id
            || verified.object().source_id() != &verification.source_id
            || verified.spool_manifest_blake3() != verification.spool.manifest_blake3
            || verified.spool_segment_blake3() != verification.spool.segment_blake3
        {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        match verified.cursor_policy() {
            CursorPolicy::MonotonicByteOffset => {}
            CursorPolicy::ContiguousNativeOffset => {
                return Err(RawSegmentArchiveError::VerificationMismatch);
            }
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
    if let Some(v3) = v3
        && let Some(entries) = &verification.checkpoint_entries
    {
        match v3
            .load_checkpoint(&verification.chain_id, &verification.source_id)
            .map_err(RawSegmentArchiveError::Archive)?
        {
            Some(RawArchiveCheckpoint::V2(loaded)) if loaded.entries() == entries => {}
            _ => return Err(RawSegmentArchiveError::VerificationMismatch),
        }
    }
    Ok(())
}

fn archive_segment(
    archive: &dyn RawObservationArchive,
    v3: Option<&RawV3Archive>,
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
    let mut checkpoint_entries = Vec::new();
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
            let receipt = archive_batch(
                archive,
                &chain_id,
                &source_id,
                segment,
                std::mem::take(&mut batch),
                batch_first_local_sequence,
            )?;
            push_archived_batch(&mut manifest_ids, &mut checkpoint_entries, receipt)?;
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
        let receipt = archive_batch(
            archive,
            &chain_id,
            &source_id,
            segment,
            batch,
            batch_first_local_sequence,
        )?;
        push_archived_batch(&mut manifest_ids, &mut checkpoint_entries, receipt)?;
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
    let summary = RawSegmentArchiveSummary {
        observation_count,
        batch_count,
        manifest_ids,
        checkpoint_entries,
    };
    if let Some(v3) = v3 {
        switch_v3_checkpoint_current(v3, &chain_id, &source_id, &summary)?;
    }
    Ok(summary)
}

fn switch_v3_checkpoint_current(
    archive: &RawV3Archive,
    chain_id: &ChainId,
    source_id: &SourceId,
    summary: &RawSegmentArchiveSummary,
) -> Result<(), RawSegmentArchiveError> {
    let Some(entries) = summary.checkpoint_entries()? else {
        return Ok(());
    };
    let expected_current = archive
        .load_checkpoint(chain_id, source_id)
        .map_err(RawSegmentArchiveError::Archive)?
        .map(|checkpoint| checkpoint.sha256());
    let target = archive
        .publish_checkpoint_v2(chain_id, source_id, entries)
        .map_err(RawSegmentArchiveError::Archive)?;
    archive
        .switch_checkpoint_current(chain_id, source_id, expected_current, target)
        .map_err(RawSegmentArchiveError::Archive)?;
    match archive
        .load_checkpoint(chain_id, source_id)
        .map_err(RawSegmentArchiveError::Archive)?
    {
        Some(RawArchiveCheckpoint::V2(loaded)) if loaded.sha256() == target => Ok(()),
        _ => Err(RawSegmentArchiveError::VerificationMismatch),
    }
}

fn archive_batch(
    archive: &dyn RawObservationArchive,
    chain_id: &ChainId,
    source_id: &domain_types::SourceId,
    segment: &CloseReceipt,
    observations: Vec<SourceObservation>,
    first_local_sequence: Option<LocalRecordSequence>,
) -> Result<ArchivedBatchReceipt, RawSegmentArchiveError> {
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
    Ok(ArchivedBatchReceipt {
        manifest_id: receipt.manifest_id().clone(),
        manifest_sha256: verified.manifest_sha256(),
        local_sequence_range: verified.local_sequence_range(),
    })
}

fn push_archived_batch(
    manifest_ids: &mut Vec<ManifestId>,
    checkpoint_entries: &mut Vec<RawArchiveCheckpointEntryV2>,
    receipt: ArchivedBatchReceipt,
) -> Result<(), RawSegmentArchiveError> {
    manifest_ids.push(receipt.manifest_id.clone());
    if let Some(range) = receipt.local_sequence_range {
        checkpoint_entries.push(RawArchiveCheckpointEntryV2::new(
            receipt.manifest_id,
            receipt.manifest_sha256,
            range,
        ));
    } else if !checkpoint_entries.is_empty() {
        return Err(RawSegmentArchiveError::VerificationMismatch);
    }
    Ok(())
}

struct ArchivedBatchReceipt {
    manifest_id: ManifestId,
    manifest_sha256: [u8; 32],
    local_sequence_range: Option<LocalRecordSequenceRange>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawSegmentArchiveSummary {
    observation_count: u64,
    batch_count: u64,
    manifest_ids: Vec<ManifestId>,
    checkpoint_entries: Vec<RawArchiveCheckpointEntryV2>,
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

    pub fn checkpoint_entries(
        &self,
    ) -> Result<Option<RawArchiveCheckpointEntriesV2>, RawSegmentArchiveError> {
        if self.checkpoint_entries.is_empty() {
            return Ok(None);
        }
        if self.checkpoint_entries.len() != self.manifest_ids.len() {
            return Err(RawSegmentArchiveError::VerificationMismatch);
        }
        RawArchiveCheckpointEntriesV2::try_new(self.checkpoint_entries.clone())
            .map(Some)
            .map_err(RawSegmentArchiveError::Archive)
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
