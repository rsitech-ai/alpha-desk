use api_contracts::WireCanonicalEventEnvelope;
use semver::Version;

use crate::{CanonicalEventEnvelope, ContractError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalUpcaster {
    supported_major: u64,
    supported_minor: u64,
}

impl CanonicalUpcaster {
    #[must_use]
    pub const fn v1() -> Self {
        Self {
            supported_major: 1,
            supported_minor: 1,
        }
    }

    pub fn upcast(&self, bytes: &[u8]) -> Result<UpcastedEnvelope, UpcastError> {
        let wire =
            WireCanonicalEventEnvelope::decode(bytes).map_err(UpcastError::MalformedEnvelope)?;
        let version = Version::parse(&wire.schema_version).map_err(|error| {
            UpcastError::MalformedVersion {
                value: wire.schema_version.clone(),
                reason: error.to_string(),
            }
        })?;
        if !version.pre.is_empty() || !version.build.is_empty() {
            return Err(UpcastError::MalformedVersion {
                value: wire.schema_version,
                reason: "pre-release and build metadata are forbidden".to_owned(),
            });
        }
        if version.major != self.supported_major || version.minor > self.supported_minor {
            return Err(UpcastError::UnsupportedVersion {
                version: version.to_string(),
            });
        }

        CanonicalEventEnvelope::decode(bytes).map_err(UpcastError::InvalidCurrentEnvelope)?;
        Ok(UpcastedEnvelope {
            schema_version: version,
            bytes: bytes.to_vec(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpcastedEnvelope {
    schema_version: Version,
    bytes: Vec<u8>,
}

impl UpcastedEnvelope {
    #[must_use]
    pub const fn schema_version(&self) -> &Version {
        &self.schema_version
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
}

#[derive(Debug, thiserror::Error)]
pub enum UpcastError {
    #[error("canonical envelope wire bytes are malformed: {0}")]
    MalformedEnvelope(#[source] prost::DecodeError),
    #[error("canonical schema version {value:?} is malformed: {reason}")]
    MalformedVersion { value: String, reason: String },
    #[error("canonical schema version {version} has no registered upcast path")]
    UnsupportedVersion { version: String },
    #[error("current canonical envelope failed validation: {0}")]
    InvalidCurrentEnvelope(#[source] ContractError),
}

impl UpcastError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::MalformedEnvelope(_) => "canonical_upcast.malformed_envelope",
            Self::MalformedVersion { .. } => "canonical_upcast.malformed_version",
            Self::UnsupportedVersion { .. } => "canonical_upcast.unsupported_version",
            Self::InvalidCurrentEnvelope(_) => "canonical_upcast.invalid_current_envelope",
        }
    }
}
