use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use api_contracts::is_canonical_trade_client_order_id;
use chrono::{DateTime, NaiveDateTime};
use domain_types::{
    Address, BlockHeight, ChainId, ClientOrderId, KnownTime, MarketId, OrderId, PositionQuantity,
    Price, ProtocolTime, Quantity, SourceId, TradeId, TransactionId, TwapId,
};
use hl_protocol::node::v1::{NodeRecordKind, NodeRecordV1, NodeStreamKind};
use serde::Deserialize;

use crate::{
    BlockEnvelope, BlockError, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass,
    ContractError, EventPayload, SourceEvidence, TradeMatched, TradeParticipantRoleV1,
    TradeParticipantV1,
};

const TRADE_ID_CONTEXT: &str = "hyperliquid-alpha-desk/trade-id/node-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MappingDisposition {
    Mapped(Vec<CanonicalEventEnvelope>),
    EmptyBlock,
    EvidenceOnly(EvidenceOnlyReason),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvidenceOnlyReason {
    MissingBlockContext,
    UnsupportedCanonicalSemantics,
}

#[derive(Debug, Clone)]
pub struct NodeV1MappingContext {
    pub chain_id: ChainId,
    pub source_id: SourceId,
    pub source_version: String,
    pub source_offset: String,
    pub observed_at: KnownTime,
    pub ingested_at: KnownTime,
    pub canonicalized_at: KnownTime,
    pub mapper_version: String,
}

#[derive(Debug, Clone)]
pub struct CommittedNodeV1MappingContext {
    pub chain_id: ChainId,
    pub source_id: SourceId,
    pub source_version: String,
    pub source_offset: String,
    pub expected_height: BlockHeight,
    pub confirmation_class: ConfirmationClass,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketCatalogV1 {
    version: String,
    markets: BTreeMap<String, MarketId>,
}

impl MarketCatalogV1 {
    pub fn try_new<'a>(
        version: impl Into<String>,
        markets: impl IntoIterator<Item = (&'a str, MarketId)>,
    ) -> Result<Self, MappingError> {
        let version = version.into();
        if version.is_empty() || version.trim() != version {
            return Err(MappingError::InvalidCatalog {
                reason: "catalog version must be non-empty without surrounding whitespace"
                    .to_owned(),
            });
        }
        let mut indexed = BTreeMap::new();
        for (symbol, market_id) in markets {
            if symbol.is_empty() || symbol.trim() != symbol {
                return Err(MappingError::InvalidCatalog {
                    reason: "market symbol must be non-empty without surrounding whitespace"
                        .to_owned(),
                });
            }
            if indexed.insert(symbol.to_owned(), market_id).is_some() {
                return Err(MappingError::InvalidCatalog {
                    reason: format!("duplicate market symbol {symbol}"),
                });
            }
        }
        Ok(Self {
            version,
            markets: indexed,
        })
    }

    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    fn resolve(&self, symbol: &str) -> Result<MarketId, MappingError> {
        self.markets
            .get(symbol)
            .cloned()
            .ok_or_else(|| MappingError::UnmappedMarket {
                symbol: symbol.to_owned(),
            })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MappingError {
    #[error("invalid market catalog: {reason}")]
    InvalidCatalog { reason: String },
    #[error("node record is malformed for canonical mapping: {reason}")]
    MalformedRecord { reason: String },
    #[error("node trade references unmapped market {symbol}")]
    UnmappedMarket { symbol: String },
    #[error("node trade contains invalid buyer or seller address")]
    InvalidAddress,
    #[error("node trade contains an invalid transaction hash")]
    InvalidTransactionHash,
    #[error("node trade contains invalid {field}: {reason}")]
    InvalidDecimal { field: &'static str, reason: String },
    #[error("node trade repeats transaction identity after a later transaction")]
    NonContiguousTransaction,
    #[error("node trade batch contains more events than the V1 index supports")]
    EventIndexOverflow,
    #[error(
        "committed node block height does not match the source cursor: expected {expected:?}, received {actual:?}"
    )]
    BlockHeightMismatch {
        expected: BlockHeight,
        actual: BlockHeight,
    },
    #[error("committed node block parent is not contiguous")]
    InvalidParentHeight,
    #[error("committed node block contains {action_bundles} unsupported action bundles")]
    UnsupportedCommittedActions { action_bundles: usize },
    #[error("committed node mapping requires a committed confirmation class")]
    InvalidCommittedConfirmation,
    #[error("canonical event contract rejected mapped source: {0}")]
    Contract(#[from] ContractError),
    #[error("canonical block contract rejected mapped source: {0}")]
    Block(#[from] BlockError),
}

impl MappingError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidCatalog { .. } => "canonical_mapping.invalid_catalog",
            Self::MalformedRecord { .. } => "canonical_mapping.malformed_record",
            Self::UnmappedMarket { .. } => "canonical_mapping.unmapped_market",
            Self::InvalidAddress => "canonical_mapping.invalid_address",
            Self::InvalidTransactionHash => "canonical_mapping.invalid_transaction_hash",
            Self::InvalidDecimal { .. } => "canonical_mapping.invalid_decimal",
            Self::NonContiguousTransaction => "canonical_mapping.non_contiguous_transaction",
            Self::EventIndexOverflow => "canonical_mapping.event_index_overflow",
            Self::BlockHeightMismatch { .. } => "canonical_mapping.block_height_mismatch",
            Self::InvalidParentHeight => "canonical_mapping.invalid_parent_height",
            Self::UnsupportedCommittedActions { .. } => {
                "canonical_mapping.unsupported_committed_actions"
            }
            Self::InvalidCommittedConfirmation => {
                "canonical_mapping.invalid_committed_confirmation"
            }
            Self::Contract(_) => "canonical_mapping.contract_rejected",
            Self::Block(_) => "canonical_mapping.block_rejected",
        }
    }
}

#[derive(Debug, Deserialize)]
struct TradeBatch {
    block_time: String,
    events: Vec<NodeTrade>,
}

#[derive(Debug, Deserialize)]
struct NodeTrade {
    coin: String,
    px: String,
    sz: String,
    hash: String,
    side_info: [TradeSide; 2],
}

#[derive(Debug, Deserialize)]
struct TradeSide {
    user: String,
    start_pos: String,
    oid: u64,
    twap_id: serde_json::Value,
    cloid: serde_json::Value,
}

pub fn map_committed_node_v1_block(
    record: &NodeRecordV1,
    context: &CommittedNodeV1MappingContext,
) -> Result<BlockEnvelope, MappingError> {
    if record.stream() != NodeStreamKind::TransactionBlocks
        || record.kind() != NodeRecordKind::TransactionBlock
    {
        return Err(MappingError::MalformedRecord {
            reason: "committed mapping requires a transaction-block record".to_owned(),
        });
    }
    if !matches!(
        context.confirmation_class,
        ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent
    ) {
        return Err(MappingError::InvalidCommittedConfirmation);
    }

    let root: serde_json::Value = serde_json::from_slice(record.payload()).map_err(|error| {
        MappingError::MalformedRecord {
            reason: error.to_string(),
        }
    })?;
    let root = root
        .as_object()
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "transaction block root must be an object".to_owned(),
        })?;
    let abci_block = root
        .get("abci_block")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "transaction block has no abci_block object".to_owned(),
        })?;
    let round = required_u64(abci_block, "round")?;
    let block_height = BlockHeight::new(round);
    if block_height != context.expected_height {
        return Err(MappingError::BlockHeightMismatch {
            expected: context.expected_height,
            actual: block_height,
        });
    }
    let parent_round = required_u64(abci_block, "parent_round")?;
    if round.checked_sub(1) != Some(parent_round) {
        return Err(MappingError::InvalidParentHeight);
    }
    let time = abci_block
        .get("time")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "transaction block has no string time".to_owned(),
        })?;
    let block_time = parse_block_time(time)?;
    let action_bundles = select_action_bundles(root, abci_block)?;
    if !action_bundles.is_empty() {
        return Err(MappingError::UnsupportedCommittedActions {
            action_bundles: action_bundles.len(),
        });
    }

    BlockEnvelope::try_new(
        context.chain_id.clone(),
        block_height,
        block_time,
        context.confirmation_class,
        Vec::new(),
        BTreeMap::from([(context.source_id.clone(), *record.content_hash().as_bytes())]),
    )
    .map_err(Into::into)
}

pub fn map_node_v1_record(
    record: &NodeRecordV1,
    catalog: &MarketCatalogV1,
    context: &NodeV1MappingContext,
) -> Result<MappingDisposition, MappingError> {
    if record.kind() == NodeRecordKind::EmptyBatch {
        return Ok(MappingDisposition::EmptyBlock);
    }
    if record.stream() != NodeStreamKind::Trades || record.kind() != NodeRecordKind::Trade {
        return Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::UnsupportedCanonicalSemantics,
        ));
    }
    let Some(block_number) = record.block_number() else {
        return Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::MissingBlockContext,
        ));
    };

    let batch: TradeBatch = serde_json::from_slice(record.payload()).map_err(|error| {
        MappingError::MalformedRecord {
            reason: error.to_string(),
        }
    })?;
    let block_time = parse_block_time(&batch.block_time)?;
    let block_height = BlockHeight::new(block_number);
    let parser_version = format!("{}/catalog:{}", context.mapper_version, catalog.version());
    let mut seen_transactions = BTreeSet::new();
    let mut current_transaction: Option<String> = None;
    let mut transaction_index = 0_u32;
    let mut canonical_event_index = 0_u32;
    let mut events = Vec::with_capacity(batch.events.len());

    for (source_index, trade) in batch.events.into_iter().enumerate() {
        let source_index =
            u32::try_from(source_index).map_err(|_| MappingError::EventIndexOverflow)?;
        match current_transaction.as_deref() {
            None => {
                seen_transactions.insert(trade.hash.clone());
                current_transaction = Some(trade.hash.clone());
            }
            Some(current) if current == trade.hash => {
                canonical_event_index = canonical_event_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
            }
            Some(_) => {
                if !seen_transactions.insert(trade.hash.clone()) {
                    return Err(MappingError::NonContiguousTransaction);
                }
                transaction_index = transaction_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
                canonical_event_index = 0;
                current_transaction = Some(trade.hash.clone());
            }
        }

        events.push(map_trade(
            trade,
            source_index,
            transaction_index,
            canonical_event_index,
            block_height,
            block_time,
            &parser_version,
            record,
            catalog,
            context,
        )?);
    }

    Ok(MappingDisposition::Mapped(events))
}

fn required_u64(
    object: &serde_json::Map<String, serde_json::Value>,
    field: &'static str,
) -> Result<u64, MappingError> {
    object
        .get(field)
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: format!("transaction block has no unsigned {field}"),
        })
}

fn select_action_bundles<'a>(
    root: &'a serde_json::Map<String, serde_json::Value>,
    abci_block: &'a serde_json::Map<String, serde_json::Value>,
) -> Result<&'a Vec<serde_json::Value>, MappingError> {
    let root_bundles = root
        .get("signed_action_bundles")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "top-level signed_action_bundles must be an array".to_owned(),
                })
        })
        .transpose()?;
    let nested_bundles = abci_block
        .get("signed_action_bundles")
        .map(|value| {
            value
                .as_array()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "abci_block signed_action_bundles must be an array".to_owned(),
                })
        })
        .transpose()?;
    match (root_bundles, nested_bundles) {
        (Some(_), Some(_)) => Err(MappingError::MalformedRecord {
            reason: "transaction block contains ambiguous action bundle locations".to_owned(),
        }),
        (Some(root), _) => Ok(root),
        (_, Some(nested)) => Ok(nested),
        (None, None) => Err(MappingError::MalformedRecord {
            reason: "transaction block has no signed_action_bundles array".to_owned(),
        }),
    }
}

#[allow(clippy::too_many_arguments)]
fn map_trade(
    trade: NodeTrade,
    source_index: u32,
    transaction_index: u32,
    canonical_event_index: u32,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    parser_version: &str,
    record: &NodeRecordV1,
    catalog: &MarketCatalogV1,
    context: &NodeV1MappingContext,
) -> Result<CanonicalEventEnvelope, MappingError> {
    let market_id = catalog.resolve(&trade.coin)?;
    let buyer =
        Address::parse_api(&trade.side_info[0].user).map_err(|_| MappingError::InvalidAddress)?;
    let seller =
        Address::parse_api(&trade.side_info[1].user).map_err(|_| MappingError::InvalidAddress)?;
    let participants = [
        map_trade_participant(&trade.side_info[0], TradeParticipantRoleV1::Buyer, buyer)?,
        map_trade_participant(&trade.side_info[1], TradeParticipantRoleV1::Seller, seller)?,
    ];
    let price = Price::from_str(&trade.px).map_err(|error| MappingError::InvalidDecimal {
        field: "price",
        reason: error.to_string(),
    })?;
    if price.raw() <= 0 {
        return Err(MappingError::InvalidDecimal {
            field: "price",
            reason: "trade price must be positive".to_owned(),
        });
    }
    let quantity = Quantity::from_str(&trade.sz).map_err(|error| MappingError::InvalidDecimal {
        field: "quantity",
        reason: error.to_string(),
    })?;
    if quantity.raw() <= 0 {
        return Err(MappingError::InvalidDecimal {
            field: "quantity",
            reason: "trade quantity must be positive".to_owned(),
        });
    }
    if !is_lowercase_hash(&trade.hash) {
        return Err(MappingError::InvalidTransactionHash);
    }
    let transaction_id =
        TransactionId::new(trade.hash.clone()).map_err(|_| MappingError::InvalidTransactionHash)?;
    let trade_id = derive_trade_id(
        &context.chain_id,
        block_height,
        &transaction_id,
        canonical_event_index,
    )?;
    let source_evidence = SourceEvidence::try_new_indexed(
        context.source_id.clone(),
        context.source_version.clone(),
        context.source_offset.clone(),
        *record.content_hash().as_bytes(),
        source_index,
    )?;

    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: context.chain_id.clone(),
        block_height,
        block_time,
        transaction_id,
        transaction_index,
        canonical_event_index,
        market_ids: vec![market_id.clone()],
        account_ids: vec![buyer, seller],
        source_evidence: vec![source_evidence],
        confirmation_class: ConfirmationClass::ProvisionalSource,
        observed_at: context.observed_at,
        ingested_at: context.ingested_at,
        canonicalized_at: context.canonicalized_at,
        parser_version: parser_version.to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched {
            trade_id: Some(trade_id),
            market_id: Some(market_id),
            maker_order_id: None,
            taker_order_id: None,
            price,
            quantity,
            deterministic_seed: 0,
            participants: Some(Box::new(participants)),
        }),
    })
    .map_err(Into::into)
}

fn map_trade_participant(
    source: &TradeSide,
    role: TradeParticipantRoleV1,
    account_id: Address,
) -> Result<TradeParticipantV1, MappingError> {
    let start_position = PositionQuantity::from_str(&source.start_pos).map_err(|error| {
        MappingError::InvalidDecimal {
            field: "start_position",
            reason: error.to_string(),
        }
    })?;
    if source.oid == 0 {
        return Err(MappingError::MalformedRecord {
            reason: "trade participant oid must be positive".to_owned(),
        });
    }
    let twap_id = match &source.twap_id {
        serde_json::Value::Null => None,
        serde_json::Value::Number(value) => Some(TwapId::new(value.as_u64().ok_or_else(|| {
            MappingError::MalformedRecord {
                reason: "trade participant twap_id must be an unsigned integer or null".to_owned(),
            }
        })?)),
        _ => {
            return Err(MappingError::MalformedRecord {
                reason: "trade participant twap_id must be an unsigned integer or null".to_owned(),
            });
        }
    };
    let client_order_id = match &source.cloid {
        serde_json::Value::Null => None,
        serde_json::Value::String(value) => {
            if !is_canonical_trade_client_order_id(value) {
                return Err(MappingError::MalformedRecord {
                    reason: "trade participant cloid must be lowercase 0x followed by exactly 32 lowercase hexadecimal digits".to_owned(),
                });
            }
            Some(ClientOrderId::new(value.clone()).map_err(|error| {
                MappingError::MalformedRecord {
                    reason: format!("invalid trade participant cloid: {error}"),
                }
            })?)
        }
        _ => {
            return Err(MappingError::MalformedRecord {
                reason: "trade participant cloid must be a string or null".to_owned(),
            });
        }
    };
    Ok(TradeParticipantV1 {
        role,
        account_id,
        start_position,
        order_id: OrderId::new(source.oid.to_string()).map_err(|error| {
            MappingError::MalformedRecord {
                reason: format!("invalid trade participant oid: {error}"),
            }
        })?,
        twap_id,
        client_order_id,
    })
}

fn parse_block_time(value: &str) -> Result<ProtocolTime, MappingError> {
    let micros = DateTime::parse_from_rfc3339(value)
        .map(|time| time.timestamp_micros())
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f")
                .map(|time| time.and_utc().timestamp_micros())
        })
        .map_err(|error| MappingError::MalformedRecord {
            reason: format!("invalid block_time: {error}"),
        })?;
    ProtocolTime::from_unix_micros(micros).map_err(|error| MappingError::MalformedRecord {
        reason: format!("invalid block_time range: {error}"),
    })
}

fn derive_trade_id(
    chain_id: &ChainId,
    block_height: BlockHeight,
    transaction_id: &TransactionId,
    canonical_event_index: u32,
) -> Result<TradeId, MappingError> {
    let mut hasher = blake3::Hasher::new_derive_key(TRADE_ID_CONTEXT);
    hash_bytes(&mut hasher, chain_id.as_str().as_bytes());
    hasher.update(&block_height.get().to_be_bytes());
    hash_bytes(&mut hasher, transaction_id.as_str().as_bytes());
    hasher.update(&canonical_event_index.to_be_bytes());
    TradeId::new(format!("trd_{}", hasher.finalize().to_hex())).map_err(|error| {
        MappingError::MalformedRecord {
            reason: format!("derived trade identity is invalid: {error}"),
        }
    })
}

fn hash_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    let length = match u64::try_from(bytes.len()) {
        Ok(length) => length,
        Err(_) => unreachable!("canonical identity fields cannot exceed u64 framing"),
    };
    hasher.update(&length.to_be_bytes());
    hasher.update(bytes);
}

fn is_lowercase_hash(value: &str) -> bool {
    value.len() == 66
        && value.starts_with("0x")
        && value[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
