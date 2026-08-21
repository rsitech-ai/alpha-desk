use async_trait::async_trait;
use domain_types::{BlockHeight, ChainId, KnownTime, ManifestId};

const MAX_IDENTITY_BYTES: usize = 512;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedPublication {
    ordinal: u32,
    message_id: String,
    subject: String,
    publication_sha256: [u8; 32],
}

impl PlannedPublication {
    pub fn try_new(
        ordinal: u32,
        message_id: impl Into<String>,
        subject: impl Into<String>,
        publication_sha256: [u8; 32],
    ) -> Result<Self, ProgressError> {
        let message_id = message_id.into();
        validate_identity(&message_id)?;
        let subject = subject.into();
        validate_subject(&subject)?;
        Ok(Self {
            ordinal,
            message_id,
            subject,
            publication_sha256,
        })
    }

    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub const fn publication_sha256(&self) -> [u8; 32] {
        self.publication_sha256
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationAcknowledgement {
    ordinal: u32,
    message_id: String,
    subject: String,
    publication_sha256: [u8; 32],
    stream: String,
    stream_sequence: u64,
    duplicate: bool,
    acknowledged_at: KnownTime,
}

impl PublicationAcknowledgement {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        ordinal: u32,
        message_id: impl Into<String>,
        subject: impl Into<String>,
        publication_sha256: [u8; 32],
        stream: impl Into<String>,
        stream_sequence: u64,
        duplicate: bool,
        acknowledged_at: KnownTime,
    ) -> Result<Self, ProgressError> {
        let message_id = message_id.into();
        validate_identity(&message_id)?;
        let subject = subject.into();
        validate_subject(&subject)?;
        let stream = stream.into();
        validate_stream(&stream)?;
        if stream_sequence == 0 {
            return Err(ProgressError::InvalidInput("zero stream sequence"));
        }
        Ok(Self {
            ordinal,
            message_id,
            subject,
            publication_sha256,
            stream,
            stream_sequence,
            duplicate,
            acknowledged_at,
        })
    }

    #[must_use]
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub const fn publication_sha256(&self) -> [u8; 32] {
        self.publication_sha256
    }

    #[must_use]
    pub fn stream(&self) -> &str {
        &self.stream
    }

    #[must_use]
    pub const fn stream_sequence(&self) -> u64 {
        self.stream_sequence
    }

    #[must_use]
    pub const fn duplicate(&self) -> bool {
        self.duplicate
    }

    #[must_use]
    pub const fn acknowledged_at(&self) -> KnownTime {
        self.acknowledged_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchivedBlockPlan {
    chain_id: ChainId,
    block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
    archive_receipt_id: String,
    archive_manifest_id: ManifestId,
    archive_object_sha256: [u8; 32],
    archive_manifest_sha256: [u8; 32],
    archive_schema_fingerprint: [u8; 32],
    publications: Vec<PlannedPublication>,
    archived_at: KnownTime,
}

impl ArchivedBlockPlan {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        chain_id: ChainId,
        block_height: BlockHeight,
        canonical_block_hash: [u8; 32],
        archive_receipt_id: impl Into<String>,
        archive_manifest_id: ManifestId,
        archive_object_sha256: [u8; 32],
        archive_manifest_sha256: [u8; 32],
        archive_schema_fingerprint: [u8; 32],
        publications: Vec<PlannedPublication>,
        archived_at: KnownTime,
    ) -> Result<Self, ProgressError> {
        let archive_receipt_id = archive_receipt_id.into();
        validate_identity(&archive_receipt_id)?;
        if publications.is_empty() {
            return Err(ProgressError::InvalidInput("empty publication plan"));
        }
        for (expected, publication) in publications.iter().enumerate() {
            let expected = u32::try_from(expected)
                .map_err(|_| ProgressError::InvalidInput("too many publications"))?;
            if publication.ordinal() != expected {
                return Err(ProgressError::InvalidInput(
                    "non-contiguous publication ordinals",
                ));
            }
            if publications[..expected as usize]
                .iter()
                .any(|candidate| candidate.message_id() == publication.message_id())
            {
                return Err(ProgressError::InvalidInput(
                    "duplicate publication message ID",
                ));
            }
        }
        Ok(Self {
            chain_id,
            block_height,
            canonical_block_hash,
            archive_receipt_id,
            archive_manifest_id,
            archive_object_sha256,
            archive_manifest_sha256,
            archive_schema_fingerprint,
            publications,
            archived_at,
        })
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
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }

    #[must_use]
    pub fn archive_receipt_id(&self) -> &str {
        &self.archive_receipt_id
    }

    #[must_use]
    pub const fn archive_manifest_id(&self) -> &ManifestId {
        &self.archive_manifest_id
    }

    #[must_use]
    pub const fn archive_object_sha256(&self) -> [u8; 32] {
        self.archive_object_sha256
    }

    #[must_use]
    pub const fn archive_manifest_sha256(&self) -> [u8; 32] {
        self.archive_manifest_sha256
    }

    #[must_use]
    pub const fn archive_schema_fingerprint(&self) -> [u8; 32] {
        self.archive_schema_fingerprint
    }

    #[must_use]
    pub fn publications(&self) -> &[PlannedPublication] {
        &self.publications
    }

    #[must_use]
    pub const fn archived_at(&self) -> KnownTime {
        self.archived_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaptureCursor {
    chain_id: ChainId,
    committed_block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
    archive_receipt_id: String,
    archive_manifest_sha256: [u8; 32],
    cursor_version: u64,
    updated_at: KnownTime,
}

impl CaptureCursor {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        chain_id: ChainId,
        committed_block_height: BlockHeight,
        canonical_block_hash: [u8; 32],
        archive_receipt_id: impl Into<String>,
        archive_manifest_sha256: [u8; 32],
        cursor_version: u64,
        updated_at: KnownTime,
    ) -> Result<Self, ProgressError> {
        let archive_receipt_id = archive_receipt_id.into();
        validate_identity(&archive_receipt_id)?;
        if cursor_version == 0 {
            return Err(ProgressError::InvalidInput("zero cursor version"));
        }
        Ok(Self {
            chain_id,
            committed_block_height,
            canonical_block_hash,
            archive_receipt_id,
            archive_manifest_sha256,
            cursor_version,
            updated_at,
        })
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn committed_block_height(&self) -> BlockHeight {
        self.committed_block_height
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }

    #[must_use]
    pub fn archive_receipt_id(&self) -> &str {
        &self.archive_receipt_id
    }

    #[must_use]
    pub const fn archive_manifest_sha256(&self) -> [u8; 32] {
        self.archive_manifest_sha256
    }

    #[must_use]
    pub const fn cursor_version(&self) -> u64 {
        self.cursor_version
    }

    #[must_use]
    pub const fn updated_at(&self) -> KnownTime {
        self.updated_at
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressRecordDisposition {
    New,
    IdenticalDuplicate,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProgressError {
    #[error("capture progress input is invalid: {0}")]
    InvalidInput(&'static str),
    #[error("capture progress chain is not initialized")]
    ChainNotInitialized,
    #[error("capture progress chain initialization conflicts")]
    ConflictingInitialization,
    #[error("capture progress block is below the configured first height")]
    BelowFirstHeight,
    #[error("capture progress block conflicts with the durable binding")]
    ConflictingBlock,
    #[error("capture progress block is unknown")]
    UnknownBlock,
    #[error("capture progress acknowledgement does not match the publication plan")]
    AcknowledgementMismatch,
    #[error("capture progress acknowledgement conflicts with the durable receipt")]
    ConflictingAcknowledgement,
    #[error("capture progress publication set is incomplete")]
    PublicationIncomplete,
    #[error("capture progress cursor advance is not contiguous")]
    NonContiguousAdvance {
        expected: BlockHeight,
        actual: BlockHeight,
    },
    #[error("capture progress cursor overflow")]
    CursorOverflow,
    #[error("capture progress scan limit must be greater than zero")]
    InvalidLimit,
    #[error("capture progress capacity of {limit} blocks is exhausted")]
    CapacityExceeded { limit: usize },
    #[error("capture progress storage failed: {0}")]
    Storage(&'static str),
    #[error("capture progress object conflicts with the durable binding")]
    ConflictingObject,
}

impl ProgressError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidInput(_) => "capture_progress.invalid_input",
            Self::ChainNotInitialized => "capture_progress.chain_not_initialized",
            Self::ConflictingInitialization => "capture_progress.conflicting_initialization",
            Self::BelowFirstHeight => "capture_progress.below_first_height",
            Self::ConflictingBlock => "capture_progress.conflicting_block",
            Self::UnknownBlock => "capture_progress.unknown_block",
            Self::AcknowledgementMismatch => "capture_progress.acknowledgement_mismatch",
            Self::ConflictingAcknowledgement => "capture_progress.conflicting_acknowledgement",
            Self::PublicationIncomplete => "capture_progress.publication_incomplete",
            Self::NonContiguousAdvance { .. } => "capture_progress.non_contiguous_advance",
            Self::CursorOverflow => "capture_progress.cursor_overflow",
            Self::InvalidLimit => "capture_progress.invalid_limit",
            Self::CapacityExceeded { .. } => "capture_progress.capacity_exceeded",
            Self::Storage(_) => "capture_progress.storage",
            Self::ConflictingObject => "capture_progress.conflicting_object",
        }
    }
}

#[async_trait]
pub trait CaptureProgressStore: Send + Sync {
    async fn initialize_chain(
        &self,
        chain_id: &ChainId,
        first_block_height: BlockHeight,
    ) -> Result<ProgressRecordDisposition, ProgressError>;

    async fn record_archived(
        &self,
        plan: &ArchivedBlockPlan,
    ) -> Result<ProgressRecordDisposition, ProgressError>;

    async fn record_acknowledgement(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
        acknowledgement: &PublicationAcknowledgement,
    ) -> Result<ProgressRecordDisposition, ProgressError>;

    async fn advance_cursor(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<CaptureCursor, ProgressError>;

    async fn load_cursor(&self, chain_id: &ChainId)
    -> Result<Option<CaptureCursor>, ProgressError>;

    async fn next_expected_height(&self, chain_id: &ChainId) -> Result<BlockHeight, ProgressError>;

    async fn load_archived_block(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Option<ArchivedBlockPlan>, ProgressError>;

    async fn load_acknowledgements(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Vec<PublicationAcknowledgement>, ProgressError>;

    async fn pending_blocks(
        &self,
        chain_id: &ChainId,
        limit: usize,
    ) -> Result<Vec<ArchivedBlockPlan>, ProgressError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalObjectPlan {
    dataset_id: String,
    bucket: String,
    key: String,
    etag: String,
    content_hash: [u8; 32],
    archive_ref: String,
    byte_count: u64,
    parser_build: String,
    archived_at: KnownTime,
}

impl HistoricalObjectPlan {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        dataset_id: impl Into<String>,
        bucket: impl Into<String>,
        key: impl Into<String>,
        etag: impl Into<String>,
        content_hash: [u8; 32],
        archive_ref: impl Into<String>,
        byte_count: u64,
        parser_build: impl Into<String>,
        archived_at: KnownTime,
    ) -> Result<Self, ProgressError> {
        let dataset_id = dataset_id.into();
        let bucket = bucket.into();
        let key = key.into();
        let etag = etag.into();
        let archive_ref = archive_ref.into();
        let parser_build = parser_build.into();
        validate_identity(&dataset_id)?;
        validate_identity(&bucket)?;
        validate_identity(&key)?;
        validate_identity(&etag)?;
        validate_identity(&archive_ref)?;
        validate_identity(&parser_build)?;
        if byte_count == 0 {
            return Err(ProgressError::InvalidInput("zero historical object bytes"));
        }
        Ok(Self {
            dataset_id,
            bucket,
            key,
            etag,
            content_hash,
            archive_ref,
            byte_count,
            parser_build,
            archived_at,
        })
    }

    #[must_use]
    pub fn dataset_id(&self) -> &str {
        &self.dataset_id
    }

    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub fn etag(&self) -> &str {
        &self.etag
    }

    #[must_use]
    pub const fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }

    #[must_use]
    pub fn archive_ref(&self) -> &str {
        &self.archive_ref
    }

    #[must_use]
    pub const fn byte_count(&self) -> u64 {
        self.byte_count
    }

    #[must_use]
    pub fn parser_build(&self) -> &str {
        &self.parser_build
    }

    #[must_use]
    pub const fn archived_at(&self) -> KnownTime {
        self.archived_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalGapRecord {
    dataset_id: String,
    bucket: String,
    key: String,
    recorded_at: KnownTime,
}

impl HistoricalGapRecord {
    pub fn try_new(
        dataset_id: impl Into<String>,
        bucket: impl Into<String>,
        key: impl Into<String>,
        recorded_at: KnownTime,
    ) -> Result<Self, ProgressError> {
        let dataset_id = dataset_id.into();
        let bucket = bucket.into();
        let key = key.into();
        validate_identity(&dataset_id)?;
        validate_identity(&bucket)?;
        validate_identity(&key)?;
        Ok(Self {
            dataset_id,
            bucket,
            key,
            recorded_at,
        })
    }

    #[must_use]
    pub fn dataset_id(&self) -> &str {
        &self.dataset_id
    }

    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    #[must_use]
    pub fn key(&self) -> &str {
        &self.key
    }

    #[must_use]
    pub const fn recorded_at(&self) -> KnownTime {
        self.recorded_at
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalBackfillCursor {
    dataset_id: String,
    last_key: String,
    parser_build: String,
    coverage_start_key: String,
    coverage_end_key: String,
    cursor_version: u64,
    updated_at: KnownTime,
}

impl HistoricalBackfillCursor {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        dataset_id: impl Into<String>,
        last_key: impl Into<String>,
        parser_build: impl Into<String>,
        coverage_start_key: impl Into<String>,
        coverage_end_key: impl Into<String>,
        cursor_version: u64,
        updated_at: KnownTime,
    ) -> Result<Self, ProgressError> {
        let dataset_id = dataset_id.into();
        let last_key = last_key.into();
        let parser_build = parser_build.into();
        let coverage_start_key = coverage_start_key.into();
        let coverage_end_key = coverage_end_key.into();
        validate_identity(&dataset_id)?;
        validate_identity(&last_key)?;
        validate_identity(&parser_build)?;
        validate_identity(&coverage_start_key)?;
        validate_identity(&coverage_end_key)?;
        if cursor_version == 0 {
            return Err(ProgressError::InvalidInput("zero cursor version"));
        }
        if coverage_start_key.as_str() > coverage_end_key.as_str() {
            return Err(ProgressError::InvalidInput(
                "historical coverage range is inverted",
            ));
        }
        Ok(Self {
            dataset_id,
            last_key,
            parser_build,
            coverage_start_key,
            coverage_end_key,
            cursor_version,
            updated_at,
        })
    }

    #[must_use]
    pub fn dataset_id(&self) -> &str {
        &self.dataset_id
    }

    #[must_use]
    pub fn last_key(&self) -> &str {
        &self.last_key
    }

    #[must_use]
    pub fn parser_build(&self) -> &str {
        &self.parser_build
    }

    #[must_use]
    pub fn coverage_start_key(&self) -> &str {
        &self.coverage_start_key
    }

    #[must_use]
    pub fn coverage_end_key(&self) -> &str {
        &self.coverage_end_key
    }

    #[must_use]
    pub const fn cursor_version(&self) -> u64 {
        self.cursor_version
    }

    #[must_use]
    pub const fn updated_at(&self) -> KnownTime {
        self.updated_at
    }
}

pub trait HistoricalBackfillProgress: Send + Sync {
    fn record_object(
        &self,
        plan: &HistoricalObjectPlan,
    ) -> Result<ProgressRecordDisposition, ProgressError>;

    fn record_gap(
        &self,
        record: &HistoricalGapRecord,
    ) -> Result<ProgressRecordDisposition, ProgressError>;

    fn persist_cursor(
        &self,
        cursor: &HistoricalBackfillCursor,
    ) -> Result<ProgressRecordDisposition, ProgressError>;

    fn load_cursor(
        &self,
        dataset_id: &str,
    ) -> Result<Option<HistoricalBackfillCursor>, ProgressError>;

    fn load_object(
        &self,
        dataset_id: &str,
        key: &str,
    ) -> Result<Option<HistoricalObjectPlan>, ProgressError>;

    fn load_gaps(&self, dataset_id: &str) -> Result<Vec<HistoricalGapRecord>, ProgressError>;
}

fn validate_identity(value: &str) -> Result<(), ProgressError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTITY_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(ProgressError::InvalidInput("invalid identity"));
    }
    Ok(())
}

fn validate_subject(value: &str) -> Result<(), ProgressError> {
    validate_identity(value)?;
    if value.split('.').any(str::is_empty)
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'.'))
    {
        return Err(ProgressError::InvalidInput("invalid subject"));
    }
    Ok(())
}

fn validate_stream(value: &str) -> Result<(), ProgressError> {
    validate_identity(value)?;
    if value
        .bytes()
        .any(|byte| !(byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_'))
    {
        return Err(ProgressError::InvalidInput("invalid stream"));
    }
    Ok(())
}
