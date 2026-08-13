use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{
    Address, BlockHeight, EventId, MarketId, OrderId, Price, ProtocolTime, Quantity,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView};

const FACT_NAMESPACE: &str = "twap-fact.v1";
const CURRENT_NAMESPACE: &str = "twap-current.v1";
const TRANSITION_NAMESPACE: &str = "twap-transition.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/twap-fact/v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/twap-current/v1";
const TRANSITION_SCHEMA: &str = "hyperliquid-alpha-desk/twap-transition/v1";
const CURRENT_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/twap-current-hash/v1";
const MAX_RECORD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalTwapReducerV1;

impl CanonicalTwapReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-twap@1.0.0";
}

impl EventReducer for CanonicalTwapReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0" && is_twap_kind(event.event_kind())
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let order_id = twap_order_id(event.payload()).ok_or_else(|| {
            reducer_error(
                "twap_state.invalid_event",
                "twap reducer received a non-twap payload",
            )
        })?;
        let [market_id] = event.market_ids() else {
            return Err(reducer_error(
                "twap_state.identity_mismatch",
                "twap event requires exactly one envelope market",
            ));
        };
        let [account_id] = event.account_addresses() else {
            return Err(reducer_error(
                "twap_state.identity_mismatch",
                "twap event requires exactly one envelope account",
            ));
        };
        let current_key =
            TwapCurrentRecordV1::state_key(market_id, order_id).map_err(codec_reducer_error)?;
        let existing = state
            .get(&current_key)
            .map(|bytes| {
                TwapCurrentRecordV1::decode_at(&current_key, bytes).map_err(codec_reducer_error)
            })
            .transpose()?;

        let (prior_state_hash, current) = match event.payload() {
            EventPayload::TwapStarted(started) => {
                if existing.is_some() {
                    return Err(reducer_error(
                        "twap_state.order_id_collision",
                        "twap identity is already present in canonical state",
                    ));
                }
                if started.order_id != *order_id
                    || started.market_id != *market_id
                    || started.account_id != *account_id
                {
                    return Err(reducer_error(
                        "twap_state.identity_mismatch",
                        "started payload and envelope identities must match exactly",
                    ));
                }
                let zero = started
                    .total_quantity
                    .checked_sub(started.total_quantity)
                    .map_err(|_| arithmetic_error())?;
                let current = TwapCurrentRecordV1 {
                    order_id: started.order_id.clone(),
                    account_id: started.account_id,
                    market_id: started.market_id.clone(),
                    lifecycle: TwapLifecycleV1::Active,
                    total_quantity: started.total_quantity,
                    filled_quantity: zero,
                    remaining_quantity: started.total_quantity,
                    last_slice_index: None,
                    end_time: started.end_time,
                    completed_average_price: None,
                    started_event_id: event.event_id().clone(),
                    last_event_id: event.event_id().clone(),
                    last_block_height: event.block_height(),
                };
                (None, current)
            }
            payload => {
                let previous = existing.ok_or_else(|| {
                    reducer_error(
                        "twap_state.twap_not_found",
                        "twap transition requires an active started twap",
                    )
                })?;
                if previous.account_id != *account_id || previous.market_id != *market_id {
                    return Err(reducer_error(
                        "twap_state.identity_mismatch",
                        "twap payload and envelope identities must match current state",
                    ));
                }
                if previous.lifecycle.is_terminal() {
                    return Err(reducer_error(
                        "twap_state.terminal_twap",
                        "terminal twap state cannot transition",
                    ));
                }
                let prior_bytes = previous.encode().map_err(codec_reducer_error)?;
                let prior_hash = hash_current(&prior_bytes);
                let next = apply_transition(previous, payload, event)?;
                (Some(prior_hash), next)
            }
        };

        let current_bytes = current.encode().map_err(codec_reducer_error)?;
        let result_state_hash = hash_current(&current_bytes);
        let fact = TwapFactRecordV1 {
            event_id: event.event_id().clone(),
            event_kind: event.event_kind(),
            order_id: order_id.clone(),
            account_id: *account_id,
            market_id: market_id.clone(),
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
        };
        let fact_key = fact.state_key().map_err(codec_reducer_error)?;
        let transition = TwapTransitionRecordV1 {
            event_id: event.event_id().clone(),
            event_kind: event.event_kind(),
            order_id: order_id.clone(),
            account_id: *account_id,
            market_id: market_id.clone(),
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
            prior_state_hash,
            result_state_hash,
            rule_version: CanonicalTwapReducerV1::VERSION.to_owned(),
        };
        let transition_key = transition.state_key().map_err(codec_reducer_error)?;
        if state.contains_key(&fact_key) || state.contains_key(&transition_key) {
            return Err(reducer_error(
                "twap_state.event_id_collision",
                "twap event identity is already present in canonical state",
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

fn apply_transition(
    mut current: TwapCurrentRecordV1,
    payload: &EventPayload,
    event: &CanonicalEventEnvelope,
) -> Result<TwapCurrentRecordV1, ReducerError> {
    match payload {
        EventPayload::TwapSliceFilled(filled) => {
            if filled.order_id != current.order_id {
                return Err(reducer_error(
                    "twap_state.identity_mismatch",
                    "slice fill order identity must match current state",
                ));
            }
            if let Some(last_index) = current.last_slice_index
                && filled.slice_index <= last_index
            {
                return Err(reducer_error(
                    "twap_state.slice_index_not_increasing",
                    "twap slice index must strictly increase",
                ));
            }
            let next_filled = current
                .filled_quantity
                .checked_add(filled.fill_quantity)
                .map_err(|_| arithmetic_error())?;
            if next_filled > current.total_quantity {
                return Err(reducer_error(
                    "twap_state.overfill",
                    "twap slice fill exceeds remaining quantity",
                ));
            }
            let remaining = current
                .total_quantity
                .checked_sub(next_filled)
                .map_err(|_| arithmetic_error())?;
            current.filled_quantity = next_filled;
            current.remaining_quantity = remaining;
            current.last_slice_index = Some(filled.slice_index);
        }
        EventPayload::TwapCompleted(completed) => {
            if completed.order_id != current.order_id {
                return Err(reducer_error(
                    "twap_state.identity_mismatch",
                    "completed order identity must match current state",
                ));
            }
            if completed.filled_quantity != current.filled_quantity {
                return Err(reducer_error(
                    "twap_state.filled_mismatch",
                    "completed filled quantity must equal accumulated slices",
                ));
            }
            if completed.filled_quantity.raw() == 0 && completed.average_price.raw() != 0 {
                return Err(reducer_error(
                    "twap_state.invalid_average_price",
                    "zero-fill twap completion requires a zero average price",
                ));
            }
            if completed.filled_quantity.raw() > 0 && completed.average_price.raw() <= 0 {
                return Err(reducer_error(
                    "twap_state.invalid_average_price",
                    "positive-fill twap completion requires a positive average price",
                ));
            }
            current.lifecycle = TwapLifecycleV1::Completed;
            current.completed_average_price = Some(completed.average_price);
        }
        EventPayload::TwapStarted(_) => {
            return Err(reducer_error(
                "twap_state.invalid_transition",
                "twap start cannot follow an existing current",
            ));
        }
        _ => {
            return Err(reducer_error(
                "twap_state.invalid_event",
                "twap reducer received a non-twap payload",
            ));
        }
    }
    current.last_event_id = event.event_id().clone();
    current.last_block_height = event.block_height();
    Ok(current)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwapLifecycleV1 {
    Active,
    Completed,
}

impl TwapLifecycleV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
        }
    }

    fn parse(value: &str) -> Result<Self, TwapStateError> {
        match value {
            "active" => Ok(Self::Active),
            "completed" => Ok(Self::Completed),
            _ => Err(TwapStateError::InvalidRecord),
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwapFactRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    order_id: OrderId,
    account_id: Address,
    market_id: MarketId,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl TwapFactRecordV1 {
    pub fn state_key_for(
        market_id: &MarketId,
        order_id: &OrderId,
        event_id: &EventId,
    ) -> Result<StateKey, TwapStateError> {
        StateKey::try_new(
            FACT_NAMESPACE,
            event_identity_key(market_id, order_id, event_id)?,
        )
        .map_err(|_| TwapStateError::InvalidKey)
    }

    fn state_key(&self) -> Result<StateKey, TwapStateError> {
        Self::state_key_for(&self.market_id, &self.order_id, &self.event_id)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TwapStateError> {
        let wire: FactWire = decode_canonical(bytes)?;
        if wire.schema != FACT_SCHEMA {
            return Err(TwapStateError::InvalidRecord);
        }
        Ok(Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TwapStateError::InvalidRecord)?,
            event_kind: parse_twap_kind(&wire.event_kind)?,
            order_id: OrderId::new(wire.order_id).map_err(|_| TwapStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| TwapStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
        })
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TwapStateError> {
        let record = Self::decode(bytes)?;
        if record.state_key()? != *key {
            return Err(TwapStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TwapStateError> {
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
pub struct TwapCurrentRecordV1 {
    order_id: OrderId,
    account_id: Address,
    market_id: MarketId,
    lifecycle: TwapLifecycleV1,
    total_quantity: Quantity,
    filled_quantity: Quantity,
    remaining_quantity: Quantity,
    last_slice_index: Option<u32>,
    end_time: ProtocolTime,
    completed_average_price: Option<Price>,
    started_event_id: EventId,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl TwapCurrentRecordV1 {
    pub fn state_key(market_id: &MarketId, order_id: &OrderId) -> Result<StateKey, TwapStateError> {
        StateKey::try_new(
            CURRENT_NAMESPACE,
            current_identity_key(market_id, order_id)?,
        )
        .map_err(|_| TwapStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TwapStateError> {
        let wire: CurrentWire = decode_canonical(bytes)?;
        if wire.schema != CURRENT_SCHEMA {
            return Err(TwapStateError::InvalidRecord);
        }
        let record = Self {
            order_id: OrderId::new(wire.order_id).map_err(|_| TwapStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| TwapStateError::InvalidRecord)?,
            lifecycle: TwapLifecycleV1::parse(&wire.lifecycle)?,
            total_quantity: Quantity::from_str(&wire.total_quantity)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            filled_quantity: Quantity::from_str(&wire.filled_quantity)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            remaining_quantity: Quantity::from_str(&wire.remaining_quantity)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            last_slice_index: wire.last_slice_index,
            end_time: ProtocolTime::from_unix_micros(wire.end_time_micros)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            completed_average_price: wire
                .completed_average_price
                .map(|value| Price::from_str(&value).map_err(|_| TwapStateError::InvalidRecord))
                .transpose()?,
            started_event_id: EventId::new(wire.started_event_id)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TwapStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.market_id, &record.order_id)? != *key {
            return Err(TwapStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TwapStateError> {
        self.validate()?;
        encode_canonical(&CurrentWire {
            schema: CURRENT_SCHEMA.to_owned(),
            order_id: self.order_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            lifecycle: self.lifecycle.as_wire_name().to_owned(),
            total_quantity: self.total_quantity.to_string(),
            filled_quantity: self.filled_quantity.to_string(),
            remaining_quantity: self.remaining_quantity.to_string(),
            last_slice_index: self.last_slice_index,
            end_time_micros: self.end_time.unix_micros(),
            completed_average_price: self.completed_average_price.map(|price| price.to_string()),
            started_event_id: self.started_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), TwapStateError> {
        if self.total_quantity.raw() <= 0
            || self.filled_quantity.raw() < 0
            || self.remaining_quantity.raw() < 0
        {
            return Err(TwapStateError::InvalidRecord);
        }
        let total = self
            .filled_quantity
            .checked_add(self.remaining_quantity)
            .map_err(|_| TwapStateError::InvalidRecord)?;
        if total != self.total_quantity {
            return Err(TwapStateError::InvalidRecord);
        }
        match self.lifecycle {
            TwapLifecycleV1::Active => {
                if self.completed_average_price.is_some() {
                    return Err(TwapStateError::InvalidRecord);
                }
            }
            TwapLifecycleV1::Completed => {
                let Some(average) = self.completed_average_price else {
                    return Err(TwapStateError::InvalidRecord);
                };
                if self.filled_quantity.raw() == 0 && average.raw() != 0 {
                    return Err(TwapStateError::InvalidRecord);
                }
                if self.filled_quantity.raw() > 0 && average.raw() <= 0 {
                    return Err(TwapStateError::InvalidRecord);
                }
            }
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
    pub const fn lifecycle(&self) -> TwapLifecycleV1 {
        self.lifecycle
    }

    #[must_use]
    pub const fn total_quantity(&self) -> Quantity {
        self.total_quantity
    }

    #[must_use]
    pub const fn filled_quantity(&self) -> Quantity {
        self.filled_quantity
    }

    #[must_use]
    pub const fn remaining_quantity(&self) -> Quantity {
        self.remaining_quantity
    }

    #[must_use]
    pub const fn last_slice_index(&self) -> Option<u32> {
        self.last_slice_index
    }

    #[must_use]
    pub const fn completed_average_price(&self) -> Option<Price> {
        self.completed_average_price
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwapTransitionRecordV1 {
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

impl TwapTransitionRecordV1 {
    pub fn state_key_for(
        market_id: &MarketId,
        order_id: &OrderId,
        event_id: &EventId,
    ) -> Result<StateKey, TwapStateError> {
        StateKey::try_new(
            TRANSITION_NAMESPACE,
            event_identity_key(market_id, order_id, event_id)?,
        )
        .map_err(|_| TwapStateError::InvalidKey)
    }

    fn state_key(&self) -> Result<StateKey, TwapStateError> {
        Self::state_key_for(&self.market_id, &self.order_id, &self.event_id)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TwapStateError> {
        let wire: TransitionWire = decode_canonical(bytes)?;
        if wire.schema != TRANSITION_SCHEMA {
            return Err(TwapStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TwapStateError::InvalidRecord)?,
            event_kind: parse_twap_kind(&wire.event_kind)?,
            order_id: OrderId::new(wire.order_id).map_err(|_| TwapStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| TwapStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| TwapStateError::InvalidRecord)?,
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
            return Err(TwapStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, TwapStateError> {
        let record = Self::decode(bytes)?;
        if record.state_key()? != *key {
            return Err(TwapStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, TwapStateError> {
        if !valid_reducer_version(&self.rule_version) {
            return Err(TwapStateError::InvalidRecord);
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
pub enum TwapStateError {
    #[error("twap-state key encoding failed")]
    InvalidKey,
    #[error("twap-state record is not valid JSON")]
    Codec,
    #[error("twap-state record is not canonical")]
    NonCanonical,
    #[error("twap-state record is semantically invalid")]
    InvalidRecord,
    #[error("twap-state record does not match its key")]
    KeyMismatch,
    #[error("twap-state record exceeds its deterministic bound")]
    LimitExceeded,
}

impl TwapStateError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "twap_state.codec.invalid_key",
            Self::Codec => "twap_state.codec.decode",
            Self::NonCanonical => "twap_state.codec.noncanonical",
            Self::InvalidRecord => "twap_state.codec.invalid_record",
            Self::KeyMismatch => "twap_state.codec.key_mismatch",
            Self::LimitExceeded => "twap_state.codec.limit_exceeded",
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
    lifecycle: String,
    total_quantity: String,
    filled_quantity: String,
    remaining_quantity: String,
    last_slice_index: Option<u32>,
    end_time_micros: i64,
    completed_average_price: Option<String>,
    started_event_id: String,
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

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, TwapStateError> {
    let bytes = serde_json::to_vec(value).map_err(|_| TwapStateError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(TwapStateError::LimitExceeded);
    }
    Ok(bytes)
}

fn decode_canonical<T>(bytes: &[u8]) -> Result<T, TwapStateError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(TwapStateError::LimitExceeded);
    }
    let value = serde_json::from_slice(bytes).map_err(|_| TwapStateError::Codec)?;
    if encode_canonical(&value)? != bytes {
        return Err(TwapStateError::NonCanonical);
    }
    Ok(value)
}

fn decode_hash(value: &str) -> Result<[u8; 32], TwapStateError> {
    if value.len() != 64 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(TwapStateError::InvalidRecord);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| TwapStateError::InvalidRecord)?;
    Ok(hash)
}

fn parse_twap_kind(value: &str) -> Result<EventKind, TwapStateError> {
    EventKind::try_from(value)
        .map_err(|_| TwapStateError::InvalidRecord)
        .and_then(|kind| {
            if is_twap_kind(kind) {
                Ok(kind)
            } else {
                Err(TwapStateError::InvalidRecord)
            }
        })
}

const fn is_twap_kind(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::TwapStarted | EventKind::TwapSliceFilled | EventKind::TwapCompleted
    )
}

fn twap_order_id(payload: &EventPayload) -> Option<&OrderId> {
    match payload {
        EventPayload::TwapStarted(value) => Some(&value.order_id),
        EventPayload::TwapSliceFilled(value) => Some(&value.order_id),
        EventPayload::TwapCompleted(value) => Some(&value.order_id),
        _ => None,
    }
}

fn current_identity_key(
    market_id: &MarketId,
    order_id: &OrderId,
) -> Result<Vec<u8>, TwapStateError> {
    let mut key = Vec::new();
    extend_frame(&mut key, market_id.as_str().as_bytes())?;
    extend_frame(&mut key, order_id.as_str().as_bytes())?;
    Ok(key)
}

fn event_identity_key(
    market_id: &MarketId,
    order_id: &OrderId,
    event_id: &EventId,
) -> Result<Vec<u8>, TwapStateError> {
    let mut key = current_identity_key(market_id, order_id)?;
    extend_frame(&mut key, event_id.as_str().as_bytes())?;
    Ok(key)
}

fn extend_frame(target: &mut Vec<u8>, value: &[u8]) -> Result<(), TwapStateError> {
    let length = u64::try_from(value.len()).map_err(|_| TwapStateError::InvalidKey)?;
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

fn arithmetic_error() -> ReducerError {
    reducer_error(
        "twap_state.arithmetic_overflow",
        "twap quantity arithmetic overflowed",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn codec_reducer_error(error: TwapStateError) -> ReducerError {
    match error {
        TwapStateError::InvalidKey => reducer_error(
            "twap_state.codec_invalid_key",
            "twap state key encoding failed",
        ),
        TwapStateError::LimitExceeded => reducer_error(
            "twap_state.codec_limit_exceeded",
            "twap state record exceeds its deterministic bound",
        ),
        TwapStateError::Codec
        | TwapStateError::NonCanonical
        | TwapStateError::InvalidRecord
        | TwapStateError::KeyMismatch => reducer_error(
            "twap_state.codec_failed",
            "twap state record encoding failed",
        ),
    }
}
