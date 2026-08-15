use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{
    Address, BlockHeight, ClientOrderId, EventId, MarketId, OrderId, OrderSide, Price, Quantity,
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView};

const FACT_NAMESPACE: &str = "order-fact.v1";
const CURRENT_NAMESPACE: &str = "order-current.v1";
const TRANSITION_NAMESPACE: &str = "order-transition.v1";
const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/order-fact/v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/order-current/v1";
const TRANSITION_SCHEMA: &str = "hyperliquid-alpha-desk/order-transition/v1";
const CURRENT_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/order-current-hash/v1";
const MAX_RECORD_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalOrderReducerV1;

impl CanonicalOrderReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-order@1.0.0";
}

impl EventReducer for CanonicalOrderReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0" && is_order_kind(event.event_kind())
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        validate_payload_semantics(event.payload())?;
        match event.payload() {
            EventPayload::OrderRejected(rejected) => reduce_rejection(state, event, rejected),
            _ => reduce_order_event(state, event),
        }
    }
}

fn reduce_rejection(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    rejected: &canonical_events::OrderRejected,
) -> Result<Vec<StateMutation>, ReducerError> {
    if !event.market_ids().is_empty()
        || event.account_addresses() != std::slice::from_ref(&rejected.account_id)
    {
        return Err(reducer_error(
            "order_state.identity_mismatch",
            "rejection payload and envelope identities must match exactly",
        ));
    }

    let fact = OrderFactRecordV1 {
        event_id: event.event_id().clone(),
        event_kind: EventKind::OrderRejected,
        order_id: None,
        client_order_id: Some(rejected.client_order_id.clone()),
        account_id: rejected.account_id,
        market_id: None,
        block_height: event.block_height(),
        payload_hash: event.payload_hash(),
    };
    let fact_key = fact.state_key().map_err(codec_reducer_error)?;
    let transition = OrderTransitionRecordV1 {
        event_id: event.event_id().clone(),
        event_kind: EventKind::OrderRejected,
        order_id: None,
        client_order_id: Some(rejected.client_order_id.clone()),
        account_id: rejected.account_id,
        market_id: None,
        block_height: event.block_height(),
        payload_hash: event.payload_hash(),
        prior_state_hash: None,
        result_state_hash: None,
        rule_version: CanonicalOrderReducerV1::VERSION.to_owned(),
        status: OrderTransitionStatusV1::RecordedRejection,
    };
    let transition_key = transition.state_key().map_err(codec_reducer_error)?;
    reject_collision(state, &fact_key, &transition_key)?;

    Ok(vec![
        StateMutation::put(fact_key, fact.encode().map_err(codec_reducer_error)?),
        StateMutation::put(
            transition_key,
            transition.encode().map_err(codec_reducer_error)?,
        ),
    ])
}

fn reduce_order_event(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
) -> Result<Vec<StateMutation>, ReducerError> {
    let order_id = order_id(event.payload()).ok_or_else(|| {
        reducer_error(
            "order_state.invalid_event",
            "order reducer received a non-order payload",
        )
    })?;
    let [market_id] = event.market_ids() else {
        return Err(reducer_error(
            "order_state.identity_mismatch",
            "order event requires exactly one envelope market",
        ));
    };
    let [account_id] = event.account_addresses() else {
        return Err(reducer_error(
            "order_state.identity_mismatch",
            "order event requires exactly one envelope account",
        ));
    };
    let current_key =
        OrderCurrentRecordV1::state_key(market_id, order_id).map_err(codec_reducer_error)?;
    let existing = state
        .get(&current_key)
        .map(|bytes| {
            OrderCurrentRecordV1::decode_at(&current_key, bytes).map_err(codec_reducer_error)
        })
        .transpose()?;

    let (prior_state_hash, current) = match event.payload() {
        EventPayload::OrderAccepted(accepted) => {
            if existing.is_some() {
                return Err(reducer_error(
                    "order_state.order_id_collision",
                    "order identity is already present in canonical state",
                ));
            }
            if accepted.order_id != *order_id
                || accepted.market_id != *market_id
                || accepted.account_id != *account_id
            {
                return Err(reducer_error(
                    "order_state.identity_mismatch",
                    "accepted payload and envelope identities must match exactly",
                ));
            }
            let current = OrderCurrentRecordV1 {
                order_id: accepted.order_id.clone(),
                account_id: accepted.account_id,
                market_id: accepted.market_id.clone(),
                side: accepted.side,
                lifecycle: OrderLifecycleV1::Accepted,
                limit_price: accepted.limit_price,
                accepted_quantity: accepted.quantity,
                filled_quantity: accepted
                    .quantity
                    .checked_sub(accepted.quantity)
                    .map_err(|_| arithmetic_error())?,
                remaining_quantity: accepted.quantity,
                accepted_event_id: event.event_id().clone(),
                last_event_id: event.event_id().clone(),
                last_block_height: event.block_height(),
            };
            (None, current)
        }
        payload => {
            let previous = existing.ok_or_else(|| {
                reducer_error(
                    "order_state.order_not_found",
                    "order transition requires an accepted order",
                )
            })?;
            if previous.account_id != *account_id || previous.market_id != *market_id {
                return Err(reducer_error(
                    "order_state.identity_mismatch",
                    "order payload and envelope identities must match current state",
                ));
            }
            if previous.lifecycle.is_terminal() {
                return Err(reducer_error(
                    "order_state.terminal_order",
                    "terminal order state cannot transition",
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
    let fact = OrderFactRecordV1 {
        event_id: event.event_id().clone(),
        event_kind: event.event_kind(),
        order_id: Some(order_id.clone()),
        client_order_id: None,
        account_id: *account_id,
        market_id: Some(market_id.clone()),
        block_height: event.block_height(),
        payload_hash: event.payload_hash(),
    };
    let fact_key = fact.state_key().map_err(codec_reducer_error)?;
    let transition = OrderTransitionRecordV1 {
        event_id: event.event_id().clone(),
        event_kind: event.event_kind(),
        order_id: Some(order_id.clone()),
        client_order_id: None,
        account_id: *account_id,
        market_id: Some(market_id.clone()),
        block_height: event.block_height(),
        payload_hash: event.payload_hash(),
        prior_state_hash,
        result_state_hash: Some(result_state_hash),
        rule_version: CanonicalOrderReducerV1::VERSION.to_owned(),
        status: OrderTransitionStatusV1::Applied,
    };
    let transition_key = transition.state_key().map_err(codec_reducer_error)?;
    reject_collision(state, &fact_key, &transition_key)?;

    Ok(vec![
        StateMutation::put(fact_key, fact.encode().map_err(codec_reducer_error)?),
        StateMutation::put(current_key, current_bytes),
        StateMutation::put(
            transition_key,
            transition.encode().map_err(codec_reducer_error)?,
        ),
    ])
}

fn apply_transition(
    mut current: OrderCurrentRecordV1,
    payload: &EventPayload,
    event: &CanonicalEventEnvelope,
) -> Result<OrderCurrentRecordV1, ReducerError> {
    if !transition_allowed(current.lifecycle, payload.kind()) {
        return Err(reducer_error(
            "order_state.invalid_transition",
            "order lifecycle transition is not allowed",
        ));
    }
    match payload {
        EventPayload::OrderRested(rested) => {
            if rested.order_id != current.order_id || rested.market_id != current.market_id {
                return Err(reducer_error(
                    "order_state.identity_mismatch",
                    "rested order identity must match current state",
                ));
            }
            if rested.limit_price != current.limit_price
                || rested.remaining_quantity != current.remaining_quantity
            {
                return Err(reducer_error(
                    "order_state.remaining_mismatch",
                    "rested payload must match the exact current order",
                ));
            }
            current.lifecycle = OrderLifecycleV1::Rested;
        }
        EventPayload::OrderModified(modified) => {
            if modified.order_id != current.order_id {
                return Err(reducer_error(
                    "order_state.identity_mismatch",
                    "modified order identity must match current state",
                ));
            }
            if modified.previous_price != current.limit_price
                || modified.previous_quantity != current.remaining_quantity
            {
                return Err(reducer_error(
                    "order_state.previous_state_mismatch",
                    "modification previous values must match current state",
                ));
            }
            current.accepted_quantity = current
                .filled_quantity
                .checked_add(modified.new_quantity)
                .map_err(|_| arithmetic_error())?;
            current.remaining_quantity = modified.new_quantity;
            current.limit_price = modified.new_price;
            current.lifecycle = OrderLifecycleV1::Modified;
        }
        EventPayload::OrderPartiallyFilled(fill) => {
            if fill.order_id != current.order_id {
                return Err(reducer_error(
                    "order_state.identity_mismatch",
                    "partial fill identity must match current state",
                ));
            }
            let remaining = current
                .remaining_quantity
                .checked_sub(fill.fill_quantity)
                .map_err(|_| arithmetic_error())?;
            if remaining.raw() < 0 {
                return Err(reducer_error(
                    "order_state.overfill",
                    "partial fill exceeds remaining quantity",
                ));
            }
            if remaining != fill.remaining_quantity {
                return Err(reducer_error(
                    "order_state.remaining_mismatch",
                    "partial fill remainder does not match checked arithmetic",
                ));
            }
            if remaining.raw() == 0 {
                return Err(reducer_error(
                    "order_state.remaining_mismatch",
                    "partial fill must leave positive remaining quantity",
                ));
            }
            current.filled_quantity = current
                .filled_quantity
                .checked_add(fill.fill_quantity)
                .map_err(|_| arithmetic_error())?;
            current.remaining_quantity = remaining;
            current.lifecycle = OrderLifecycleV1::PartiallyFilled;
        }
        EventPayload::OrderFilled(fill) => {
            if fill.order_id != current.order_id {
                return Err(reducer_error(
                    "order_state.identity_mismatch",
                    "terminal fill identity must match current state",
                ));
            }
            if fill.fill_quantity > current.remaining_quantity {
                return Err(reducer_error(
                    "order_state.overfill",
                    "terminal fill exceeds remaining quantity",
                ));
            }
            if fill.fill_quantity != current.remaining_quantity {
                return Err(reducer_error(
                    "order_state.remaining_mismatch",
                    "terminal fill must consume exact remaining quantity",
                ));
            }
            current.filled_quantity = current
                .filled_quantity
                .checked_add(fill.fill_quantity)
                .map_err(|_| arithmetic_error())?;
            current.remaining_quantity = current
                .remaining_quantity
                .checked_sub(current.remaining_quantity)
                .map_err(|_| arithmetic_error())?;
            current.lifecycle = OrderLifecycleV1::Filled;
        }
        EventPayload::OrderCancelled(cancelled) => {
            if cancelled.order_id != current.order_id {
                return Err(reducer_error(
                    "order_state.identity_mismatch",
                    "cancelled order identity must match current state",
                ));
            }
            if cancelled.remaining_quantity != current.remaining_quantity {
                return Err(reducer_error(
                    "order_state.remaining_mismatch",
                    "cancellation remainder must match current state",
                ));
            }
            current.lifecycle = OrderLifecycleV1::Cancelled;
        }
        EventPayload::OrderAccepted(_) | EventPayload::OrderRejected(_) => {
            return Err(reducer_error(
                "order_state.invalid_event",
                "invalid order transition payload",
            ));
        }
        _ => {
            return Err(reducer_error(
                "order_state.invalid_event",
                "order reducer received a non-order payload",
            ));
        }
    }
    current.last_event_id = event.event_id().clone();
    current.last_block_height = event.block_height();
    Ok(current)
}

fn validate_payload_semantics(payload: &EventPayload) -> Result<(), ReducerError> {
    match payload {
        EventPayload::OrderAccepted(value) => {
            positive_price(value.limit_price)?;
            positive_quantity(value.quantity)
        }
        EventPayload::OrderRested(value) => {
            positive_price(value.limit_price)?;
            positive_quantity(value.remaining_quantity)
        }
        EventPayload::OrderModified(value) => {
            positive_price(value.previous_price)?;
            positive_price(value.new_price)?;
            positive_quantity(value.previous_quantity)?;
            positive_quantity(value.new_quantity)?;
            if value.previous_price == value.new_price
                && value.previous_quantity == value.new_quantity
            {
                return Err(reducer_error(
                    "order_state.invalid_modification",
                    "order modification must change price or quantity",
                ));
            }
            Ok(())
        }
        EventPayload::OrderPartiallyFilled(value) => {
            positive_price(value.fill_price)?;
            positive_quantity(value.fill_quantity)?;
            positive_quantity(value.remaining_quantity)
        }
        EventPayload::OrderFilled(value) => {
            positive_price(value.fill_price)?;
            positive_quantity(value.fill_quantity)
        }
        EventPayload::OrderCancelled(value) => {
            if value.remaining_quantity.raw() < 0 {
                return Err(reducer_error(
                    "order_state.invalid_quantity",
                    "order quantity must be nonnegative",
                ));
            }
            Ok(())
        }
        EventPayload::OrderRejected(value) => {
            if invalid_text(&value.reason_code, 128) || invalid_text(&value.reason, 1_024) {
                return Err(reducer_error(
                    "order_state.invalid_rejection",
                    "order rejection reason contract is invalid",
                ));
            }
            Ok(())
        }
        _ => Err(reducer_error(
            "order_state.invalid_event",
            "order reducer received a non-order payload",
        )),
    }
}

fn positive_price(value: Price) -> Result<(), ReducerError> {
    if value.raw() <= 0 {
        return Err(reducer_error(
            "order_state.invalid_price",
            "order price must be positive",
        ));
    }
    Ok(())
}

fn positive_quantity(value: Quantity) -> Result<(), ReducerError> {
    if value.raw() <= 0 {
        return Err(reducer_error(
            "order_state.invalid_quantity",
            "order quantity must be positive",
        ));
    }
    Ok(())
}

fn invalid_text(value: &str, max_bytes: usize) -> bool {
    value.is_empty()
        || value.len() > max_bytes
        || value.trim() != value
        || value.chars().any(char::is_control)
}

const fn transition_allowed(lifecycle: OrderLifecycleV1, kind: EventKind) -> bool {
    match kind {
        EventKind::OrderRested => {
            matches!(
                lifecycle,
                OrderLifecycleV1::Accepted | OrderLifecycleV1::Modified
            )
        }
        EventKind::OrderModified => matches!(
            lifecycle,
            OrderLifecycleV1::Accepted
                | OrderLifecycleV1::Rested
                | OrderLifecycleV1::Modified
                | OrderLifecycleV1::PartiallyFilled
        ),
        EventKind::OrderPartiallyFilled | EventKind::OrderFilled | EventKind::OrderCancelled => {
            !lifecycle.is_terminal()
        }
        _ => false,
    }
}

fn reject_collision(
    state: &StateView<'_>,
    fact_key: &StateKey,
    transition_key: &StateKey,
) -> Result<(), ReducerError> {
    if state.contains_key(fact_key) || state.contains_key(transition_key) {
        return Err(reducer_error(
            "order_state.event_id_collision",
            "order event identity is already present in canonical state",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderLifecycleV1 {
    Accepted,
    Rested,
    Modified,
    PartiallyFilled,
    Filled,
    Cancelled,
}

impl OrderLifecycleV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Rested => "rested",
            Self::Modified => "modified",
            Self::PartiallyFilled => "partially_filled",
            Self::Filled => "filled",
            Self::Cancelled => "cancelled",
        }
    }

    fn parse(value: &str) -> Result<Self, OrderStateError> {
        match value {
            "accepted" => Ok(Self::Accepted),
            "rested" => Ok(Self::Rested),
            "modified" => Ok(Self::Modified),
            "partially_filled" => Ok(Self::PartiallyFilled),
            "filled" => Ok(Self::Filled),
            "cancelled" => Ok(Self::Cancelled),
            _ => Err(OrderStateError::InvalidRecord),
        }
    }

    const fn is_terminal(self) -> bool {
        matches!(self, Self::Filled | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderTransitionStatusV1 {
    Applied,
    RecordedRejection,
}

impl OrderTransitionStatusV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::RecordedRejection => "recorded_rejection",
        }
    }

    fn parse(value: &str) -> Result<Self, OrderStateError> {
        match value {
            "applied" => Ok(Self::Applied),
            "recorded_rejection" => Ok(Self::RecordedRejection),
            _ => Err(OrderStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderFactRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    order_id: Option<OrderId>,
    client_order_id: Option<ClientOrderId>,
    account_id: Address,
    market_id: Option<MarketId>,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl OrderFactRecordV1 {
    pub fn state_key_for_order(
        market_id: &MarketId,
        order_id: &OrderId,
        event_id: &EventId,
    ) -> Result<StateKey, OrderStateError> {
        StateKey::try_new(
            FACT_NAMESPACE,
            order_identity_key(market_id, order_id, event_id)?,
        )
        .map_err(|_| OrderStateError::InvalidKey)
    }

    pub fn state_key_for_rejection(
        account_id: &Address,
        client_order_id: &ClientOrderId,
        event_id: &EventId,
    ) -> Result<StateKey, OrderStateError> {
        StateKey::try_new(
            FACT_NAMESPACE,
            rejection_identity_key(account_id, client_order_id, event_id)?,
        )
        .map_err(|_| OrderStateError::InvalidKey)
    }

    fn state_key(&self) -> Result<StateKey, OrderStateError> {
        match (&self.market_id, &self.order_id, &self.client_order_id) {
            (Some(market_id), Some(order_id), None) => {
                Self::state_key_for_order(market_id, order_id, &self.event_id)
            }
            (None, None, Some(client_order_id)) => {
                Self::state_key_for_rejection(&self.account_id, client_order_id, &self.event_id)
            }
            _ => Err(OrderStateError::InvalidRecord),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, OrderStateError> {
        let wire: FactWire = decode_canonical(bytes)?;
        if wire.schema != FACT_SCHEMA {
            return Err(OrderStateError::InvalidRecord);
        }
        let event_kind = parse_order_kind(&wire.event_kind)?;
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| OrderStateError::InvalidRecord)?,
            event_kind,
            order_id: wire
                .order_id
                .map(OrderId::new)
                .transpose()
                .map_err(|_| OrderStateError::InvalidRecord)?,
            client_order_id: wire
                .client_order_id
                .map(ClientOrderId::new)
                .transpose()
                .map_err(|_| OrderStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            market_id: wire
                .market_id
                .map(MarketId::new)
                .transpose()
                .map_err(|_| OrderStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
        };
        if !record.valid_identity_shape() {
            return Err(OrderStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, OrderStateError> {
        let record = Self::decode(bytes)?;
        if record.state_key()? != *key {
            return Err(OrderStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, OrderStateError> {
        if !self.valid_identity_shape() {
            return Err(OrderStateError::InvalidRecord);
        }
        encode_canonical(&FactWire {
            schema: FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            order_id: self
                .order_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            client_order_id: self
                .client_order_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            account_id: self.account_id.to_api_string(),
            market_id: self
                .market_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
        })
    }

    fn valid_identity_shape(&self) -> bool {
        if self.event_kind == EventKind::OrderRejected {
            self.order_id.is_none() && self.market_id.is_none() && self.client_order_id.is_some()
        } else {
            is_order_kind(self.event_kind)
                && self.order_id.is_some()
                && self.market_id.is_some()
                && self.client_order_id.is_none()
        }
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
    pub const fn order_id(&self) -> Option<&OrderId> {
        self.order_id.as_ref()
    }

    #[must_use]
    pub const fn client_order_id(&self) -> Option<&ClientOrderId> {
        self.client_order_id.as_ref()
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn market_id(&self) -> Option<&MarketId> {
        self.market_id.as_ref()
    }

    #[must_use]
    pub const fn payload_hash(&self) -> [u8; 32] {
        self.payload_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderCurrentRecordV1 {
    order_id: OrderId,
    account_id: Address,
    market_id: MarketId,
    side: OrderSide,
    lifecycle: OrderLifecycleV1,
    limit_price: Price,
    accepted_quantity: Quantity,
    filled_quantity: Quantity,
    remaining_quantity: Quantity,
    accepted_event_id: EventId,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl OrderCurrentRecordV1 {
    pub fn state_key(
        market_id: &MarketId,
        order_id: &OrderId,
    ) -> Result<StateKey, OrderStateError> {
        StateKey::try_new(
            CURRENT_NAMESPACE,
            current_identity_key(market_id, order_id)?,
        )
        .map_err(|_| OrderStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, OrderStateError> {
        let wire: CurrentWire = decode_canonical(bytes)?;
        if wire.schema != CURRENT_SCHEMA {
            return Err(OrderStateError::InvalidRecord);
        }
        let record = Self {
            order_id: OrderId::new(wire.order_id).map_err(|_| OrderStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| OrderStateError::InvalidRecord)?,
            side: OrderSide::parse_wire(&wire.side).map_err(|_| OrderStateError::InvalidRecord)?,
            lifecycle: OrderLifecycleV1::parse(&wire.lifecycle)?,
            limit_price: Price::from_str(&wire.limit_price)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            accepted_quantity: Quantity::from_str(&wire.accepted_quantity)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            filled_quantity: Quantity::from_str(&wire.filled_quantity)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            remaining_quantity: Quantity::from_str(&wire.remaining_quantity)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            accepted_event_id: EventId::new(wire.accepted_event_id)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, OrderStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.market_id, &record.order_id)? != *key {
            return Err(OrderStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, OrderStateError> {
        self.validate()?;
        encode_canonical(&CurrentWire {
            schema: CURRENT_SCHEMA.to_owned(),
            order_id: self.order_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            side: self.side.as_wire_name().to_owned(),
            lifecycle: self.lifecycle.as_wire_name().to_owned(),
            limit_price: self.limit_price.to_string(),
            accepted_quantity: self.accepted_quantity.to_string(),
            filled_quantity: self.filled_quantity.to_string(),
            remaining_quantity: self.remaining_quantity.to_string(),
            accepted_event_id: self.accepted_event_id.as_str().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), OrderStateError> {
        if self.limit_price.raw() <= 0
            || self.accepted_quantity.raw() <= 0
            || self.filled_quantity.raw() < 0
            || self.remaining_quantity.raw() < 0
        {
            return Err(OrderStateError::InvalidRecord);
        }
        let total = self
            .filled_quantity
            .checked_add(self.remaining_quantity)
            .map_err(|_| OrderStateError::InvalidRecord)?;
        let filled_remaining_invalid = match self.lifecycle {
            OrderLifecycleV1::Filled => self.remaining_quantity.raw() != 0,
            OrderLifecycleV1::Accepted
            | OrderLifecycleV1::Rested
            | OrderLifecycleV1::Modified
            | OrderLifecycleV1::PartiallyFilled
            | OrderLifecycleV1::Cancelled => false,
        };
        if total != self.accepted_quantity
            || filled_remaining_invalid
            || (!self.lifecycle.is_terminal() && self.remaining_quantity.raw() <= 0)
        {
            return Err(OrderStateError::InvalidRecord);
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
    pub const fn side(&self) -> OrderSide {
        self.side
    }

    #[must_use]
    pub const fn lifecycle(&self) -> OrderLifecycleV1 {
        self.lifecycle
    }

    #[must_use]
    pub const fn limit_price(&self) -> Price {
        self.limit_price
    }

    #[must_use]
    pub const fn accepted_quantity(&self) -> Quantity {
        self.accepted_quantity
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
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderTransitionRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    order_id: Option<OrderId>,
    client_order_id: Option<ClientOrderId>,
    account_id: Address,
    market_id: Option<MarketId>,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
    prior_state_hash: Option<[u8; 32]>,
    result_state_hash: Option<[u8; 32]>,
    rule_version: String,
    status: OrderTransitionStatusV1,
}

impl OrderTransitionRecordV1 {
    pub fn state_key_for_order(
        market_id: &MarketId,
        order_id: &OrderId,
        event_id: &EventId,
    ) -> Result<StateKey, OrderStateError> {
        StateKey::try_new(
            TRANSITION_NAMESPACE,
            order_identity_key(market_id, order_id, event_id)?,
        )
        .map_err(|_| OrderStateError::InvalidKey)
    }

    pub fn state_key_for_rejection(
        account_id: &Address,
        client_order_id: &ClientOrderId,
        event_id: &EventId,
    ) -> Result<StateKey, OrderStateError> {
        StateKey::try_new(
            TRANSITION_NAMESPACE,
            rejection_identity_key(account_id, client_order_id, event_id)?,
        )
        .map_err(|_| OrderStateError::InvalidKey)
    }

    fn state_key(&self) -> Result<StateKey, OrderStateError> {
        match (&self.market_id, &self.order_id, &self.client_order_id) {
            (Some(market_id), Some(order_id), None) => {
                Self::state_key_for_order(market_id, order_id, &self.event_id)
            }
            (None, None, Some(client_order_id)) => {
                Self::state_key_for_rejection(&self.account_id, client_order_id, &self.event_id)
            }
            _ => Err(OrderStateError::InvalidRecord),
        }
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, OrderStateError> {
        let wire: TransitionWire = decode_canonical(bytes)?;
        if wire.schema != TRANSITION_SCHEMA {
            return Err(OrderStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| OrderStateError::InvalidRecord)?,
            event_kind: parse_order_kind(&wire.event_kind)?,
            order_id: wire
                .order_id
                .map(OrderId::new)
                .transpose()
                .map_err(|_| OrderStateError::InvalidRecord)?,
            client_order_id: wire
                .client_order_id
                .map(ClientOrderId::new)
                .transpose()
                .map_err(|_| OrderStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| OrderStateError::InvalidRecord)?,
            market_id: wire
                .market_id
                .map(MarketId::new)
                .transpose()
                .map_err(|_| OrderStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
            prior_state_hash: wire
                .prior_state_blake3
                .as_deref()
                .map(decode_hash)
                .transpose()?,
            result_state_hash: wire
                .result_state_blake3
                .as_deref()
                .map(decode_hash)
                .transpose()?,
            rule_version: wire.rule_version,
            status: OrderTransitionStatusV1::parse(&wire.status)?,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, OrderStateError> {
        let record = Self::decode(bytes)?;
        if record.state_key()? != *key {
            return Err(OrderStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, OrderStateError> {
        self.validate()?;
        encode_canonical(&TransitionWire {
            schema: TRANSITION_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            order_id: self
                .order_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            client_order_id: self
                .client_order_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            account_id: self.account_id.to_api_string(),
            market_id: self
                .market_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
            prior_state_blake3: self.prior_state_hash.map(hex::encode),
            result_state_blake3: self.result_state_hash.map(hex::encode),
            rule_version: self.rule_version.clone(),
            status: self.status.as_wire_name().to_owned(),
        })
    }

    fn validate(&self) -> Result<(), OrderStateError> {
        if self.rule_version != CanonicalOrderReducerV1::VERSION {
            return Err(OrderStateError::InvalidRecord);
        }
        match self.status {
            OrderTransitionStatusV1::Applied => {
                if self.event_kind == EventKind::OrderRejected
                    || self.order_id.is_none()
                    || self.market_id.is_none()
                    || self.client_order_id.is_some()
                    || self.result_state_hash.is_none()
                    || (self.event_kind == EventKind::OrderAccepted
                        && self.prior_state_hash.is_some())
                    || (self.event_kind != EventKind::OrderAccepted
                        && self.prior_state_hash.is_none())
                {
                    return Err(OrderStateError::InvalidRecord);
                }
            }
            OrderTransitionStatusV1::RecordedRejection => {
                if self.event_kind != EventKind::OrderRejected
                    || self.order_id.is_some()
                    || self.market_id.is_some()
                    || self.client_order_id.is_none()
                    || self.prior_state_hash.is_some()
                    || self.result_state_hash.is_some()
                {
                    return Err(OrderStateError::InvalidRecord);
                }
            }
        }
        Ok(())
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
    pub const fn result_state_hash(&self) -> Option<[u8; 32]> {
        self.result_state_hash
    }

    #[must_use]
    pub fn rule_version(&self) -> &str {
        &self.rule_version
    }

    #[must_use]
    pub const fn status(&self) -> OrderTransitionStatusV1 {
        self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OrderStateError {
    #[error("order-state key is invalid")]
    InvalidKey,
    #[error("order-state record cannot be decoded")]
    Codec,
    #[error("order-state record bytes are not canonical")]
    NonCanonical,
    #[error("order-state record is invalid")]
    InvalidRecord,
    #[error("order-state record identity does not match its key")]
    KeyMismatch,
    #[error("order-state record exceeds its deterministic bound")]
    LimitExceeded,
}

impl OrderStateError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "order_state.codec.invalid_key",
            Self::Codec => "order_state.codec.decode",
            Self::NonCanonical => "order_state.codec.noncanonical",
            Self::InvalidRecord => "order_state.codec.invalid_record",
            Self::KeyMismatch => "order_state.codec.key_mismatch",
            Self::LimitExceeded => "order_state.codec.limit_exceeded",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactWire {
    schema: String,
    event_id: String,
    event_kind: String,
    order_id: Option<String>,
    client_order_id: Option<String>,
    account_id: String,
    market_id: Option<String>,
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
    side: String,
    lifecycle: String,
    limit_price: String,
    accepted_quantity: String,
    filled_quantity: String,
    remaining_quantity: String,
    accepted_event_id: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransitionWire {
    schema: String,
    event_id: String,
    event_kind: String,
    order_id: Option<String>,
    client_order_id: Option<String>,
    account_id: String,
    market_id: Option<String>,
    block_height: u64,
    payload_blake3: String,
    prior_state_blake3: Option<String>,
    result_state_blake3: Option<String>,
    rule_version: String,
    status: String,
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, OrderStateError> {
    let bytes = serde_json::to_vec(value).map_err(|_| OrderStateError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(OrderStateError::LimitExceeded);
    }
    Ok(bytes)
}

fn decode_canonical<T>(bytes: &[u8]) -> Result<T, OrderStateError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(OrderStateError::LimitExceeded);
    }
    let value = serde_json::from_slice(bytes).map_err(|_| OrderStateError::Codec)?;
    if encode_canonical(&value)? != bytes {
        return Err(OrderStateError::NonCanonical);
    }
    Ok(value)
}

fn decode_hash(value: &str) -> Result<[u8; 32], OrderStateError> {
    if value.len() != 64 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(OrderStateError::InvalidRecord);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| OrderStateError::InvalidRecord)?;
    Ok(hash)
}

fn parse_order_kind(value: &str) -> Result<EventKind, OrderStateError> {
    EventKind::try_from(value)
        .map_err(|_| OrderStateError::InvalidRecord)
        .and_then(|kind| {
            if is_order_kind(kind) {
                Ok(kind)
            } else {
                Err(OrderStateError::InvalidRecord)
            }
        })
}

const fn is_order_kind(kind: EventKind) -> bool {
    matches!(
        kind,
        EventKind::OrderAccepted
            | EventKind::OrderRested
            | EventKind::OrderModified
            | EventKind::OrderPartiallyFilled
            | EventKind::OrderFilled
            | EventKind::OrderCancelled
            | EventKind::OrderRejected
    )
}

fn order_id(payload: &EventPayload) -> Option<&OrderId> {
    match payload {
        EventPayload::OrderAccepted(value) => Some(&value.order_id),
        EventPayload::OrderRested(value) => Some(&value.order_id),
        EventPayload::OrderModified(value) => Some(&value.order_id),
        EventPayload::OrderPartiallyFilled(value) => Some(&value.order_id),
        EventPayload::OrderFilled(value) => Some(&value.order_id),
        EventPayload::OrderCancelled(value) => Some(&value.order_id),
        _ => None,
    }
}

fn current_identity_key(
    market_id: &MarketId,
    order_id: &OrderId,
) -> Result<Vec<u8>, OrderStateError> {
    let mut key = vec![0];
    extend_frame(&mut key, market_id.as_str().as_bytes())?;
    extend_frame(&mut key, order_id.as_str().as_bytes())?;
    Ok(key)
}

fn order_identity_key(
    market_id: &MarketId,
    order_id: &OrderId,
    event_id: &EventId,
) -> Result<Vec<u8>, OrderStateError> {
    let mut key = current_identity_key(market_id, order_id)?;
    extend_frame(&mut key, event_id.as_str().as_bytes())?;
    Ok(key)
}

fn rejection_identity_key(
    account_id: &Address,
    client_order_id: &ClientOrderId,
    event_id: &EventId,
) -> Result<Vec<u8>, OrderStateError> {
    let mut key = vec![1];
    extend_frame(&mut key, account_id.as_bytes())?;
    extend_frame(&mut key, client_order_id.as_str().as_bytes())?;
    extend_frame(&mut key, event_id.as_str().as_bytes())?;
    Ok(key)
}

fn extend_frame(target: &mut Vec<u8>, value: &[u8]) -> Result<(), OrderStateError> {
    let length = u64::try_from(value.len()).map_err(|_| OrderStateError::InvalidKey)?;
    target.extend_from_slice(&length.to_be_bytes());
    target.extend_from_slice(value);
    Ok(())
}

fn hash_current(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(CURRENT_HASH_CONTEXT);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn arithmetic_error() -> ReducerError {
    reducer_error(
        "order_state.arithmetic_overflow",
        "order quantity arithmetic overflowed",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn codec_reducer_error(error: OrderStateError) -> ReducerError {
    match error {
        OrderStateError::InvalidKey => reducer_error(
            "order_state.codec_invalid_key",
            "order state key encoding failed",
        ),
        OrderStateError::LimitExceeded => reducer_error(
            "order_state.codec_limit_exceeded",
            "order state record exceeds its deterministic bound",
        ),
        OrderStateError::Codec
        | OrderStateError::NonCanonical
        | OrderStateError::InvalidRecord
        | OrderStateError::KeyMismatch => reducer_error(
            "order_state.codec_failed",
            "order state record encoding failed",
        ),
    }
}
