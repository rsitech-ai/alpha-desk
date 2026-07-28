use bytes::Bytes;
use domain_types::SourceId;
use serde::{Deserialize, Serialize};

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_WARNING_CODE_BYTES: usize = 128;
const MAX_WARNING_DETAIL_BYTES: usize = 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObservationClass {
    CommittedBlock,
    AuxiliaryOrderStatus,
    AuxiliaryBookDiff,
    AuxiliaryLedger,
    Snapshot,
    HistoricalBlock,
    PublicMarketData,
    ProvisionalFeed,
    ProvisionalMempool,
}

impl ObservationClass {
    pub const ALL: [Self; 9] = [
        Self::CommittedBlock,
        Self::AuxiliaryOrderStatus,
        Self::AuxiliaryBookDiff,
        Self::AuxiliaryLedger,
        Self::Snapshot,
        Self::HistoricalBlock,
        Self::PublicMarketData,
        Self::ProvisionalFeed,
        Self::ProvisionalMempool,
    ];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCursor {
    epoch: String,
    offset: u64,
}

impl SourceCursor {
    pub fn new(epoch: impl Into<String>, offset: u64) -> Result<Self, ObservationError> {
        let epoch = epoch.into();
        validate_identity(&epoch).map_err(|_| ObservationError::InvalidCursorEpoch)?;
        Ok(Self { epoch, offset })
    }

    #[must_use]
    pub fn epoch(&self) -> &str {
        &self.epoch
    }

    #[must_use]
    pub const fn offset(&self) -> u64 {
        self.offset
    }

    pub fn validate_successor_of(
        &self,
        previous: &Self,
    ) -> Result<CursorTransition, ObservationError> {
        if self.epoch != previous.epoch {
            return Ok(CursorTransition::EpochChanged);
        }
        match self.offset.cmp(&previous.offset) {
            std::cmp::Ordering::Less => Err(ObservationError::CursorRegression),
            std::cmp::Ordering::Equal => Ok(CursorTransition::Duplicate),
            std::cmp::Ordering::Greater => Ok(CursorTransition::Advanced {
                by: self.offset - previous.offset,
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorTransition {
    Duplicate,
    Advanced { by: u64 },
    EpochChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReceiveTimestamps {
    wall_micros: i64,
    monotonic_nanos: u64,
}

impl ReceiveTimestamps {
    pub fn new(wall_micros: i64, monotonic_nanos: u64) -> Result<Self, ObservationError> {
        if wall_micros < 0 {
            return Err(ObservationError::InvalidWallTimestamp);
        }
        Ok(Self {
            wall_micros,
            monotonic_nanos,
        })
    }

    #[must_use]
    pub const fn wall_micros(self) -> i64 {
        self.wall_micros
    }

    #[must_use]
    pub const fn monotonic_nanos(self) -> u64 {
        self.monotonic_nanos
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseWarning {
    code: String,
    detail: String,
}

impl ParseWarning {
    pub fn new(
        code: impl Into<String>,
        detail: impl Into<String>,
    ) -> Result<Self, ObservationError> {
        let code = code.into();
        let detail = detail.into();
        validate_bounded_text(&code, MAX_WARNING_CODE_BYTES)
            .map_err(|_| ObservationError::InvalidWarningCode)?;
        validate_bounded_text(&detail, MAX_WARNING_DETAIL_BYTES)
            .map_err(|_| ObservationError::InvalidWarningDetail)?;
        Ok(Self { code, detail })
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }

    #[must_use]
    pub fn detail(&self) -> &str {
        &self.detail
    }
}

#[derive(Debug, Clone)]
pub struct SourceObservation {
    source_id: SourceId,
    source_version: String,
    observation_class: ObservationClass,
    cursor: SourceCursor,
    received: ReceiveTimestamps,
    parser_schema_version: String,
    payload: Bytes,
    content_hash: blake3::Hash,
    warnings: Vec<ParseWarning>,
}

impl SourceObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        source_id: SourceId,
        source_version: impl Into<String>,
        observation_class: ObservationClass,
        cursor: SourceCursor,
        received: ReceiveTimestamps,
        parser_schema_version: impl Into<String>,
        payload: Bytes,
        warnings: Vec<ParseWarning>,
        max_payload_bytes: usize,
    ) -> Result<Self, ObservationError> {
        let source_version = source_version.into();
        validate_identity(&source_version).map_err(|_| ObservationError::InvalidSourceVersion)?;
        let parser_schema_version = parser_schema_version.into();
        validate_identity(&parser_schema_version)
            .map_err(|_| ObservationError::InvalidParserSchemaVersion)?;
        if max_payload_bytes == 0 {
            return Err(ObservationError::InvalidPayloadLimit);
        }
        if payload.is_empty() {
            return Err(ObservationError::EmptyPayload);
        }
        if payload.len() > max_payload_bytes {
            return Err(ObservationError::PayloadTooLarge {
                observation_class,
                actual: payload.len(),
                maximum: max_payload_bytes,
            });
        }
        let content_hash = blake3::hash(&payload);
        Ok(Self {
            source_id,
            source_version,
            observation_class,
            cursor,
            received,
            parser_schema_version,
            payload,
            content_hash,
            warnings,
        })
    }

    #[must_use]
    pub fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub fn source_version(&self) -> &str {
        &self.source_version
    }

    #[must_use]
    pub const fn observation_class(&self) -> ObservationClass {
        self.observation_class
    }

    #[must_use]
    pub const fn cursor(&self) -> &SourceCursor {
        &self.cursor
    }

    #[must_use]
    pub const fn received(&self) -> ReceiveTimestamps {
        self.received
    }

    #[must_use]
    pub fn parser_schema_version(&self) -> &str {
        &self.parser_schema_version
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        self.content_hash
    }

    #[must_use]
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum ObservationError {
    #[error("invalid source version")]
    InvalidSourceVersion,
    #[error("invalid parser schema version")]
    InvalidParserSchemaVersion,
    #[error("invalid source cursor epoch")]
    InvalidCursorEpoch,
    #[error("invalid wall timestamp")]
    InvalidWallTimestamp,
    #[error("invalid parse-warning code")]
    InvalidWarningCode,
    #[error("invalid parse-warning detail")]
    InvalidWarningDetail,
    #[error("payload limit must be greater than zero")]
    InvalidPayloadLimit,
    #[error("source payload is empty")]
    EmptyPayload,
    #[error(
        "source payload for {observation_class:?} is {actual} bytes; maximum is {maximum} bytes"
    )]
    PayloadTooLarge {
        observation_class: ObservationClass,
        actual: usize,
        maximum: usize,
    },
    #[error("source cursor regressed within one epoch")]
    CursorRegression,
}

impl ObservationError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidSourceVersion => "observation.invalid_source_version",
            Self::InvalidParserSchemaVersion => "observation.invalid_parser_schema_version",
            Self::InvalidCursorEpoch => "observation.invalid_cursor_epoch",
            Self::InvalidWallTimestamp => "observation.invalid_wall_timestamp",
            Self::InvalidWarningCode => "observation.invalid_warning_code",
            Self::InvalidWarningDetail => "observation.invalid_warning_detail",
            Self::InvalidPayloadLimit => "observation.invalid_payload_limit",
            Self::EmptyPayload => "observation.empty_payload",
            Self::PayloadTooLarge { .. } => "observation.payload_too_large",
            Self::CursorRegression => "observation.cursor_regression",
        }
    }

    #[must_use]
    pub const fn observation_class(&self) -> Option<ObservationClass> {
        match self {
            Self::PayloadTooLarge {
                observation_class, ..
            } => Some(*observation_class),
            _ => None,
        }
    }
}

fn validate_identity(value: &str) -> Result<(), ()> {
    validate_bounded_text(value, MAX_IDENTITY_BYTES)
}

fn validate_bounded_text(value: &str, maximum: usize) -> Result<(), ()> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > maximum
        || value.chars().any(char::is_control)
    {
        Err(())
    } else {
        Ok(())
    }
}
