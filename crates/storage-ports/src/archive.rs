use std::{collections::BTreeMap, path::PathBuf};

use canonical_events::BlockEnvelope;
use domain_types::{BlockHeight, BlockRange, ChainId, KnownTime, ManifestId, SourceId};
use hl_protocol::SourceObservation;

pub const ARCHIVE_MANIFEST_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-manifest/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveReceipt {
    receipt_id: String,
    manifest_id: ManifestId,
    block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
    object_sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
    durable_at: KnownTime,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionReceipt {
    manifest_id: ManifestId,
    block_range: BlockRange,
    input_object_count: u64,
    output_object_sha256: [u8; 32],
    row_count: u64,
    rolling_content_sha256: [u8; 32],
    completed_at: KnownTime,
}

impl CompactionReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        manifest_id: ManifestId,
        block_range: BlockRange,
        input_object_count: u64,
        output_object_sha256: [u8; 32],
        row_count: u64,
        rolling_content_sha256: [u8; 32],
        completed_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        if input_object_count < 2 {
            return Err(ArchiveError::InvalidInput(
                "compaction requires at least two input objects",
            ));
        }
        Ok(Self {
            manifest_id,
            block_range,
            input_object_count,
            output_object_sha256,
            row_count,
            rolling_content_sha256,
            completed_at,
        })
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn block_range(&self) -> BlockRange {
        self.block_range
    }

    #[must_use]
    pub const fn input_object_count(&self) -> u64 {
        self.input_object_count
    }

    #[must_use]
    pub const fn output_object_sha256(&self) -> [u8; 32] {
        self.output_object_sha256
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn rolling_content_sha256(&self) -> [u8; 32] {
        self.rolling_content_sha256
    }

    #[must_use]
    pub const fn completed_at(&self) -> KnownTime {
        self.completed_at
    }
}

impl ArchiveReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        receipt_id: impl Into<String>,
        manifest_id: ManifestId,
        block_height: BlockHeight,
        canonical_block_hash: [u8; 32],
        object_sha256: [u8; 32],
        manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
        durable_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        let receipt_id = receipt_id.into();
        validate_identity(&receipt_id, "receipt ID")?;
        Ok(Self {
            receipt_id,
            manifest_id,
            block_height,
            canonical_block_hash,
            object_sha256,
            manifest_sha256,
            schema_fingerprint,
            durable_at,
        })
    }

    #[must_use]
    pub fn receipt_id(&self) -> &str {
        &self.receipt_id
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }

    #[must_use]
    pub const fn object_sha256(&self) -> [u8; 32] {
        self.object_sha256
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> [u8; 32] {
        self.schema_fingerprint
    }

    #[must_use]
    pub const fn durable_at(&self) -> KnownTime {
        self.durable_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawObservationReceipt {
    receipt_id: String,
    manifest_id: ManifestId,
    chain_id: ChainId,
    source_id: SourceId,
    cursor_epoch: String,
    start_offset: u64,
    end_offset: u64,
    spool_manifest_blake3: [u8; 32],
    spool_segment_blake3: [u8; 32],
    rolling_content_sha256: [u8; 32],
    object_sha256: [u8; 32],
    manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
    durable_at: KnownTime,
}

impl RawObservationReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        receipt_id: impl Into<String>,
        manifest_id: ManifestId,
        chain_id: ChainId,
        source_id: SourceId,
        cursor_epoch: impl Into<String>,
        start_offset: u64,
        end_offset: u64,
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
        rolling_content_sha256: [u8; 32],
        object_sha256: [u8; 32],
        manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
        durable_at: KnownTime,
    ) -> Result<Self, ArchiveError> {
        let receipt_id = receipt_id.into();
        validate_identity(&receipt_id, "receipt ID")?;
        let cursor_epoch = cursor_epoch.into();
        validate_identity(&cursor_epoch, "cursor epoch")?;
        if start_offset > end_offset {
            return Err(ArchiveError::InvalidInput(
                "raw observation receipt cursor range",
            ));
        }
        Ok(Self {
            receipt_id,
            manifest_id,
            chain_id,
            source_id,
            cursor_epoch,
            start_offset,
            end_offset,
            spool_manifest_blake3,
            spool_segment_blake3,
            rolling_content_sha256,
            object_sha256,
            manifest_sha256,
            schema_fingerprint,
            durable_at,
        })
    }

    #[must_use]
    pub fn receipt_id(&self) -> &str {
        &self.receipt_id
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub fn cursor_epoch(&self) -> &str {
        &self.cursor_epoch
    }

    #[must_use]
    pub const fn start_offset(&self) -> u64 {
        self.start_offset
    }

    #[must_use]
    pub const fn end_offset(&self) -> u64 {
        self.end_offset
    }

    #[must_use]
    pub const fn spool_manifest_blake3(&self) -> [u8; 32] {
        self.spool_manifest_blake3
    }

    #[must_use]
    pub const fn spool_segment_blake3(&self) -> [u8; 32] {
        self.spool_segment_blake3
    }

    #[must_use]
    pub const fn rolling_content_sha256(&self) -> [u8; 32] {
        self.rolling_content_sha256
    }

    #[must_use]
    pub const fn object_sha256(&self) -> [u8; 32] {
        self.object_sha256
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> [u8; 32] {
        self.schema_fingerprint
    }

    #[must_use]
    pub const fn durable_at(&self) -> KnownTime {
        self.durable_at
    }
}

#[derive(Debug, Clone)]
pub struct RawObservationBatch {
    chain_id: ChainId,
    observations: Vec<SourceObservation>,
    spool_manifest_blake3: [u8; 32],
    spool_segment_blake3: [u8; 32],
}

impl RawObservationBatch {
    pub fn try_new(
        chain_id: ChainId,
        observations: Vec<SourceObservation>,
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
    ) -> Result<Self, ArchiveError> {
        let first = observations
            .first()
            .ok_or(ArchiveError::InvalidInput("raw observation batch is empty"))?;
        let mut previous = first.cursor().offset();
        for (index, observation) in observations.iter().enumerate() {
            if observation.source_id() != first.source_id()
                || observation.source_version() != first.source_version()
                || observation.observation_class() != first.observation_class()
                || observation.cursor().epoch() != first.cursor().epoch()
                || observation.parser_schema_version() != first.parser_schema_version()
            {
                return Err(ArchiveError::InvalidInput(
                    "raw observation batch metadata is inconsistent",
                ));
            }
            if index != 0 {
                let expected = previous.checked_add(1).ok_or(ArchiveError::InvalidInput(
                    "raw observation cursor overflows",
                ))?;
                if observation.cursor().offset() != expected {
                    return Err(ArchiveError::InvalidInput(
                        "raw observation cursors are not contiguous",
                    ));
                }
                previous = observation.cursor().offset();
            }
        }
        Ok(Self {
            chain_id,
            observations,
            spool_manifest_blake3,
            spool_segment_blake3,
        })
    }

    #[must_use]
    pub fn observations(&self) -> &[SourceObservation] {
        &self.observations
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn spool_manifest_blake3(&self) -> [u8; 32] {
        self.spool_manifest_blake3
    }

    #[must_use]
    pub const fn spool_segment_blake3(&self) -> [u8; 32] {
        self.spool_segment_blake3
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawObservationRange {
    epoch: String,
    start_offset: u64,
    end_offset: u64,
}

impl RawObservationRange {
    pub fn try_new(
        epoch: impl Into<String>,
        start_offset: u64,
        end_offset: u64,
    ) -> Result<Self, ArchiveError> {
        let epoch = epoch.into();
        validate_identity(&epoch, "raw observation cursor epoch")?;
        if start_offset > end_offset {
            return Err(ArchiveError::InvalidInput("raw observation cursor range"));
        }
        Ok(Self {
            epoch,
            start_offset,
            end_offset,
        })
    }

    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    #[must_use]
    pub const fn start_offset(&self) -> u64 {
        self.start_offset
    }

    #[must_use]
    pub const fn end_offset(&self) -> u64 {
        self.end_offset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveObject {
    relative_path: PathBuf,
    sha256: [u8; 32],
    size_bytes: u64,
    row_count: u64,
    chain_id: ChainId,
    source_id: SourceId,
    cursor_range: RawObservationRange,
}

impl RawArchiveObject {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
        chain_id: ChainId,
        source_id: SourceId,
        cursor_range: RawObservationRange,
    ) -> Result<Self, ArchiveError> {
        validate_relative_path(&relative_path)?;
        if size_bytes == 0 || row_count == 0 {
            return Err(ArchiveError::InvalidInput(
                "raw archive object size or row count",
            ));
        }
        Ok(Self {
            relative_path,
            sha256,
            size_bytes,
            row_count,
            chain_id,
            source_id,
            cursor_range,
        })
    }

    #[must_use]
    pub fn relative_path(&self) -> &std::path::Path {
        &self.relative_path
    }

    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn cursor_range(&self) -> &RawObservationRange {
        &self.cursor_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedRawManifest {
    manifest_id: ManifestId,
    manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
    rolling_content_sha256: [u8; 32],
    spool_manifest_blake3: [u8; 32],
    spool_segment_blake3: [u8; 32],
    object: RawArchiveObject,
}

impl VerifiedRawManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        manifest_id: ManifestId,
        manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
        rolling_content_sha256: [u8; 32],
        spool_manifest_blake3: [u8; 32],
        spool_segment_blake3: [u8; 32],
        object: RawArchiveObject,
    ) -> Self {
        Self {
            manifest_id,
            manifest_sha256,
            schema_fingerprint,
            rolling_content_sha256,
            spool_manifest_blake3,
            spool_segment_blake3,
            object,
        }
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> [u8; 32] {
        self.schema_fingerprint
    }

    #[must_use]
    pub const fn rolling_content_sha256(&self) -> [u8; 32] {
        self.rolling_content_sha256
    }

    #[must_use]
    pub const fn spool_manifest_blake3(&self) -> [u8; 32] {
        self.spool_manifest_blake3
    }

    #[must_use]
    pub const fn spool_segment_blake3(&self) -> [u8; 32] {
        self.spool_segment_blake3
    }

    #[must_use]
    pub const fn object(&self) -> &RawArchiveObject {
        &self.object
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceWatermark {
    source_id: SourceId,
    epoch: String,
    offset: u64,
}

impl SourceWatermark {
    pub fn try_new(
        source_id: SourceId,
        epoch: impl Into<String>,
        offset: u64,
    ) -> Result<Self, ArchiveError> {
        let epoch = epoch.into();
        validate_identity(&epoch, "source watermark epoch")?;
        Ok(Self {
            source_id,
            epoch,
            offset,
        })
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveObject {
    relative_path: PathBuf,
    sha256: [u8; 32],
    size_bytes: u64,
    row_count: u64,
    block_range: BlockRange,
}

impl ArchiveObject {
    pub fn try_new(
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
        block_range: BlockRange,
    ) -> Result<Self, ArchiveError> {
        validate_relative_path(&relative_path)?;
        if size_bytes == 0 {
            return Err(ArchiveError::InvalidInput(
                "archive object size must be nonzero",
            ));
        }
        Ok(Self {
            relative_path,
            sha256,
            size_bytes,
            row_count,
            block_range,
        })
    }

    #[must_use]
    pub fn relative_path(&self) -> &std::path::Path {
        &self.relative_path
    }

    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn block_range(&self) -> BlockRange {
        self.block_range
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedManifest {
    manifest_id: ManifestId,
    chain_id: ChainId,
    object_count: u64,
    row_count: u64,
    block_range: BlockRange,
    manifest_sha256: [u8; 32],
    previous_manifest_sha256: Option<[u8; 32]>,
    schema_fingerprints: BTreeMap<String, [u8; 32]>,
    source_watermarks: Vec<SourceWatermark>,
    objects: Vec<ArchiveObject>,
}

impl VerifiedManifest {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        manifest_id: ManifestId,
        chain_id: ChainId,
        row_count: u64,
        block_range: BlockRange,
        manifest_sha256: [u8; 32],
        previous_manifest_sha256: Option<[u8; 32]>,
        schema_fingerprints: BTreeMap<String, [u8; 32]>,
        source_watermarks: Vec<SourceWatermark>,
        objects: Vec<ArchiveObject>,
    ) -> Result<Self, ArchiveError> {
        if schema_fingerprints.is_empty() {
            return Err(ArchiveError::InvalidInput(
                "verified manifest requires a schema fingerprint",
            ));
        }
        let object_count = u64::try_from(objects.len())
            .map_err(|_| ArchiveError::InvalidInput("archive object count exceeds u64"))?;
        Ok(Self {
            manifest_id,
            chain_id,
            object_count,
            row_count,
            block_range,
            manifest_sha256,
            previous_manifest_sha256,
            schema_fingerprints,
            source_watermarks,
            objects,
        })
    }

    #[must_use]
    pub const fn manifest_id(&self) -> &ManifestId {
        &self.manifest_id
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn object_count(&self) -> u64 {
        self.object_count
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }

    #[must_use]
    pub const fn block_range(&self) -> BlockRange {
        self.block_range
    }

    #[must_use]
    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    #[must_use]
    pub const fn previous_manifest_sha256(&self) -> Option<[u8; 32]> {
        self.previous_manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprints(&self) -> &BTreeMap<String, [u8; 32]> {
        &self.schema_fingerprints
    }

    #[must_use]
    pub fn source_watermarks(&self) -> &[SourceWatermark] {
        &self.source_watermarks
    }

    #[must_use]
    pub fn objects(&self) -> &[ArchiveObject] {
        &self.objects
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("archive I/O failed while {0}")]
    Io(&'static str),
    #[error("manifest verification failed: {0}")]
    ManifestVerification(&'static str),
    #[error("archive range is unavailable")]
    RangeUnavailable,
    #[error("archive schema fingerprint mismatch")]
    SchemaMismatch,
    #[error("archive object is corrupt: {0}")]
    CorruptObject(String),
    #[error("archive contains conflicting canonical content at block {0:?}")]
    ConflictingBlock(BlockHeight),
    #[error(
        "archive contains a conflicting raw range for source {source_id} epoch {epoch} offsets {start}..={end}"
    )]
    ConflictingRawRange {
        source_id: SourceId,
        epoch: String,
        start: u64,
        end: u64,
    },
    #[error("archive input is invalid: {0}")]
    InvalidInput(&'static str),
    #[error("archive path is unsafe")]
    UnsafePath,
    #[error("archive writer is already active")]
    WriterBusy,
    #[error("canonical archive codec failed: {0}")]
    Codec(String),
}

impl ArchiveError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Io(_) => "archive.io",
            Self::ManifestVerification(_) => "archive.manifest_verification",
            Self::RangeUnavailable => "archive.range_unavailable",
            Self::SchemaMismatch => "archive.schema_mismatch",
            Self::CorruptObject(_) => "archive.corrupt_object",
            Self::ConflictingBlock(_) => "archive.conflicting_block",
            Self::ConflictingRawRange { .. } => "archive.conflicting_raw_range",
            Self::InvalidInput(_) => "archive.invalid_input",
            Self::UnsafePath => "archive.unsafe_path",
            Self::WriterBusy => "archive.writer_busy",
            Self::Codec(_) => "archive.codec",
        }
    }
}

pub type BlockIterator = Box<dyn Iterator<Item = Result<BlockEnvelope, ArchiveError>> + Send>;
pub type RawObservationIterator =
    Box<dyn Iterator<Item = Result<SourceObservation, ArchiveError>> + Send>;

pub trait CanonicalArchive: Send + Sync {
    fn append_block(&self, block: &BlockEnvelope) -> Result<ArchiveReceipt, ArchiveError>;

    fn read_range(&self, chain: &ChainId, range: BlockRange)
    -> Result<BlockIterator, ArchiveError>;

    fn verify_manifest(&self, manifest: &ManifestId) -> Result<VerifiedManifest, ArchiveError>;

    fn read_manifest_blocks(&self, manifest: &ManifestId) -> Result<BlockIterator, ArchiveError>;
}

pub trait CanonicalArchiveMaintenance: Send + Sync {
    fn compact_range(
        &self,
        chain: &ChainId,
        range: BlockRange,
    ) -> Result<CompactionReceipt, ArchiveError>;
}

pub trait RawObservationArchive: Send + Sync {
    fn append_batch(
        &self,
        batch: &RawObservationBatch,
    ) -> Result<RawObservationReceipt, ArchiveError>;

    fn read_observations(
        &self,
        chain: &ChainId,
        source: &SourceId,
        range: RawObservationRange,
    ) -> Result<RawObservationIterator, ArchiveError>;

    fn verify_raw_manifest(
        &self,
        manifest: &ManifestId,
    ) -> Result<VerifiedRawManifest, ArchiveError>;
}

fn validate_identity(value: &str, label: &'static str) -> Result<(), ArchiveError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > 256
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ArchiveError::InvalidInput(label));
    }
    Ok(())
}

fn validate_relative_path(path: &std::path::Path) -> Result<(), ArchiveError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(ArchiveError::UnsafePath);
    }
    for component in path.components() {
        match component {
            std::path::Component::Normal(_) => {}
            _ => return Err(ArchiveError::UnsafePath),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use domain_types::{BlockHeight, BlockRange};

    use super::{ArchiveError, ArchiveObject};

    #[test]
    fn archive_object_rejects_unsafe_paths_and_empty_files() {
        let range = BlockRange::new(BlockHeight::new(1), BlockHeight::new(1)).expect("range");

        assert!(matches!(
            ArchiveObject::try_new(PathBuf::from("../escape.parquet"), [1; 32], 1, 1, range),
            Err(ArchiveError::UnsafePath)
        ));
        assert!(matches!(
            ArchiveObject::try_new(PathBuf::from("safe.parquet"), [1; 32], 0, 1, range),
            Err(ArchiveError::InvalidInput(_))
        ));
    }
}
