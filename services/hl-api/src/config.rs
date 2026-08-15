use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::auth::{CredentialError, load_credential};
use crate::budget::{BudgetLoadError, QueryBudgets};

const MAX_CONFIG_BYTES: u64 = 1024 * 1024;

/// Authentication is fail-closed: missing `auth.mode` or missing credentials
/// in `credential` mode refuse to start. `loopback-dev` is a local exception
/// that requires a loopback bind and does not present a bearer secret. It is
/// not production OIDC, mTLS, RBAC, or an SLO.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuthMode {
    LoopbackDev,
    Credential,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiConfig {
    bind: SocketAddr,
    auth_mode: AuthMode,
    credential: Option<Vec<u8>>,
    canonical_health_path: Option<PathBuf>,
    capture_status_path: Option<PathBuf>,
    query_budgets: QueryBudgets,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    listen: RawListenConfig,
    auth: RawAuthConfig,
    #[serde(default)]
    snapshots: RawSnapshotConfig,
    #[serde(default)]
    query_budgets: Option<RawQueryBudgetsRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawListenConfig {
    bind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawAuthConfig {
    mode: AuthMode,
    #[serde(default)]
    credential_file: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSnapshotConfig {
    #[serde(default)]
    canonical_health: Option<String>,
    #[serde(default)]
    capture_status: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQueryBudgetsRef {
    file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    #[error("api_config.unreadable")]
    Unreadable,
    #[error("api_config.too_large")]
    TooLarge,
    #[error("api_config.invalid_toml")]
    InvalidToml,
    #[error("api_config.invalid_bind")]
    InvalidBind,
    #[error("api_config.loopback_required")]
    LoopbackRequired,
    #[error("api_config.credential_file_not_allowed")]
    CredentialFileNotAllowed,
    #[error("api_config.missing_credentials")]
    MissingCredentials,
    #[error("api_config.invalid_credentials")]
    InvalidCredentials,
    #[error("api_config.empty_path")]
    EmptyPath,
    #[error("api_config.missing_query_budgets")]
    MissingQueryBudgets,
    #[error("api_config.invalid_query_budgets")]
    InvalidQueryBudgets,
}

impl ConfigError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Unreadable => "api_config.unreadable",
            Self::TooLarge => "api_config.too_large",
            Self::InvalidToml => "api_config.invalid_toml",
            Self::InvalidBind => "api_config.invalid_bind",
            Self::LoopbackRequired => "api_config.loopback_required",
            Self::CredentialFileNotAllowed => "api_config.credential_file_not_allowed",
            Self::MissingCredentials => "api_config.missing_credentials",
            Self::InvalidCredentials => "api_config.invalid_credentials",
            Self::EmptyPath => "api_config.empty_path",
            Self::MissingQueryBudgets => "api_config.missing_query_budgets",
            Self::InvalidQueryBudgets => "api_config.invalid_query_budgets",
        }
    }
}

impl ApiConfig {
    pub fn from_path(path: &Path) -> Result<Self, ConfigError> {
        let metadata = fs::metadata(path).map_err(|_| ConfigError::Unreadable)?;
        if metadata.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::TooLarge);
        }
        let source = fs::read_to_string(path).map_err(|_| ConfigError::Unreadable)?;
        Self::from_toml(&source, path.parent().unwrap_or_else(|| Path::new(".")))
    }

    pub fn from_toml(source: &str, base_directory: &Path) -> Result<Self, ConfigError> {
        let raw: RawConfig = toml::from_str(source).map_err(|_| ConfigError::InvalidToml)?;
        let bind: SocketAddr = raw
            .listen
            .bind
            .parse()
            .map_err(|_| ConfigError::InvalidBind)?;
        match raw.auth.mode {
            AuthMode::LoopbackDev => {
                // loopback-dev is a local-only bind exception, not production auth.
                if !bind.ip().is_loopback() {
                    return Err(ConfigError::LoopbackRequired);
                }
                if raw.auth.credential_file.is_some() {
                    return Err(ConfigError::CredentialFileNotAllowed);
                }
            }
            AuthMode::Credential => {
                if raw.auth.credential_file.is_none() {
                    return Err(ConfigError::MissingCredentials);
                }
            }
        }
        let credential = match raw.auth.mode {
            AuthMode::LoopbackDev => None,
            AuthMode::Credential => {
                let relative = raw
                    .auth
                    .credential_file
                    .as_deref()
                    .ok_or(ConfigError::MissingCredentials)?;
                let path = resolve_path(base_directory, relative)?;
                Some(load_credential(&path).map_err(|error| match error {
                    CredentialError::Missing => ConfigError::MissingCredentials,
                    CredentialError::Invalid => ConfigError::InvalidCredentials,
                })?)
            }
        };
        let query_budgets_ref = raw.query_budgets.ok_or(ConfigError::MissingQueryBudgets)?;
        let query_budgets_path = resolve_path(base_directory, &query_budgets_ref.file)?;
        let query_budgets =
            QueryBudgets::from_path(&query_budgets_path).map_err(|error| match error {
                BudgetLoadError::Missing => ConfigError::MissingQueryBudgets,
                BudgetLoadError::Invalid => ConfigError::InvalidQueryBudgets,
            })?;
        Ok(Self {
            bind,
            auth_mode: raw.auth.mode,
            credential,
            canonical_health_path: optional_path(base_directory, raw.snapshots.canonical_health)?,
            capture_status_path: optional_path(base_directory, raw.snapshots.capture_status)?,
            query_budgets,
        })
    }

    #[must_use]
    pub const fn bind(&self) -> SocketAddr {
        self.bind
    }

    #[must_use]
    pub const fn auth_mode(&self) -> AuthMode {
        self.auth_mode
    }

    #[must_use]
    pub fn credential(&self) -> Option<&[u8]> {
        self.credential.as_deref()
    }

    #[must_use]
    pub fn canonical_health_path(&self) -> Option<&Path> {
        self.canonical_health_path.as_deref()
    }

    #[must_use]
    pub fn capture_status_path(&self) -> Option<&Path> {
        self.capture_status_path.as_deref()
    }

    #[must_use]
    pub const fn query_budgets(&self) -> &QueryBudgets {
        &self.query_budgets
    }
}

fn optional_path(
    base_directory: &Path,
    value: Option<String>,
) -> Result<Option<PathBuf>, ConfigError> {
    match value {
        None => Ok(None),
        Some(text) if text.is_empty() => Err(ConfigError::EmptyPath),
        Some(text) => Ok(Some(resolve_path(base_directory, &text)?)),
    }
}

fn resolve_path(base_directory: &Path, value: &str) -> Result<PathBuf, ConfigError> {
    if value.is_empty() {
        return Err(ConfigError::EmptyPath);
    }
    let path = Path::new(value);
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(base_directory.join(path))
    }
}
