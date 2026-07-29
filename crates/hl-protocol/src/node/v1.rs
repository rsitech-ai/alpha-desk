use bytes::Bytes;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::{ObservationClass, SourceError};

const MAX_NODE_RECORD_BYTES: usize = 256 * 1024 * 1024;
type EventObjects<'a> = (Vec<&'a Map<String, Value>>, Option<u64>);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeStreamKind {
    TransactionBlocks,
    Trades,
    Fills,
    OrderStatuses,
    RawBookDiffs,
    MiscEvents,
    MarketMetadata,
}

impl NodeStreamKind {
    #[must_use]
    pub const fn observation_class(self) -> ObservationClass {
        match self {
            Self::TransactionBlocks => ObservationClass::CommittedBlock,
            Self::Trades | Self::Fills | Self::MiscEvents => ObservationClass::AuxiliaryLedger,
            Self::OrderStatuses => ObservationClass::AuxiliaryOrderStatus,
            Self::RawBookDiffs => ObservationClass::AuxiliaryBookDiff,
            Self::MarketMetadata => ObservationClass::Snapshot,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NodeRecordKind {
    EmptyBatch,
    TransactionBlock,
    Trade,
    Fill,
    OrderStatus,
    RawBookDiff,
    Liquidation,
    Transfer,
    MiscEvent,
    MarketMetadata,
}

#[derive(Debug, Clone)]
pub struct NodeRecordV1 {
    stream: NodeStreamKind,
    kind: NodeRecordKind,
    block_number: Option<u64>,
    payload: Bytes,
    content_hash: blake3::Hash,
}

impl NodeRecordV1 {
    #[must_use]
    pub const fn stream(&self) -> NodeStreamKind {
        self.stream
    }

    #[must_use]
    pub const fn kind(&self) -> NodeRecordKind {
        self.kind
    }

    #[must_use]
    pub const fn observation_class(&self) -> ObservationClass {
        self.stream.observation_class()
    }

    #[must_use]
    pub const fn block_number(&self) -> Option<u64> {
        self.block_number
    }

    #[must_use]
    pub const fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        self.content_hash
    }

    #[must_use]
    pub fn into_payload(self) -> Bytes {
        self.payload
    }
}

pub fn parse_node_record(
    stream: NodeStreamKind,
    payload: Bytes,
) -> Result<NodeRecordV1, SourceError> {
    if payload.is_empty() || payload.len() > MAX_NODE_RECORD_BYTES {
        return Err(SourceError::MalformedPayload(
            "node record size is outside the supported range".to_owned(),
        ));
    }
    let root: Value = serde_json::from_slice(&payload)
        .map_err(|_| SourceError::MalformedPayload("node record is not valid JSON".to_owned()))?;
    let object = root.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("node record root must be an object".to_owned())
    })?;
    let (events, block_number) = event_objects(object)?;
    let mut kinds = events
        .iter()
        .map(|event| classify_event(stream, event))
        .collect::<Result<Vec<_>, _>>()?;
    if kinds.is_empty() && block_number.is_some() {
        let content_hash = blake3::hash(&payload);
        return Ok(NodeRecordV1 {
            stream,
            kind: NodeRecordKind::EmptyBatch,
            block_number,
            payload,
            content_hash,
        });
    }
    let first = kinds
        .pop()
        .ok_or_else(|| SourceError::MalformedPayload("node event batch is empty".to_owned()))?;
    if kinds.iter().any(|kind| *kind != first) {
        return Err(SourceError::SchemaDrift(
            "node event batch mixes source variants".to_owned(),
        ));
    }
    let content_hash = blake3::hash(&payload);
    Ok(NodeRecordV1 {
        stream,
        kind: first,
        block_number,
        payload,
        content_hash,
    })
}

fn event_objects(root: &Map<String, Value>) -> Result<EventObjects<'_>, SourceError> {
    let Some(events) = root.get("events") else {
        return Ok((vec![root], None));
    };
    require_string(root, "local_time")?;
    require_string(root, "block_time")?;
    let block_number = root
        .get("block_number")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            SourceError::MalformedPayload(
                "batched node record has no unsigned block_number".to_owned(),
            )
        })?;
    let events = events.as_array().ok_or_else(|| {
        SourceError::MalformedPayload("batched node events must be an array".to_owned())
    })?;
    let objects = events
        .iter()
        .map(|event| {
            event.as_object().ok_or_else(|| {
                SourceError::MalformedPayload("node event must be an object".to_owned())
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((objects, Some(block_number)))
}

fn classify_event(
    stream: NodeStreamKind,
    event: &Map<String, Value>,
) -> Result<NodeRecordKind, SourceError> {
    match stream {
        NodeStreamKind::TransactionBlocks => {
            require_object(event, "abci_block")?;
            Ok(NodeRecordKind::TransactionBlock)
        }
        NodeStreamKind::Trades => {
            for field in [
                "coin",
                "side",
                "time",
                "px",
                "sz",
                "hash",
                "trade_dir_override",
            ] {
                require_string(event, field)?;
            }
            let side_info = event
                .get("side_info")
                .and_then(Value::as_array)
                .filter(|value| value.len() == 2)
                .ok_or_else(|| {
                    SourceError::MalformedPayload(
                        "node trade side_info must contain buyer and seller".to_owned(),
                    )
                })?;
            for side in side_info {
                let side = side.as_object().ok_or_else(|| {
                    SourceError::MalformedPayload(
                        "node trade side_info entry must be an object".to_owned(),
                    )
                })?;
                require_string(side, "user")?;
                require_string(side, "start_pos")?;
                require_u64(side, "oid")?;
                require_optional_u64(side, "twap_id")?;
                require_optional_string(side, "cloid")?;
            }
            Ok(NodeRecordKind::Trade)
        }
        NodeStreamKind::Fills => {
            for field in ["coin", "px", "sz", "hash"] {
                require_string(event, field)?;
            }
            require_u64(event, "time")?;
            require_u64(event, "tid")?;
            Ok(NodeRecordKind::Fill)
        }
        NodeStreamKind::OrderStatuses => {
            require_string(event, "time")?;
            require_string(event, "user")?;
            let status = require_string(event, "status")?;
            if !is_known_order_status(status) {
                return Err(SourceError::SchemaDrift(
                    "unknown node order-status variant".to_owned(),
                ));
            }
            require_object(event, "order")?;
            Ok(NodeRecordKind::OrderStatus)
        }
        NodeStreamKind::RawBookDiffs => {
            require_u64(event, "oid")?;
            require_string(event, "coin")?;
            validate_book_diff(event.get("raw_book_diff"))?;
            Ok(NodeRecordKind::RawBookDiff)
        }
        NodeStreamKind::MiscEvents => classify_misc_event(event),
        NodeStreamKind::MarketMetadata => {
            event
                .get("universe")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    SourceError::MalformedPayload(
                        "market metadata has no universe array".to_owned(),
                    )
                })?;
            Ok(NodeRecordKind::MarketMetadata)
        }
    }
}

fn classify_misc_event(event: &Map<String, Value>) -> Result<NodeRecordKind, SourceError> {
    require_string(event, "time")?;
    require_string(event, "hash")?;
    let inner = require_singleton_object(event, "inner")?;
    let (variant, value) = inner
        .iter()
        .next()
        .ok_or_else(|| SourceError::MalformedPayload("misc event is empty".to_owned()))?;
    match variant.as_str() {
        "CDeposit" | "Delegation" | "CWithdrawal" | "ValidatorRewards" | "Funding" => {
            require_value_object(value, "misc event payload")?;
            Ok(NodeRecordKind::MiscEvent)
        }
        "LedgerUpdate" => {
            let ledger = require_value_object(value, "ledger update payload")?;
            let delta = require_singleton_object(ledger, "delta")?;
            let (delta_variant, delta_value) = delta
                .iter()
                .next()
                .ok_or_else(|| SourceError::MalformedPayload("ledger delta is empty".to_owned()))?;
            require_value_object(delta_value, "ledger delta payload")?;
            match delta_variant.as_str() {
                "Liquidation" => Ok(NodeRecordKind::Liquidation),
                "InternalTransfer"
                | "AccountClassTransfer"
                | "SubAccountTransfer"
                | "SpotTransfer"
                | "PerpDexClassTransfer" => Ok(NodeRecordKind::Transfer),
                "Withdraw"
                | "Deposit"
                | "VaultCreate"
                | "VaultDeposit"
                | "VaultWithdraw"
                | "VaultDistribution"
                | "VaultLeaderCommission"
                | "SpotGenesis"
                | "RewardsClaim"
                | "AccountActivationGas"
                | "DeployGasAuction" => Ok(NodeRecordKind::MiscEvent),
                _ => Err(SourceError::SchemaDrift(
                    "unknown node ledger-delta variant".to_owned(),
                )),
            }
        }
        _ => Err(SourceError::SchemaDrift(
            "unknown node misc-event variant".to_owned(),
        )),
    }
}

fn validate_book_diff(value: Option<&Value>) -> Result<(), SourceError> {
    match value {
        Some(Value::String(operation)) if operation == "remove" => Ok(()),
        Some(Value::Object(operation)) if operation.len() == 1 => {
            let Some((variant, payload)) = operation.iter().next() else {
                return Err(SourceError::MalformedPayload(
                    "raw book diff is empty".to_owned(),
                ));
            };
            if !matches!(variant.as_str(), "new" | "update") {
                return Err(SourceError::SchemaDrift(
                    "unknown raw-book-diff variant".to_owned(),
                ));
            }
            require_value_object(payload, "raw book diff payload").map(|_| ())
        }
        Some(_) => Err(SourceError::MalformedPayload(
            "raw book diff has an invalid shape".to_owned(),
        )),
        None => Err(SourceError::MalformedPayload(
            "raw book diff is missing".to_owned(),
        )),
    }
}

fn require_string<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, SourceError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SourceError::MalformedPayload(format!(
                "node record has no non-empty string field {field}"
            ))
        })
}

fn require_u64(object: &Map<String, Value>, field: &str) -> Result<u64, SourceError> {
    object.get(field).and_then(Value::as_u64).ok_or_else(|| {
        SourceError::MalformedPayload(format!("node record has no unsigned integer field {field}"))
    })
}

fn require_optional_u64(object: &Map<String, Value>, field: &str) -> Result<(), SourceError> {
    match object.get(field) {
        Some(Value::Null) => Ok(()),
        Some(Value::Number(value)) if value.as_u64().is_some() => Ok(()),
        _ => Err(SourceError::MalformedPayload(format!(
            "node record field {field} must be null or an unsigned integer"
        ))),
    }
}

fn require_optional_string(object: &Map<String, Value>, field: &str) -> Result<(), SourceError> {
    match object.get(field) {
        Some(Value::Null | Value::String(_)) => Ok(()),
        _ => Err(SourceError::MalformedPayload(format!(
            "node record field {field} must be null or a string"
        ))),
    }
}

fn require_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>, SourceError> {
    object.get(field).and_then(Value::as_object).ok_or_else(|| {
        SourceError::MalformedPayload(format!("node record has no object field {field}"))
    })
}

fn require_singleton_object<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a Map<String, Value>, SourceError> {
    let value = require_object(object, field)?;
    if value.len() == 1 {
        Ok(value)
    } else {
        Err(SourceError::MalformedPayload(format!(
            "node record field {field} must contain one variant"
        )))
    }
}

fn require_value_object<'a>(
    value: &'a Value,
    context: &str,
) -> Result<&'a Map<String, Value>, SourceError> {
    value
        .as_object()
        .ok_or_else(|| SourceError::MalformedPayload(format!("{context} must be an object")))
}

fn is_known_order_status(status: &str) -> bool {
    matches!(
        status,
        "open"
            | "filled"
            | "canceled"
            | "triggered"
            | "rejected"
            | "marginCanceled"
            | "vaultWithdrawalCanceled"
            | "openInterestCapCanceled"
            | "selfTradeCanceled"
            | "reduceOnlyCanceled"
            | "siblingFilledCanceled"
            | "delistedCanceled"
            | "liquidatedCanceled"
            | "scheduledCancel"
            | "tickRejected"
            | "minTradeNtlRejected"
            | "perpMarginRejected"
            | "reduceOnlyRejected"
            | "badAloPxRejected"
            | "iocCancelRejected"
            | "badTriggerPxRejected"
            | "marketOrderNoLiquidityRejected"
            | "positionIncreaseAtOpenInterestCapRejected"
            | "positionFlipAtOpenInterestCapRejected"
            | "tooAggressiveAtOpenInterestCapRejected"
            | "openInterestIncreaseRejected"
            | "insufficientSpotBalanceRejected"
            | "oracleRejected"
            | "perpMaxPositionRejected"
    )
}
