use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};

use domain_types::SourceId;
use hl_protocol::ObservationClass;
use serde::{Deserialize, Serialize};

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_QUEUE_CAPACITY: usize = 1_000_000;
const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
const MIN_SEGMENT_TARGET_BYTES: u64 = 1024 * 1024;
const MAX_SEGMENT_TARGET_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAX_ROTATION_INTERVAL_SECONDS: u64 = 86_400;
const MAX_BATCH_RECORDS: u32 = 100_000;
const MAX_BATCH_DELAY_MILLIS: u64 = 60_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfig {
    parser_version: String,
    spool: SpoolConfig,
    sources: Vec<SourceConfig>,
}

impl CaptureConfig {
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(source).map_err(|_| ConfigError::InvalidToml)?;
        config.validate()?;
        Ok(config)
    }

    pub fn to_toml(&self) -> Result<String, ConfigError> {
        toml::to_string_pretty(self).map_err(|_| ConfigError::Serialization)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        validate_identity(&self.parser_version).map_err(|_| ConfigError::InvalidParserVersion)?;
        self.spool.validate()?;
        if self.sources.is_empty() {
            return Err(ConfigError::MissingSources);
        }
        let mut ids = BTreeSet::new();
        for source in &self.sources {
            source.validate()?;
            if !ids.insert(source.id.as_str()) {
                return Err(ConfigError::DuplicateSource);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn parser_version(&self) -> &str {
        &self.parser_version
    }

    #[must_use]
    pub const fn spool(&self) -> &SpoolConfig {
        &self.spool
    }

    #[must_use]
    pub fn sources(&self) -> &[SourceConfig] {
        &self.sources
    }

    #[must_use]
    pub fn source(&self, id: &str) -> Option<&SourceConfig> {
        self.sources.iter().find(|source| source.id == id)
    }

    #[must_use]
    pub fn payload_limit(&self, id: &str) -> Option<usize> {
        self.source(id).map(SourceConfig::max_payload_bytes)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpoolConfig {
    path: PathBuf,
    segment_target_bytes: u64,
    rotation_interval_seconds: u64,
    committed_durability: DurabilityPolicy,
    provisional_durability: DurabilityPolicy,
}

impl SpoolConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        validate_spool_path(&self.path)?;
        if !(MIN_SEGMENT_TARGET_BYTES..=MAX_SEGMENT_TARGET_BYTES)
            .contains(&self.segment_target_bytes)
        {
            return Err(ConfigError::InvalidSegmentTarget);
        }
        if !(1..=MAX_ROTATION_INTERVAL_SECONDS).contains(&self.rotation_interval_seconds) {
            return Err(ConfigError::InvalidRotationInterval);
        }
        self.committed_durability.validate()?;
        self.provisional_durability.validate()?;
        if !matches!(
            self.committed_durability,
            DurabilityPolicy::FsyncEveryRecord
        ) {
            return Err(ConfigError::InvalidDurabilityPolicy);
        }
        Ok(())
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub const fn segment_target_bytes(&self) -> u64 {
        self.segment_target_bytes
    }

    #[must_use]
    pub const fn rotation_interval_seconds(&self) -> u64 {
        self.rotation_interval_seconds
    }

    #[must_use]
    pub const fn committed_durability(&self) -> &DurabilityPolicy {
        &self.committed_durability
    }

    #[must_use]
    pub const fn provisional_durability(&self) -> &DurabilityPolicy {
        &self.provisional_durability
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DurabilityPolicy {
    FsyncEveryRecord,
    Batched {
        max_records: u32,
        max_delay_millis: u64,
    },
}

impl DurabilityPolicy {
    fn validate(self) -> Result<(), ConfigError> {
        match self {
            Self::FsyncEveryRecord => Ok(()),
            Self::Batched {
                max_records,
                max_delay_millis,
            } if (1..=MAX_BATCH_RECORDS).contains(&max_records)
                && (1..=MAX_BATCH_DELAY_MILLIS).contains(&max_delay_millis) =>
            {
                Ok(())
            }
            Self::Batched { .. } => Err(ConfigError::InvalidDurabilityPolicy),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceConfig {
    id: String,
    #[serde(rename = "class")]
    observation_class: ObservationClass,
    queue_capacity: usize,
    max_payload_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_path: Option<PathBuf>,
}

impl SourceConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        SourceId::new(self.id.clone()).map_err(|_| ConfigError::InvalidSourceId)?;
        if self.id.len() > MAX_IDENTITY_BYTES || self.id.chars().any(char::is_control) {
            return Err(ConfigError::InvalidSourceId);
        }
        if !(1..=MAX_QUEUE_CAPACITY).contains(&self.queue_capacity) {
            return Err(ConfigError::InvalidQueueCapacity);
        }
        if !(1..=MAX_PAYLOAD_BYTES).contains(&self.max_payload_bytes) {
            return Err(ConfigError::InvalidPayloadLimit);
        }
        if let Some(path) = &self.credential_path {
            validate_credential_path(path)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn observation_class(&self) -> ObservationClass {
        self.observation_class
    }

    #[must_use]
    pub const fn queue_capacity(&self) -> usize {
        self.queue_capacity
    }

    #[must_use]
    pub const fn max_payload_bytes(&self) -> usize {
        self.max_payload_bytes
    }

    #[must_use]
    pub fn credential_path(&self) -> Option<&Path> {
        self.credential_path.as_deref()
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum ConfigError {
    #[error("capture configuration TOML is invalid")]
    InvalidToml,
    #[error("capture parser version is invalid")]
    InvalidParserVersion,
    #[error("capture spool path is invalid")]
    InvalidSpoolPath,
    #[error("capture segment target is outside the supported range")]
    InvalidSegmentTarget,
    #[error("capture rotation interval is outside the supported range")]
    InvalidRotationInterval,
    #[error("capture durability policy is invalid")]
    InvalidDurabilityPolicy,
    #[error("capture source identifier is invalid")]
    InvalidSourceId,
    #[error("capture source identifier is duplicated")]
    DuplicateSource,
    #[error("capture queue capacity is outside the supported range")]
    InvalidQueueCapacity,
    #[error("capture payload limit is outside the supported range")]
    InvalidPayloadLimit,
    #[error("capture credential reference is not an absolute protected path")]
    InvalidCredentialPath,
    #[error("capture configuration has no sources")]
    MissingSources,
    #[error("validated capture configuration could not be serialized")]
    Serialization,
}

impl ConfigError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidToml => "capture_config.invalid_toml",
            Self::InvalidParserVersion => "capture_config.invalid_parser_version",
            Self::InvalidSpoolPath => "capture_config.invalid_spool_path",
            Self::InvalidSegmentTarget => "capture_config.invalid_segment_target",
            Self::InvalidRotationInterval => "capture_config.invalid_rotation_interval",
            Self::InvalidDurabilityPolicy => "capture_config.invalid_durability_policy",
            Self::InvalidSourceId => "capture_config.invalid_source_id",
            Self::DuplicateSource => "capture_config.duplicate_source",
            Self::InvalidQueueCapacity => "capture_config.invalid_queue_capacity",
            Self::InvalidPayloadLimit => "capture_config.invalid_payload_limit",
            Self::InvalidCredentialPath => "capture_config.invalid_credential_path",
            Self::MissingSources => "capture_config.missing_sources",
            Self::Serialization => "capture_config.serialization",
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

fn validate_spool_path(path: &Path) -> Result<(), ConfigError> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        Err(ConfigError::InvalidSpoolPath)
    } else {
        Ok(())
    }
}

fn validate_credential_path(path: &Path) -> Result<(), ConfigError> {
    if !path.is_absolute()
        || path == Path::new("/")
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        Err(ConfigError::InvalidCredentialPath)
    } else {
        Ok(())
    }
}
