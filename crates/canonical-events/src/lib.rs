#![forbid(unsafe_code)]

use api_contracts::{WireCanonicalEventEnvelope, WireSourceEvidence};
use domain_types::{
    AccountId, BlockHeight, ChainId, EventId, MarketId, ProtocolTime, SourceId, TransactionId,
};

pub const SCHEMA_MAJOR: u64 = 1;
const HASH_LENGTH: usize = 32;

#[derive(Debug, thiserror::Error)]
pub enum ContractError {
    #[error("unsupported schema version {0}")]
    UnsupportedSchema(String),
    #[error("missing required field {0}")]
    Missing(&'static str),
    #[error("invalid field {field}: {reason}")]
    Invalid { field: &'static str, reason: String },
    #[error("wire decode failed: {0}")]
    Decode(#[from] prost::DecodeError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationClass {
    ProvisionalSource,
    CommittedPrimary,
    CommittedIndependent,
    ReconciledSnapshot,
    Corrected,
    Expired,
}

impl ConfirmationClass {
    const fn wire_value(self) -> i32 {
        match self {
            Self::ProvisionalSource => 1,
            Self::CommittedPrimary => 2,
            Self::CommittedIndependent => 3,
            Self::ReconciledSnapshot => 4,
            Self::Corrected => 5,
            Self::Expired => 6,
        }
    }
}

impl TryFrom<i32> for ConfirmationClass {
    type Error = ContractError;

    fn try_from(value: i32) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::ProvisionalSource),
            2 => Ok(Self::CommittedPrimary),
            3 => Ok(Self::CommittedIndependent),
            4 => Ok(Self::ReconciledSnapshot),
            5 => Ok(Self::Corrected),
            6 => Ok(Self::Expired),
            other => Err(ContractError::Invalid {
                field: "confirmation_class",
                reason: format!("unknown numeric value {other}"),
            }),
        }
    }
}

macro_rules! event_kinds {
    ($($kind:ident),+ $(,)?) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub enum EventKind {
            $($kind),+
        }

        impl EventKind {
            pub const ALL: [Self; event_kinds!(@count $($kind),+)] = [
                $(Self::$kind),+
            ];

            #[must_use]
            pub const fn as_wire_name(self) -> &'static str {
                match self {
                    $(Self::$kind => stringify!($kind)),+
                }
            }
        }

        impl TryFrom<&str> for EventKind {
            type Error = ContractError;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                match value {
                    $(stringify!($kind) => Ok(Self::$kind)),+,
                    other => Err(ContractError::Invalid {
                        field: "event_kind",
                        reason: format!("unknown event kind {other}"),
                    }),
                }
            }
        }
    };
    (@count $($kind:ident),+) => {
        <[()]>::len(&[$(event_kinds!(@unit $kind)),+])
    };
    (@unit $kind:ident) => { () };
}

event_kinds!(
    OrderAccepted,
    OrderRested,
    OrderModified,
    OrderPartiallyFilled,
    OrderFilled,
    OrderCancelled,
    OrderRejected,
    TriggerOrderActivated,
    TwapStarted,
    TwapSliceFilled,
    TwapCompleted,
    TradeMatched,
    DepositCredited,
    WithdrawalDebited,
    SpotTransfer,
    PerpTransfer,
    SubaccountTransfer,
    VaultDeposit,
    VaultWithdrawal,
    FeeCharged,
    BuilderFeeCharged,
    FundingPaid,
    FundingReceived,
    ReferralReward,
    AccountModeChanged,
    MarginModeChanged,
    LeverageChanged,
    LiquidationStarted,
    LiquidationFill,
    BackstopLiquidation,
    PositionSettled,
    MarketHalted,
    MarketResumed,
    OpenInterestCapChanged,
    MarginTableChanged,
    MarketCreated,
    MarketMetadataChanged,
    OracleUpdated,
    FundingRateUpdated,
    AssetContextUpdated,
    DexCreated,
    OutcomeCreated,
    OutcomeResolved,
);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidence {
    source_id: SourceId,
    source_version: String,
    source_offset: String,
    content_hash: [u8; HASH_LENGTH],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventOrderingKey<'a> {
    pub chain_id: &'a str,
    pub block_height: u64,
    pub transaction_index: u32,
    pub event_index: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEventEnvelope {
    schema_version: String,
    chain_id: ChainId,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    transaction_id: TransactionId,
    transaction_index: u32,
    event_index: u32,
    event_id: EventId,
    event_kind: EventKind,
    market_ids: Vec<MarketId>,
    account_ids: Vec<AccountId>,
    source_evidence: Vec<SourceEvidence>,
    confirmation_class: ConfirmationClass,
    observed_at: ProtocolTime,
    ingested_at: ProtocolTime,
    canonicalized_at: ProtocolTime,
    payload_hash: [u8; HASH_LENGTH],
    parser_version: String,
    payload: Vec<u8>,
}

impl CanonicalEventEnvelope {
    pub fn decode(bytes: &[u8]) -> Result<Self, ContractError> {
        WireCanonicalEventEnvelope::decode(bytes)?.try_into()
    }

    pub fn encode_to_vec(&self) -> Result<Vec<u8>, ContractError> {
        Ok(WireCanonicalEventEnvelope::from(self).encode_to_vec())
    }

    pub fn fixture() -> Result<Self, ContractError> {
        WireCanonicalEventEnvelope {
            schema_version: "1.0.0".to_owned(),
            chain_id: "hyperliquid-mainnet".to_owned(),
            block_height: 42,
            block_time_micros: 1_700_000_000_000_000,
            transaction_id: "tx-42".to_owned(),
            transaction_index: 7,
            event_index: 9,
            event_id: "event-42-7-9".to_owned(),
            event_kind: "TradeMatched".to_owned(),
            market_ids: vec!["BTC-USD".to_owned()],
            account_ids: vec!["0xaccount".to_owned()],
            source_evidence: vec![WireSourceEvidence {
                source_id: "primary-node".to_owned(),
                source_version: "2026.07".to_owned(),
                source_offset: "block:42/event:9".to_owned(),
                content_hash: vec![0x11; HASH_LENGTH],
            }],
            confirmation_class: 2,
            observed_at_micros: 1_700_000_000_000_010,
            ingested_at_micros: 1_700_000_000_000_020,
            canonicalized_at_micros: 1_700_000_000_000_030,
            payload_hash: vec![0x22; HASH_LENGTH],
            parser_version: "parser-v1".to_owned(),
            payload: vec![0x0a, 0x01, 0x01],
        }
        .try_into()
    }

    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    #[must_use]
    pub const fn confirmation_class(&self) -> ConfirmationClass {
        self.confirmation_class
    }

    #[must_use]
    pub fn ordering_key(&self) -> EventOrderingKey<'_> {
        EventOrderingKey {
            chain_id: self.chain_id.as_str(),
            block_height: self.block_height.get(),
            transaction_index: self.transaction_index,
            event_index: self.event_index,
        }
    }
}

impl TryFrom<WireCanonicalEventEnvelope> for CanonicalEventEnvelope {
    type Error = ContractError;

    fn try_from(value: WireCanonicalEventEnvelope) -> Result<Self, Self::Error> {
        validate_schema_version(&value.schema_version)?;
        if value.source_evidence.is_empty() {
            return Err(ContractError::Missing("source_evidence"));
        }

        Ok(Self {
            schema_version: required(value.schema_version, "schema_version")?,
            chain_id: parse_id(value.chain_id, "chain_id", ChainId::new)?,
            block_height: BlockHeight::new(value.block_height),
            block_time: parse_time(value.block_time_micros, "block_time_micros")?,
            transaction_id: parse_id(value.transaction_id, "transaction_id", TransactionId::new)?,
            transaction_index: value.transaction_index,
            event_index: value.event_index,
            event_id: parse_id(value.event_id, "event_id", EventId::new)?,
            event_kind: EventKind::try_from(required(value.event_kind, "event_kind")?.as_str())?,
            market_ids: value
                .market_ids
                .into_iter()
                .map(|id| parse_list_id(id, "market_ids", MarketId::new))
                .collect::<Result<_, _>>()?,
            account_ids: value
                .account_ids
                .into_iter()
                .map(|id| parse_list_id(id, "account_ids", AccountId::new))
                .collect::<Result<_, _>>()?,
            source_evidence: value
                .source_evidence
                .into_iter()
                .map(SourceEvidence::try_from)
                .collect::<Result<_, _>>()?,
            confirmation_class: value.confirmation_class.try_into()?,
            observed_at: parse_time(value.observed_at_micros, "observed_at_micros")?,
            ingested_at: parse_time(value.ingested_at_micros, "ingested_at_micros")?,
            canonicalized_at: parse_time(value.canonicalized_at_micros, "canonicalized_at_micros")?,
            payload_hash: fixed_hash(value.payload_hash, "payload_hash")?,
            parser_version: required(value.parser_version, "parser_version")?,
            payload: required_bytes(value.payload, "payload")?,
        })
    }
}

impl TryFrom<WireSourceEvidence> for SourceEvidence {
    type Error = ContractError;

    fn try_from(value: WireSourceEvidence) -> Result<Self, Self::Error> {
        Ok(Self {
            source_id: parse_id(value.source_id, "source_evidence.source_id", SourceId::new)?,
            source_version: required(value.source_version, "source_evidence.source_version")?,
            source_offset: required(value.source_offset, "source_evidence.source_offset")?,
            content_hash: fixed_hash(value.content_hash, "source_evidence.content_hash")?,
        })
    }
}

impl From<&CanonicalEventEnvelope> for WireCanonicalEventEnvelope {
    fn from(value: &CanonicalEventEnvelope) -> Self {
        Self {
            schema_version: value.schema_version.clone(),
            chain_id: value.chain_id.to_string(),
            block_height: value.block_height.get(),
            block_time_micros: value.block_time.unix_micros(),
            transaction_id: value.transaction_id.to_string(),
            transaction_index: value.transaction_index,
            event_index: value.event_index,
            event_id: value.event_id.to_string(),
            event_kind: value.event_kind.as_wire_name().to_owned(),
            market_ids: value.market_ids.iter().map(ToString::to_string).collect(),
            account_ids: value.account_ids.iter().map(ToString::to_string).collect(),
            source_evidence: value.source_evidence.iter().map(Into::into).collect(),
            confirmation_class: value.confirmation_class.wire_value(),
            observed_at_micros: value.observed_at.unix_micros(),
            ingested_at_micros: value.ingested_at.unix_micros(),
            canonicalized_at_micros: value.canonicalized_at.unix_micros(),
            payload_hash: value.payload_hash.to_vec(),
            parser_version: value.parser_version.clone(),
            payload: value.payload.clone(),
        }
    }
}

impl From<&SourceEvidence> for WireSourceEvidence {
    fn from(value: &SourceEvidence) -> Self {
        Self {
            source_id: value.source_id.to_string(),
            source_version: value.source_version.clone(),
            source_offset: value.source_offset.clone(),
            content_hash: value.content_hash.to_vec(),
        }
    }
}

fn validate_schema_version(value: &str) -> Result<(), ContractError> {
    if value.is_empty() {
        return Err(ContractError::Missing("schema_version"));
    }
    let components = value.split('.').collect::<Vec<_>>();
    if components.len() != 3
        || components
            .iter()
            .any(|component| component.is_empty() || component.parse::<u64>().is_err())
    {
        return Err(ContractError::Invalid {
            field: "schema_version",
            reason: "expected numeric MAJOR.MINOR.PATCH".to_owned(),
        });
    }
    let major = components[0]
        .parse::<u64>()
        .map_err(|error| ContractError::Invalid {
            field: "schema_version",
            reason: error.to_string(),
        })?;
    if major != SCHEMA_MAJOR {
        return Err(ContractError::UnsupportedSchema(value.to_owned()));
    }
    Ok(())
}

fn required(value: String, field: &'static str) -> Result<String, ContractError> {
    if value.is_empty() {
        Err(ContractError::Missing(field))
    } else if value.trim() != value {
        Err(ContractError::Invalid {
            field,
            reason: "leading or trailing whitespace is forbidden".to_owned(),
        })
    } else {
        Ok(value)
    }
}

fn required_bytes(value: Vec<u8>, field: &'static str) -> Result<Vec<u8>, ContractError> {
    if value.is_empty() {
        Err(ContractError::Missing(field))
    } else {
        Ok(value)
    }
}

fn parse_id<T>(
    value: String,
    field: &'static str,
    constructor: impl FnOnce(String) -> Result<T, domain_types::ValueError>,
) -> Result<T, ContractError> {
    if value.is_empty() {
        return Err(ContractError::Missing(field));
    }
    constructor(value).map_err(|error| ContractError::Invalid {
        field,
        reason: error.to_string(),
    })
}

fn parse_list_id<T>(
    value: String,
    field: &'static str,
    constructor: impl FnOnce(String) -> Result<T, domain_types::ValueError>,
) -> Result<T, ContractError> {
    constructor(value).map_err(|error| ContractError::Invalid {
        field,
        reason: error.to_string(),
    })
}

fn parse_time(value: i64, field: &'static str) -> Result<ProtocolTime, ContractError> {
    ProtocolTime::from_unix_micros(value).map_err(|error| ContractError::Invalid {
        field,
        reason: error.to_string(),
    })
}

fn fixed_hash(value: Vec<u8>, field: &'static str) -> Result<[u8; HASH_LENGTH], ContractError> {
    let actual = value.len();
    value.try_into().map_err(|_| ContractError::Invalid {
        field,
        reason: format!("expected {HASH_LENGTH} bytes, received {actual}"),
    })
}
