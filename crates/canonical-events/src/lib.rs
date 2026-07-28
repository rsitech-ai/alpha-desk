#![forbid(unsafe_code)]

use api_contracts::{
    WireCanonicalEventEnvelope, WireSourceEvidence, WireTradeMatched, decode_trade_matched,
    encode_default_event_payload, encode_trade_matched, validate_event_payload,
};
use domain_types::{
    Address, BlockHeight, ChainId, EventId, KnownTime, MarketId, OrderId, Price, ProtocolTime,
    Quantity, SourceId, TradeId, TransactionId,
};
use semver::Version;
use std::str::FromStr;

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

/// Fully mapped V1 trade payload used by the deterministic Task 4 fixture boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeMatched {
    pub trade_id: Option<TradeId>,
    pub market_id: Option<MarketId>,
    pub maker_order_id: Option<OrderId>,
    pub taker_order_id: Option<OrderId>,
    pub price: Price,
    pub quantity: Quantity,
    pub deterministic_seed: u64,
}

impl TradeMatched {
    #[must_use]
    pub fn without_identities(price: Price, quantity: Quantity, deterministic_seed: u64) -> Self {
        Self {
            trade_id: None,
            market_id: None,
            maker_order_id: None,
            taker_order_id: None,
            price,
            quantity,
            deterministic_seed,
        }
    }
}

macro_rules! opaque_payloads {
    ($($kind:ident),+ $(,)?) => {
        $(
            /// Closed, schema-validated V1 payload.
            ///
            /// The original encoded message is intentionally retained verbatim so
            /// fields not yet promoted into domain types cannot be silently lost.
            #[derive(Debug, Clone, PartialEq, Eq)]
            pub struct $kind {
                encoded: Vec<u8>,
            }
        )+

        #[derive(Debug, Clone, PartialEq, Eq)]
        pub enum EventPayload {
            $($kind($kind),)+
            TradeMatched(TradeMatched),
        }

        impl EventPayload {
            #[must_use]
            pub const fn kind(&self) -> EventKind {
                match self {
                    $(Self::$kind(_) => EventKind::$kind,)+
                    Self::TradeMatched(_) => EventKind::TradeMatched,
                }
            }

            pub fn encode_to_vec(&self) -> Result<Vec<u8>, ContractError> {
                match self {
                    $(
                        Self::$kind(value) => {
                            validate_payload(EventKind::$kind, &value.encoded)?;
                            Ok(value.encoded.clone())
                        }
                    )+
                    Self::TradeMatched(value) => encode_trade_matched(&WireTradeMatched {
                        trade_id: value.trade_id.as_ref().map(ToString::to_string),
                        market_id: value.market_id.as_ref().map(ToString::to_string),
                        maker_order_id: value.maker_order_id.as_ref().map(ToString::to_string),
                        taker_order_id: value.taker_order_id.as_ref().map(ToString::to_string),
                        price: value.price.to_string(),
                        quantity: value.quantity.to_string(),
                        deterministic_seed: value.deterministic_seed,
                    })
                    .map_err(payload_error),
                }
            }

            pub fn decode(kind: EventKind, bytes: &[u8]) -> Result<Self, ContractError> {
                let payload = Self::decode_preserving(kind, bytes)?;
                if payload.encode_to_vec()? != bytes {
                    return Err(ContractError::Invalid {
                        field: "payload",
                        reason: format!(
                            "non-canonical {} bytes require an enclosing wire-preserving envelope",
                            kind.as_wire_name()
                        ),
                    });
                }
                Ok(payload)
            }

            fn decode_preserving(
                kind: EventKind,
                bytes: &[u8],
            ) -> Result<Self, ContractError> {
                required_payload(bytes)?;
                match kind {
                    $(
                        EventKind::$kind => {
                            validate_payload(kind, bytes)?;
                            Ok(Self::$kind($kind {
                                encoded: bytes.to_vec(),
                            }))
                        }
                    )+
                    EventKind::TradeMatched => {
                        let value = decode_trade_matched(bytes).map_err(payload_error)?;
                        Ok(Self::TradeMatched(TradeMatched {
                            trade_id: value
                                .trade_id
                                .map(TradeId::new)
                                .transpose()
                                .map_err(|error| ContractError::Invalid {
                                    field: "payload",
                                    reason: format!("invalid TradeMatched trade_id: {error}"),
                                })?,
                            market_id: value
                                .market_id
                                .map(MarketId::new)
                                .transpose()
                                .map_err(|error| ContractError::Invalid {
                                    field: "payload",
                                    reason: format!("invalid TradeMatched market_id: {error}"),
                                })?,
                            maker_order_id: value
                                .maker_order_id
                                .map(OrderId::new)
                                .transpose()
                                .map_err(|error| ContractError::Invalid {
                                    field: "payload",
                                    reason: format!(
                                        "invalid TradeMatched maker_order_id: {error}"
                                    ),
                                })?,
                            taker_order_id: value
                                .taker_order_id
                                .map(OrderId::new)
                                .transpose()
                                .map_err(|error| ContractError::Invalid {
                                    field: "payload",
                                    reason: format!(
                                        "invalid TradeMatched taker_order_id: {error}"
                                    ),
                                })?,
                            price: Price::from_str(&value.price).map_err(|error| {
                                ContractError::Invalid {
                                    field: "payload",
                                    reason: format!("invalid TradeMatched price: {error}"),
                                }
                            })?,
                            quantity: Quantity::from_str(&value.quantity).map_err(|error| {
                                ContractError::Invalid {
                                    field: "payload",
                                    reason: format!("invalid TradeMatched quantity: {error}"),
                                }
                            })?,
                            deterministic_seed: value.deterministic_seed,
                        }))
                    }
                }
            }

            pub fn fixtures() -> Result<Vec<Self>, ContractError> {
                EventKind::ALL
                    .into_iter()
                    .map(|kind| {
                        let bytes = encode_default_event_payload(kind.as_wire_name())
                            .map_err(payload_error)?;
                        Self::decode(kind, &bytes)
                    })
                    .collect()
            }
        }
    };
}

opaque_payloads!(
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
    market_ids: Vec<MarketId>,
    account_ids: Vec<Address>,
    source_evidence: Vec<SourceEvidence>,
    confirmation_class: ConfirmationClass,
    observed_at: KnownTime,
    ingested_at: KnownTime,
    canonicalized_at: KnownTime,
    payload_hash: [u8; HASH_LENGTH],
    parser_version: String,
    payload: EventPayload,
    encoded_payload: Vec<u8>,
}

impl CanonicalEventEnvelope {
    pub fn decode(bytes: &[u8]) -> Result<Self, ContractError> {
        WireCanonicalEventEnvelope::decode(bytes)?.try_into()
    }

    pub fn encode_to_vec(&self) -> Result<Vec<u8>, ContractError> {
        Ok(self.to_wire()?.encode_to_vec())
    }

    /// Builds a deterministic, fixture-safe envelope.
    ///
    /// This convenience constructor deliberately derives lifecycle timestamps
    /// and source evidence from its stable inputs. Live ingestion must instead
    /// decode a wire envelope carrying independently observed lifecycle and
    /// evidence values.
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        schema_version: &str,
        chain_id: &str,
        block_height: BlockHeight,
        block_time: ProtocolTime,
        transaction_id: TransactionId,
        transaction_index: u32,
        event_index: u32,
        event_id: EventId,
        market_ids: Vec<MarketId>,
        account_ids: Vec<Address>,
        confirmation_class: ConfirmationClass,
        payload: EventPayload,
        parser_version: &str,
    ) -> Result<Self, ContractError> {
        validate_schema_version(schema_version)?;
        let chain_id = parse_id(chain_id.to_owned(), "chain_id", ChainId::new)?;
        let parser_version = required(parser_version.to_owned(), "parser_version")?;
        let payload_bytes = payload.encode_to_vec()?;
        let payload_hash = *blake3::hash(&payload_bytes).as_bytes();
        let lifecycle_time =
            KnownTime::from_unix_micros(block_time.unix_micros()).map_err(|error| {
                ContractError::Invalid {
                    field: "block_time_micros",
                    reason: error.to_string(),
                }
            })?;
        let source_offset = format!(
            "{}:{}:{}:{}",
            chain_id.as_str(),
            block_height.get(),
            transaction_index,
            event_index
        );
        let source_evidence = vec![SourceEvidence {
            source_id: SourceId::new("deterministic-fixture-constructor").map_err(|error| {
                ContractError::Invalid {
                    field: "source_evidence.source_id",
                    reason: error.to_string(),
                }
            })?,
            source_version: "v1".to_owned(),
            source_offset,
            content_hash: payload_hash,
        }];

        Ok(Self {
            schema_version: schema_version.to_owned(),
            chain_id,
            block_height,
            block_time,
            transaction_id,
            transaction_index,
            event_index,
            event_id,
            market_ids,
            account_ids,
            source_evidence,
            confirmation_class,
            observed_at: lifecycle_time,
            ingested_at: lifecycle_time,
            canonicalized_at: lifecycle_time,
            payload_hash,
            parser_version,
            payload,
            encoded_payload: payload_bytes,
        })
    }

    pub fn fixture() -> Result<Self, ContractError> {
        Self::try_new(
            "1.0.0",
            "hyperliquid-mainnet",
            BlockHeight::new(42),
            ProtocolTime::from_unix_micros(1_700_000_000_000_000).map_err(|error| {
                ContractError::Invalid {
                    field: "block_time_micros",
                    reason: error.to_string(),
                }
            })?,
            TransactionId::new("tx-42").map_err(|error| ContractError::Invalid {
                field: "transaction_id",
                reason: error.to_string(),
            })?,
            7,
            9,
            EventId::new("event-42-7-9").map_err(|error| ContractError::Invalid {
                field: "event_id",
                reason: error.to_string(),
            })?,
            vec![
                MarketId::new("BTC-USD").map_err(|error| ContractError::Invalid {
                    field: "market_ids",
                    reason: error.to_string(),
                })?,
            ],
            vec![
                Address::from_bytes([0x11; 20]),
                Address::from_bytes([0x22; 20]),
            ],
            ConfirmationClass::CommittedPrimary,
            EventPayload::TradeMatched(TradeMatched::without_identities(
                Price::parse_at_scale("65000", 6).map_err(|error| ContractError::Invalid {
                    field: "payload",
                    reason: error.to_string(),
                })?,
                Quantity::parse_at_scale("0.01", 8).map_err(|error| ContractError::Invalid {
                    field: "payload",
                    reason: error.to_string(),
                })?,
                7,
            )),
            "parser-v1",
        )
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
    pub const fn block_time(&self) -> ProtocolTime {
        self.block_time
    }

    #[must_use]
    pub const fn observed_at(&self) -> KnownTime {
        self.observed_at
    }

    #[must_use]
    pub const fn ingested_at(&self) -> KnownTime {
        self.ingested_at
    }

    #[must_use]
    pub const fn canonicalized_at(&self) -> KnownTime {
        self.canonicalized_at
    }

    #[must_use]
    pub fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.payload.kind()
    }

    #[must_use]
    pub fn payload(&self) -> &EventPayload {
        &self.payload
    }

    #[must_use]
    pub fn payload_hash(&self) -> [u8; HASH_LENGTH] {
        self.payload_hash
    }

    #[must_use]
    pub fn account_addresses(&self) -> &[Address] {
        &self.account_ids
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

    fn to_wire(&self) -> Result<WireCanonicalEventEnvelope, ContractError> {
        let decoded_payload =
            EventPayload::decode_preserving(self.payload.kind(), &self.encoded_payload)?;
        if decoded_payload != self.payload {
            return Err(ContractError::Invalid {
                field: "payload",
                reason: "preserved wire bytes do not match the typed payload".to_owned(),
            });
        }
        let payload = self.encoded_payload.clone();
        let payload_hash = *blake3::hash(&payload).as_bytes();
        if payload_hash != self.payload_hash {
            return Err(ContractError::Invalid {
                field: "payload_hash",
                reason: "stored hash no longer matches the typed payload".to_owned(),
            });
        }
        Ok(WireCanonicalEventEnvelope {
            schema_version: self.schema_version.clone(),
            chain_id: self.chain_id.to_string(),
            block_height: self.block_height.get(),
            block_time_micros: self.block_time.unix_micros(),
            transaction_id: self.transaction_id.to_string(),
            transaction_index: self.transaction_index,
            event_index: self.event_index,
            event_id: self.event_id.to_string(),
            event_kind: self.payload.kind().as_wire_name().to_owned(),
            market_ids: self.market_ids.iter().map(ToString::to_string).collect(),
            account_ids: self
                .account_ids
                .iter()
                .copied()
                .map(Address::to_api_string)
                .collect(),
            source_evidence: self.source_evidence.iter().map(Into::into).collect(),
            confirmation_class: self.confirmation_class.wire_value(),
            observed_at_micros: self.observed_at.unix_micros(),
            ingested_at_micros: self.ingested_at.unix_micros(),
            canonicalized_at_micros: self.canonicalized_at.unix_micros(),
            payload_hash: payload_hash.to_vec(),
            parser_version: self.parser_version.clone(),
            payload,
        })
    }
}

impl TryFrom<WireCanonicalEventEnvelope> for CanonicalEventEnvelope {
    type Error = ContractError;

    fn try_from(value: WireCanonicalEventEnvelope) -> Result<Self, Self::Error> {
        validate_schema_version(&value.schema_version)?;
        if value.source_evidence.is_empty() {
            return Err(ContractError::Missing("source_evidence"));
        }
        let event_kind = EventKind::try_from(required(value.event_kind, "event_kind")?.as_str())?;
        let payload_bytes = required_bytes(value.payload, "payload")?;
        let payload = EventPayload::decode_preserving(event_kind, &payload_bytes)?;
        let payload_hash = fixed_hash(value.payload_hash, "payload_hash")?;
        let computed_hash = *blake3::hash(&payload_bytes).as_bytes();
        if computed_hash != payload_hash {
            return Err(ContractError::Invalid {
                field: "payload_hash",
                reason: "does not match the canonical payload bytes".to_owned(),
            });
        }

        Ok(Self {
            schema_version: required(value.schema_version, "schema_version")?,
            chain_id: parse_id(value.chain_id, "chain_id", ChainId::new)?,
            block_height: BlockHeight::new(value.block_height),
            block_time: parse_protocol_time(value.block_time_micros, "block_time_micros")?,
            transaction_id: parse_id(value.transaction_id, "transaction_id", TransactionId::new)?,
            transaction_index: value.transaction_index,
            event_index: value.event_index,
            event_id: parse_id(value.event_id, "event_id", EventId::new)?,
            market_ids: value
                .market_ids
                .into_iter()
                .map(|id| parse_list_id(id, "market_ids", MarketId::new))
                .collect::<Result<_, _>>()?,
            account_ids: value
                .account_ids
                .into_iter()
                .map(|id| {
                    Address::parse_api(&id).map_err(|error| ContractError::Invalid {
                        field: "account_ids",
                        reason: error.to_string(),
                    })
                })
                .collect::<Result<_, _>>()?,
            source_evidence: value
                .source_evidence
                .into_iter()
                .map(SourceEvidence::try_from)
                .collect::<Result<_, _>>()?,
            confirmation_class: value.confirmation_class.try_into()?,
            observed_at: parse_known_time(value.observed_at_micros, "observed_at_micros")?,
            ingested_at: parse_known_time(value.ingested_at_micros, "ingested_at_micros")?,
            canonicalized_at: parse_known_time(
                value.canonicalized_at_micros,
                "canonicalized_at_micros",
            )?,
            payload_hash,
            parser_version: required(value.parser_version, "parser_version")?,
            payload,
            encoded_payload: payload_bytes,
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

fn required_payload(value: &[u8]) -> Result<(), ContractError> {
    if value.is_empty() {
        Err(ContractError::Missing("payload"))
    } else {
        Ok(())
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

fn parse_protocol_time(value: i64, field: &'static str) -> Result<ProtocolTime, ContractError> {
    ProtocolTime::from_unix_micros(value).map_err(|error| ContractError::Invalid {
        field,
        reason: error.to_string(),
    })
}

fn parse_known_time(value: i64, field: &'static str) -> Result<KnownTime, ContractError> {
    KnownTime::from_unix_micros(value).map_err(|error| ContractError::Invalid {
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

fn validate_payload(kind: EventKind, bytes: &[u8]) -> Result<(), ContractError> {
    validate_event_payload(kind.as_wire_name(), bytes).map_err(payload_error)
}

fn payload_error(error: api_contracts::PayloadCodecError) -> ContractError {
    ContractError::Invalid {
        field: "payload",
        reason: error.to_string(),
    }
}
