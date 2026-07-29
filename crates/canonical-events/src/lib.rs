#![forbid(unsafe_code)]

mod block;
mod event_id;
mod input;
mod node_mapping;
mod upcast;

pub use block::{BlockEnvelope, BlockError};
pub use event_id::{EventIdentityInput, compute_event_id};
pub use input::CanonicalEventInput;
pub use node_mapping::{
    CommittedNodeV1MappingContext, EvidenceOnlyReason, MappingDisposition, MappingError,
    MarketCatalogV1, NodeV1MappingContext, map_committed_node_v1_block, map_node_v1_record,
};
pub use upcast::{CanonicalUpcaster, UpcastError, UpcastedEnvelope};

use api_contracts::{
    WireAssetContextUpdated, WireCanonicalEventEnvelope, WireDexCreated, WireMarketCreated,
    WireMarketMetadataChanged, WireOrderAccepted, WireOrderCancelled, WireOrderFilled,
    WireOrderModified, WireOrderPartiallyFilled, WireOrderRejected, WireOrderRested,
    WireSourceEvidence, WireTradeMatched, decode_asset_context_updated, decode_dex_created,
    decode_market_created, decode_market_metadata_changed, decode_order_accepted,
    decode_order_cancelled, decode_order_filled, decode_order_modified,
    decode_order_partially_filled, decode_order_rejected, decode_order_rested,
    decode_trade_matched, encode_asset_context_updated, encode_default_event_payload,
    encode_dex_created, encode_market_created, encode_market_metadata_changed,
    encode_order_accepted, encode_order_cancelled, encode_order_filled, encode_order_modified,
    encode_order_partially_filled, encode_order_rejected, encode_order_rested,
    encode_trade_matched, validate_event_payload,
};
use domain_types::{
    Address, AssetId, BlockHeight, ChainId, ClientOrderId, DexId, EventId, KnownTime, MarketId,
    OrderId, OrderSide, Price, ProtocolTime, Quantity, SourceId, TradeId, TransactionId,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderAccepted {
    pub order_id: OrderId,
    pub account_id: Address,
    pub market_id: MarketId,
    pub side: OrderSide,
    pub limit_price: Price,
    pub quantity: Quantity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRested {
    pub order_id: OrderId,
    pub market_id: MarketId,
    pub remaining_quantity: Quantity,
    pub limit_price: Price,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderModified {
    pub order_id: OrderId,
    pub previous_price: Price,
    pub new_price: Price,
    pub previous_quantity: Quantity,
    pub new_quantity: Quantity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderPartiallyFilled {
    pub order_id: OrderId,
    pub trade_id: TradeId,
    pub fill_price: Price,
    pub fill_quantity: Quantity,
    pub remaining_quantity: Quantity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderFilled {
    pub order_id: OrderId,
    pub trade_id: TradeId,
    pub fill_price: Price,
    pub fill_quantity: Quantity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderCancelled {
    pub order_id: OrderId,
    pub reason: String,
    pub remaining_quantity: Quantity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderRejected {
    pub client_order_id: ClientOrderId,
    pub account_id: Address,
    pub reason_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DexCreated {
    pub dex_id: DexId,
    pub name: String,
    pub operator_account_id: Address,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetContextUpdated {
    pub asset_id: AssetId,
    pub context_version: String,
    pub context_hash: [u8; HASH_LENGTH],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketCreated {
    pub market_id: MarketId,
    pub dex_id: DexId,
    pub base_asset_id: AssetId,
    pub quote_asset_id: AssetId,
    pub tick_size: Price,
    pub lot_size: Quantity,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketMetadataChanged {
    pub market_id: MarketId,
    pub metadata_version: String,
    pub metadata_hash: [u8; HASH_LENGTH],
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
            OrderAccepted(OrderAccepted),
            OrderRested(OrderRested),
            OrderModified(OrderModified),
            OrderPartiallyFilled(OrderPartiallyFilled),
            OrderFilled(OrderFilled),
            OrderCancelled(OrderCancelled),
            OrderRejected(OrderRejected),
            DexCreated(DexCreated),
            AssetContextUpdated(AssetContextUpdated),
            MarketCreated(MarketCreated),
            MarketMetadataChanged(MarketMetadataChanged),
            TradeMatched(TradeMatched),
        }

        impl EventPayload {
            #[must_use]
            pub const fn kind(&self) -> EventKind {
                match self {
                    $(Self::$kind(_) => EventKind::$kind,)+
                    Self::OrderAccepted(_) => EventKind::OrderAccepted,
                    Self::OrderRested(_) => EventKind::OrderRested,
                    Self::OrderModified(_) => EventKind::OrderModified,
                    Self::OrderPartiallyFilled(_) => EventKind::OrderPartiallyFilled,
                    Self::OrderFilled(_) => EventKind::OrderFilled,
                    Self::OrderCancelled(_) => EventKind::OrderCancelled,
                    Self::OrderRejected(_) => EventKind::OrderRejected,
                    Self::DexCreated(_) => EventKind::DexCreated,
                    Self::AssetContextUpdated(_) => EventKind::AssetContextUpdated,
                    Self::MarketCreated(_) => EventKind::MarketCreated,
                    Self::MarketMetadataChanged(_) => EventKind::MarketMetadataChanged,
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
                    Self::OrderAccepted(value) => encode_order_accepted(&WireOrderAccepted {
                        order_id: value.order_id.to_string(),
                        account_id: value.account_id.to_api_string(),
                        market_id: value.market_id.to_string(),
                        side: value.side.as_wire_name().to_owned(),
                        limit_price: value.limit_price.to_string(),
                        quantity: value.quantity.to_string(),
                    })
                    .map_err(payload_error),
                    Self::OrderRested(value) => encode_order_rested(&WireOrderRested {
                        order_id: value.order_id.to_string(),
                        market_id: value.market_id.to_string(),
                        remaining_quantity: value.remaining_quantity.to_string(),
                        limit_price: value.limit_price.to_string(),
                    })
                    .map_err(payload_error),
                    Self::OrderModified(value) => encode_order_modified(&WireOrderModified {
                        order_id: value.order_id.to_string(),
                        previous_price: value.previous_price.to_string(),
                        new_price: value.new_price.to_string(),
                        previous_quantity: value.previous_quantity.to_string(),
                        new_quantity: value.new_quantity.to_string(),
                    })
                    .map_err(payload_error),
                    Self::OrderPartiallyFilled(value) => {
                        encode_order_partially_filled(&WireOrderPartiallyFilled {
                            order_id: value.order_id.to_string(),
                            trade_id: value.trade_id.to_string(),
                            fill_price: value.fill_price.to_string(),
                            fill_quantity: value.fill_quantity.to_string(),
                            remaining_quantity: value.remaining_quantity.to_string(),
                        })
                        .map_err(payload_error)
                    }
                    Self::OrderFilled(value) => encode_order_filled(&WireOrderFilled {
                        order_id: value.order_id.to_string(),
                        trade_id: value.trade_id.to_string(),
                        fill_price: value.fill_price.to_string(),
                        fill_quantity: value.fill_quantity.to_string(),
                    })
                    .map_err(payload_error),
                    Self::OrderCancelled(value) => encode_order_cancelled(&WireOrderCancelled {
                        order_id: value.order_id.to_string(),
                        reason: value.reason.clone(),
                        remaining_quantity: value.remaining_quantity.to_string(),
                    })
                    .map_err(payload_error),
                    Self::OrderRejected(value) => encode_order_rejected(&WireOrderRejected {
                        client_order_id: value.client_order_id.to_string(),
                        account_id: value.account_id.to_api_string(),
                        reason_code: value.reason_code.clone(),
                        reason: value.reason.clone(),
                    })
                    .map_err(payload_error),
                    Self::DexCreated(value) => encode_dex_created(&WireDexCreated {
                        dex_id: value.dex_id.to_string(),
                        name: value.name.clone(),
                        operator_account_id: value.operator_account_id.to_api_string(),
                    })
                    .map_err(payload_error),
                    Self::AssetContextUpdated(value) => {
                        encode_asset_context_updated(&WireAssetContextUpdated {
                            asset_id: value.asset_id.to_string(),
                            context_version: value.context_version.clone(),
                            context_hash: value.context_hash.to_vec(),
                        })
                        .map_err(payload_error)
                    }
                    Self::MarketCreated(value) => {
                        validate_market_created_semantics(value)?;
                        encode_market_created(&WireMarketCreated {
                            market_id: value.market_id.to_string(),
                            dex_id: value.dex_id.to_string(),
                            base_asset_id: value.base_asset_id.to_string(),
                            quote_asset_id: value.quote_asset_id.to_string(),
                            tick_size: value.tick_size.to_string(),
                            lot_size: value.lot_size.to_string(),
                        })
                        .map_err(payload_error)
                    }
                    Self::MarketMetadataChanged(value) => {
                        encode_market_metadata_changed(&WireMarketMetadataChanged {
                            market_id: value.market_id.to_string(),
                            metadata_version: value.metadata_version.clone(),
                            metadata_hash: value.metadata_hash.to_vec(),
                        })
                        .map_err(payload_error)
                    }
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
                    EventKind::OrderAccepted => {
                        decode_order_accepted_payload(bytes).map(Self::OrderAccepted)
                    }
                    EventKind::OrderRested => {
                        decode_order_rested_payload(bytes).map(Self::OrderRested)
                    }
                    EventKind::OrderModified => {
                        decode_order_modified_payload(bytes).map(Self::OrderModified)
                    }
                    EventKind::OrderPartiallyFilled => {
                        decode_order_partially_filled_payload(bytes).map(Self::OrderPartiallyFilled)
                    }
                    EventKind::OrderFilled => {
                        decode_order_filled_payload(bytes).map(Self::OrderFilled)
                    }
                    EventKind::OrderCancelled => {
                        decode_order_cancelled_payload(bytes).map(Self::OrderCancelled)
                    }
                    EventKind::OrderRejected => {
                        decode_order_rejected_payload(bytes).map(Self::OrderRejected)
                    }
                    EventKind::DexCreated => {
                        decode_dex_created_payload(bytes).map(Self::DexCreated)
                    }
                    EventKind::AssetContextUpdated => {
                        decode_asset_context_updated_payload(bytes).map(Self::AssetContextUpdated)
                    }
                    EventKind::MarketCreated => {
                        decode_market_created_payload(bytes).map(Self::MarketCreated)
                    }
                    EventKind::MarketMetadataChanged => decode_market_metadata_changed_payload(bytes)
                        .map(Self::MarketMetadataChanged),
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
                        let bytes = fixture_payload_bytes(kind)?;
                        Self::decode(kind, &bytes)
                    })
                    .collect()
            }
        }
    };
}

opaque_payloads!(
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
    OracleUpdated,
    FundingRateUpdated,
    OutcomeCreated,
    OutcomeResolved,
);

fn decode_order_accepted_payload(bytes: &[u8]) -> Result<OrderAccepted, ContractError> {
    let value = decode_order_accepted(bytes).map_err(payload_error)?;
    let limit_price = parse_positive_price(&value.limit_price)?;
    let quantity = parse_positive_quantity(&value.quantity)?;
    Ok(OrderAccepted {
        order_id: payload_value(OrderId::new(value.order_id))?,
        account_id: payload_value(Address::parse_api(&value.account_id))?,
        market_id: payload_value(MarketId::new(value.market_id))?,
        side: payload_value(OrderSide::parse_wire(&value.side))?,
        limit_price,
        quantity,
    })
}

fn decode_order_rested_payload(bytes: &[u8]) -> Result<OrderRested, ContractError> {
    let value = decode_order_rested(bytes).map_err(payload_error)?;
    Ok(OrderRested {
        order_id: payload_value(OrderId::new(value.order_id))?,
        market_id: payload_value(MarketId::new(value.market_id))?,
        remaining_quantity: parse_positive_quantity(&value.remaining_quantity)?,
        limit_price: parse_positive_price(&value.limit_price)?,
    })
}

fn decode_order_modified_payload(bytes: &[u8]) -> Result<OrderModified, ContractError> {
    let value = decode_order_modified(bytes).map_err(payload_error)?;
    let modified = OrderModified {
        order_id: payload_value(OrderId::new(value.order_id))?,
        previous_price: parse_positive_price(&value.previous_price)?,
        new_price: parse_positive_price(&value.new_price)?,
        previous_quantity: parse_positive_quantity(&value.previous_quantity)?,
        new_quantity: parse_positive_quantity(&value.new_quantity)?,
    };
    if modified.previous_price == modified.new_price
        && modified.previous_quantity == modified.new_quantity
    {
        return Err(ContractError::Invalid {
            field: "payload",
            reason: "OrderModified must change price or quantity".to_owned(),
        });
    }
    Ok(modified)
}

fn decode_order_partially_filled_payload(
    bytes: &[u8],
) -> Result<OrderPartiallyFilled, ContractError> {
    let value = decode_order_partially_filled(bytes).map_err(payload_error)?;
    Ok(OrderPartiallyFilled {
        order_id: payload_value(OrderId::new(value.order_id))?,
        trade_id: payload_value(TradeId::new(value.trade_id))?,
        fill_price: parse_positive_price(&value.fill_price)?,
        fill_quantity: parse_positive_quantity(&value.fill_quantity)?,
        remaining_quantity: parse_positive_quantity(&value.remaining_quantity)?,
    })
}

fn decode_order_filled_payload(bytes: &[u8]) -> Result<OrderFilled, ContractError> {
    let value = decode_order_filled(bytes).map_err(payload_error)?;
    Ok(OrderFilled {
        order_id: payload_value(OrderId::new(value.order_id))?,
        trade_id: payload_value(TradeId::new(value.trade_id))?,
        fill_price: parse_positive_price(&value.fill_price)?,
        fill_quantity: parse_positive_quantity(&value.fill_quantity)?,
    })
}

fn decode_order_cancelled_payload(bytes: &[u8]) -> Result<OrderCancelled, ContractError> {
    let value = decode_order_cancelled(bytes).map_err(payload_error)?;
    Ok(OrderCancelled {
        order_id: payload_value(OrderId::new(value.order_id))?,
        reason: value.reason,
        remaining_quantity: parse_nonnegative_quantity(&value.remaining_quantity)?,
    })
}

fn decode_order_rejected_payload(bytes: &[u8]) -> Result<OrderRejected, ContractError> {
    let value = decode_order_rejected(bytes).map_err(payload_error)?;
    Ok(OrderRejected {
        client_order_id: payload_value(ClientOrderId::new(value.client_order_id))?,
        account_id: payload_value(Address::parse_api(&value.account_id))?,
        reason_code: value.reason_code,
        reason: value.reason,
    })
}

fn decode_dex_created_payload(bytes: &[u8]) -> Result<DexCreated, ContractError> {
    let value = decode_dex_created(bytes).map_err(payload_error)?;
    Ok(DexCreated {
        dex_id: payload_value(DexId::new(value.dex_id))?,
        name: value.name,
        operator_account_id: payload_value(Address::parse_api(&value.operator_account_id))?,
    })
}

fn decode_asset_context_updated_payload(
    bytes: &[u8],
) -> Result<AssetContextUpdated, ContractError> {
    let value = decode_asset_context_updated(bytes).map_err(payload_error)?;
    Ok(AssetContextUpdated {
        asset_id: payload_value(AssetId::new(value.asset_id))?,
        context_version: value.context_version,
        context_hash: hash_array(value.context_hash, "AssetContextUpdated context_hash")?,
    })
}

fn decode_market_created_payload(bytes: &[u8]) -> Result<MarketCreated, ContractError> {
    let value = decode_market_created(bytes).map_err(payload_error)?;
    let created = MarketCreated {
        market_id: payload_value(MarketId::new(value.market_id))?,
        dex_id: payload_value(DexId::new(value.dex_id))?,
        base_asset_id: payload_value(AssetId::new(value.base_asset_id))?,
        quote_asset_id: payload_value(AssetId::new(value.quote_asset_id))?,
        tick_size: payload_value(Price::from_str(&value.tick_size))?,
        lot_size: payload_value(Quantity::from_str(&value.lot_size))?,
    };
    validate_market_created_semantics(&created)?;
    Ok(created)
}

fn decode_market_metadata_changed_payload(
    bytes: &[u8],
) -> Result<MarketMetadataChanged, ContractError> {
    let value = decode_market_metadata_changed(bytes).map_err(payload_error)?;
    Ok(MarketMetadataChanged {
        market_id: payload_value(MarketId::new(value.market_id))?,
        metadata_version: value.metadata_version,
        metadata_hash: hash_array(value.metadata_hash, "MarketMetadataChanged metadata_hash")?,
    })
}

fn validate_market_created_semantics(value: &MarketCreated) -> Result<(), ContractError> {
    if value.base_asset_id == value.quote_asset_id {
        return Err(ContractError::Invalid {
            field: "payload",
            reason: "MarketCreated base and quote assets must differ".to_owned(),
        });
    }
    if value.tick_size.raw() <= 0 || value.lot_size.raw() <= 0 {
        return Err(ContractError::Invalid {
            field: "payload",
            reason: "MarketCreated tick and lot sizes must be positive".to_owned(),
        });
    }
    Ok(())
}

fn hash_array(value: Vec<u8>, name: &str) -> Result<[u8; HASH_LENGTH], ContractError> {
    value
        .try_into()
        .map_err(|value: Vec<u8>| ContractError::Invalid {
            field: "payload",
            reason: format!(
                "{name} must contain exactly {HASH_LENGTH} bytes, got {}",
                value.len()
            ),
        })
}

fn parse_positive_price(value: &str) -> Result<Price, ContractError> {
    let price = payload_value(Price::from_str(value))?;
    if price.raw() <= 0 {
        return Err(ContractError::Invalid {
            field: "payload",
            reason: "order price must be positive".to_owned(),
        });
    }
    Ok(price)
}

fn parse_positive_quantity(value: &str) -> Result<Quantity, ContractError> {
    let quantity = payload_value(Quantity::from_str(value))?;
    if quantity.raw() <= 0 {
        return Err(ContractError::Invalid {
            field: "payload",
            reason: "order quantity must be positive".to_owned(),
        });
    }
    Ok(quantity)
}

fn parse_nonnegative_quantity(value: &str) -> Result<Quantity, ContractError> {
    let quantity = payload_value(Quantity::from_str(value))?;
    if quantity.raw() < 0 {
        return Err(ContractError::Invalid {
            field: "payload",
            reason: "order quantity must be nonnegative".to_owned(),
        });
    }
    Ok(quantity)
}

fn payload_value<T>(value: Result<T, domain_types::ValueError>) -> Result<T, ContractError> {
    value.map_err(|error| ContractError::Invalid {
        field: "payload",
        reason: error.to_string(),
    })
}

fn fixture_payload_bytes(kind: EventKind) -> Result<Vec<u8>, ContractError> {
    match kind {
        EventKind::OrderAccepted => encode_order_accepted(&WireOrderAccepted {
            order_id: "fixture-order".to_owned(),
            account_id: Address::from_bytes([0x11; 20]).to_api_string(),
            market_id: "perp:BTC".to_owned(),
            side: "buy".to_owned(),
            limit_price: "1.000000".to_owned(),
            quantity: "1.00000000".to_owned(),
        })
        .map_err(payload_error),
        EventKind::OrderRested => encode_order_rested(&WireOrderRested {
            order_id: "fixture-order".to_owned(),
            market_id: "perp:BTC".to_owned(),
            remaining_quantity: "1.00000000".to_owned(),
            limit_price: "1.000000".to_owned(),
        })
        .map_err(payload_error),
        EventKind::OrderModified => encode_order_modified(&WireOrderModified {
            order_id: "fixture-order".to_owned(),
            previous_price: "1.000000".to_owned(),
            new_price: "2.000000".to_owned(),
            previous_quantity: "1.00000000".to_owned(),
            new_quantity: "1.00000000".to_owned(),
        })
        .map_err(payload_error),
        EventKind::OrderPartiallyFilled => {
            encode_order_partially_filled(&WireOrderPartiallyFilled {
                order_id: "fixture-order".to_owned(),
                trade_id: "fixture-trade".to_owned(),
                fill_price: "1.000000".to_owned(),
                fill_quantity: "0.50000000".to_owned(),
                remaining_quantity: "0.50000000".to_owned(),
            })
            .map_err(payload_error)
        }
        EventKind::OrderFilled => encode_order_filled(&WireOrderFilled {
            order_id: "fixture-order".to_owned(),
            trade_id: "fixture-trade".to_owned(),
            fill_price: "1.000000".to_owned(),
            fill_quantity: "1.00000000".to_owned(),
        })
        .map_err(payload_error),
        EventKind::OrderCancelled => encode_order_cancelled(&WireOrderCancelled {
            order_id: "fixture-order".to_owned(),
            reason: "fixture_cancel".to_owned(),
            remaining_quantity: "0.00000000".to_owned(),
        })
        .map_err(payload_error),
        EventKind::OrderRejected => encode_order_rejected(&WireOrderRejected {
            client_order_id: "fixture-client-order".to_owned(),
            account_id: Address::from_bytes([0x11; 20]).to_api_string(),
            reason_code: "fixture_rejection".to_owned(),
            reason: "fixture rejection".to_owned(),
        })
        .map_err(payload_error),
        EventKind::DexCreated => encode_dex_created(&WireDexCreated {
            dex_id: "validator".to_owned(),
            name: "Validator Perpetuals".to_owned(),
            operator_account_id: Address::from_bytes([0x11; 20]).to_api_string(),
        })
        .map_err(payload_error),
        EventKind::AssetContextUpdated => encode_asset_context_updated(&WireAssetContextUpdated {
            asset_id: "USDC".to_owned(),
            context_version: "fixture-asset-context".to_owned(),
            context_hash: vec![0x22; HASH_LENGTH],
        })
        .map_err(payload_error),
        EventKind::MarketCreated => encode_market_created(&WireMarketCreated {
            market_id: "perp:BTC".to_owned(),
            dex_id: "validator".to_owned(),
            base_asset_id: "BTC".to_owned(),
            quote_asset_id: "USDC".to_owned(),
            tick_size: "0.100000".to_owned(),
            lot_size: "0.00001000".to_owned(),
        })
        .map_err(payload_error),
        EventKind::MarketMetadataChanged => {
            encode_market_metadata_changed(&WireMarketMetadataChanged {
                market_id: "perp:BTC".to_owned(),
                metadata_version: "fixture-market-metadata".to_owned(),
                metadata_hash: vec![0x33; HASH_LENGTH],
            })
            .map_err(payload_error)
        }
        _ => encode_default_event_payload(kind.as_wire_name()).map_err(payload_error),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEvidence {
    source_id: SourceId,
    source_version: String,
    source_offset: String,
    content_hash: [u8; HASH_LENGTH],
    source_event_index: Option<u32>,
}

impl SourceEvidence {
    pub fn try_new(
        source_id: SourceId,
        source_version: impl Into<String>,
        source_offset: impl Into<String>,
        content_hash: [u8; HASH_LENGTH],
    ) -> Result<Self, ContractError> {
        Ok(Self {
            source_id,
            source_version: required(source_version.into(), "source_evidence.source_version")?,
            source_offset: required(source_offset.into(), "source_evidence.source_offset")?,
            content_hash,
            source_event_index: None,
        })
    }

    pub fn try_new_indexed(
        source_id: SourceId,
        source_version: impl Into<String>,
        source_offset: impl Into<String>,
        content_hash: [u8; HASH_LENGTH],
        source_event_index: u32,
    ) -> Result<Self, ContractError> {
        Self::try_new(source_id, source_version, source_offset, content_hash).map(|mut evidence| {
            evidence.source_event_index = Some(source_event_index);
            evidence
        })
    }

    #[must_use]
    pub const fn source_id(&self) -> &SourceId {
        &self.source_id
    }

    #[must_use]
    pub fn source_version(&self) -> &str {
        &self.source_version
    }

    #[must_use]
    pub fn source_offset(&self) -> &str {
        &self.source_offset
    }

    #[must_use]
    pub const fn content_hash(&self) -> [u8; HASH_LENGTH] {
        self.content_hash
    }

    #[must_use]
    pub const fn source_event_index(&self) -> Option<u32> {
        self.source_event_index
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum EvidenceMergeError {
    #[error("canonical event content differs")]
    CanonicalContentMismatch,
    #[error("source evidence locator has conflicting content for source {source_id}")]
    SourceEvidenceConflict {
        source_id: SourceId,
        existing_hash: [u8; HASH_LENGTH],
        conflicting_hash: [u8; HASH_LENGTH],
    },
}

impl EvidenceMergeError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::CanonicalContentMismatch => "canonical_event.content_mismatch",
            Self::SourceEvidenceConflict { .. } => "canonical_event.source_evidence_conflict",
        }
    }
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

    pub fn from_input(input: CanonicalEventInput) -> Result<Self, ContractError> {
        validate_schema_version(&input.schema_version)?;
        if input.source_evidence.is_empty() {
            return Err(ContractError::Missing("source_evidence"));
        }
        if input.observed_at > input.ingested_at || input.ingested_at > input.canonicalized_at {
            return Err(ContractError::Invalid {
                field: "lifecycle_times",
                reason: "expected observed_at <= ingested_at <= canonicalized_at".to_owned(),
            });
        }
        let parser_version = required(input.parser_version, "parser_version")?;
        let payload_bytes = input.payload.encode_to_vec()?;
        let payload_hash = *blake3::hash(&payload_bytes).as_bytes();
        let event_id = compute_event_id(&EventIdentityInput {
            chain_id: &input.chain_id,
            block_height: input.block_height,
            transaction_identity: &input.transaction_id,
            canonical_event_index: input.canonical_event_index,
            event_kind: input.payload.kind(),
            schema_major: SCHEMA_MAJOR,
        });

        Ok(Self {
            schema_version: input.schema_version,
            chain_id: input.chain_id,
            block_height: input.block_height,
            block_time: input.block_time,
            transaction_id: input.transaction_id,
            transaction_index: input.transaction_index,
            event_index: input.canonical_event_index,
            event_id,
            market_ids: input.market_ids,
            account_ids: input.account_ids,
            source_evidence: input.source_evidence,
            confirmation_class: input.confirmation_class,
            observed_at: input.observed_at,
            ingested_at: input.ingested_at,
            canonicalized_at: input.canonicalized_at,
            payload_hash,
            parser_version,
            payload: input.payload,
            encoded_payload: payload_bytes,
        })
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
            source_event_index: None,
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
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
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
    pub fn expected_event_id(&self) -> EventId {
        compute_event_id(&EventIdentityInput {
            chain_id: &self.chain_id,
            block_height: self.block_height,
            transaction_identity: &self.transaction_id,
            canonical_event_index: self.event_index,
            event_kind: self.payload.kind(),
            schema_major: SCHEMA_MAJOR,
        })
    }

    #[must_use]
    pub const fn transaction_id(&self) -> &TransactionId {
        &self.transaction_id
    }

    #[must_use]
    pub const fn transaction_index(&self) -> u32 {
        self.transaction_index
    }

    #[must_use]
    pub const fn canonical_event_index(&self) -> u32 {
        self.event_index
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
    pub fn market_ids(&self) -> &[MarketId] {
        &self.market_ids
    }

    #[must_use]
    pub fn account_addresses(&self) -> &[Address] {
        &self.account_ids
    }

    #[must_use]
    pub fn source_evidence(&self) -> &[SourceEvidence] {
        &self.source_evidence
    }

    pub fn merge_matching_source_evidence(&self, other: &Self) -> Result<Self, EvidenceMergeError> {
        if !self.has_same_canonical_content(other) {
            return Err(EvidenceMergeError::CanonicalContentMismatch);
        }

        for existing in &self.source_evidence {
            for conflicting in &other.source_evidence {
                if same_evidence_locator(existing, conflicting)
                    && existing.content_hash != conflicting.content_hash
                {
                    return Err(EvidenceMergeError::SourceEvidenceConflict {
                        source_id: existing.source_id.clone(),
                        existing_hash: existing.content_hash,
                        conflicting_hash: conflicting.content_hash,
                    });
                }
            }
        }

        let mut merged = self.clone();
        merged
            .source_evidence
            .extend(other.source_evidence.iter().cloned());
        merged.source_evidence.sort_by(compare_source_evidence);
        merged.source_evidence.dedup();
        Ok(merged)
    }

    #[must_use]
    pub fn parser_version(&self) -> &str {
        &self.parser_version
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

impl CanonicalEventEnvelope {
    fn has_same_canonical_content(&self, other: &Self) -> bool {
        self.schema_version == other.schema_version
            && self.chain_id == other.chain_id
            && self.block_height == other.block_height
            && self.block_time == other.block_time
            && self.transaction_id == other.transaction_id
            && self.transaction_index == other.transaction_index
            && self.event_index == other.event_index
            && self.event_id == other.event_id
            && self.market_ids == other.market_ids
            && self.account_ids == other.account_ids
            && self.payload_hash == other.payload_hash
            && self.payload == other.payload
            && self.encoded_payload == other.encoded_payload
    }
}

fn same_evidence_locator(left: &SourceEvidence, right: &SourceEvidence) -> bool {
    left.source_id == right.source_id
        && left.source_version == right.source_version
        && left.source_offset == right.source_offset
        && left.source_event_index == right.source_event_index
}

fn compare_source_evidence(left: &SourceEvidence, right: &SourceEvidence) -> std::cmp::Ordering {
    left.source_id
        .cmp(&right.source_id)
        .then_with(|| left.source_version.cmp(&right.source_version))
        .then_with(|| left.source_offset.cmp(&right.source_offset))
        .then_with(|| left.source_event_index.cmp(&right.source_event_index))
        .then_with(|| left.content_hash.cmp(&right.content_hash))
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
            source_event_index: value.source_event_index,
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
            source_event_index: value.source_event_index,
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
