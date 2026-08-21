use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use api_contracts::{
    WireAccountClassTransfer, WireInternalTransfer, WireNonUserOrderCancelled, WireRewardClaimed,
    WireSpotGenesisApplied, WireStakingDelegated, WireStakingDeposit, WireStakingUndelegated,
    WireStakingWithdrawalCompleted, WireStakingWithdrawalQueued, WireValidatorRewardPaid,
    WireVaultCreated, WireVaultDistribution, WireVaultLeaderCommissionPaid,
    encode_account_class_transfer, encode_internal_transfer, encode_non_user_order_cancelled,
    encode_reward_claimed, encode_spot_genesis_applied, encode_staking_delegated,
    encode_staking_deposit, encode_staking_undelegated, encode_staking_withdrawal_completed,
    encode_staking_withdrawal_queued, encode_validator_reward_paid, encode_vault_created,
    encode_vault_distribution, encode_vault_leader_commission_paid,
    is_canonical_trade_client_order_id,
};
use chrono::{DateTime, NaiveDateTime};
use domain_types::{
    Address, BlockHeight, ChainId, ClientOrderId, KnownTime, MarketId, OrderId, OrderSide,
    PositionQuantity, Price, ProtocolTime, Quantity, SourceId, TradeId, TransactionId, TwapId,
};
use hl_protocol::node::misc::parse_misc_event;
use hl_protocol::node::order_status::{
    BookSide, OrderStatusClass, OrderStatusV1, parse_order_status_batch,
};
use hl_protocol::node::raw_book_diff::{RawBookDiffOp, RawBookDiffV1, parse_raw_book_diff_batch};
use hl_protocol::node::trade::{TradeV1, parse_trade_batch};
use hl_protocol::node::v1::{NodeRecordKind, NodeRecordV1, NodeStreamKind};
use serde_json::{Map, Value};

use crate::{
    BlockEnvelope, BlockError, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass,
    ContractError, EventKind, EventPayload, OrderAccepted, OrderCancelled, OrderFilled,
    OrderRejected, OrderRested, SourceEvidence, TradeMatched, TradeParticipantRoleV1,
    TradeParticipantV1, TriggerOrderActivated,
};

const TRADE_ID_CONTEXT: &str = "hyperliquid-alpha-desk/trade-id/node-v1";
const ORDER_FILL_ID_CONTEXT: &str = "hyperliquid-alpha-desk/trade-id/order-status-fill-v1";

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
    OneSidedFill,
    AuxiliaryOrderStatus,
    AuxiliaryBookDiff,
    IncompleteLedgerTransfer,
    IncompleteLiquidation,
    AuxiliaryMarketMetadata,
}

impl EvidenceOnlyReason {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::MissingBlockContext => "canonical_mapping.missing_block_context",
            Self::UnsupportedCanonicalSemantics => {
                "canonical_mapping.unsupported_canonical_semantics"
            }
            Self::OneSidedFill => "canonical_mapping.one_sided_fill",
            Self::AuxiliaryOrderStatus => "canonical_mapping.auxiliary_order_status",
            Self::AuxiliaryBookDiff => "canonical_mapping.auxiliary_book_diff",
            Self::IncompleteLedgerTransfer => "canonical_mapping.incomplete_ledger_transfer",
            Self::IncompleteLiquidation => "canonical_mapping.incomplete_liquidation",
            Self::AuxiliaryMarketMetadata => "canonical_mapping.auxiliary_market_metadata",
        }
    }
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

#[must_use]
pub fn node_trade_match_key(trade_id: &TradeId) -> String {
    format!("node-trade:{}", trade_id.as_str())
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
    admit_committed_confirmation(context.confirmation_class)?;

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

/// Admit only committed primary and independent lanes into committed mapping.
///
/// Provisional, reconciled, corrected, and expired classes fail closed with
/// `InvalidCommittedConfirmation`. This does not qualify those lanes.
fn admit_committed_confirmation(class: ConfirmationClass) -> Result<(), MappingError> {
    match class {
        ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => Ok(()),
        ConfirmationClass::ProvisionalSource
        | ConfirmationClass::ReconciledSnapshot
        | ConfirmationClass::Corrected
        | ConfirmationClass::Expired => Err(MappingError::InvalidCommittedConfirmation),
    }
}

pub fn map_node_v1_record(
    record: &NodeRecordV1,
    catalog: &MarketCatalogV1,
    context: &NodeV1MappingContext,
) -> Result<MappingDisposition, MappingError> {
    match record.kind() {
        NodeRecordKind::EmptyBatch => Ok(MappingDisposition::EmptyBlock),
        NodeRecordKind::Trade if record.stream() == NodeStreamKind::Trades => {
            map_trade_record(record, catalog, context)
        }
        NodeRecordKind::OrderStatus => map_order_status_record(record, catalog, context),
        NodeRecordKind::RawBookDiff => map_raw_book_diff_record(record, catalog, context),
        NodeRecordKind::Fill => Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::OneSidedFill,
        )),
        NodeRecordKind::Liquidation => Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::IncompleteLiquidation,
        )),
        NodeRecordKind::MarketMetadata => Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::AuxiliaryMarketMetadata,
        )),
        NodeRecordKind::Transfer => map_misc_like_record(
            record,
            context,
            EvidenceOnlyReason::IncompleteLedgerTransfer,
        ),
        NodeRecordKind::MiscEvent => map_misc_like_record(
            record,
            context,
            EvidenceOnlyReason::UnsupportedCanonicalSemantics,
        ),
        NodeRecordKind::Trade
        | NodeRecordKind::TransactionBlock
        | NodeRecordKind::AbciStateSnapshot
        | NodeRecordKind::L4Snapshot => Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::UnsupportedCanonicalSemantics,
        )),
    }
}

fn map_trade_record(
    record: &NodeRecordV1,
    catalog: &MarketCatalogV1,
    context: &NodeV1MappingContext,
) -> Result<MappingDisposition, MappingError> {
    if record.block_number().is_none() {
        return Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::MissingBlockContext,
        ));
    }
    let batch = parse_trade_batch(record.payload().clone()).map_err(source_to_mapping)?;
    let block_time =
        parse_block_time(
            batch
                .block_time()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "trade batch has no block_time".to_owned(),
                })?,
        )?;
    let block_height =
        BlockHeight::new(
            batch
                .block_number()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "trade batch has no block_number".to_owned(),
                })?,
        );
    let parser_version = format!("{}/catalog:{}", context.mapper_version, catalog.version());
    let mut seen_transactions = BTreeSet::new();
    let mut current_transaction: Option<String> = None;
    let mut transaction_index = 0_u32;
    let mut canonical_event_index = 0_u32;
    let mut events = Vec::with_capacity(batch.trades().len());

    for (source_index, trade) in batch.trades().iter().enumerate() {
        let source_index =
            u32::try_from(source_index).map_err(|_| MappingError::EventIndexOverflow)?;
        match current_transaction.as_deref() {
            None => {
                seen_transactions.insert(trade.hash().to_owned());
                current_transaction = Some(trade.hash().to_owned());
            }
            Some(current) if current == trade.hash() => {
                canonical_event_index = canonical_event_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
            }
            Some(_) => {
                if !seen_transactions.insert(trade.hash().to_owned()) {
                    return Err(MappingError::NonContiguousTransaction);
                }
                transaction_index = transaction_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
                canonical_event_index = 0;
                current_transaction = Some(trade.hash().to_owned());
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

fn map_order_status_record(
    record: &NodeRecordV1,
    catalog: &MarketCatalogV1,
    context: &NodeV1MappingContext,
) -> Result<MappingDisposition, MappingError> {
    if record.block_number().is_none() {
        return Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::AuxiliaryOrderStatus,
        ));
    }
    let batch = parse_order_status_batch(record.payload().clone()).map_err(source_to_mapping)?;
    let block_time =
        parse_block_time(
            batch
                .block_time()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "order-status batch has no block_time".to_owned(),
                })?,
        )?;
    let block_height =
        BlockHeight::new(
            batch
                .block_number()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "order-status batch has no block_number".to_owned(),
                })?,
        );
    let parser_version = format!("{}/catalog:{}", context.mapper_version, catalog.version());
    let mut seen_transactions = BTreeSet::new();
    let mut current_transaction: Option<String> = None;
    let mut transaction_index = 0_u32;
    let mut canonical_event_index = 0_u32;
    let mut events = Vec::with_capacity(batch.statuses().len());
    for (source_index, status) in batch.statuses().iter().enumerate() {
        let source_index =
            u32::try_from(source_index).map_err(|_| MappingError::EventIndexOverflow)?;
        let identity = format!("node-order:{}", status.order().oid());
        match current_transaction.as_deref() {
            None => {
                seen_transactions.insert(identity.clone());
                current_transaction = Some(identity);
            }
            Some(current) if current == identity => {
                canonical_event_index = canonical_event_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
            }
            Some(_) => {
                if !seen_transactions.insert(identity.clone()) {
                    return Err(MappingError::NonContiguousTransaction);
                }
                transaction_index = transaction_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
                canonical_event_index = 0;
                current_transaction = Some(identity);
            }
        }
        events.push(map_order_status(
            status,
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

fn map_raw_book_diff_record(
    record: &NodeRecordV1,
    catalog: &MarketCatalogV1,
    context: &NodeV1MappingContext,
) -> Result<MappingDisposition, MappingError> {
    if record.block_number().is_none() {
        return Ok(MappingDisposition::EvidenceOnly(
            EvidenceOnlyReason::AuxiliaryBookDiff,
        ));
    }
    let batch = parse_raw_book_diff_batch(record.payload().clone()).map_err(source_to_mapping)?;
    let block_time =
        parse_block_time(
            batch
                .block_time()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "raw-book-diff batch has no block_time".to_owned(),
                })?,
        )?;
    let block_height =
        BlockHeight::new(
            batch
                .block_number()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "raw-book-diff batch has no block_number".to_owned(),
                })?,
        );
    let parser_version = format!("{}/catalog:{}", context.mapper_version, catalog.version());
    let mut seen_transactions = BTreeSet::new();
    let mut current_transaction: Option<String> = None;
    let mut transaction_index = 0_u32;
    let mut canonical_event_index = 0_u32;
    let mut events = Vec::with_capacity(batch.diffs().len());
    for (source_index, diff) in batch.diffs().iter().enumerate() {
        let source_index =
            u32::try_from(source_index).map_err(|_| MappingError::EventIndexOverflow)?;
        let identity = format!("node-l4:{}", diff.oid());
        match current_transaction.as_deref() {
            None => {
                seen_transactions.insert(identity.clone());
                current_transaction = Some(identity);
            }
            Some(current) if current == identity => {
                canonical_event_index = canonical_event_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
            }
            Some(_) => {
                if !seen_transactions.insert(identity.clone()) {
                    return Err(MappingError::NonContiguousTransaction);
                }
                transaction_index = transaction_index
                    .checked_add(1)
                    .ok_or(MappingError::EventIndexOverflow)?;
                canonical_event_index = 0;
                current_transaction = Some(identity);
            }
        }
        events.push(map_raw_book_diff(
            diff,
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
    root: &'a Map<String, Value>,
    abci_block: &'a Map<String, Value>,
) -> Result<&'a Vec<Value>, MappingError> {
    // Fail-closed owner for ambiguous signed_action_bundles. Parse-layer
    // `signed_action_bundles` returns empty on the same cases so existing
    // mapping tests can unwrap parse first.
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
    trade: &TradeV1,
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
    let market_id = catalog.resolve(trade.coin())?;
    let buyer =
        Address::parse_api(trade.buyer().user()).map_err(|_| MappingError::InvalidAddress)?;
    let seller =
        Address::parse_api(trade.seller().user()).map_err(|_| MappingError::InvalidAddress)?;
    if buyer == seller {
        return Err(MappingError::MalformedRecord {
            reason: "trade participant accounts must differ".to_owned(),
        });
    }
    let participants = [
        map_trade_participant(trade.buyer(), TradeParticipantRoleV1::Buyer, buyer)?,
        map_trade_participant(trade.seller(), TradeParticipantRoleV1::Seller, seller)?,
    ];
    let price = Price::from_str(trade.px()).map_err(|error| MappingError::InvalidDecimal {
        field: "price",
        reason: error.to_string(),
    })?;
    if price.raw() <= 0 {
        return Err(MappingError::InvalidDecimal {
            field: "price",
            reason: "trade price must be positive".to_owned(),
        });
    }
    let quantity =
        Quantity::from_str(trade.sz()).map_err(|error| MappingError::InvalidDecimal {
            field: "quantity",
            reason: error.to_string(),
        })?;
    if quantity.raw() <= 0 {
        return Err(MappingError::InvalidDecimal {
            field: "quantity",
            reason: "trade quantity must be positive".to_owned(),
        });
    }
    if !is_lowercase_hash(trade.hash()) {
        return Err(MappingError::InvalidTransactionHash);
    }
    let transaction_id = TransactionId::new(trade.hash().to_owned())
        .map_err(|_| MappingError::InvalidTransactionHash)?;
    let trade_id = derive_trade_id(
        &context.chain_id,
        block_height,
        &transaction_id,
        canonical_event_index,
    )?;
    let maker_order_id = Some(order_id_from_oid(trade.maker_oid())?);
    let taker_order_id = Some(order_id_from_oid(trade.taker_oid())?);
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
            maker_order_id,
            taker_order_id,
            price,
            quantity,
            deterministic_seed: 0,
            participants: Some(Box::new(participants)),
        }),
    })
    .map_err(Into::into)
}

fn map_trade_participant(
    source: &hl_protocol::node::trade::TradeSideV1,
    role: TradeParticipantRoleV1,
    account_id: Address,
) -> Result<TradeParticipantV1, MappingError> {
    let start_position = PositionQuantity::from_str(source.start_pos()).map_err(|error| {
        MappingError::InvalidDecimal {
            field: "start_position",
            reason: error.to_string(),
        }
    })?;
    if source.oid() == 0 {
        return Err(MappingError::MalformedRecord {
            reason: "trade participant oid must be positive".to_owned(),
        });
    }
    let twap_id = source.twap_id().map(TwapId::new);
    let client_order_id = match source.cloid() {
        None => None,
        Some(value) => {
            if !is_canonical_trade_client_order_id(value) {
                return Err(MappingError::MalformedRecord {
                    reason: "trade participant cloid must be lowercase 0x followed by exactly 32 lowercase hexadecimal digits".to_owned(),
                });
            }
            Some(ClientOrderId::new(value.to_owned()).map_err(|error| {
                MappingError::MalformedRecord {
                    reason: format!("invalid trade participant cloid: {error}"),
                }
            })?)
        }
    };
    Ok(TradeParticipantV1 {
        role,
        account_id,
        start_position,
        order_id: order_id_from_oid(source.oid())?,
        twap_id,
        client_order_id,
    })
}

#[allow(clippy::too_many_arguments)]
fn map_order_status(
    status: &OrderStatusV1,
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
    let order = status.order();
    let market_id = catalog.resolve(order.coin())?;
    let account_id = Address::parse_api(status.user()).map_err(|_| MappingError::InvalidAddress)?;
    let order_id = order_id_from_oid(order.oid())?;
    let side = match order.side() {
        BookSide::Bid => OrderSide::Buy,
        BookSide::Ask => OrderSide::Sell,
    };
    let limit_price = positive_price(order.limit_px())?;
    let remaining = nonnegative_quantity(order.sz())?;
    let original = positive_quantity(order.orig_sz())?;
    let transaction_id =
        TransactionId::new(format!("node-order:{}", order.oid())).map_err(|error| {
            MappingError::MalformedRecord {
                reason: format!("invalid order transaction identity: {error}"),
            }
        })?;
    let source_evidence = indexed_evidence(record, context, source_index)?;
    let payload = match status.class() {
        OrderStatusClass::Open => EventPayload::OrderAccepted(OrderAccepted {
            order_id,
            account_id,
            market_id: market_id.clone(),
            side,
            limit_price,
            quantity: original,
        }),
        OrderStatusClass::Canceled if is_system_cancel(status.status()) => opaque_payload(
            EventKind::NonUserOrderCancelled,
            encode_non_user_order_cancelled(&WireNonUserOrderCancelled {
                order_id: order_id.to_string(),
                reason: status.status().to_owned(),
                remaining_quantity: remaining.to_string(),
            })
            .map_err(payload_codec_mapping)?,
        )?,
        OrderStatusClass::Canceled => EventPayload::OrderCancelled(OrderCancelled {
            order_id,
            reason: status.status().to_owned(),
            remaining_quantity: remaining,
        }),
        OrderStatusClass::Rejected => match order.cloid() {
            Some(cloid) if is_canonical_trade_client_order_id(cloid) => {
                EventPayload::OrderRejected(OrderRejected {
                    client_order_id: ClientOrderId::new(cloid.to_owned()).map_err(|error| {
                        MappingError::MalformedRecord {
                            reason: format!("invalid rejected cloid: {error}"),
                        }
                    })?,
                    account_id,
                    reason_code: status.status().to_owned(),
                    reason: status.status().to_owned(),
                })
            }
            _ => EventPayload::OrderCancelled(OrderCancelled {
                order_id,
                reason: status.status().to_owned(),
                remaining_quantity: remaining,
            }),
        },
        OrderStatusClass::Filled => EventPayload::OrderFilled(OrderFilled {
            order_id,
            trade_id: derive_status_fill_id(
                &context.chain_id,
                block_height,
                order.oid(),
                order.timestamp(),
            )?,
            // ponytail: node order-status has limitPx, not a fill print.
            // fill_price is limitPx and fill_quantity is origSz until the trades stream.
            fill_price: limit_price,
            fill_quantity: original,
        }),
        OrderStatusClass::Triggered => {
            let trigger_price = positive_price(order.trigger_px())?;
            EventPayload::TriggerOrderActivated(TriggerOrderActivated {
                order_id,
                trigger_price,
                // ponytail: node order-status has triggerPx, not an oracle print.
                // Both fields carry triggerPx until a qualified corpus documents oracle.
                oracle_price: trigger_price,
            })
        }
    };
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: context.chain_id.clone(),
        block_height,
        block_time,
        transaction_id,
        transaction_index,
        canonical_event_index,
        market_ids: vec![market_id],
        account_ids: vec![account_id],
        source_evidence: vec![source_evidence],
        confirmation_class: ConfirmationClass::ProvisionalSource,
        observed_at: context.observed_at,
        ingested_at: context.ingested_at,
        canonicalized_at: context.canonicalized_at,
        parser_version: parser_version.to_owned(),
        payload,
    })
    .map_err(Into::into)
}

#[allow(clippy::too_many_arguments)]
fn map_raw_book_diff(
    diff: &RawBookDiffV1,
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
    let market_id = catalog.resolve(diff.coin())?;
    let account_id = Address::parse_api(diff.user()).map_err(|_| MappingError::InvalidAddress)?;
    let order_id = order_id_from_oid(diff.oid())?;
    let limit_price = positive_price(diff.px())?;
    let transaction_id =
        TransactionId::new(format!("node-l4:{}", diff.oid())).map_err(|error| {
            MappingError::MalformedRecord {
                reason: format!("invalid l4 transaction identity: {error}"),
            }
        })?;
    let source_evidence = indexed_evidence(record, context, source_index)?;
    let payload = match diff.op() {
        RawBookDiffOp::New { sz } | RawBookDiffOp::Update { sz } => {
            EventPayload::OrderRested(OrderRested {
                order_id,
                market_id: market_id.clone(),
                remaining_quantity: positive_quantity(sz)?,
                limit_price,
            })
        }
        RawBookDiffOp::Remove => EventPayload::OrderCancelled(OrderCancelled {
            order_id,
            reason: "raw_book_diff_remove".to_owned(),
            remaining_quantity: Quantity::from_str("0").map_err(|error| {
                MappingError::InvalidDecimal {
                    field: "quantity",
                    reason: error.to_string(),
                }
            })?,
        }),
    };
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: context.chain_id.clone(),
        block_height,
        block_time,
        transaction_id,
        transaction_index,
        canonical_event_index,
        market_ids: vec![market_id],
        account_ids: vec![account_id],
        source_evidence: vec![source_evidence],
        confirmation_class: ConfirmationClass::ProvisionalSource,
        observed_at: context.observed_at,
        ingested_at: context.ingested_at,
        canonicalized_at: context.canonicalized_at,
        parser_version: parser_version.to_owned(),
        payload,
    })
    .map_err(Into::into)
}

fn indexed_evidence(
    record: &NodeRecordV1,
    context: &NodeV1MappingContext,
    source_index: u32,
) -> Result<SourceEvidence, MappingError> {
    SourceEvidence::try_new_indexed(
        context.source_id.clone(),
        context.source_version.clone(),
        context.source_offset.clone(),
        *record.content_hash().as_bytes(),
        source_index,
    )
    .map_err(Into::into)
}

fn order_id_from_oid(oid: u64) -> Result<OrderId, MappingError> {
    OrderId::new(oid.to_string()).map_err(|error| MappingError::MalformedRecord {
        reason: format!("invalid order id: {error}"),
    })
}

fn positive_price(value: &str) -> Result<Price, MappingError> {
    let price = Price::from_str(value).map_err(|error| MappingError::InvalidDecimal {
        field: "price",
        reason: error.to_string(),
    })?;
    if price.raw() <= 0 {
        return Err(MappingError::InvalidDecimal {
            field: "price",
            reason: "price must be positive".to_owned(),
        });
    }
    Ok(price)
}

fn positive_quantity(value: &str) -> Result<Quantity, MappingError> {
    let quantity = Quantity::from_str(value).map_err(|error| MappingError::InvalidDecimal {
        field: "quantity",
        reason: error.to_string(),
    })?;
    if quantity.raw() <= 0 {
        return Err(MappingError::InvalidDecimal {
            field: "quantity",
            reason: "quantity must be positive".to_owned(),
        });
    }
    Ok(quantity)
}

fn nonnegative_quantity(value: &str) -> Result<Quantity, MappingError> {
    let quantity = Quantity::from_str(value).map_err(|error| MappingError::InvalidDecimal {
        field: "quantity",
        reason: error.to_string(),
    })?;
    if quantity.raw() < 0 {
        return Err(MappingError::InvalidDecimal {
            field: "quantity",
            reason: "quantity must be non-negative".to_owned(),
        });
    }
    Ok(quantity)
}

fn map_misc_like_record(
    record: &NodeRecordV1,
    context: &NodeV1MappingContext,
    unmapped: EvidenceOnlyReason,
) -> Result<MappingDisposition, MappingError> {
    if record.block_number().is_none() {
        return Ok(MappingDisposition::EvidenceOnly(unmapped));
    }
    let (events, block_time, block_height) = batched_misc_events(record.payload())?;
    let mut mapped = Vec::new();
    let mut transaction_index = 0_u32;
    for (source_index, event) in events.iter().enumerate() {
        let source_index =
            u32::try_from(source_index).map_err(|_| MappingError::EventIndexOverflow)?;
        parse_misc_event(event, Some(block_height.get()), source_index)
            .map_err(source_to_mapping)?;
        let Some(pieces) = map_misc_payload(event)? else {
            return Ok(MappingDisposition::EvidenceOnly(unmapped));
        };
        if source_index > 0 {
            transaction_index = transaction_index
                .checked_add(1)
                .ok_or(MappingError::EventIndexOverflow)?;
        }
        let mut canonical_event_index = 0_u32;
        for piece in pieces {
            mapped.push(envelope_from_misc(
                piece,
                source_index,
                transaction_index,
                canonical_event_index,
                block_height,
                block_time,
                record,
                context,
            )?);
            canonical_event_index = canonical_event_index
                .checked_add(1)
                .ok_or(MappingError::EventIndexOverflow)?;
        }
    }
    Ok(MappingDisposition::Mapped(mapped))
}

struct MiscPiece {
    payload: EventPayload,
    account_ids: Vec<Address>,
    transaction_id: TransactionId,
}

#[allow(clippy::too_many_arguments)]
fn envelope_from_misc(
    piece: MiscPiece,
    source_index: u32,
    transaction_index: u32,
    canonical_event_index: u32,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    record: &NodeRecordV1,
    context: &NodeV1MappingContext,
) -> Result<CanonicalEventEnvelope, MappingError> {
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: context.chain_id.clone(),
        block_height,
        block_time,
        transaction_id: piece.transaction_id,
        transaction_index,
        canonical_event_index,
        market_ids: Vec::new(),
        account_ids: piece.account_ids,
        source_evidence: vec![indexed_evidence(record, context, source_index)?],
        confirmation_class: ConfirmationClass::ProvisionalSource,
        observed_at: context.observed_at,
        ingested_at: context.ingested_at,
        canonicalized_at: context.canonicalized_at,
        parser_version: context.mapper_version.clone(),
        payload: piece.payload,
    })
    .map_err(Into::into)
}

#[allow(clippy::type_complexity)]
fn batched_misc_events(
    payload: &[u8],
) -> Result<(Vec<Map<String, Value>>, ProtocolTime, BlockHeight), MappingError> {
    let root: Value =
        serde_json::from_slice(payload).map_err(|error| MappingError::MalformedRecord {
            reason: error.to_string(),
        })?;
    let object = root
        .as_object()
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "misc record root must be an object".to_owned(),
        })?;
    let events = object
        .get("events")
        .and_then(Value::as_array)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "batched misc record has no events array".to_owned(),
        })?;
    let block_time = parse_block_time(
        object
            .get("block_time")
            .and_then(Value::as_str)
            .ok_or_else(|| MappingError::MalformedRecord {
                reason: "batched misc record has no block_time".to_owned(),
            })?,
    )?;
    let block_height = BlockHeight::new(
        object
            .get("block_number")
            .and_then(Value::as_u64)
            .ok_or_else(|| MappingError::MalformedRecord {
                reason: "batched misc record has no block_number".to_owned(),
            })?,
    );
    let objects = events
        .iter()
        .map(|event| {
            event
                .as_object()
                .cloned()
                .ok_or_else(|| MappingError::MalformedRecord {
                    reason: "misc event must be an object".to_owned(),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok((objects, block_time, block_height))
}

fn map_misc_payload(event: &Map<String, Value>) -> Result<Option<Vec<MiscPiece>>, MappingError> {
    let hash = json_string(event, "hash")?;
    let transaction_id =
        TransactionId::new(hash).map_err(|error| MappingError::MalformedRecord {
            reason: format!("invalid misc transaction identity: {error}"),
        })?;
    let inner = event
        .get("inner")
        .and_then(Value::as_object)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "misc event has no inner object".to_owned(),
        })?;
    let (variant, value) = inner
        .iter()
        .next()
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "misc event inner is empty".to_owned(),
        })?;
    let body = value
        .as_object()
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "misc event payload must be an object".to_owned(),
        })?;
    match variant.as_str() {
        "CDeposit" => Ok(Some(vec![staking_piece(
            EventKind::StakingDeposit,
            encode_staking_deposit(&WireStakingDeposit {
                account_id: json_string(body, "user")?,
                amount: json_string(body, "amount")?,
            })
            .map_err(payload_codec_mapping)?,
            json_string(body, "user")?,
            transaction_id,
        )?])),
        "Delegation" => {
            let undelegate = json_bool(body, "is_undelegate")?;
            let (kind, bytes) = if undelegate {
                (
                    EventKind::StakingUndelegated,
                    encode_staking_undelegated(&WireStakingUndelegated {
                        account_id: json_string(body, "user")?,
                        validator: json_string(body, "validator")?,
                        amount: json_string(body, "amount")?,
                    })
                    .map_err(payload_codec_mapping)?,
                )
            } else {
                (
                    EventKind::StakingDelegated,
                    encode_staking_delegated(&WireStakingDelegated {
                        account_id: json_string(body, "user")?,
                        validator: json_string(body, "validator")?,
                        amount: json_string(body, "amount")?,
                    })
                    .map_err(payload_codec_mapping)?,
                )
            };
            Ok(Some(vec![staking_piece(
                kind,
                bytes,
                json_string(body, "user")?,
                transaction_id,
            )?]))
        }
        "CWithdrawal" => {
            let finalized = json_bool(body, "is_finalized")?;
            let (kind, bytes) = if finalized {
                (
                    EventKind::StakingWithdrawalCompleted,
                    encode_staking_withdrawal_completed(&WireStakingWithdrawalCompleted {
                        account_id: json_string(body, "user")?,
                        amount: json_string(body, "amount")?,
                    })
                    .map_err(payload_codec_mapping)?,
                )
            } else {
                (
                    EventKind::StakingWithdrawalQueued,
                    encode_staking_withdrawal_queued(&WireStakingWithdrawalQueued {
                        account_id: json_string(body, "user")?,
                        amount: json_string(body, "amount")?,
                    })
                    .map_err(payload_codec_mapping)?,
                )
            };
            Ok(Some(vec![staking_piece(
                kind,
                bytes,
                json_string(body, "user")?,
                transaction_id,
            )?]))
        }
        "ValidatorRewards" => map_validator_rewards(body, transaction_id),
        "LedgerUpdate" => map_ledger_delta(body, transaction_id),
        _ => Ok(None),
    }
}

fn map_validator_rewards(
    body: &Map<String, Value>,
    transaction_id: TransactionId,
) -> Result<Option<Vec<MiscPiece>>, MappingError> {
    let pairs = body
        .get("validator_to_reward")
        .and_then(Value::as_array)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "validator rewards have no validator_to_reward array".to_owned(),
        })?;
    if pairs.is_empty() {
        return Ok(None);
    }
    let mut pieces = Vec::new();
    for pair in pairs {
        let pair = pair
            .as_array()
            .filter(|value| value.len() == 2)
            .ok_or_else(|| MappingError::MalformedRecord {
                reason: "validator_to_reward entries must be pairs".to_owned(),
            })?;
        let validator = pair[0]
            .as_str()
            .ok_or_else(|| MappingError::MalformedRecord {
                reason: "validator_to_reward key must be a string".to_owned(),
            })?
            .to_owned();
        let amount = pair[1]
            .as_str()
            .ok_or_else(|| MappingError::MalformedRecord {
                reason: "validator_to_reward amount must be a decimal string".to_owned(),
            })?
            .to_owned();
        pieces.push(MiscPiece {
            payload: opaque_payload(
                EventKind::ValidatorRewardPaid,
                encode_validator_reward_paid(&WireValidatorRewardPaid {
                    validator: validator.clone(),
                    amount,
                })
                .map_err(payload_codec_mapping)?,
            )?,
            account_ids: Vec::new(),
            transaction_id: transaction_id.clone(),
        });
    }
    Ok(Some(pieces))
}

fn map_ledger_delta(
    ledger: &Map<String, Value>,
    transaction_id: TransactionId,
) -> Result<Option<Vec<MiscPiece>>, MappingError> {
    let users = ledger_users(ledger)?;
    let delta = ledger
        .get("delta")
        .and_then(Value::as_object)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "ledger update has no delta object".to_owned(),
        })?;
    let (variant, value) = delta
        .iter()
        .next()
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "ledger delta is empty".to_owned(),
        })?;
    let body = value
        .as_object()
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "ledger delta payload must be an object".to_owned(),
        })?;
    let (kind, bytes, accounts) = match variant.as_str() {
        "InternalTransfer" => (
            EventKind::InternalTransfer,
            encode_internal_transfer(&WireInternalTransfer {
                from_account_id: json_string(body, "user")?,
                to_account_id: json_string(body, "destination")?,
                amount: json_string(body, "usdc")?,
                fee: json_string(body, "fee")?,
            })
            .map_err(payload_codec_mapping)?,
            vec![
                parse_account(json_string(body, "user")?)?,
                parse_account(json_string(body, "destination")?)?,
            ],
        ),
        "AccountClassTransfer" => {
            let account = users.first().copied().ok_or(MappingError::InvalidAddress)?;
            (
                EventKind::AccountClassTransfer,
                encode_account_class_transfer(&WireAccountClassTransfer {
                    account_id: account.to_api_string(),
                    amount: json_string(body, "usdc")?,
                    to_perp: json_bool(body, "toPerp")?,
                })
                .map_err(payload_codec_mapping)?,
                vec![account],
            )
        }
        "VaultCreate" => (
            EventKind::VaultCreated,
            encode_vault_created(&WireVaultCreated {
                vault_id: json_string(body, "vault")?,
                amount: json_string(body, "usdc")?,
                fee: json_string(body, "fee")?,
            })
            .map_err(payload_codec_mapping)?,
            users,
        ),
        "VaultDistribution" => (
            EventKind::VaultDistribution,
            encode_vault_distribution(&WireVaultDistribution {
                vault_id: json_string(body, "vault")?,
                amount: json_string(body, "usdc")?,
            })
            .map_err(payload_codec_mapping)?,
            users,
        ),
        "VaultLeaderCommission" => (
            EventKind::VaultLeaderCommissionPaid,
            encode_vault_leader_commission_paid(&WireVaultLeaderCommissionPaid {
                vault_id: body
                    .get("vault")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                account_id: users
                    .first()
                    .map(|account| account.to_api_string())
                    .unwrap_or_default(),
                amount: body
                    .get("usdc")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            })
            .map_err(payload_codec_mapping)?,
            users,
        ),
        "RewardsClaim" => {
            let account = users.first().copied().ok_or(MappingError::InvalidAddress)?;
            (
                EventKind::RewardClaimed,
                encode_reward_claimed(&WireRewardClaimed {
                    account_id: account.to_api_string(),
                    amount: json_string(body, "amount")?,
                })
                .map_err(payload_codec_mapping)?,
                vec![account],
            )
        }
        "SpotGenesis" => (
            EventKind::SpotGenesisApplied,
            encode_spot_genesis_applied(&WireSpotGenesisApplied {
                token: json_string(body, "token")?,
                amount: json_string(body, "amount")?,
            })
            .map_err(payload_codec_mapping)?,
            users,
        ),
        _ => return Ok(None),
    };
    Ok(Some(vec![MiscPiece {
        payload: opaque_payload(kind, bytes)?,
        account_ids: accounts,
        transaction_id,
    }]))
}

fn staking_piece(
    kind: EventKind,
    bytes: Vec<u8>,
    user: String,
    transaction_id: TransactionId,
) -> Result<MiscPiece, MappingError> {
    Ok(MiscPiece {
        payload: opaque_payload(kind, bytes)?,
        account_ids: vec![parse_account(user)?],
        transaction_id,
    })
}

fn ledger_users(ledger: &Map<String, Value>) -> Result<Vec<Address>, MappingError> {
    let users = ledger
        .get("users")
        .and_then(Value::as_array)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: "ledger update has no users array".to_owned(),
        })?;
    users
        .iter()
        .map(|user| {
            let text = user.as_str().ok_or(MappingError::InvalidAddress)?;
            parse_account(text.to_owned())
        })
        .collect()
}

fn parse_account(value: String) -> Result<Address, MappingError> {
    Address::parse_api(&value).map_err(|_| MappingError::InvalidAddress)
}

fn json_string(object: &Map<String, Value>, field: &str) -> Result<String, MappingError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: format!("missing string field {field}"),
        })
}

fn json_bool(object: &Map<String, Value>, field: &str) -> Result<bool, MappingError> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| MappingError::MalformedRecord {
            reason: format!("missing boolean field {field}"),
        })
}

fn is_system_cancel(status: &str) -> bool {
    matches!(
        status,
        "marginCanceled"
            | "vaultWithdrawalCanceled"
            | "openInterestCapCanceled"
            | "selfTradeCanceled"
            | "reduceOnlyCanceled"
            | "siblingFilledCanceled"
            | "delistedCanceled"
            | "liquidatedCanceled"
    )
}

fn opaque_payload(kind: EventKind, bytes: Vec<u8>) -> Result<EventPayload, MappingError> {
    EventPayload::decode(kind, &bytes).map_err(Into::into)
}

fn payload_codec_mapping(error: api_contracts::PayloadCodecError) -> MappingError {
    MappingError::MalformedRecord {
        reason: error.to_string(),
    }
}

fn source_to_mapping(error: hl_protocol::SourceError) -> MappingError {
    MappingError::MalformedRecord {
        reason: error.to_string(),
    }
}

fn derive_status_fill_id(
    chain_id: &ChainId,
    block_height: BlockHeight,
    oid: u64,
    timestamp: u64,
) -> Result<TradeId, MappingError> {
    let mut hasher = blake3::Hasher::new_derive_key(ORDER_FILL_ID_CONTEXT);
    hash_bytes(&mut hasher, chain_id.as_str().as_bytes());
    hasher.update(&block_height.get().to_be_bytes());
    hasher.update(&oid.to_be_bytes());
    hasher.update(&timestamp.to_be_bytes());
    TradeId::new(format!("trd_{}", hasher.finalize().to_hex())).map_err(|error| {
        MappingError::MalformedRecord {
            reason: format!("derived fill identity is invalid: {error}"),
        }
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
