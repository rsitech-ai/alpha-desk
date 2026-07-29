use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{Address, BlockHeight, EventId, MarketId, Price, Quantity, TradeId};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView};

const TRADE_NAMESPACE: &str = "trade.v1";
const PARTICIPANT_NAMESPACE: &str = "trade-participant.v1";
const RECONCILIATION_NAMESPACE: &str = "reconciliation.v1";
const TRADE_SCHEMA: &str = "hyperliquid-alpha-desk/trade-state/v1";
const PARTICIPANT_SCHEMA: &str = "hyperliquid-alpha-desk/trade-participant/v1";
const RECONCILIATION_SCHEMA: &str = "hyperliquid-alpha-desk/trade-quantity-symmetry/v1";
const RECONCILIATION_CHECK_VERSION: &str = "trade-quantity-symmetry@1.0.0";
const MAX_RECORD_BYTES: usize = 16 * 1024;
const EVIDENCE_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/trade-reconciliation-evidence/v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalTradeReducerV1;

impl CanonicalTradeReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-trade@1.0.0";
}

impl EventReducer for CanonicalTradeReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::TradeMatched && event.schema_version() == "1.0.0"
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let EventPayload::TradeMatched(trade) = event.payload() else {
            return Err(reducer_error(
                "trade_state.invalid_event",
                "trade reducer received a non-trade payload",
            ));
        };
        let trade_id = trade.trade_id.as_ref().ok_or_else(|| {
            reducer_error("trade_state.invalid_trade_id", "trade identity is required")
        })?;
        let market_id = trade.market_id.as_ref().ok_or_else(|| {
            reducer_error(
                "trade_state.invalid_market",
                "trade payload market is required",
            )
        })?;
        if event.market_ids() != std::slice::from_ref(market_id) {
            return Err(reducer_error(
                "trade_state.invalid_market",
                "trade payload and envelope market must match exactly",
            ));
        }
        let participants: [Address; 2] = event.account_addresses().try_into().map_err(|_| {
            reducer_error(
                "trade_state.invalid_participants",
                "trade requires exactly two participants",
            )
        })?;
        if participants[0] == participants[1] {
            return Err(reducer_error(
                "trade_state.invalid_participants",
                "trade participants must be distinct",
            ));
        }
        if trade.price.raw() <= 0 {
            return Err(reducer_error(
                "trade_state.invalid_price",
                "trade price must be positive",
            ));
        }
        if trade.quantity.raw() <= 0 {
            return Err(reducer_error(
                "trade_state.invalid_quantity",
                "trade quantity must be positive",
            ));
        }

        let trade_key = TradeStateRecordV1::state_key(trade_id).map_err(codec_reducer_error)?;
        if state.contains_key(&trade_key) {
            return Err(reducer_error(
                "trade_state.trade_id_collision",
                "trade identity is already present in canonical state",
            ));
        }

        let record = TradeStateRecordV1 {
            event_id: event.event_id().clone(),
            trade_id: trade_id.clone(),
            market_id: market_id.clone(),
            price: trade.price,
            quantity: trade.quantity,
            participants,
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
        };
        let reconciliation = TradeReconciliationRecordV1 {
            event_id: event.event_id().clone(),
            trade_id: trade_id.clone(),
            market_id: market_id.clone(),
            quantity: trade.quantity,
            participant_count: 2,
            block_height: event.block_height(),
            evidence_hash: reconciliation_evidence_hash(&record),
        };

        Ok(vec![
            StateMutation::put(trade_key, record.encode().map_err(codec_reducer_error)?),
            StateMutation::put(
                TradeParticipantRecordV1::state_key(trade_id, 0).map_err(codec_reducer_error)?,
                TradeParticipantRecordV1 {
                    event_id: event.event_id().clone(),
                    trade_id: trade_id.clone(),
                    ordinal: 0,
                    participant: participants[0],
                    quantity: trade.quantity,
                    block_height: event.block_height(),
                }
                .encode()
                .map_err(codec_reducer_error)?,
            ),
            StateMutation::put(
                TradeParticipantRecordV1::state_key(trade_id, 1).map_err(codec_reducer_error)?,
                TradeParticipantRecordV1 {
                    event_id: event.event_id().clone(),
                    trade_id: trade_id.clone(),
                    ordinal: 1,
                    participant: participants[1],
                    quantity: trade.quantity,
                    block_height: event.block_height(),
                }
                .encode()
                .map_err(codec_reducer_error)?,
            ),
            StateMutation::put(
                TradeReconciliationRecordV1::state_key(trade_id).map_err(codec_reducer_error)?,
                reconciliation.encode().map_err(codec_reducer_error)?,
            ),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeStateRecordV1 {
    event_id: EventId,
    trade_id: TradeId,
    market_id: MarketId,
    price: Price,
    quantity: Quantity,
    participants: [Address; 2],
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl TradeStateRecordV1 {
    pub fn state_key(trade_id: &TradeId) -> Result<StateKey, TradeStateError> {
        StateKey::try_new(TRADE_NAMESPACE, trade_id.as_str().as_bytes().to_vec())
            .map_err(|_| TradeStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TradeStateError> {
        let wire: TradeStateWire = decode_canonical(bytes)?;
        if wire.schema != TRADE_SCHEMA {
            return Err(TradeStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TradeStateError::InvalidRecord)?,
            trade_id: TradeId::new(wire.trade_id).map_err(|_| TradeStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| TradeStateError::InvalidRecord)?,
            price: Price::from_str(&wire.price).map_err(|_| TradeStateError::InvalidRecord)?,
            quantity: Quantity::from_str(&wire.quantity)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            participants: [
                Address::parse_api(&wire.participant_0)
                    .map_err(|_| TradeStateError::InvalidRecord)?,
                Address::parse_api(&wire.participant_1)
                    .map_err(|_| TradeStateError::InvalidRecord)?,
            ],
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
        };
        if record.price.raw() <= 0
            || record.quantity.raw() <= 0
            || record.participants[0] == record.participants[1]
        {
            return Err(TradeStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TradeStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.trade_id)? != *key {
            return Err(TradeStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TradeStateError> {
        encode_canonical(&TradeStateWire {
            schema: TRADE_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            trade_id: self.trade_id.as_str().to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            price: self.price.to_string(),
            quantity: self.quantity.to_string(),
            participant_0: self.participants[0].to_api_string(),
            participant_1: self.participants[1].to_api_string(),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
        })
    }

    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn trade_id(&self) -> &TradeId {
        &self.trade_id
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn price(&self) -> Price {
        self.price
    }

    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    #[must_use]
    pub const fn participants(&self) -> [Address; 2] {
        self.participants
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn payload_hash(&self) -> [u8; 32] {
        self.payload_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeParticipantRecordV1 {
    event_id: EventId,
    trade_id: TradeId,
    ordinal: u8,
    participant: Address,
    quantity: Quantity,
    block_height: BlockHeight,
}

impl TradeParticipantRecordV1 {
    pub fn state_key(trade_id: &TradeId, ordinal: u8) -> Result<StateKey, TradeStateError> {
        if ordinal > 1 {
            return Err(TradeStateError::InvalidKey);
        }
        let id = trade_id.as_str().as_bytes();
        let length = u16::try_from(id.len()).map_err(|_| TradeStateError::InvalidKey)?;
        let mut key = Vec::with_capacity(2 + id.len() + 1);
        key.extend_from_slice(&length.to_be_bytes());
        key.extend_from_slice(id);
        key.push(ordinal);
        StateKey::try_new(PARTICIPANT_NAMESPACE, key).map_err(|_| TradeStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TradeStateError> {
        let wire: ParticipantWire = decode_canonical(bytes)?;
        if wire.schema != PARTICIPANT_SCHEMA || wire.ordinal > 1 {
            return Err(TradeStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TradeStateError::InvalidRecord)?,
            trade_id: TradeId::new(wire.trade_id).map_err(|_| TradeStateError::InvalidRecord)?,
            ordinal: wire.ordinal,
            participant: Address::parse_api(&wire.participant)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            quantity: Quantity::from_str(&wire.quantity)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
        };
        if record.quantity.raw() <= 0 {
            return Err(TradeStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TradeStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.trade_id, record.ordinal)? != *key {
            return Err(TradeStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TradeStateError> {
        encode_canonical(&ParticipantWire {
            schema: PARTICIPANT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            trade_id: self.trade_id.as_str().to_owned(),
            ordinal: self.ordinal,
            participant: self.participant.to_api_string(),
            quantity: self.quantity.to_string(),
            block_height: self.block_height.get(),
        })
    }

    #[must_use]
    pub const fn ordinal(&self) -> u8 {
        self.ordinal
    }

    #[must_use]
    pub const fn participant(&self) -> Address {
        self.participant
    }

    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeReconciliationRecordV1 {
    event_id: EventId,
    trade_id: TradeId,
    market_id: MarketId,
    quantity: Quantity,
    participant_count: u8,
    block_height: BlockHeight,
    evidence_hash: [u8; 32],
}

impl TradeReconciliationRecordV1 {
    pub fn state_key(trade_id: &TradeId) -> Result<StateKey, TradeStateError> {
        StateKey::try_new(
            RECONCILIATION_NAMESPACE,
            trade_id.as_str().as_bytes().to_vec(),
        )
        .map_err(|_| TradeStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TradeStateError> {
        let wire: ReconciliationWire = decode_canonical(bytes)?;
        if wire.schema != RECONCILIATION_SCHEMA
            || wire.check_version != RECONCILIATION_CHECK_VERSION
            || wire.status != "passed"
            || wire.participant_count != 2
        {
            return Err(TradeStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TradeStateError::InvalidRecord)?,
            trade_id: TradeId::new(wire.trade_id).map_err(|_| TradeStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| TradeStateError::InvalidRecord)?,
            quantity: Quantity::from_str(&wire.quantity)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            participant_count: wire.participant_count,
            block_height: BlockHeight::new(wire.block_height),
            evidence_hash: decode_hash(&wire.evidence_blake3)?,
        };
        if record.quantity.raw() <= 0 {
            return Err(TradeStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TradeStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.trade_id)? != *key {
            return Err(TradeStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TradeStateError> {
        encode_canonical(&ReconciliationWire {
            schema: RECONCILIATION_SCHEMA.to_owned(),
            check_version: RECONCILIATION_CHECK_VERSION.to_owned(),
            status: "passed".to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            trade_id: self.trade_id.as_str().to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            quantity: self.quantity.to_string(),
            participant_count: self.participant_count,
            block_height: self.block_height.get(),
            evidence_blake3: hex::encode(self.evidence_hash),
        })
    }

    #[must_use]
    pub const fn passed(&self) -> bool {
        true
    }

    #[must_use]
    pub const fn trade_id(&self) -> &TradeId {
        &self.trade_id
    }

    #[must_use]
    pub const fn quantity(&self) -> Quantity {
        self.quantity
    }

    #[must_use]
    pub const fn participant_count(&self) -> u8 {
        self.participant_count
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn evidence_hash(&self) -> [u8; 32] {
        self.evidence_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TradeStateError {
    #[error("trade-state key is invalid")]
    InvalidKey,
    #[error("trade-state record cannot be decoded")]
    Codec,
    #[error("trade-state record bytes are not canonical")]
    NonCanonical,
    #[error("trade-state record is invalid")]
    InvalidRecord,
    #[error("trade-state record identity does not match its key")]
    KeyMismatch,
    #[error("trade-state record exceeds its deterministic bound")]
    LimitExceeded,
}

impl TradeStateError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "trade_state.codec.invalid_key",
            Self::Codec => "trade_state.codec.decode",
            Self::NonCanonical => "trade_state.codec.noncanonical",
            Self::InvalidRecord => "trade_state.codec.invalid_record",
            Self::KeyMismatch => "trade_state.codec.key_mismatch",
            Self::LimitExceeded => "trade_state.codec.limit_exceeded",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeStateWire {
    schema: String,
    event_id: String,
    trade_id: String,
    market_id: String,
    price: String,
    quantity: String,
    participant_0: String,
    participant_1: String,
    block_height: u64,
    payload_blake3: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParticipantWire {
    schema: String,
    event_id: String,
    trade_id: String,
    ordinal: u8,
    participant: String,
    quantity: String,
    block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReconciliationWire {
    schema: String,
    check_version: String,
    status: String,
    event_id: String,
    trade_id: String,
    market_id: String,
    quantity: String,
    participant_count: u8,
    block_height: u64,
    evidence_blake3: String,
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, TradeStateError> {
    let bytes = serde_json::to_vec(value).map_err(|_| TradeStateError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(TradeStateError::LimitExceeded);
    }
    Ok(bytes)
}

fn decode_canonical<T>(bytes: &[u8]) -> Result<T, TradeStateError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(TradeStateError::LimitExceeded);
    }
    let value = serde_json::from_slice(bytes).map_err(|_| TradeStateError::Codec)?;
    if encode_canonical(&value)? != bytes {
        return Err(TradeStateError::NonCanonical);
    }
    Ok(value)
}

fn decode_hash(value: &str) -> Result<[u8; 32], TradeStateError> {
    if value.len() != 64 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(TradeStateError::InvalidRecord);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| TradeStateError::InvalidRecord)?;
    Ok(hash)
}

fn reconciliation_evidence_hash(record: &TradeStateRecordV1) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(EVIDENCE_HASH_CONTEXT);
    frame(&mut hasher, record.event_id.as_str().as_bytes());
    frame(&mut hasher, record.trade_id.as_str().as_bytes());
    frame(&mut hasher, record.market_id.as_str().as_bytes());
    frame(&mut hasher, record.quantity.to_string().as_bytes());
    hasher.update(record.participants[0].as_bytes());
    hasher.update(record.participants[1].as_bytes());
    hasher.update(&record.block_height.get().to_be_bytes());
    hasher.update(&record.payload_hash);
    *hasher.finalize().as_bytes()
}

fn frame(hasher: &mut blake3::Hasher, value: &[u8]) {
    let length = u64::try_from(value.len()).expect("bounded trade evidence field");
    hasher.update(&length.to_be_bytes());
    hasher.update(value);
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn codec_reducer_error(error: TradeStateError) -> ReducerError {
    match error {
        TradeStateError::InvalidKey => reducer_error(
            "trade_state.codec_invalid_key",
            "trade state key encoding failed",
        ),
        TradeStateError::LimitExceeded => reducer_error(
            "trade_state.codec_limit_exceeded",
            "trade state record exceeds its deterministic bound",
        ),
        TradeStateError::Codec
        | TradeStateError::NonCanonical
        | TradeStateError::InvalidRecord
        | TradeStateError::KeyMismatch => reducer_error(
            "trade_state.codec_failed",
            "trade state record encoding failed",
        ),
    }
}
