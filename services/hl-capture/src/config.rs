use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use domain_types::{BlockHeight, ChainId, SourceId};
use hl_protocol::node::v1::NodeStreamKind;
use hl_protocol::{ObservationClass, SourceAdmission, SourceTrust};
use serde::{Deserialize, Serialize};

const MAX_IDENTITY_BYTES: usize = 256;
const MAX_QUEUE_CAPACITY: usize = 1_000_000;
const MAX_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;
const MIN_SEGMENT_TARGET_BYTES: u64 = 1024 * 1024;
const MAX_SEGMENT_TARGET_BYTES: u64 = 512 * 1024 * 1024;
const MAX_ROTATION_INTERVAL_SECONDS: u64 = 86_400;
const MAX_BATCH_RECORDS: u32 = 100_000;
const MAX_BATCH_DELAY_MILLIS: u64 = 60_000;
const MAX_SOURCE_POLL_INTERVAL_MILLIS: u64 = 60_000;
const MAX_RUNTIME_TIMEOUT_MILLIS: u64 = 300_000;
const MAX_RUNTIME_BLOCK_CAPACITY: usize = 10_000_000;
const MAX_RUNTIME_PATH_BYTES: usize = 4_096;
const MAX_DISK_RESERVE_BYTES: u64 = 16 * 1024 * 1024 * 1024 * 1024;
const MAX_NATS_SERVER_BYTES: usize = 2_048;
const MAX_NATS_ACK_INFLIGHT: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureConfig {
    parser_version: String,
    runtime: RuntimeConfig,
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
        if self.parser_version.starts_with("quarantine-v1:") {
            return Err(ConfigError::InvalidParserVersion);
        }
        self.runtime.validate()?;
        self.spool.validate()?;
        if self.sources.is_empty() {
            return Err(ConfigError::MissingSources);
        }
        let mut ids = BTreeSet::new();
        let mut primary_committed_sources = 0_u8;
        let mut independent_committed_sources = 0_u8;
        for source in &self.sources {
            source.validate()?;
            if !ids.insert(source.id.as_str()) {
                return Err(ConfigError::DuplicateSource);
            }
            match source.trust {
                SourceTrust::LocallyVerifiedCommitted => {
                    if source.observation_class == ObservationClass::CommittedBlock {
                        primary_committed_sources = primary_committed_sources.saturating_add(1);
                        validate_committed_source_adapter(source)?;
                    }
                }
                SourceTrust::IndependentCommitted => {
                    if source.observation_class == ObservationClass::CommittedBlock {
                        independent_committed_sources =
                            independent_committed_sources.saturating_add(1);
                        validate_committed_source_adapter(source)?;
                    }
                }
                SourceTrust::ReconciledSnapshot => {}
                SourceTrust::RecoveryOnly => {}
                SourceTrust::ThirdPartyProvisional => {}
                SourceTrust::MempoolProvisional => {}
            }
        }
        if primary_committed_sources == 0 {
            return Err(ConfigError::MissingPrimaryCommittedSource);
        }
        if primary_committed_sources > 1 {
            return Err(ConfigError::DuplicatePrimaryCommittedSource);
        }
        if independent_committed_sources > 1 {
            return Err(ConfigError::DuplicateIndependentCommittedSource);
        }
        Ok(())
    }

    #[must_use]
    pub fn parser_version(&self) -> &str {
        &self.parser_version
    }

    #[must_use]
    pub const fn runtime(&self) -> &RuntimeConfig {
        &self.runtime
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

fn validate_committed_source_adapter(source: &SourceConfig) -> Result<(), ConfigError> {
    if matches!(
        source.adapter,
        Some(SourceAdapterConfig::NodeBlockDirectory { .. })
    ) {
        Ok(())
    } else {
        Err(ConfigError::InvalidCommittedSourceAdapter)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    chain_id: String,
    first_height: u64,
    archive_path: PathBuf,
    status_path: PathBuf,
    failover_state_path: PathBuf,
    postgres_url_path: PathBuf,
    nats_server_url: String,
    nats_stream: String,
    nats_username: String,
    nats_password_path: PathBuf,
    postgres_operation_timeout_millis: u64,
    max_pending_blocks: usize,
    retained_committed_blocks: usize,
    publisher_ledger_capacity: usize,
    nats_max_ack_inflight: usize,
    publish_timeout_millis: u64,
    backpressure_timeout_millis: u64,
    shutdown_grace_millis: u64,
    disk_reserve_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    status_listen: Option<String>,
}

impl RuntimeConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        ChainId::new(self.chain_id.clone()).map_err(|_| ConfigError::InvalidChainId)?;
        validate_runtime_path(&self.archive_path)?;
        validate_runtime_path(&self.status_path)?;
        validate_runtime_path(&self.failover_state_path)?;
        validate_credential_path(&self.postgres_url_path)?;
        validate_nats_server(&self.nats_server_url)?;
        validate_identity(&self.nats_stream).map_err(|_| ConfigError::InvalidRuntimeIdentity)?;
        if self.nats_stream != crate::bus::CANONICAL_STREAM {
            return Err(ConfigError::InvalidNatsStream);
        }
        validate_identity(&self.nats_username).map_err(|_| ConfigError::InvalidRuntimeIdentity)?;
        validate_credential_path(&self.nats_password_path)?;
        if !(1..=MAX_RUNTIME_TIMEOUT_MILLIS).contains(&self.postgres_operation_timeout_millis)
            || !(1..=MAX_RUNTIME_BLOCK_CAPACITY).contains(&self.max_pending_blocks)
            || !(1..=MAX_RUNTIME_BLOCK_CAPACITY).contains(&self.retained_committed_blocks)
            || !(1..=MAX_RUNTIME_BLOCK_CAPACITY).contains(&self.publisher_ledger_capacity)
            || !(1..=MAX_NATS_ACK_INFLIGHT).contains(&self.nats_max_ack_inflight)
            || !(1..=MAX_RUNTIME_TIMEOUT_MILLIS).contains(&self.publish_timeout_millis)
            || !(1..=MAX_RUNTIME_TIMEOUT_MILLIS).contains(&self.backpressure_timeout_millis)
            || !(1..=MAX_RUNTIME_TIMEOUT_MILLIS).contains(&self.shutdown_grace_millis)
            || !(1..=MAX_DISK_RESERVE_BYTES).contains(&self.disk_reserve_bytes)
        {
            return Err(ConfigError::InvalidRuntimeLimit);
        }
        if let Some(listen) = &self.status_listen {
            validate_status_listen(listen)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn chain_id(&self) -> ChainId {
        ChainId::new(self.chain_id.clone())
            .expect("RuntimeConfig is constructed only through validated deserialization")
    }

    #[must_use]
    pub const fn first_height(&self) -> BlockHeight {
        BlockHeight::new(self.first_height)
    }

    #[must_use]
    pub fn archive_path(&self) -> &Path {
        &self.archive_path
    }

    #[must_use]
    pub fn status_path(&self) -> &Path {
        &self.status_path
    }

    #[must_use]
    pub fn failover_state_path(&self) -> &Path {
        &self.failover_state_path
    }

    #[must_use]
    pub fn postgres_url_path(&self) -> &Path {
        &self.postgres_url_path
    }

    #[must_use]
    pub fn nats_server_url(&self) -> &str {
        &self.nats_server_url
    }

    #[must_use]
    pub fn nats_stream(&self) -> &str {
        &self.nats_stream
    }

    #[must_use]
    pub fn nats_username(&self) -> &str {
        &self.nats_username
    }

    #[must_use]
    pub fn nats_password_path(&self) -> &Path {
        &self.nats_password_path
    }

    #[must_use]
    pub const fn postgres_operation_timeout_millis(&self) -> u64 {
        self.postgres_operation_timeout_millis
    }

    #[must_use]
    pub const fn max_pending_blocks(&self) -> usize {
        self.max_pending_blocks
    }

    #[must_use]
    pub const fn retained_committed_blocks(&self) -> usize {
        self.retained_committed_blocks
    }

    #[must_use]
    pub const fn publisher_ledger_capacity(&self) -> usize {
        self.publisher_ledger_capacity
    }

    #[must_use]
    pub const fn nats_max_ack_inflight(&self) -> usize {
        self.nats_max_ack_inflight
    }

    #[must_use]
    pub const fn publish_timeout_millis(&self) -> u64 {
        self.publish_timeout_millis
    }

    #[must_use]
    pub const fn backpressure_timeout_millis(&self) -> u64 {
        self.backpressure_timeout_millis
    }

    #[must_use]
    pub const fn shutdown_grace_millis(&self) -> u64 {
        self.shutdown_grace_millis
    }

    #[must_use]
    pub const fn disk_reserve_bytes(&self) -> u64 {
        self.disk_reserve_bytes
    }

    #[must_use]
    pub fn status_listen(&self) -> Option<SocketAddr> {
        self.status_listen.as_ref().map(|value| {
            value
                .parse()
                .expect("RuntimeConfig is constructed only through validated deserialization")
        })
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
    source_version: String,
    trust: SourceTrust,
    #[serde(rename = "class")]
    observation_class: ObservationClass,
    queue_capacity: usize,
    max_payload_bytes: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    adapter: Option<SourceAdapterConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    credential_path: Option<PathBuf>,
}

impl SourceConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        SourceId::new(self.id.clone()).map_err(|_| ConfigError::InvalidSourceId)?;
        if !is_safe_source_path_component(&self.id) {
            return Err(ConfigError::InvalidSourceId);
        }
        validate_identity(&self.source_version).map_err(|_| ConfigError::InvalidSourceVersion)?;
        if !(1..=MAX_QUEUE_CAPACITY).contains(&self.queue_capacity) {
            return Err(ConfigError::InvalidQueueCapacity);
        }
        if !(1..=MAX_PAYLOAD_BYTES).contains(&self.max_payload_bytes) {
            return Err(ConfigError::InvalidPayloadLimit);
        }
        SourceAdmission::new(self.trust, self.observation_class)
            .map_err(|_| ConfigError::InvalidSourceTrust)?;
        if let Some(path) = &self.credential_path {
            validate_credential_path(path)?;
        }
        if let Some(adapter) = &self.adapter {
            adapter.validate(self.observation_class)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
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
    pub const fn trust(&self) -> SourceTrust {
        self.trust
    }

    pub fn admission(&self) -> Result<SourceAdmission, ConfigError> {
        SourceAdmission::new(self.trust, self.observation_class)
            .map_err(|_| ConfigError::InvalidSourceTrust)
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
    pub const fn adapter(&self) -> Option<&SourceAdapterConfig> {
        self.adapter.as_ref()
    }

    #[must_use]
    pub fn credential_path(&self) -> Option<&Path> {
        self.credential_path.as_deref()
    }
}

fn is_safe_source_path_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_IDENTITY_BYTES
        && value != "."
        && value != ".."
        && value.bytes().enumerate().all(|(index, byte)| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' => true,
            b'-' | b'_' | b'.' => index > 0,
            _ => false,
        })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum SourceAdapterConfig {
    NodeLine {
        path: PathBuf,
        stream_name: String,
        stream: NodeStreamKind,
        poll_interval_millis: u64,
    },
    NodeBlockDirectory {
        path: PathBuf,
        stream_name: String,
        start_height: u64,
        poll_interval_millis: u64,
        replica_cmds_style: NodeReplicaCmdsStyle,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeReplicaCmdsStyle {
    Actions,
    ActionsAndResponses,
    RecentActions,
}

impl SourceAdapterConfig {
    fn validate(&self, observation_class: ObservationClass) -> Result<(), ConfigError> {
        let (path, stream_name, poll_interval_millis, expected_class, replica_cmds_style) =
            match self {
                Self::NodeLine {
                    path,
                    stream_name,
                    stream,
                    poll_interval_millis,
                } => (
                    path,
                    stream_name,
                    *poll_interval_millis,
                    stream.observation_class(),
                    None,
                ),
                Self::NodeBlockDirectory {
                    path,
                    stream_name,
                    start_height: _,
                    poll_interval_millis,
                    replica_cmds_style,
                } => (
                    path,
                    stream_name,
                    *poll_interval_millis,
                    ObservationClass::CommittedBlock,
                    Some(*replica_cmds_style),
                ),
            };
        validate_node_source_path(path)?;
        validate_identity(stream_name).map_err(|_| ConfigError::InvalidSourceAdapter)?;
        if !(1..=MAX_SOURCE_POLL_INTERVAL_MILLIS).contains(&poll_interval_millis)
            || observation_class != expected_class
            || replica_cmds_style
                .is_some_and(|style| style != NodeReplicaCmdsStyle::ActionsAndResponses)
        {
            return Err(ConfigError::InvalidSourceAdapter);
        }
        Ok(())
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
    #[error("capture source version is invalid")]
    InvalidSourceVersion,
    #[error("capture queue capacity is outside the supported range")]
    InvalidQueueCapacity,
    #[error("capture payload limit is outside the supported range")]
    InvalidPayloadLimit,
    #[error("capture source trust is incompatible with its observation class")]
    InvalidSourceTrust,
    #[error("capture source adapter is invalid")]
    InvalidSourceAdapter,
    #[error("capture configuration requires exactly one primary committed source")]
    MissingPrimaryCommittedSource,
    #[error("capture configuration contains multiple primary committed sources")]
    DuplicatePrimaryCommittedSource,
    #[error("capture configuration contains multiple independent committed sources")]
    DuplicateIndependentCommittedSource,
    #[error("committed capture source requires a node block-directory adapter")]
    InvalidCommittedSourceAdapter,
    #[error("capture credential reference is not an absolute protected path")]
    InvalidCredentialPath,
    #[error("capture configuration has no sources")]
    MissingSources,
    #[error("capture runtime chain identifier is invalid")]
    InvalidChainId,
    #[error("capture runtime path is unsafe")]
    InvalidRuntimePath,
    #[error("capture runtime identity is invalid")]
    InvalidRuntimeIdentity,
    #[error("capture NATS server URL is invalid or contains inline credentials")]
    InvalidNatsServer,
    #[error("capture NATS stream is not the canonical production stream")]
    InvalidNatsStream,
    #[error("capture runtime limit is outside the supported range")]
    InvalidRuntimeLimit,
    #[error("capture operator status listen address is invalid")]
    InvalidStatusListen,
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
            Self::InvalidSourceVersion => "capture_config.invalid_source_version",
            Self::InvalidQueueCapacity => "capture_config.invalid_queue_capacity",
            Self::InvalidPayloadLimit => "capture_config.invalid_payload_limit",
            Self::InvalidSourceTrust => "capture_config.invalid_source_trust",
            Self::InvalidSourceAdapter => "capture_config.invalid_source_adapter",
            Self::MissingPrimaryCommittedSource => {
                "capture_config.missing_primary_committed_source"
            }
            Self::DuplicatePrimaryCommittedSource => {
                "capture_config.duplicate_primary_committed_source"
            }
            Self::DuplicateIndependentCommittedSource => {
                "capture_config.duplicate_independent_committed_source"
            }
            Self::InvalidCommittedSourceAdapter => {
                "capture_config.invalid_committed_source_adapter"
            }
            Self::InvalidCredentialPath => "capture_config.invalid_credential_path",
            Self::MissingSources => "capture_config.missing_sources",
            Self::InvalidChainId => "capture_config.invalid_chain_id",
            Self::InvalidRuntimePath => "capture_config.invalid_runtime_path",
            Self::InvalidRuntimeIdentity => "capture_config.invalid_runtime_identity",
            Self::InvalidNatsServer => "capture_config.invalid_nats_server",
            Self::InvalidNatsStream => "capture_config.invalid_nats_stream",
            Self::InvalidRuntimeLimit => "capture_config.invalid_runtime_limit",
            Self::InvalidStatusListen => "capture_config.invalid_status_listen",
            Self::Serialization => "capture_config.serialization",
        }
    }
}

fn validate_runtime_path(path: &Path) -> Result<(), ConfigError> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path.as_os_str().len() > MAX_RUNTIME_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        Err(ConfigError::InvalidRuntimePath)
    } else {
        Ok(())
    }
}

fn validate_nats_server(value: &str) -> Result<(), ConfigError> {
    if value.len() > MAX_NATS_SERVER_BYTES
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(ConfigError::InvalidNatsServer);
    }

    let address = value
        .parse::<async_nats::ServerAddr>()
        .map_err(|_| ConfigError::InvalidNatsServer)?;
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
        return Err(ConfigError::InvalidNatsServer);
    }

    Ok(())
}

fn validate_status_listen(value: &str) -> Result<(), ConfigError> {
    let address: SocketAddr = value
        .parse()
        .map_err(|_| ConfigError::InvalidStatusListen)?;
    if address.ip().is_loopback() {
        Ok(())
    } else {
        Err(ConfigError::InvalidStatusListen)
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

fn validate_node_source_path(path: &Path) -> Result<(), ConfigError> {
    if !path.is_absolute()
        || path == Path::new("/")
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        Err(ConfigError::InvalidSourceAdapter)
    } else {
        Ok(())
    }
}
