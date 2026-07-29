use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, MarketId, ProtocolTime, TransactionId,
};

use crate::{ConfirmationClass, EventPayload, SourceEvidence};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CanonicalEventInput {
    pub schema_version: String,
    pub chain_id: ChainId,
    pub block_height: BlockHeight,
    pub block_time: ProtocolTime,
    pub transaction_id: TransactionId,
    pub transaction_index: u32,
    pub canonical_event_index: u32,
    pub market_ids: Vec<MarketId>,
    pub account_ids: Vec<Address>,
    pub source_evidence: Vec<SourceEvidence>,
    pub confirmation_class: ConfirmationClass,
    pub observed_at: KnownTime,
    pub ingested_at: KnownTime,
    pub canonicalized_at: KnownTime,
    pub parser_version: String,
    pub payload: EventPayload,
}
