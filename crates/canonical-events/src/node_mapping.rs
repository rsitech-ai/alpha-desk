use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use chrono::{DateTime, NaiveDateTime};
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, Price, ProtocolTime, Quantity, SourceId,
    TradeId, TransactionId,
};
use hl_protocol::node::v1::{NodeRecordKind, NodeRecordV1, NodeStreamKind};
use serde::Deserialize;

use crate::{
    CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, ContractError, EventPayload,
    SourceEvidence, TradeMatched,
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
    #[error("canonical event contract rejected mapped source: {0}")]
    Contract(#[from] ContractError),
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
            Self::Contract(_) => "canonical_mapping.contract_rejected",
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
        }),
    })
    .map_err(Into::into)
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
