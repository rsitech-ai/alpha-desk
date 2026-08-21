use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

use domain_types::{BlockHeight, ChainId, KnownTime, SourceId};
use hl_protocol::node::v1::NodeStreamKind;
use hl_protocol::{
    AgreementStatus, NetworkId, ObservationClass, OperatorKind, ProviderLicense,
    RedistributionPolicy, RetentionClass, SourceAdmission, SourceCatalogError, SourceCatalogRecord,
    SourceDescriptor, SourceRole, SourceTrust, inferred_operator_kind,
    validate_operator_kind_trust, validate_role_trust,
};
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    egress: Vec<EgressConfig>,
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
                SourceTrust::LocallyVerifiedCommitted => match source.observation_class {
                    ObservationClass::CommittedBlock => {
                        primary_committed_sources = primary_committed_sources.saturating_add(1);
                        validate_committed_source_adapter(source)?;
                    }
                    ObservationClass::AuxiliaryOrderStatus
                    | ObservationClass::AuxiliaryBookDiff
                    | ObservationClass::AuxiliaryLedger
                    | ObservationClass::Snapshot
                    | ObservationClass::HistoricalBlock
                    | ObservationClass::PublicMarketData
                    | ObservationClass::ProvisionalFeed
                    | ObservationClass::ProvisionalMempool => {}
                },
                SourceTrust::IndependentCommitted => match source.observation_class {
                    ObservationClass::CommittedBlock => {
                        independent_committed_sources =
                            independent_committed_sources.saturating_add(1);
                        validate_committed_source_adapter(source)?;
                    }
                    ObservationClass::AuxiliaryOrderStatus
                    | ObservationClass::AuxiliaryBookDiff
                    | ObservationClass::AuxiliaryLedger
                    | ObservationClass::Snapshot
                    | ObservationClass::HistoricalBlock
                    | ObservationClass::PublicMarketData
                    | ObservationClass::ProvisionalFeed
                    | ObservationClass::ProvisionalMempool => {}
                },
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
        let mut egress_ids = BTreeSet::new();
        for egress in &self.egress {
            egress.validate()?;
            if !egress_ids.insert(egress.id.as_str()) {
                return Err(ConfigError::DuplicateEgress);
            }
        }
        for source in &self.sources {
            if let Some(SourceAdapterConfig::OfficialInfo { egress_id, .. }) =
                source.adapter.as_ref()
            {
                if source.trust != SourceTrust::ReconciledSnapshot {
                    return Err(ConfigError::InvalidSourceAdapter);
                }
                if !egress_ids.contains(egress_id.as_str()) {
                    return Err(ConfigError::InvalidSourceAdapter);
                }
            }
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

    #[must_use]
    pub fn scheduled_sources(&self, at: KnownTime) -> Vec<&SourceConfig> {
        self.sources
            .iter()
            .filter(|source| source.allows_scheduled_work(at))
            .collect()
    }

    #[must_use]
    pub fn egress(&self) -> &[EgressConfig] {
        &self.egress
    }
}

fn validate_committed_source_adapter(source: &SourceConfig) -> Result<(), ConfigError> {
    match source.adapter.as_ref() {
        Some(SourceAdapterConfig::NodeBlockDirectory { .. }) => Ok(()),
        Some(SourceAdapterConfig::NodeLine { .. } | SourceAdapterConfig::OfficialInfo { .. })
        | None => Err(ConfigError::InvalidCommittedSourceAdapter),
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
        match self.committed_durability {
            DurabilityPolicy::FsyncEveryRecord => Ok(()),
            DurabilityPolicy::Batched {
                max_records: _,
                max_delay_millis: _,
            } => Err(ConfigError::InvalidDurabilityPolicy),
        }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    catalog: Option<SourceCatalogConfig>,
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
        let _ = self.catalog_record()?;
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

    #[must_use]
    pub const fn catalog(&self) -> Option<&SourceCatalogConfig> {
        self.catalog.as_ref()
    }

    pub fn catalog_record(&self) -> Result<Option<SourceCatalogRecord>, ConfigError> {
        let Some(catalog) = &self.catalog else {
            return Ok(None);
        };
        Ok(Some(catalog.record(self)?))
    }

    #[must_use]
    pub fn allows_scheduled_work(&self, at: KnownTime) -> bool {
        match self.catalog_record() {
            Ok(None) => true,
            Ok(Some(record)) => record.allows_scheduled_work(at),
            Err(_) => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceCatalogConfig {
    network: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    role: Option<SourceRole>,
    operator: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operator_kind: Option<OperatorKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    dataset_version: Option<String>,
    retention_class: RetentionClass,
    redistribution: RedistributionPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    license_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agreement_status: Option<AgreementStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agreement_expires_at_micros: Option<i64>,
}

impl SourceCatalogConfig {
    fn record(&self, source: &SourceConfig) -> Result<SourceCatalogRecord, ConfigError> {
        let network = NetworkId::new(self.network.clone()).map_err(catalog_error)?;
        let role = self
            .role
            .unwrap_or_else(|| SourceRole::from_trust(source.trust));
        validate_role_trust(role, source.trust).map_err(catalog_error)?;
        let operator_kind = self
            .operator_kind
            .unwrap_or_else(|| inferred_operator_kind(source.trust, role));
        validate_operator_kind_trust(operator_kind, source.trust).map_err(catalog_error)?;
        let descriptor = SourceDescriptor::new(
            SourceId::new(source.id.clone()).map_err(|_| ConfigError::InvalidSourceId)?,
            network,
            role,
            self.operator.clone(),
            self.dataset_version.clone(),
            self.retention_class,
            self.redistribution,
        )
        .map_err(catalog_error)?;
        let license = match self.license_name.as_ref() {
            Some(name) => Some(
                ProviderLicense::new(
                    name.clone(),
                    self.agreement_status.unwrap_or(AgreementStatus::Active),
                    match self.agreement_expires_at_micros {
                        Some(micros) => Some(
                            KnownTime::from_unix_micros(micros)
                                .map_err(|_| ConfigError::InvalidSourceCatalog)?,
                        ),
                        None => None,
                    },
                )
                .map_err(catalog_error)?,
            ),
            None => None,
        };
        SourceCatalogRecord::new(
            descriptor,
            1,
            operator_kind,
            source.observation_class,
            license,
            KnownTime::from_unix_micros(0).map_err(|_| ConfigError::InvalidSourceCatalog)?,
            None,
        )
        .map_err(catalog_error)
    }

    #[must_use]
    pub fn network(&self) -> &str {
        &self.network
    }

    #[must_use]
    pub const fn role(&self) -> Option<SourceRole> {
        self.role
    }

    #[must_use]
    pub fn operator(&self) -> &str {
        &self.operator
    }

    #[must_use]
    pub const fn operator_kind(&self) -> Option<OperatorKind> {
        self.operator_kind
    }

    #[must_use]
    pub fn dataset_version(&self) -> Option<&str> {
        self.dataset_version.as_deref()
    }

    #[must_use]
    pub const fn retention_class(&self) -> RetentionClass {
        self.retention_class
    }

    #[must_use]
    pub const fn redistribution(&self) -> RedistributionPolicy {
        self.redistribution
    }

    #[must_use]
    pub fn license_name(&self) -> Option<&str> {
        self.license_name.as_deref()
    }

    #[must_use]
    pub const fn agreement_status(&self) -> Option<AgreementStatus> {
        self.agreement_status
    }

    #[must_use]
    pub const fn agreement_expires_at_micros(&self) -> Option<i64> {
        self.agreement_expires_at_micros
    }
}

fn catalog_error(error: SourceCatalogError) -> ConfigError {
    match error {
        SourceCatalogError::IncompatibleRole => ConfigError::IncompatibleSourceRole,
        SourceCatalogError::IncompatibleOperatorKind => ConfigError::IncompatibleOperatorKind,
        SourceCatalogError::MissingProviderLicense => ConfigError::MissingProviderLicense,
        SourceCatalogError::InvalidNetwork
        | SourceCatalogError::InvalidSourceId
        | SourceCatalogError::InvalidOperator
        | SourceCatalogError::InvalidDatasetVersion
        | SourceCatalogError::InvalidLicense
        | SourceCatalogError::MissingCommittedEvidence
        | SourceCatalogError::InvalidVersion
        | SourceCatalogError::InvalidValidityWindow
        | SourceCatalogError::ConflictingIdentity => ConfigError::InvalidSourceCatalog,
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
    OfficialInfo {
        egress_id: String,
        capability_id: String,
        request_timeout_millis: u64,
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
        match self {
            Self::OfficialInfo {
                egress_id,
                capability_id,
                request_timeout_millis,
            } => {
                validate_identity(egress_id).map_err(|_| ConfigError::InvalidSourceAdapter)?;
                validate_identity(capability_id).map_err(|_| ConfigError::InvalidSourceAdapter)?;
                if !(1..=MAX_RUNTIME_TIMEOUT_MILLIS).contains(request_timeout_millis)
                    || observation_class != ObservationClass::Snapshot
                {
                    return Err(ConfigError::InvalidSourceAdapter);
                }
                Ok(())
            }
            Self::NodeLine {
                path,
                stream_name,
                stream,
                poll_interval_millis,
            } => Self::validate_node(
                path,
                stream_name,
                *poll_interval_millis,
                stream.observation_class(),
                observation_class,
                None,
            ),
            Self::NodeBlockDirectory {
                path,
                stream_name,
                start_height: _,
                poll_interval_millis,
                replica_cmds_style,
            } => Self::validate_node(
                path,
                stream_name,
                *poll_interval_millis,
                ObservationClass::CommittedBlock,
                observation_class,
                Some(*replica_cmds_style),
            ),
        }
    }

    fn validate_node(
        path: &Path,
        stream_name: &str,
        poll_interval_millis: u64,
        expected_class: ObservationClass,
        observation_class: ObservationClass,
        replica_cmds_style: Option<NodeReplicaCmdsStyle>,
    ) -> Result<(), ConfigError> {
        validate_node_source_path(path)?;
        validate_identity(stream_name).map_err(|_| ConfigError::InvalidSourceAdapter)?;
        if !(1..=MAX_SOURCE_POLL_INTERVAL_MILLIS).contains(&poll_interval_millis)
            || observation_class != expected_class
        {
            return Err(ConfigError::InvalidSourceAdapter);
        }
        match replica_cmds_style {
            None | Some(NodeReplicaCmdsStyle::ActionsAndResponses) => Ok(()),
            Some(NodeReplicaCmdsStyle::Actions | NodeReplicaCmdsStyle::RecentActions) => {
                Err(ConfigError::InvalidSourceAdapter)
            }
        }
    }
}

const OFFICIAL_REST_WEIGHT_PER_MINUTE: u32 = 1_200;
const MIN_SAFETY_ENVELOPE_PERCENT: u8 = 70;
const MAX_SAFETY_ENVELOPE_PERCENT: u8 = 80;

fn default_priority_reserve_percent() -> u8 {
    40
}

pub const OFFICIAL_INFO_URLS: &[&str] = &[
    "https://api.hyperliquid.xyz",
    "https://api.hyperliquid.xyz/info",
    "https://api.hyperliquid-testnet.xyz",
    "https://api.hyperliquid-testnet.xyz/info",
];

pub const OFFICIAL_INFO_REQUEST_URL: &str = "https://api.hyperliquid.xyz/info";
pub const OFFICIAL_INFO_TESTNET_REQUEST_URL: &str = "https://api.hyperliquid-testnet.xyz/info";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EgressKind {
    OfficialInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EgressProxyConfig {
    #[serde(default)]
    mode: String,
    #[serde(default)]
    rotate: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EgressConfig {
    id: String,
    kind: EgressKind,
    base_url: String,
    weight_per_minute: u32,
    safety_envelope_percent: u8,
    #[serde(default = "default_priority_reserve_percent")]
    priority_reserve_percent: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy: Option<EgressProxyConfig>,
}

impl EgressConfig {
    fn validate(&self) -> Result<(), ConfigError> {
        if !is_safe_source_path_component(&self.id) {
            return Err(ConfigError::InvalidEgress);
        }
        if self.proxy.is_some() {
            return Err(ConfigError::AnonymousProxyRejected);
        }
        match self.kind {
            EgressKind::OfficialInfo => {
                if self.base_url.contains("/exchange") {
                    return Err(ConfigError::ExchangeEndpointForbidden);
                }
                if self.weight_per_minute != OFFICIAL_REST_WEIGHT_PER_MINUTE
                    || !(MIN_SAFETY_ENVELOPE_PERCENT..=MAX_SAFETY_ENVELOPE_PERCENT)
                        .contains(&self.safety_envelope_percent)
                    || !(1..=90).contains(&self.priority_reserve_percent)
                    || !OFFICIAL_INFO_URLS.contains(&self.base_url.as_str())
                {
                    return Err(ConfigError::InvalidEgress);
                }
                Ok(())
            }
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn kind(&self) -> EgressKind {
        self.kind
    }

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    #[must_use]
    pub const fn weight_per_minute(&self) -> u32 {
        self.weight_per_minute
    }

    #[must_use]
    pub const fn safety_envelope_percent(&self) -> u8 {
        self.safety_envelope_percent
    }

    #[must_use]
    pub const fn priority_reserve_percent(&self) -> u8 {
        self.priority_reserve_percent
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
    #[error("capture source catalog is invalid")]
    InvalidSourceCatalog,
    #[error("capture source role is incompatible with source trust")]
    IncompatibleSourceRole,
    #[error("capture operator kind is incompatible with source trust")]
    IncompatibleOperatorKind,
    #[error("provider capture source requires licensing and redistribution policy")]
    MissingProviderLicense,
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
    #[error("capture egress identifier is duplicated")]
    DuplicateEgress,
    #[error("capture egress configuration is invalid")]
    InvalidEgress,
    #[error("anonymous proxy rotation is forbidden")]
    AnonymousProxyRejected,
    #[error("capture refused an /exchange endpoint")]
    ExchangeEndpointForbidden,
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
            Self::InvalidSourceCatalog => "capture_config.invalid_source_catalog",
            Self::IncompatibleSourceRole => "capture_config.incompatible_source_role",
            Self::IncompatibleOperatorKind => "capture_config.incompatible_operator_kind",
            Self::MissingProviderLicense => "capture_config.missing_provider_license",
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
            Self::DuplicateEgress => "capture_config.duplicate_egress",
            Self::InvalidEgress => "capture_config.invalid_egress",
            Self::AnonymousProxyRejected => "capture_config.anonymous_proxy",
            Self::ExchangeEndpointForbidden => "capture_config.exchange_forbidden",
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
