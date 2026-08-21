pub mod accounts;
pub mod builders_agents;
pub mod fees_referrals;
pub mod general;
pub mod orders;
pub mod pagination;
pub mod registry;
pub mod request;
pub mod response;
pub mod twap;

mod decode;

use crate::ErrorDisposition;

const MAX_IDENTITY_BYTES: usize = 256;

pub use accounts::*;
pub use builders_agents::*;
pub use decode::{
    BookSide, InfoObservationKind, UserHistoryMeta, history_coverage, market_id_from_coin,
};
pub use fees_referrals::*;
pub use general::*;
pub use orders::*;
pub use pagination::*;
pub use registry::*;
pub use request::*;
pub use response::*;
pub use twap::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CapabilityId(String);

impl CapabilityId {
    pub fn new(value: impl Into<String>) -> Result<Self, InfoError> {
        let value = value.into();
        validate_identity(&value).map_err(|()| InfoError::InvalidCapabilityId)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArchiveRef(String);

impl ArchiveRef {
    pub fn new(value: impl Into<String>) -> Result<Self, InfoError> {
        let value = value.into();
        validate_identity(&value).map_err(|()| InfoError::InvalidArchiveRef)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JsonPath(String);

impl JsonPath {
    pub fn new(value: impl Into<String>) -> Result<Self, InfoError> {
        let value = value.into();
        if value.is_empty()
            || !value.starts_with('/')
            || value.trim() != value
            || value.len() > MAX_IDENTITY_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(InfoError::InvalidJsonPath);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SchemaFingerprint(blake3::Hash);

impl SchemaFingerprint {
    #[must_use]
    pub const fn from_hash(hash: blake3::Hash) -> Self {
        Self(hash)
    }

    #[must_use]
    pub const fn hash(self) -> blake3::Hash {
        self.0
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InfoError {
    #[error("unknown info capability id")]
    UnknownCapability,
    #[error("unknown info request type")]
    UnknownIdentifier,
    #[error("duplicate info capability registration")]
    DuplicateCapability,
    #[error("duplicate info identifier registration")]
    DuplicateIdentifier,
    #[error("invalid info capability id")]
    InvalidCapabilityId,
    #[error("invalid info archive ref")]
    InvalidArchiveRef,
    #[error("invalid json path")]
    InvalidJsonPath,
    #[error("info request params must not include type")]
    TypeFieldConflict,
    #[error("info payload is empty")]
    EmptyPayload,
    #[error("info payload is not valid json")]
    MalformedJson,
    #[error("info payload is malformed at {path}: {reason}")]
    MalformedPayload { path: String, reason: &'static str },
    #[error("info decimal overflow at {path}")]
    DecimalOverflow { path: String },
    #[error("info decimal scale is invalid at {path}")]
    DecimalInvalidScale { path: String },
    #[error("info decimal is invalid at {path}")]
    DecimalInvalid { path: String },
    #[error("json number is forbidden on the info accounting path at {path}")]
    ForbiddenJsonNumber { path: String },
    #[error("unknown state-affecting variant {value} at {path}")]
    UnknownStateAffectingVariant { path: String, value: String },
    #[error("invalid time page cursor")]
    InvalidCursor,
    #[error("invalid time page coverage")]
    InvalidCoverage,
}

impl InfoError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::UnknownCapability => "info.unknown_capability",
            Self::UnknownIdentifier => "info.unknown_identifier",
            Self::DuplicateCapability => "info.duplicate_capability",
            Self::DuplicateIdentifier => "info.duplicate_identifier",
            Self::InvalidCapabilityId => "info.invalid_capability_id",
            Self::InvalidArchiveRef => "info.invalid_archive_ref",
            Self::InvalidJsonPath => "info.invalid_json_path",
            Self::TypeFieldConflict => "info.type_field_conflict",
            Self::EmptyPayload => "info.empty_payload",
            Self::MalformedJson => "info.malformed_json",
            Self::MalformedPayload { .. } => "info.malformed_payload",
            Self::DecimalOverflow { .. } => "info.decimal_overflow",
            Self::DecimalInvalidScale { .. } => "info.decimal_invalid_scale",
            Self::DecimalInvalid { .. } => "info.decimal_invalid",
            Self::ForbiddenJsonNumber { .. } => "info.forbidden_json_number",
            Self::UnknownStateAffectingVariant { .. } => "info.unknown_state_affecting_variant",
            Self::InvalidCursor => "info.invalid_cursor",
            Self::InvalidCoverage => "info.invalid_coverage",
        }
    }

    #[must_use]
    pub const fn disposition(&self) -> ErrorDisposition {
        match self {
            Self::UnknownStateAffectingVariant { .. }
            | Self::DecimalOverflow { .. }
            | Self::DecimalInvalidScale { .. }
            | Self::DecimalInvalid { .. }
            | Self::ForbiddenJsonNumber { .. }
            | Self::MalformedJson
            | Self::MalformedPayload { .. }
            | Self::EmptyPayload => ErrorDisposition::Quarantine,
            Self::UnknownCapability
            | Self::UnknownIdentifier
            | Self::DuplicateCapability
            | Self::DuplicateIdentifier
            | Self::InvalidCapabilityId
            | Self::InvalidArchiveRef
            | Self::InvalidJsonPath
            | Self::TypeFieldConflict
            | Self::InvalidCursor
            | Self::InvalidCoverage => ErrorDisposition::Stop,
        }
    }
}

fn validate_identity(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTITY_BYTES
        || value.chars().any(char::is_control)
    {
        Err(())
    } else {
        Ok(())
    }
}

fn child_path(parent: &str, segment: &str) -> String {
    format!("{parent}/{segment}")
}
