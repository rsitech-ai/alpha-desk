use api_contracts::WireCanonicalSnapshotEnvelope;
use domain_types::{BlockHeight, ChainId, KnownTime};
use semver::Version;

use crate::{ContractError, HASH_LENGTH, SCHEMA_MAJOR};

pub const SNAPSHOT_ENVELOPE_SCHEMA: &str = "1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotClass {
    Reconciled,
    Reference,
}

impl SnapshotClass {
    const fn wire_value(self) -> i32 {
        match self {
            Self::Reconciled => 1,
            Self::Reference => 2,
        }
    }

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Reconciled => "reconciled",
            Self::Reference => "reference",
        }
    }
}

impl TryFrom<i32> for SnapshotClass {
    type Error = ContractError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Reconciled),
            2 => Ok(Self::Reference),
            other => Err(ContractError::Invalid {
                field: "snapshot_class",
                reason: format!("unknown numeric value {other}"),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotFamily {
    MarketContext,
    AccountClearinghouse,
    AccountAllDex,
    SpotState,
    OpenOrders,
    TwapState,
    VaultDetails,
    FeeReferral,
    StakingState,
    BorrowLendState,
    RoleAbstraction,
    QuoteAlignment,
    ProviderPositions,
    EvmPrecompile,
}

impl SnapshotFamily {
    pub const ALL: [Self; 14] = [
        Self::MarketContext,
        Self::AccountClearinghouse,
        Self::AccountAllDex,
        Self::SpotState,
        Self::OpenOrders,
        Self::TwapState,
        Self::VaultDetails,
        Self::FeeReferral,
        Self::StakingState,
        Self::BorrowLendState,
        Self::RoleAbstraction,
        Self::QuoteAlignment,
        Self::ProviderPositions,
        Self::EvmPrecompile,
    ];

    const fn wire_value(self) -> i32 {
        match self {
            Self::MarketContext => 1,
            Self::AccountClearinghouse => 2,
            Self::AccountAllDex => 3,
            Self::SpotState => 4,
            Self::OpenOrders => 5,
            Self::TwapState => 6,
            Self::VaultDetails => 7,
            Self::FeeReferral => 8,
            Self::StakingState => 9,
            Self::BorrowLendState => 10,
            Self::RoleAbstraction => 11,
            Self::QuoteAlignment => 12,
            Self::ProviderPositions => 13,
            Self::EvmPrecompile => 14,
        }
    }

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::MarketContext => "MarketContext",
            Self::AccountClearinghouse => "AccountClearinghouse",
            Self::AccountAllDex => "AccountAllDex",
            Self::SpotState => "SpotState",
            Self::OpenOrders => "OpenOrders",
            Self::TwapState => "TwapState",
            Self::VaultDetails => "VaultDetails",
            Self::FeeReferral => "FeeReferral",
            Self::StakingState => "StakingState",
            Self::BorrowLendState => "BorrowLendState",
            Self::RoleAbstraction => "RoleAbstraction",
            Self::QuoteAlignment => "QuoteAlignment",
            Self::ProviderPositions => "ProviderPositions",
            Self::EvmPrecompile => "EvmPrecompile",
        }
    }
}

impl TryFrom<i32> for SnapshotFamily {
    type Error = ContractError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::MarketContext),
            2 => Ok(Self::AccountClearinghouse),
            3 => Ok(Self::AccountAllDex),
            4 => Ok(Self::SpotState),
            5 => Ok(Self::OpenOrders),
            6 => Ok(Self::TwapState),
            7 => Ok(Self::VaultDetails),
            8 => Ok(Self::FeeReferral),
            9 => Ok(Self::StakingState),
            10 => Ok(Self::BorrowLendState),
            11 => Ok(Self::RoleAbstraction),
            12 => Ok(Self::QuoteAlignment),
            13 => Ok(Self::ProviderPositions),
            14 => Ok(Self::EvmPrecompile),
            other => Err(ContractError::Invalid {
                field: "snapshot_family",
                reason: format!("unknown numeric value {other}"),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalSnapshotEnvelope {
    schema_version: String,
    family: SnapshotFamily,
    class: SnapshotClass,
    chain_id: ChainId,
    as_of_block: Option<BlockHeight>,
    observed_at: KnownTime,
    payload_hash: [u8; HASH_LENGTH],
    parser_version: String,
    payload: Vec<u8>,
}

impl CanonicalSnapshotEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        schema_version: &str,
        family: SnapshotFamily,
        class: SnapshotClass,
        chain_id: ChainId,
        as_of_block: Option<BlockHeight>,
        observed_at: KnownTime,
        parser_version: impl Into<String>,
        payload: Vec<u8>,
    ) -> Result<Self, ContractError> {
        validate_snapshot_schema(schema_version)?;
        let parser_version = parser_version.into();
        if parser_version.is_empty() {
            return Err(ContractError::Missing("parser_version"));
        }
        if payload.is_empty() {
            return Err(ContractError::Missing("payload"));
        }
        let payload_hash = *blake3::hash(&payload).as_bytes();
        Ok(Self {
            schema_version: schema_version.to_owned(),
            family,
            class,
            chain_id,
            as_of_block,
            observed_at,
            payload_hash,
            parser_version,
            payload,
        })
    }

    pub fn encode_to_vec(&self) -> Vec<u8> {
        WireCanonicalSnapshotEnvelope {
            schema_version: self.schema_version.clone(),
            family: self.family.wire_value(),
            class: self.class.wire_value(),
            chain_id: self.chain_id.to_string(),
            as_of_block: self.as_of_block.map(BlockHeight::get),
            observed_at_micros: self.observed_at.unix_micros(),
            payload_hash: self.payload_hash.to_vec(),
            parser_version: self.parser_version.clone(),
            payload: self.payload.clone(),
        }
        .encode_to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, ContractError> {
        let wire = WireCanonicalSnapshotEnvelope::decode(bytes)?;
        let payload_hash =
            wire.payload_hash
                .as_slice()
                .try_into()
                .map_err(|_| ContractError::Invalid {
                    field: "payload_hash",
                    reason: format!("expected {HASH_LENGTH} bytes"),
                })?;
        if payload_hash != *blake3::hash(&wire.payload).as_bytes() {
            return Err(ContractError::Invalid {
                field: "payload_hash",
                reason: "does not match the snapshot payload bytes".to_owned(),
            });
        }
        if wire.payload.is_empty() {
            return Err(ContractError::Missing("payload"));
        }
        let parser_version = if wire.parser_version.is_empty() {
            return Err(ContractError::Missing("parser_version"));
        } else {
            wire.parser_version
        };
        validate_snapshot_schema(&wire.schema_version)?;
        Ok(Self {
            schema_version: wire.schema_version,
            family: SnapshotFamily::try_from(wire.family)?,
            class: SnapshotClass::try_from(wire.class)?,
            chain_id: ChainId::new(wire.chain_id).map_err(|error| ContractError::Invalid {
                field: "chain_id",
                reason: error.to_string(),
            })?,
            as_of_block: wire.as_of_block.map(BlockHeight::new),
            observed_at: KnownTime::from_unix_micros(wire.observed_at_micros).map_err(|error| {
                ContractError::Invalid {
                    field: "observed_at_micros",
                    reason: error.to_string(),
                }
            })?,
            payload_hash,
            parser_version,
            payload: wire.payload,
        })
    }

    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    #[must_use]
    pub const fn family(&self) -> SnapshotFamily {
        self.family
    }

    #[must_use]
    pub const fn class(&self) -> SnapshotClass {
        self.class
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn as_of_block(&self) -> Option<BlockHeight> {
        self.as_of_block
    }

    #[must_use]
    pub const fn observed_at(&self) -> KnownTime {
        self.observed_at
    }

    #[must_use]
    pub fn payload_hash(&self) -> [u8; HASH_LENGTH] {
        self.payload_hash
    }

    #[must_use]
    pub fn parser_version(&self) -> &str {
        &self.parser_version
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

pub fn admit_snapshot_as_ledger_transition(
    snapshot: &CanonicalSnapshotEnvelope,
) -> Result<crate::CanonicalEventEnvelope, ContractError> {
    let _ = snapshot;
    Err(ContractError::Invalid {
        field: "event_kind",
        reason: "snapshots are not ledger transitions".to_owned(),
    })
}

fn validate_snapshot_schema(value: &str) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(ContractError::Missing("schema_version"));
    }
    let version = Version::parse(value).map_err(|error| ContractError::Invalid {
        field: "schema_version",
        reason: format!("expected canonical numeric MAJOR.MINOR.PATCH: {error}"),
    })?;
    if !version.pre.is_empty() || !version.build.is_empty() {
        return Err(ContractError::Invalid {
            field: "schema_version",
            reason: "pre-release and build metadata are forbidden".to_owned(),
        });
    }
    if version.major != SCHEMA_MAJOR {
        return Err(ContractError::UnsupportedSchema(value.to_owned()));
    }
    Ok(())
}
