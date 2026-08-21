use domain_types::{KnownTime, SourceId};
use serde::{Deserialize, Serialize};

use crate::{ObservationClass, SourceTrust, observation_qualifies_committed_source};

const MAX_IDENTITY_BYTES: usize = 256;

/// Catalog authority. Distinct from [`SourceTrust`], which is the completeness contract.
///
/// Every variant maps onto exactly one canonical [`SourceTrust`]. Mempool sources keep
/// `SourceTrust::MempoolProvisional` while still carrying [`SourceRole::ProvisionalRealtime`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SourceRole {
    CommittedPrimary,
    CommittedIndependent,
    ProvisionalRealtime,
    ReconciliationSnapshot,
    HistoricalBackfill,
    AttributionEnrichment,
    DiscoveryOnly,
}

impl SourceRole {
    pub const ALL: [Self; 7] = [
        Self::CommittedPrimary,
        Self::CommittedIndependent,
        Self::ProvisionalRealtime,
        Self::ReconciliationSnapshot,
        Self::HistoricalBackfill,
        Self::AttributionEnrichment,
        Self::DiscoveryOnly,
    ];

    #[must_use]
    pub const fn trust(self) -> SourceTrust {
        match self {
            Self::CommittedPrimary => SourceTrust::LocallyVerifiedCommitted,
            Self::CommittedIndependent => SourceTrust::IndependentCommitted,
            Self::ProvisionalRealtime => SourceTrust::ThirdPartyProvisional,
            Self::ReconciliationSnapshot => SourceTrust::ReconciledSnapshot,
            Self::HistoricalBackfill => SourceTrust::RecoveryOnly,
            Self::AttributionEnrichment | Self::DiscoveryOnly => SourceTrust::ThirdPartyProvisional,
        }
    }

    #[must_use]
    pub const fn from_trust(trust: SourceTrust) -> Self {
        match trust {
            SourceTrust::LocallyVerifiedCommitted => Self::CommittedPrimary,
            SourceTrust::IndependentCommitted => Self::CommittedIndependent,
            SourceTrust::ThirdPartyProvisional | SourceTrust::MempoolProvisional => {
                Self::ProvisionalRealtime
            }
            SourceTrust::ReconciledSnapshot => Self::ReconciliationSnapshot,
            SourceTrust::RecoveryOnly => Self::HistoricalBackfill,
        }
    }

    #[must_use]
    pub const fn compatible_with(self, trust: SourceTrust) -> bool {
        match self {
            Self::CommittedPrimary => matches!(trust, SourceTrust::LocallyVerifiedCommitted),
            Self::CommittedIndependent => matches!(trust, SourceTrust::IndependentCommitted),
            Self::ProvisionalRealtime => matches!(
                trust,
                SourceTrust::ThirdPartyProvisional | SourceTrust::MempoolProvisional
            ),
            Self::ReconciliationSnapshot => matches!(trust, SourceTrust::ReconciledSnapshot),
            Self::HistoricalBackfill => matches!(trust, SourceTrust::RecoveryOnly),
            Self::AttributionEnrichment | Self::DiscoveryOnly => {
                matches!(trust, SourceTrust::ThirdPartyProvisional)
            }
        }
    }

    #[must_use]
    pub const fn is_committed(self) -> bool {
        matches!(self, Self::CommittedPrimary | Self::CommittedIndependent)
    }

    #[must_use]
    pub const fn default_operator_kind(self) -> OperatorKind {
        match self {
            Self::CommittedPrimary => OperatorKind::LocalNode,
            Self::CommittedIndependent => OperatorKind::IndependentNode,
            Self::AttributionEnrichment => OperatorKind::Provider,
            Self::DiscoveryOnly => OperatorKind::Community,
            Self::ProvisionalRealtime | Self::ReconciliationSnapshot | Self::HistoricalBackfill => {
                OperatorKind::Official
            }
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CommittedPrimary => "committed-primary",
            Self::CommittedIndependent => "committed-independent",
            Self::ProvisionalRealtime => "provisional-realtime",
            Self::ReconciliationSnapshot => "reconciliation-snapshot",
            Self::HistoricalBackfill => "historical-backfill",
            Self::AttributionEnrichment => "attribution-enrichment",
            Self::DiscoveryOnly => "discovery-only",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OperatorKind {
    LocalNode,
    IndependentNode,
    Official,
    Provider,
    Community,
}

impl OperatorKind {
    pub const ALL: [Self; 5] = [
        Self::LocalNode,
        Self::IndependentNode,
        Self::Official,
        Self::Provider,
        Self::Community,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LocalNode => "local-node",
            Self::IndependentNode => "independent-node",
            Self::Official => "official",
            Self::Provider => "provider",
            Self::Community => "community",
        }
    }

    #[must_use]
    pub const fn compatible_with(self, trust: SourceTrust) -> bool {
        match self {
            Self::LocalNode | Self::IndependentNode | Self::Official => matches!(
                trust,
                SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted
            ),
            Self::Provider => matches!(
                trust,
                SourceTrust::ThirdPartyProvisional
                    | SourceTrust::MempoolProvisional
                    | SourceTrust::ReconciledSnapshot
                    | SourceTrust::RecoveryOnly
            ),
            Self::Community => matches!(trust, SourceTrust::ThirdPartyProvisional),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RetentionClass {
    RawIndefinite,
    RawHotLocal,
    RawWarmObject,
    CompactedCanonical,
    UnknownQuarantine,
}

impl RetentionClass {
    pub const ALL: [Self; 5] = [
        Self::RawIndefinite,
        Self::RawHotLocal,
        Self::RawWarmObject,
        Self::CompactedCanonical,
        Self::UnknownQuarantine,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RawIndefinite => "raw-indefinite",
            Self::RawHotLocal => "raw-hot-local",
            Self::RawWarmObject => "raw-warm-object",
            Self::CompactedCanonical => "compacted-canonical",
            Self::UnknownQuarantine => "unknown-quarantine",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RedistributionPolicy {
    PrivateOperatorEvidence,
    InternalOnly,
    FieldRestricted,
    Redistributable,
}

impl RedistributionPolicy {
    pub const ALL: [Self; 4] = [
        Self::PrivateOperatorEvidence,
        Self::InternalOnly,
        Self::FieldRestricted,
        Self::Redistributable,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PrivateOperatorEvidence => "private-operator-evidence",
            Self::InternalOnly => "internal-only",
            Self::FieldRestricted => "field-restricted",
            Self::Redistributable => "redistributable",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgreementStatus {
    Active,
    Disabled,
    Expired,
}

impl AgreementStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Disabled => "disabled",
            Self::Expired => "expired",
        }
    }

    #[must_use]
    pub const fn allows_scheduled_work(self) -> bool {
        matches!(self, Self::Active)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NetworkId(String);

impl NetworkId {
    pub fn new(value: impl Into<String>) -> Result<Self, SourceCatalogError> {
        let value = value.into();
        validate_scoped_identity(&value).map_err(|()| SourceCatalogError::InvalidNetwork)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for NetworkId {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderLicense {
    license_name: String,
    status: AgreementStatus,
    expires_at: Option<KnownTime>,
}

impl ProviderLicense {
    pub fn new(
        license_name: impl Into<String>,
        status: AgreementStatus,
        expires_at: Option<KnownTime>,
    ) -> Result<Self, SourceCatalogError> {
        let license_name = license_name.into();
        validate_identity(&license_name).map_err(|()| SourceCatalogError::InvalidLicense)?;
        Ok(Self {
            license_name,
            status,
            expires_at,
        })
    }

    #[must_use]
    pub fn license_name(&self) -> &str {
        &self.license_name
    }

    #[must_use]
    pub const fn status(&self) -> AgreementStatus {
        self.status
    }

    #[must_use]
    pub const fn expires_at(&self) -> Option<KnownTime> {
        self.expires_at
    }

    #[must_use]
    pub fn allows_scheduled_work(&self, at: KnownTime) -> bool {
        if !self.status.allows_scheduled_work() {
            return false;
        }
        match self.expires_at {
            Some(expires_at) if expires_at.unix_micros() <= at.unix_micros() => false,
            Some(_) | None => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceDescriptor {
    source_id: SourceId,
    network: NetworkId,
    role: SourceRole,
    operator: String,
    dataset_version: Option<String>,
    retention_class: RetentionClass,
    redistribution: RedistributionPolicy,
}

impl SourceDescriptor {
    pub fn new(
        source_id: SourceId,
        network: NetworkId,
        role: SourceRole,
        operator: impl Into<String>,
        dataset_version: Option<String>,
        retention_class: RetentionClass,
        redistribution: RedistributionPolicy,
    ) -> Result<Self, SourceCatalogError> {
        if source_id.as_str().contains(':') {
            return Err(SourceCatalogError::InvalidSourceId);
        }
        let operator = operator.into();
        validate_identity(&operator).map_err(|()| SourceCatalogError::InvalidOperator)?;
        let dataset_version = match dataset_version {
            Some(value) => {
                validate_identity(&value)
                    .map_err(|()| SourceCatalogError::InvalidDatasetVersion)?;
                Some(value)
            }
            None => None,
        };
        Ok(Self {
            source_id,
            network,
            role,
            operator,
            dataset_version,
            retention_class,
            redistribution,
        })
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub const fn network(&self) -> &NetworkId {
        &self.network
    }

    #[must_use]
    pub const fn role(&self) -> SourceRole {
        self.role
    }

    #[must_use]
    pub fn operator(&self) -> &str {
        &self.operator
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
    pub fn stable_id(&self) -> String {
        network_scoped_source_identity(&self.network, &self.source_id)
    }
}

#[must_use]
pub fn network_scoped_source_identity(network: &NetworkId, source_id: &SourceId) -> String {
    format!("{}:{}", network.as_str(), source_id.as_str())
}

pub fn validate_role_trust(role: SourceRole, trust: SourceTrust) -> Result<(), SourceCatalogError> {
    if role.compatible_with(trust) {
        Ok(())
    } else {
        Err(SourceCatalogError::IncompatibleRole)
    }
}

pub fn validate_operator_kind_trust(
    operator_kind: OperatorKind,
    trust: SourceTrust,
) -> Result<(), SourceCatalogError> {
    if operator_kind.compatible_with(trust) {
        Ok(())
    } else {
        Err(SourceCatalogError::IncompatibleOperatorKind)
    }
}

#[must_use]
pub const fn role_requires_provider_license(role: SourceRole, operator_kind: OperatorKind) -> bool {
    matches!(role, SourceRole::AttributionEnrichment)
        || matches!(operator_kind, OperatorKind::Provider)
}

/// Operator kind used when capture TOML omits `operator_kind`.
///
/// Every non-committed trust infers [`OperatorKind::Provider`] so an omitted
/// `catalog.role` cannot become Official and skip the provider license.
/// [`SourceRole::DiscoveryOnly`] stays Community. Committed trusts keep
/// [`SourceRole::default_operator_kind`].
#[must_use]
pub const fn inferred_operator_kind(trust: SourceTrust, role: SourceRole) -> OperatorKind {
    match trust {
        SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted => {
            role.default_operator_kind()
        }
        SourceTrust::ThirdPartyProvisional
        | SourceTrust::MempoolProvisional
        | SourceTrust::ReconciledSnapshot
        | SourceTrust::RecoveryOnly => match role {
            SourceRole::DiscoveryOnly => OperatorKind::Community,
            SourceRole::CommittedPrimary
            | SourceRole::CommittedIndependent
            | SourceRole::ProvisionalRealtime
            | SourceRole::ReconciliationSnapshot
            | SourceRole::HistoricalBackfill
            | SourceRole::AttributionEnrichment => OperatorKind::Provider,
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceCatalogRecord {
    descriptor: SourceDescriptor,
    version: u32,
    operator_kind: OperatorKind,
    evidence_class: ObservationClass,
    license: Option<ProviderLicense>,
    valid_from: KnownTime,
    valid_to: Option<KnownTime>,
}

impl SourceCatalogRecord {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        descriptor: SourceDescriptor,
        version: u32,
        operator_kind: OperatorKind,
        evidence_class: ObservationClass,
        license: Option<ProviderLicense>,
        valid_from: KnownTime,
        valid_to: Option<KnownTime>,
    ) -> Result<Self, SourceCatalogError> {
        if version == 0 {
            return Err(SourceCatalogError::InvalidVersion);
        }
        if let Some(end) = valid_to
            && end.unix_micros() <= valid_from.unix_micros()
        {
            return Err(SourceCatalogError::InvalidValidityWindow);
        }
        if descriptor.role().is_committed()
            && !observation_qualifies_committed_source(evidence_class)
        {
            return Err(SourceCatalogError::MissingCommittedEvidence);
        }
        validate_operator_kind_trust(operator_kind, descriptor.role().trust())?;
        let requires_license = role_requires_provider_license(descriptor.role(), operator_kind);
        if requires_license {
            if license.is_none() {
                return Err(SourceCatalogError::MissingProviderLicense);
            }
        } else if license.is_some() {
            return Err(SourceCatalogError::InvalidLicense);
        }
        Ok(Self {
            descriptor,
            version,
            operator_kind,
            evidence_class,
            license,
            valid_from,
            valid_to,
        })
    }

    #[must_use]
    pub const fn descriptor(&self) -> &SourceDescriptor {
        &self.descriptor
    }

    #[must_use]
    pub const fn version(&self) -> u32 {
        self.version
    }

    #[must_use]
    pub const fn operator_kind(&self) -> OperatorKind {
        self.operator_kind
    }

    #[must_use]
    pub const fn evidence_class(&self) -> ObservationClass {
        self.evidence_class
    }

    #[must_use]
    pub const fn license(&self) -> Option<&ProviderLicense> {
        self.license.as_ref()
    }

    #[must_use]
    pub const fn valid_from(&self) -> KnownTime {
        self.valid_from
    }

    #[must_use]
    pub const fn valid_to(&self) -> Option<KnownTime> {
        self.valid_to
    }

    #[must_use]
    pub fn is_current(&self) -> bool {
        self.valid_to.is_none()
    }

    #[must_use]
    pub fn allows_scheduled_work(&self, at: KnownTime) -> bool {
        if !self.is_current() {
            return false;
        }
        match &self.license {
            Some(license) => license.allows_scheduled_work(at),
            None => true,
        }
    }

    pub fn with_closed_validity(&self, valid_to: KnownTime) -> Result<Self, SourceCatalogError> {
        Self::new(
            self.descriptor.clone(),
            self.version,
            self.operator_kind,
            self.evidence_class,
            self.license.clone(),
            self.valid_from,
            Some(valid_to),
        )
    }

    pub fn successor(
        &self,
        descriptor: SourceDescriptor,
        operator_kind: OperatorKind,
        evidence_class: ObservationClass,
        license: Option<ProviderLicense>,
        valid_from: KnownTime,
    ) -> Result<Self, SourceCatalogError> {
        let next_version = self
            .version
            .checked_add(1)
            .ok_or(SourceCatalogError::InvalidVersion)?;
        if valid_from.unix_micros() <= self.valid_from.unix_micros() {
            return Err(SourceCatalogError::InvalidValidityWindow);
        }
        if descriptor.network() != self.descriptor.network()
            || descriptor.source_id() != self.descriptor.source_id()
        {
            return Err(SourceCatalogError::ConflictingIdentity);
        }
        Self::new(
            descriptor,
            next_version,
            operator_kind,
            evidence_class,
            license,
            valid_from,
            None,
        )
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum SourceCatalogError {
    #[error("source catalog network is invalid")]
    InvalidNetwork,
    #[error("source catalog source id is invalid")]
    InvalidSourceId,
    #[error("source catalog operator is invalid")]
    InvalidOperator,
    #[error("source catalog dataset version is invalid")]
    InvalidDatasetVersion,
    #[error("source catalog license is invalid")]
    InvalidLicense,
    #[error("source role is incompatible with source trust")]
    IncompatibleRole,
    #[error("operator kind is incompatible with source trust")]
    IncompatibleOperatorKind,
    #[error("committed source requires a qualifying evidence class")]
    MissingCommittedEvidence,
    #[error("provider source requires licensing and redistribution policy")]
    MissingProviderLicense,
    #[error("source catalog version is invalid")]
    InvalidVersion,
    #[error("source catalog validity window is invalid")]
    InvalidValidityWindow,
    #[error("source catalog identity does not match the prior record")]
    ConflictingIdentity,
}

impl SourceCatalogError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidNetwork => "source_catalog.invalid_network",
            Self::InvalidSourceId => "source_catalog.invalid_source_id",
            Self::InvalidOperator => "source_catalog.invalid_operator",
            Self::InvalidDatasetVersion => "source_catalog.invalid_dataset_version",
            Self::InvalidLicense => "source_catalog.invalid_license",
            Self::IncompatibleRole => "source_catalog.incompatible_role",
            Self::IncompatibleOperatorKind => "source_catalog.incompatible_operator_kind",
            Self::MissingCommittedEvidence => "source_catalog.missing_committed_evidence",
            Self::MissingProviderLicense => "source_catalog.missing_provider_license",
            Self::InvalidVersion => "source_catalog.invalid_version",
            Self::InvalidValidityWindow => "source_catalog.invalid_validity_window",
            Self::ConflictingIdentity => "source_catalog.conflicting_identity",
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

fn validate_scoped_identity(value: &str) -> Result<(), ()> {
    validate_identity(value)?;
    if value.contains(':') { Err(()) } else { Ok(()) }
}
