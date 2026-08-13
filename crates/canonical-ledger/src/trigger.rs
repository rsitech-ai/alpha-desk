use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{Address, BlockHeight, EventId, MarketId, OrderId, Price};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView};

const FACT_NAMESPACE: &str = "trigger-fact.v1";
const CURRENT_NAMESPACE: &str = "trigger-current.v1";
const TRANSITION_NAMESPACE: &str = "trigger-transition.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/trigger-fact/v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/trigger-current/v1";
const TRANSITION_SCHEMA: &str = "hyperliquid-alpha-desk/trigger-transition/v1";
const CURRENT_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/trigger-current-hash/v1";
const MAX_RECORD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalTriggerReducerV1;

impl CanonicalTriggerReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-trigger@1.0.0";
}

impl EventReducer for CanonicalTriggerReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0" && event.event_kind() == EventKind::TriggerOrderActivated
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let EventPayload::TriggerOrderActivated(activated) = event.payload() else {
            return Err(reducer_error(
                "trigger_state.invalid_event",
                "trigger reducer received a non-trigger payload",
            ));
        };
        let [market_id] = event.market_ids() else {
            return Err(reducer_error(
                "trigger_state.identity_mismatch",
                "trigger event requires exactly one envelope market",
            ));
        };
        let [account_id] = event.account_addresses() else {
            return Err(reducer_error(
                "trigger_state.identity_mismatch",
                "trigger event requires exactly one envelope account",
            ));
        };

        let current_key = TriggerCurrentRecordV1::state_key(market_id, &activated.order_id)
            .map_err(codec_reducer_error)?;
        if state.contains_key(&current_key) {
            return Err(reducer_error(
                "trigger_state.order_id_collision",
                "trigger identity is already present in canonical state",
            ));
        }

        let current = TriggerCurrentRecordV1 {
            order_id: activated.order_id.clone(),
            account_id: *account_id,
            market_id: market_id.clone(),
            trigger_price: activated.trigger_price,
            oracle_price: activated.oracle_price,
            activated_event_id: event.event_id().clone(),
            last_event_id: event.event_id().clone(),
            last_block_height: event.block_height(),
        };
        let current_bytes = current.encode().map_err(codec_reducer_error)?;
        let result_state_hash = hash_current(&current_bytes);
        let fact = TriggerFactRecordV1 {
            event_id: event.event_id().clone(),
            event_kind: EventKind::TriggerOrderActivated,
            order_id: activated.order_id.clone(),
            account_id: *account_id,
            market_id: market_id.clone(),
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
        };
        let fact_key = fact.state_key().map_err(codec_reducer_error)?;
        let transition = TriggerTransitionRecordV1 {
            event_id: event.event_id().clone(),
            event_kind: EventKind::TriggerOrderActivated,
            order_id: activated.order_id.clone(),
            account_id: *account_id,
            market_id: market_id.clone(),
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
            prior_state_hash: None,
            result_state_hash,
            rule_version: CanonicalTriggerReducerV1::VERSION.to_owned(),
        };
        let transition_key = transition.state_key().map_err(codec_reducer_error)?;
        if state.contains_key(&fact_key) || state.contains_key(&transition_key) {
            return Err(reducer_error(
                "trigger_state.event_id_collision",
                "trigger event identity is already present in canonical state",
            ));
        }

        Ok(vec![
            StateMutation::put(fact_key, fact.encode().map_err(codec_reducer_error)?),
            StateMutation::put(current_key, current_bytes),
            StateMutation::put(
                transition_key,
                transition.encode().map_err(codec_reducer_error)?,
            ),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerFactRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    order_id: OrderId,
    account_id: Address,
    market_id: MarketId,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl TriggerFactRecordV1 {
    pub fn state_key_for(
        market_id: &MarketId,
        order_id: &OrderId,
        event_id: &EventId,
    ) -> Result<StateKey, TriggerStateError> {
        StateKey::try_new(
            FACT_NAMESPACE,
            event_identity_key(market_id, order_id, event_id)?,
        )
        .map_err(|_| TriggerStateError::InvalidKey)
    }

    fn state_key(&self) -> Result<StateKey, TriggerStateError> {
        Self::state_key_for(&self.market_id, &self.order_id, &self.event_id)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TriggerStateError> {
        let wire: FactWire = decode_canonical(bytes)?;
        if wire.schema != FACT_SCHEMA {
            return Err(TriggerStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TriggerStateError::InvalidRecord)?,
            event_kind: parse_trigger_kind(&wire.event_kind)?,
            order_id: OrderId::new(wire.order_id).map_err(|_| TriggerStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
        };
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TriggerStateError> {
        let record = Self::decode(bytes)?;
        if record.state_key()? != *key {
            return Err(TriggerStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TriggerStateError> {
        encode_canonical(&FactWire {
            schema: FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            order_id: self.order_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
        })
    }

    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    #[must_use]
    pub const fn order_id(&self) -> &OrderId {
        &self.order_id
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn payload_hash(&self) -> [u8; 32] {
        self.payload_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerCurrentRecordV1 {
    order_id: OrderId,
    account_id: Address,
    market_id: MarketId,
    trigger_price: Price,
    oracle_price: Price,
    activated_event_id: EventId,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl TriggerCurrentRecordV1 {
    pub fn state_key(
        market_id: &MarketId,
        order_id: &OrderId,
    ) -> Result<StateKey, TriggerStateError> {
        StateKey::try_new(
            CURRENT_NAMESPACE,
            current_identity_key(market_id, order_id)?,
        )
        .map_err(|_| TriggerStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TriggerStateError> {
        let wire: CurrentWire = decode_canonical(bytes)?;
        if wire.schema != CURRENT_SCHEMA {
            return Err(TriggerStateError::InvalidRecord);
        }
        let record = Self {
            order_id: OrderId::new(wire.order_id).map_err(|_| TriggerStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            trigger_price: Price::from_str(&wire.trigger_price)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            oracle_price: Price::from_str(&wire.oracle_price)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            activated_event_id: EventId::new(wire.activated_event_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TriggerStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.market_id, &record.order_id)? != *key {
            return Err(TriggerStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TriggerStateError> {
        self.validate()?;
        encode_canonical(&CurrentWire {
            schema: CURRENT_SCHEMA.to_owned(),
            order_id: self.order_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            trigger_price: self.trigger_price.to_string(),
            oracle_price: self.oracle_price.to_string(),
            activated_event_id: self.activated_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), TriggerStateError> {
        if self.trigger_price.raw() <= 0 || self.oracle_price.raw() <= 0 {
            return Err(TriggerStateError::InvalidRecord);
        }
        Ok(())
    }

    #[must_use]
    pub const fn order_id(&self) -> &OrderId {
        &self.order_id
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn trigger_price(&self) -> Price {
        self.trigger_price
    }

    #[must_use]
    pub const fn oracle_price(&self) -> Price {
        self.oracle_price
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TriggerTransitionRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    order_id: OrderId,
    account_id: Address,
    market_id: MarketId,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
    prior_state_hash: Option<[u8; 32]>,
    result_state_hash: [u8; 32],
    rule_version: String,
}

impl TriggerTransitionRecordV1 {
    pub fn state_key_for(
        market_id: &MarketId,
        order_id: &OrderId,
        event_id: &EventId,
    ) -> Result<StateKey, TriggerStateError> {
        StateKey::try_new(
            TRANSITION_NAMESPACE,
            event_identity_key(market_id, order_id, event_id)?,
        )
        .map_err(|_| TriggerStateError::InvalidKey)
    }

    fn state_key(&self) -> Result<StateKey, TriggerStateError> {
        Self::state_key_for(&self.market_id, &self.order_id, &self.event_id)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TriggerStateError> {
        let wire: TransitionWire = decode_canonical(bytes)?;
        if wire.schema != TRANSITION_SCHEMA {
            return Err(TriggerStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TriggerStateError::InvalidRecord)?,
            event_kind: parse_trigger_kind(&wire.event_kind)?,
            order_id: OrderId::new(wire.order_id).map_err(|_| TriggerStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| TriggerStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
            prior_state_hash: wire
                .prior_state_blake3
                .as_deref()
                .map(decode_hash)
                .transpose()?,
            result_state_hash: decode_hash(&wire.result_state_blake3)?,
            rule_version: wire.rule_version,
        };
        if !valid_reducer_version(&record.rule_version) {
            return Err(TriggerStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TriggerStateError> {
        let record = Self::decode(bytes)?;
        if record.state_key()? != *key {
            return Err(TriggerStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TriggerStateError> {
        if !valid_reducer_version(&self.rule_version) {
            return Err(TriggerStateError::InvalidRecord);
        }
        encode_canonical(&TransitionWire {
            schema: TRANSITION_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            order_id: self.order_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
            prior_state_blake3: self.prior_state_hash.map(hex::encode),
            result_state_blake3: hex::encode(self.result_state_hash),
            rule_version: self.rule_version.clone(),
        })
    }

    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn payload_hash(&self) -> [u8; 32] {
        self.payload_hash
    }

    #[must_use]
    pub const fn prior_state_hash(&self) -> Option<[u8; 32]> {
        self.prior_state_hash
    }

    #[must_use]
    pub const fn result_state_hash(&self) -> [u8; 32] {
        self.result_state_hash
    }

    #[must_use]
    pub fn rule_version(&self) -> &str {
        &self.rule_version
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum TriggerStateError {
    #[error("trigger-state key encoding failed")]
    InvalidKey,
    #[error("trigger-state record is not valid JSON")]
    Codec,
    #[error("trigger-state record is not canonical")]
    NonCanonical,
    #[error("trigger-state record is semantically invalid")]
    InvalidRecord,
    #[error("trigger-state record does not match its key")]
    KeyMismatch,
    #[error("trigger-state record exceeds its deterministic bound")]
    LimitExceeded,
}

impl TriggerStateError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "trigger_state.codec.invalid_key",
            Self::Codec => "trigger_state.codec.decode",
            Self::NonCanonical => "trigger_state.codec.noncanonical",
            Self::InvalidRecord => "trigger_state.codec.invalid_record",
            Self::KeyMismatch => "trigger_state.codec.key_mismatch",
            Self::LimitExceeded => "trigger_state.codec.limit_exceeded",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactWire {
    schema: String,
    event_id: String,
    event_kind: String,
    order_id: String,
    account_id: String,
    market_id: String,
    block_height: u64,
    payload_blake3: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CurrentWire {
    schema: String,
    order_id: String,
    account_id: String,
    market_id: String,
    trigger_price: String,
    oracle_price: String,
    activated_event_id: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionWire {
    schema: String,
    event_id: String,
    event_kind: String,
    order_id: String,
    account_id: String,
    market_id: String,
    block_height: u64,
    payload_blake3: String,
    prior_state_blake3: Option<String>,
    result_state_blake3: String,
    rule_version: String,
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, TriggerStateError> {
    let bytes = serde_json::to_vec(value).map_err(|_| TriggerStateError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(TriggerStateError::LimitExceeded);
    }
    Ok(bytes)
}

fn decode_canonical<T>(bytes: &[u8]) -> Result<T, TriggerStateError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(TriggerStateError::LimitExceeded);
    }
    let value = serde_json::from_slice(bytes).map_err(|_| TriggerStateError::Codec)?;
    if encode_canonical(&value)? != bytes {
        return Err(TriggerStateError::NonCanonical);
    }
    Ok(value)
}

fn decode_hash(value: &str) -> Result<[u8; 32], TriggerStateError> {
    if value.len() != 64 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(TriggerStateError::InvalidRecord);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| TriggerStateError::InvalidRecord)?;
    Ok(hash)
}

fn parse_trigger_kind(value: &str) -> Result<EventKind, TriggerStateError> {
    EventKind::try_from(value)
        .map_err(|_| TriggerStateError::InvalidRecord)
        .and_then(|kind| {
            if kind == EventKind::TriggerOrderActivated {
                Ok(kind)
            } else {
                Err(TriggerStateError::InvalidRecord)
            }
        })
}

fn current_identity_key(
    market_id: &MarketId,
    order_id: &OrderId,
) -> Result<Vec<u8>, TriggerStateError> {
    let mut key = Vec::new();
    extend_frame(&mut key, market_id.as_str().as_bytes())?;
    extend_frame(&mut key, order_id.as_str().as_bytes())?;
    Ok(key)
}

fn event_identity_key(
    market_id: &MarketId,
    order_id: &OrderId,
    event_id: &EventId,
) -> Result<Vec<u8>, TriggerStateError> {
    let mut key = current_identity_key(market_id, order_id)?;
    extend_frame(&mut key, event_id.as_str().as_bytes())?;
    Ok(key)
}

fn extend_frame(target: &mut Vec<u8>, value: &[u8]) -> Result<(), TriggerStateError> {
    let length = u64::try_from(value.len()).map_err(|_| TriggerStateError::InvalidKey)?;
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(value);
    Ok(())
}

fn hash_current(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(CURRENT_HASH_CONTEXT);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn valid_reducer_version(value: &str) -> bool {
    !value.is_empty() && value.trim() == value && value.len() <= 128
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn codec_reducer_error(error: TriggerStateError) -> ReducerError {
    match error {
        TriggerStateError::InvalidKey => reducer_error(
            "trigger_state.codec_invalid_key",
            "trigger state key encoding failed",
        ),
        TriggerStateError::LimitExceeded => reducer_error(
            "trigger_state.codec_limit_exceeded",
            "trigger state record exceeds its deterministic bound",
        ),
        TriggerStateError::Codec
        | TriggerStateError::NonCanonical
        | TriggerStateError::InvalidRecord
        | TriggerStateError::KeyMismatch => reducer_error(
            "trigger_state.codec_failed",
            "trigger state record encoding failed",
        ),
    }
}
