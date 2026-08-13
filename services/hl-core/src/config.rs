use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use domain_types::{BlockHeight, ChainId};
use serde::Deserialize;

use crate::{
    CANONICAL_STREAM, JetStreamReplayAuth, JetStreamReplayConfig, JetStreamReplayConfigError,
};

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_RUNTIME_PATH_BYTES: usize = 4_096;
const MAX_RUNTIME_TIMEOUT_MILLIS: u64 = 300_000;
const MAX_NATS_SERVER_BYTES: usize = 2_048;

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoreConfig {
    chain_id: String,
    first_height: u64,
    shutdown_grace_millis: u64,
    idle_poll_millis: u64,
    #[serde(default)]
    store: Option<StoreConfig>,
    #[serde(default)]
    nats: Option<NatsConfig>,
    #[serde(default)]
    status: Option<StatusConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreConfig {
    path: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StatusConfig {
    listen: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct NatsConfig {
    server_url: String,
    stream: String,
    username: String,
    password_path: PathBuf,
    connect_timeout_millis: u64,
    acknowledgement_timeout_millis: u64,
    max_ack_inflight: usize,
    durable_name: String,
    fetch_batch: usize,
}

impl CoreConfig {
    pub fn from_toml(source: &str) -> Result<Self, CoreConfigError> {
        let config: Self = toml::from_str(source).map_err(|_| CoreConfigError::InvalidToml)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), CoreConfigError> {
        ChainId::new(self.chain_id.clone()).map_err(|_| CoreConfigError::InvalidChainId)?;
        if !(1..=MAX_RUNTIME_TIMEOUT_MILLIS).contains(&self.shutdown_grace_millis)
            || !(1..=MAX_RUNTIME_TIMEOUT_MILLIS).contains(&self.idle_poll_millis)
        {
            return Err(CoreConfigError::InvalidRuntimeLimit);
        }
        let store = self.store.as_ref().ok_or(CoreConfigError::MissingStore)?;
        validate_runtime_path(&store.path)?;
        let nats = self.nats.as_ref().ok_or(CoreConfigError::MissingNats)?;
        validate_nats_server(&nats.server_url)?;
        validate_identity(&nats.stream).map_err(|_| CoreConfigError::InvalidNatsStream)?;
        if nats.stream != CANONICAL_STREAM {
            return Err(CoreConfigError::InvalidNatsStream);
        }
        validate_identity(&nats.username).map_err(|_| CoreConfigError::InvalidNatsIdentity)?;
        validate_credential_path(&nats.password_path)?;
        if let Some(status) = &self.status {
            validate_status_listen(&status.listen)?;
        }
        self.jetstream_config().map(|_| ())?;
        Ok(())
    }

    pub fn jetstream_config(&self) -> Result<JetStreamReplayConfig, CoreConfigError> {
        let nats = self.nats.as_ref().ok_or(CoreConfigError::MissingNats)?;
        JetStreamReplayConfig::try_new(
            nats.server_url.clone(),
            JetStreamReplayAuth::UserPasswordFile {
                username: nats.username.clone(),
                password_path: nats.password_path.clone(),
            },
            Duration::from_millis(nats.connect_timeout_millis),
            Duration::from_millis(nats.acknowledgement_timeout_millis),
            nats.max_ack_inflight,
            nats.durable_name.clone(),
            nats.fetch_batch,
        )
        .map_err(CoreConfigError::from)
    }

    #[must_use]
    pub fn chain_id(&self) -> ChainId {
        ChainId::new(self.chain_id.clone())
            .expect("CoreConfig is constructed only through validated deserialization")
    }

    #[must_use]
    pub const fn first_height(&self) -> BlockHeight {
        BlockHeight::new(self.first_height)
    }

    #[must_use]
    pub fn store_path(&self) -> &Path {
        self.store
            .as_ref()
            .map(|store| store.path.as_path())
            .expect("CoreConfig is constructed only through validated deserialization")
    }

    #[must_use]
    pub const fn shutdown_grace(&self) -> Duration {
        Duration::from_millis(self.shutdown_grace_millis)
    }

    #[must_use]
    pub const fn idle_poll(&self) -> Duration {
        Duration::from_millis(self.idle_poll_millis)
    }

    #[must_use]
    pub fn status_listen(&self) -> Option<SocketAddr> {
        self.status.as_ref().map(|status| {
            validate_status_listen(&status.listen)
                .expect("CoreConfig is constructed only through validated deserialization")
        })
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum CoreConfigError {
    #[error("hl-core configuration TOML is invalid")]
    InvalidToml,
    #[error("hl-core configuration is missing a file-store path")]
    MissingStore,
    #[error("hl-core configuration is missing NATS JetStream settings")]
    MissingNats,
    #[error("hl-core chain identifier is invalid")]
    InvalidChainId,
    #[error("hl-core store path is unsafe")]
    InvalidStorePath,
    #[error("hl-core NATS server URL is invalid or contains inline credentials")]
    InvalidNatsServer,
    #[error("hl-core NATS stream is not the canonical production stream")]
    InvalidNatsStream,
    #[error("hl-core NATS identity is invalid")]
    InvalidNatsIdentity,
    #[error("hl-core NATS credential reference is not an absolute protected path")]
    InvalidCredentialPath,
    #[error("hl-core runtime limit is outside the supported bound")]
    InvalidRuntimeLimit,
    #[error("hl-core status listen address is invalid or not loopback")]
    InvalidStatusListen,
    #[error("hl-core JetStream consumer configuration is invalid")]
    InvalidJetStream,
}

impl CoreConfigError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidToml => "core_config.invalid_toml",
            Self::MissingStore => "core_config.missing_store",
            Self::MissingNats => "core_config.missing_nats",
            Self::InvalidChainId => "core_config.invalid_chain_id",
            Self::InvalidStorePath => "core_config.invalid_store_path",
            Self::InvalidNatsServer => "core_config.invalid_nats_server",
            Self::InvalidNatsStream => "core_config.invalid_nats_stream",
            Self::InvalidNatsIdentity => "core_config.invalid_nats_identity",
            Self::InvalidCredentialPath => "core_config.invalid_credential_path",
            Self::InvalidRuntimeLimit => "core_config.invalid_runtime_limit",
            Self::InvalidStatusListen => "core_config.invalid_status_listen",
            Self::InvalidJetStream => "core_config.invalid_jetstream",
        }
    }
}

impl From<JetStreamReplayConfigError> for CoreConfigError {
    fn from(error: JetStreamReplayConfigError) -> Self {
        match error {
            JetStreamReplayConfigError::UnsafeServerUrl => Self::InvalidNatsServer,
            JetStreamReplayConfigError::UnsafeCredentialsPath => Self::InvalidCredentialPath,
            JetStreamReplayConfigError::InvalidUsername
            | JetStreamReplayConfigError::InvalidDurableName => Self::InvalidNatsIdentity,
            JetStreamReplayConfigError::InvalidConnectTimeout
            | JetStreamReplayConfigError::InvalidAcknowledgementTimeout
            | JetStreamReplayConfigError::InvalidMaxAckInflight
            | JetStreamReplayConfigError::InvalidFetchBatch => Self::InvalidRuntimeLimit,
        }
    }
}

fn validate_runtime_path(path: &Path) -> Result<(), CoreConfigError> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path.as_os_str().len() > MAX_RUNTIME_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        Err(CoreConfigError::InvalidStorePath)
    } else {
        Ok(())
    }
}

fn validate_status_listen(value: &str) -> Result<SocketAddr, CoreConfigError> {
    if value.trim() != value || value.chars().any(char::is_control) {
        return Err(CoreConfigError::InvalidStatusListen);
    }
    let address: SocketAddr = value
        .parse()
        .map_err(|_| CoreConfigError::InvalidStatusListen)?;
    if address.ip().is_loopback() {
        Ok(address)
    } else {
        Err(CoreConfigError::InvalidStatusListen)
    }
}

fn validate_nats_server(value: &str) -> Result<(), CoreConfigError> {
    if value.len() > MAX_NATS_SERVER_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(CoreConfigError::InvalidNatsServer);
    }

    let address = value
        .parse::<async_nats::ServerAddr>()
        .map_err(|_| CoreConfigError::InvalidNatsServer)?;
    let url = address.clone().into_inner();
    let scheme_is_supported = matches!(address.scheme(), "nats" | "tls");
    let unencrypted_host_is_loopback =
        address.scheme() != "nats" || matches!(address.host(), "127.0.0.1" | "::1");
    let has_only_authority =
        matches!(url.path(), "" | "/") && url.query().is_none() && url.fragment().is_none();

    if !scheme_is_supported
        || !unencrypted_host_is_loopback
        || address.host().is_empty()
        || address.port() == 0
        || address.has_user_pass()
        || address.is_websocket()
        || !has_only_authority
    {
        return Err(CoreConfigError::InvalidNatsServer);
    }

    Ok(())
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

fn validate_credential_path(path: &Path) -> Result<(), CoreConfigError> {
    if !path.is_absolute()
        || path == Path::new("/")
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        Err(CoreConfigError::InvalidCredentialPath)
    } else {
        Ok(())
    }
}
