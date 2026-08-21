use std::path::{Component, Path};

use domain_types::{BlockHeight, ChainId, CheckpointId};
use serde::{Deserialize, Serialize};

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_PATH_BYTES: usize = 4_096;
const MAX_DISK_RESERVE_BYTES: u64 = 16 * 1024 * 1024 * 1024 * 1024;
const MAX_BACKLOG: usize = 10_000;
const MAX_NATS_SERVER_BYTES: usize = 2_048;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreConfig {
    chain_id: String,
    genesis_height: u64,
    resume: ResumeConfig,
    state_path: String,
    checkpoint_path: String,
    disk_reserve_bytes: u64,
    max_pending_blocks: usize,
    shutdown_grace_millis: u64,
    #[serde(default)]
    nats: Option<NatsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeConfig {
    mode: ResumeModeWire,
    #[serde(default)]
    checkpoint_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ResumeModeWire {
    Genesis,
    Durable,
    Checkpoint,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NatsConfig {
    server_url: String,
    durable_name: String,
    fetch_batch: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    #[error("core config TOML is invalid")]
    InvalidToml,
    #[error("core config identity is invalid")]
    InvalidIdentity,
    #[error("core config path is unsafe")]
    UnsafePath,
    #[error("core config resume mode is invalid")]
    InvalidResume,
    #[error("core config resource bound is invalid")]
    InvalidBound,
}

impl ConfigError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidToml => "core.config.invalid_toml",
            Self::InvalidIdentity => "core.config.invalid_identity",
            Self::UnsafePath => "core.config.unsafe_path",
            Self::InvalidResume => "core.config.invalid_resume",
            Self::InvalidBound => "core.config.invalid_bound",
        }
    }
}

impl CoreConfig {
    pub fn from_toml(source: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(source).map_err(|_| ConfigError::InvalidToml)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        validate_identity(&self.chain_id)?;
        ChainId::new(&self.chain_id).map_err(|_| ConfigError::InvalidIdentity)?;
        if self.genesis_height == 0 {
            return Err(ConfigError::InvalidBound);
        }
        validate_path(&self.state_path)?;
        validate_path(&self.checkpoint_path)?;
        if !(1..=MAX_DISK_RESERVE_BYTES).contains(&self.disk_reserve_bytes) {
            return Err(ConfigError::InvalidBound);
        }
        if !(1..=MAX_BACKLOG).contains(&self.max_pending_blocks) {
            return Err(ConfigError::InvalidBound);
        }
        if self.shutdown_grace_millis == 0 {
            return Err(ConfigError::InvalidBound);
        }
        match self.resume.mode {
            ResumeModeWire::Checkpoint => {
                let Some(checkpoint_id) = self.resume.checkpoint_id.as_deref() else {
                    return Err(ConfigError::InvalidResume);
                };
                CheckpointId::new(checkpoint_id).map_err(|_| ConfigError::InvalidResume)?;
            }
            ResumeModeWire::Genesis | ResumeModeWire::Durable => {
                if self.resume.checkpoint_id.is_some() {
                    return Err(ConfigError::InvalidResume);
                }
            }
        }
        if let Some(nats) = &self.nats {
            validate_nats_url(&nats.server_url)?;
            validate_identity(&nats.durable_name)?;
            if !(1..=MAX_BACKLOG).contains(&nats.fetch_batch) {
                return Err(ConfigError::InvalidBound);
            }
        }
        Ok(())
    }

    #[must_use]
    pub fn chain_id(&self) -> ChainId {
        ChainId::new(&self.chain_id).expect("validated")
    }

    #[must_use]
    pub fn genesis_height(&self) -> BlockHeight {
        BlockHeight::new(self.genesis_height)
    }

    #[must_use]
    pub fn state_path(&self) -> &Path {
        Path::new(&self.state_path)
    }

    #[must_use]
    pub fn checkpoint_path(&self) -> &Path {
        Path::new(&self.checkpoint_path)
    }

    #[must_use]
    pub const fn disk_reserve_bytes(&self) -> u64 {
        self.disk_reserve_bytes
    }

    #[must_use]
    pub const fn max_pending_blocks(&self) -> usize {
        self.max_pending_blocks
    }

    #[must_use]
    pub const fn shutdown_grace_millis(&self) -> u64 {
        self.shutdown_grace_millis
    }

    #[must_use]
    pub fn nats(&self) -> Option<&NatsConfig> {
        self.nats.as_ref()
    }

    pub fn resume_mode(&self) -> Result<crate::state_runtime::ResumeMode, ConfigError> {
        match self.resume.mode {
            ResumeModeWire::Genesis => Ok(crate::state_runtime::ResumeMode::Genesis),
            ResumeModeWire::Durable => Ok(crate::state_runtime::ResumeMode::Durable),
            ResumeModeWire::Checkpoint => {
                let id = self
                    .resume
                    .checkpoint_id
                    .as_deref()
                    .ok_or(ConfigError::InvalidResume)?;
                Ok(crate::state_runtime::ResumeMode::Checkpoint(
                    CheckpointId::new(id).map_err(|_| ConfigError::InvalidResume)?,
                ))
            }
        }
    }
}

impl NatsConfig {
    #[must_use]
    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    #[must_use]
    pub fn durable_name(&self) -> &str {
        &self.durable_name
    }

    #[must_use]
    pub const fn fetch_batch(&self) -> usize {
        self.fetch_batch
    }
}

fn validate_identity(value: &str) -> Result<(), ConfigError> {
    if value.is_empty()
        || value.len() > MAX_IDENTITY_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ConfigError::InvalidIdentity);
    }
    Ok(())
}

fn validate_path(value: &str) -> Result<(), ConfigError> {
    let path = Path::new(value);
    if value.is_empty()
        || value.len() > MAX_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(ConfigError::UnsafePath);
    }
    Ok(())
}

fn validate_nats_url(value: &str) -> Result<(), ConfigError> {
    if value.len() > MAX_NATS_SERVER_BYTES
        || !(value.starts_with("nats://") || value.starts_with("tls://"))
        || value.contains('@')
    {
        return Err(ConfigError::InvalidIdentity);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::CoreConfig;

    #[test]
    fn example_core_config_parses() {
        let config = CoreConfig::from_toml(include_str!("../../../config/core.example.toml"))
            .expect("example");
        assert_eq!(config.genesis_height().get(), 1);
        assert!(config.nats().is_some());
    }
}
