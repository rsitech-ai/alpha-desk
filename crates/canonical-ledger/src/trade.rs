use std::str::FromStr;

use canonical_events::{
    CanonicalEventEnvelope, EventKind, EventPayload, TradeParticipantRoleV1, TradeParticipantV1,
};
use domain_types::{
    Address, BlockHeight, ClientOrderId, EventId, MarketId, OrderId, PositionQuantity, Price,
    Quantity, TradeId, TwapId,
};
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
const TRADE_V2_NAMESPACE: &str = "trade.v2";
const PARTICIPANT_V2_NAMESPACE: &str = "trade-participant.v2";
const RECONCILIATION_V2_NAMESPACE: &str = "trade-reconciliation.v2";
const TRADE_V2_SCHEMA: &str = "hyperliquid-alpha-desk/trade-state/v2";
const PARTICIPANT_V2_SCHEMA: &str = "hyperliquid-alpha-desk/trade-participant/v2";
const RECONCILIATION_V2_SCHEMA: &str = "hyperliquid-alpha-desk/trade-reconciliation/v2";
const RECONCILIATION_V2_CHECK_VERSION: &str = "trade-position-symmetry@2.0.0";
const EVIDENCE_V2_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/trade-reconciliation-evidence/v2";

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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalTradeReducerV2;

impl CanonicalTradeReducerV2 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-trade@2.0.0";
}

impl EventReducer for CanonicalTradeReducerV2 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.event_kind() == EventKind::TradeMatched
            && event.schema_version() == "1.0.0"
            && matches!(
                event.payload(),
                EventPayload::TradeMatched(trade) if trade.participants.is_some()
            )
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
        if event.schema_version() != "1.0.0"
            || event.market_ids() != std::slice::from_ref(market_id)
        {
            return Err(reducer_error(
                "trade_state.invalid_market",
                "trade payload and envelope market must match exactly",
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
        let participants = trade.participants.as_deref().ok_or_else(|| {
            reducer_error(
                "trade_state.missing_participant_anchors",
                "V2 trade state requires exact participant anchors",
            )
        })?;
        let [buyer, seller] = participants;
        if buyer.role != TradeParticipantRoleV1::Buyer
            || seller.role != TradeParticipantRoleV1::Seller
            || buyer.account_id == seller.account_id
            || event.account_addresses() != [buyer.account_id, seller.account_id]
        {
            return Err(reducer_error(
                "trade_state.invalid_participants",
                "trade participant anchors must be buyer then seller and match the envelope",
            ));
        }

        let trade_key = TradeStateRecordV2::state_key(trade_id).map_err(codec_reducer_error)?;
        let buyer_key =
            TradeParticipantRecordV2::state_key(trade_id, 0).map_err(codec_reducer_error)?;
        let seller_key =
            TradeParticipantRecordV2::state_key(trade_id, 1).map_err(codec_reducer_error)?;
        let reconciliation_key =
            TradeReconciliationRecordV2::state_key(trade_id).map_err(codec_reducer_error)?;
        reject_prior_v2_facts(
            state,
            &trade_key,
            &buyer_key,
            &seller_key,
            &reconciliation_key,
        )?;

        let buyer_effect = signed_effect(trade.quantity, TradeParticipantRoleV1::Buyer)
            .map_err(codec_reducer_error)?;
        let seller_effect = signed_effect(trade.quantity, TradeParticipantRoleV1::Seller)
            .map_err(codec_reducer_error)?;
        let record = TradeStateRecordV2 {
            event_id: event.event_id().clone(),
            trade_id: trade_id.clone(),
            market_id: market_id.clone(),
            price: trade.price,
            quantity: trade.quantity,
            buyer_account_id: buyer.account_id,
            seller_account_id: seller.account_id,
            buyer_start_position: buyer.start_position,
            seller_start_position: seller.start_position,
            buyer_order_id: buyer.order_id.clone(),
            seller_order_id: seller.order_id.clone(),
            buyer_twap_id: buyer.twap_id,
            seller_twap_id: seller.twap_id,
            buyer_client_order_id: buyer.client_order_id.clone(),
            seller_client_order_id: seller.client_order_id.clone(),
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
        };
        let buyer_record =
            participant_record(event, trade_id, trade.quantity, 0, buyer, buyer_effect);
        let seller_record =
            participant_record(event, trade_id, trade.quantity, 1, seller, seller_effect);
        let reconciliation = TradeReconciliationRecordV2 {
            event_id: event.event_id().clone(),
            trade_id: trade_id.clone(),
            market_id: market_id.clone(),
            absolute_quantity: trade.quantity,
            buyer_effect,
            seller_effect,
            participant_count: 2,
            block_height: event.block_height(),
            evidence_hash: reconciliation_v2_evidence_hash(&record, &buyer_record, &seller_record),
        };

        Ok(vec![
            StateMutation::put(trade_key, record.encode().map_err(codec_reducer_error)?),
            StateMutation::put(
                buyer_key,
                buyer_record.encode().map_err(codec_reducer_error)?,
            ),
            StateMutation::put(
                seller_key,
                seller_record.encode().map_err(codec_reducer_error)?,
            ),
            StateMutation::put(
                reconciliation_key,
                reconciliation.encode().map_err(codec_reducer_error)?,
            ),
        ])
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalTradeReducerSetV2;

impl CanonicalTradeReducerSetV2 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-trade-set@2.0.0";
}

impl EventReducer for CanonicalTradeReducerSetV2 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        CanonicalTradeReducerV1.supports(event)
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let mut mutations = CanonicalTradeReducerV1.reduce(state, event, context)?;
        if CanonicalTradeReducerV2.supports(event) {
            let v2 = CanonicalTradeReducerV2.reduce(state, event, context)?;
            mutations.extend(v2);
        }
        Ok(mutations)
    }

    fn validate_block(
        &self,
        state: &StateView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        CanonicalTradeReducerV1.validate_block(state, context)?;
        CanonicalTradeReducerV2.validate_block(state, context)
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
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn trade_id(&self) -> &TradeId {
        &self.trade_id
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

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeStateRecordV2 {
    event_id: EventId,
    trade_id: TradeId,
    market_id: MarketId,
    price: Price,
    quantity: Quantity,
    buyer_account_id: Address,
    seller_account_id: Address,
    buyer_start_position: PositionQuantity,
    seller_start_position: PositionQuantity,
    buyer_order_id: OrderId,
    seller_order_id: OrderId,
    buyer_twap_id: Option<TwapId>,
    seller_twap_id: Option<TwapId>,
    buyer_client_order_id: Option<ClientOrderId>,
    seller_client_order_id: Option<ClientOrderId>,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
}

impl TradeStateRecordV2 {
    pub fn state_key(trade_id: &TradeId) -> Result<StateKey, TradeStateError> {
        StateKey::try_new(TRADE_V2_NAMESPACE, trade_id.as_str().as_bytes().to_vec())
            .map_err(|_| TradeStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TradeStateError> {
        let wire: TradeStateWireV2 = decode_canonical(bytes)?;
        if wire.schema != TRADE_V2_SCHEMA {
            return Err(TradeStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TradeStateError::InvalidRecord)?,
            trade_id: TradeId::new(wire.trade_id).map_err(|_| TradeStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| TradeStateError::InvalidRecord)?,
            price: Price::from_str(&wire.price).map_err(|_| TradeStateError::InvalidRecord)?,
            quantity: Quantity::from_str(&wire.quantity)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            buyer_account_id: Address::parse_api(&wire.buyer_account_id)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            seller_account_id: Address::parse_api(&wire.seller_account_id)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            buyer_start_position: PositionQuantity::from_str(&wire.buyer_start_position)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            seller_start_position: PositionQuantity::from_str(&wire.seller_start_position)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            buyer_order_id: OrderId::new(wire.buyer_order_id)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            seller_order_id: OrderId::new(wire.seller_order_id)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            buyer_twap_id: wire.buyer_twap_id.map(TwapId::new),
            seller_twap_id: wire.seller_twap_id.map(TwapId::new),
            buyer_client_order_id: parse_optional_cloid(wire.buyer_client_order_id)?,
            seller_client_order_id: parse_optional_cloid(wire.seller_client_order_id)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
        };
        if record.price.raw() <= 0
            || record.quantity.raw() <= 0
            || record.buyer_account_id == record.seller_account_id
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
        encode_canonical(&TradeStateWireV2 {
            schema: TRADE_V2_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            trade_id: self.trade_id.as_str().to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            price: self.price.to_string(),
            quantity: self.quantity.to_string(),
            buyer_account_id: self.buyer_account_id.to_api_string(),
            seller_account_id: self.seller_account_id.to_api_string(),
            buyer_start_position: self.buyer_start_position.to_string(),
            seller_start_position: self.seller_start_position.to_string(),
            buyer_order_id: self.buyer_order_id.as_str().to_owned(),
            seller_order_id: self.seller_order_id.as_str().to_owned(),
            buyer_twap_id: self.buyer_twap_id.map(TwapId::get),
            seller_twap_id: self.seller_twap_id.map(TwapId::get),
            buyer_client_order_id: self
                .buyer_client_order_id
                .as_ref()
                .map(ClientOrderId::as_str)
                .map(str::to_owned),
            seller_client_order_id: self
                .seller_client_order_id
                .as_ref()
                .map(ClientOrderId::as_str)
                .map(str::to_owned),
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
    pub const fn buyer_account_id(&self) -> Address {
        self.buyer_account_id
    }

    #[must_use]
    pub const fn seller_account_id(&self) -> Address {
        self.seller_account_id
    }

    #[must_use]
    pub const fn buyer_start_position(&self) -> PositionQuantity {
        self.buyer_start_position
    }

    #[must_use]
    pub const fn seller_start_position(&self) -> PositionQuantity {
        self.seller_start_position
    }

    #[must_use]
    pub const fn buyer_order_id(&self) -> &OrderId {
        &self.buyer_order_id
    }

    #[must_use]
    pub const fn seller_order_id(&self) -> &OrderId {
        &self.seller_order_id
    }

    #[must_use]
    pub const fn buyer_twap_id(&self) -> Option<TwapId> {
        self.buyer_twap_id
    }

    #[must_use]
    pub const fn seller_twap_id(&self) -> Option<TwapId> {
        self.seller_twap_id
    }

    #[must_use]
    pub const fn buyer_client_order_id(&self) -> Option<&ClientOrderId> {
        self.buyer_client_order_id.as_ref()
    }

    #[must_use]
    pub const fn seller_client_order_id(&self) -> Option<&ClientOrderId> {
        self.seller_client_order_id.as_ref()
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
pub struct TradeParticipantRecordV2 {
    event_id: EventId,
    trade_id: TradeId,
    ordinal: u8,
    role: TradeParticipantRoleV1,
    account_id: Address,
    start_position: PositionQuantity,
    order_id: OrderId,
    twap_id: Option<TwapId>,
    client_order_id: Option<ClientOrderId>,
    fill_quantity: Quantity,
    position_effect: PositionQuantity,
    block_height: BlockHeight,
}

impl TradeParticipantRecordV2 {
    pub fn state_key(trade_id: &TradeId, ordinal: u8) -> Result<StateKey, TradeStateError> {
        framed_participant_key(PARTICIPANT_V2_NAMESPACE, trade_id, ordinal)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TradeStateError> {
        let wire: ParticipantWireV2 = decode_canonical(bytes)?;
        if wire.schema != PARTICIPANT_V2_SCHEMA || wire.ordinal > 1 {
            return Err(TradeStateError::InvalidRecord);
        }
        let role = decode_role(&wire.role)?;
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TradeStateError::InvalidRecord)?,
            trade_id: TradeId::new(wire.trade_id).map_err(|_| TradeStateError::InvalidRecord)?,
            ordinal: wire.ordinal,
            role,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            start_position: PositionQuantity::from_str(&wire.start_position)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            order_id: OrderId::new(wire.order_id).map_err(|_| TradeStateError::InvalidRecord)?,
            twap_id: wire.twap_id.map(TwapId::new),
            client_order_id: parse_optional_cloid(wire.client_order_id)?,
            fill_quantity: Quantity::from_str(&wire.fill_quantity)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            position_effect: PositionQuantity::from_str(&wire.position_effect)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
        };
        if record.fill_quantity.raw() <= 0
            || !participant_ordinal_matches_role(record.ordinal, record.role)
            || signed_effect(record.fill_quantity, record.role)? != record.position_effect
        {
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
        encode_canonical(&ParticipantWireV2 {
            schema: PARTICIPANT_V2_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            trade_id: self.trade_id.as_str().to_owned(),
            ordinal: self.ordinal,
            role: encode_role(self.role).to_owned(),
            account_id: self.account_id.to_api_string(),
            start_position: self.start_position.to_string(),
            order_id: self.order_id.as_str().to_owned(),
            twap_id: self.twap_id.map(TwapId::get),
            client_order_id: self
                .client_order_id
                .as_ref()
                .map(ClientOrderId::as_str)
                .map(str::to_owned),
            fill_quantity: self.fill_quantity.to_string(),
            position_effect: self.position_effect.to_string(),
            block_height: self.block_height.get(),
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
    pub const fn ordinal(&self) -> u8 {
        self.ordinal
    }

    #[must_use]
    pub const fn role(&self) -> TradeParticipantRoleV1 {
        self.role
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn start_position(&self) -> PositionQuantity {
        self.start_position
    }

    #[must_use]
    pub const fn order_id(&self) -> &OrderId {
        &self.order_id
    }

    #[must_use]
    pub const fn twap_id(&self) -> Option<TwapId> {
        self.twap_id
    }

    #[must_use]
    pub const fn client_order_id(&self) -> Option<&ClientOrderId> {
        self.client_order_id.as_ref()
    }

    #[must_use]
    pub const fn fill_quantity(&self) -> Quantity {
        self.fill_quantity
    }

    #[must_use]
    pub const fn position_effect(&self) -> PositionQuantity {
        self.position_effect
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeReconciliationRecordV2 {
    event_id: EventId,
    trade_id: TradeId,
    market_id: MarketId,
    absolute_quantity: Quantity,
    buyer_effect: PositionQuantity,
    seller_effect: PositionQuantity,
    participant_count: u8,
    block_height: BlockHeight,
    evidence_hash: [u8; 32],
}

impl TradeReconciliationRecordV2 {
    pub fn state_key(trade_id: &TradeId) -> Result<StateKey, TradeStateError> {
        StateKey::try_new(
            RECONCILIATION_V2_NAMESPACE,
            trade_id.as_str().as_bytes().to_vec(),
        )
        .map_err(|_| TradeStateError::InvalidKey)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, TradeStateError> {
        let wire: ReconciliationWireV2 = decode_canonical(bytes)?;
        if wire.schema != RECONCILIATION_V2_SCHEMA
            || wire.check_version != RECONCILIATION_V2_CHECK_VERSION
            || wire.status != "passed"
            || wire.participant_count != 2
        {
            return Err(TradeStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| TradeStateError::InvalidRecord)?,
            trade_id: TradeId::new(wire.trade_id).map_err(|_| TradeStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id).map_err(|_| TradeStateError::InvalidRecord)?,
            absolute_quantity: Quantity::from_str(&wire.absolute_quantity)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            buyer_effect: PositionQuantity::from_str(&wire.buyer_effect)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            seller_effect: PositionQuantity::from_str(&wire.seller_effect)
                .map_err(|_| TradeStateError::InvalidRecord)?,
            participant_count: wire.participant_count,
            block_height: BlockHeight::new(wire.block_height),
            evidence_hash: decode_hash(&wire.evidence_blake3)?,
        };
        if record.absolute_quantity.raw() <= 0
            || signed_effect(record.absolute_quantity, TradeParticipantRoleV1::Buyer)?
                != record.buyer_effect
            || signed_effect(record.absolute_quantity, TradeParticipantRoleV1::Seller)?
                != record.seller_effect
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
        encode_canonical(&ReconciliationWireV2 {
            schema: RECONCILIATION_V2_SCHEMA.to_owned(),
            check_version: RECONCILIATION_V2_CHECK_VERSION.to_owned(),
            status: "passed".to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            trade_id: self.trade_id.as_str().to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            absolute_quantity: self.absolute_quantity.to_string(),
            buyer_effect: self.buyer_effect.to_string(),
            seller_effect: self.seller_effect.to_string(),
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
    pub const fn absolute_quantity(&self) -> Quantity {
        self.absolute_quantity
    }

    #[must_use]
    pub const fn buyer_effect(&self) -> PositionQuantity {
        self.buyer_effect
    }

    #[must_use]
    pub const fn seller_effect(&self) -> PositionQuantity {
        self.seller_effect
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TradeStateWireV2 {
    schema: String,
    event_id: String,
    trade_id: String,
    market_id: String,
    price: String,
    quantity: String,
    buyer_account_id: String,
    seller_account_id: String,
    buyer_start_position: String,
    seller_start_position: String,
    buyer_order_id: String,
    seller_order_id: String,
    buyer_twap_id: Option<u64>,
    seller_twap_id: Option<u64>,
    buyer_client_order_id: Option<String>,
    seller_client_order_id: Option<String>,
    block_height: u64,
    payload_blake3: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ParticipantWireV2 {
    schema: String,
    event_id: String,
    trade_id: String,
    ordinal: u8,
    role: String,
    account_id: String,
    start_position: String,
    order_id: String,
    twap_id: Option<u64>,
    client_order_id: Option<String>,
    fill_quantity: String,
    position_effect: String,
    block_height: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReconciliationWireV2 {
    schema: String,
    check_version: String,
    status: String,
    event_id: String,
    trade_id: String,
    market_id: String,
    absolute_quantity: String,
    buyer_effect: String,
    seller_effect: String,
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

fn parse_optional_cloid(value: Option<String>) -> Result<Option<ClientOrderId>, TradeStateError> {
    value
        .map(|value| {
            if !is_canonical_trade_cloid(&value) {
                return Err(TradeStateError::InvalidRecord);
            }
            ClientOrderId::new(value).map_err(|_| TradeStateError::InvalidRecord)
        })
        .transpose()
}

fn is_canonical_trade_cloid(value: &str) -> bool {
    value.len() == 34
        && value.starts_with("0x")
        && value[2..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

const fn encode_role(role: TradeParticipantRoleV1) -> &'static str {
    match role {
        TradeParticipantRoleV1::Buyer => "buyer",
        TradeParticipantRoleV1::Seller => "seller",
    }
}

fn decode_role(value: &str) -> Result<TradeParticipantRoleV1, TradeStateError> {
    match value {
        "buyer" => Ok(TradeParticipantRoleV1::Buyer),
        "seller" => Ok(TradeParticipantRoleV1::Seller),
        _ => Err(TradeStateError::InvalidRecord),
    }
}

const fn participant_ordinal_matches_role(ordinal: u8, role: TradeParticipantRoleV1) -> bool {
    matches!(
        (ordinal, role),
        (0, TradeParticipantRoleV1::Buyer) | (1, TradeParticipantRoleV1::Seller)
    )
}

fn signed_effect(
    quantity: Quantity,
    role: TradeParticipantRoleV1,
) -> Result<PositionQuantity, TradeStateError> {
    let raw = match role {
        TradeParticipantRoleV1::Buyer => quantity.raw(),
        TradeParticipantRoleV1::Seller => quantity
            .raw()
            .checked_neg()
            .ok_or(TradeStateError::InvalidRecord)?,
    };
    PositionQuantity::from_raw(raw, quantity.scale()).map_err(|_| TradeStateError::InvalidRecord)
}

fn framed_participant_key(
    namespace: &'static str,
    trade_id: &TradeId,
    ordinal: u8,
) -> Result<StateKey, TradeStateError> {
    if ordinal > 1 {
        return Err(TradeStateError::InvalidKey);
    }
    let id = trade_id.as_str().as_bytes();
    let length = u16::try_from(id.len()).map_err(|_| TradeStateError::InvalidKey)?;
    let mut key = Vec::with_capacity(2 + id.len() + 1);
    key.extend_from_slice(&length.to_be_bytes());
    key.extend_from_slice(id);
    key.push(ordinal);
    StateKey::try_new(namespace, key).map_err(|_| TradeStateError::InvalidKey)
}

fn participant_record(
    event: &CanonicalEventEnvelope,
    trade_id: &TradeId,
    quantity: Quantity,
    ordinal: u8,
    participant: &TradeParticipantV1,
    position_effect: PositionQuantity,
) -> TradeParticipantRecordV2 {
    TradeParticipantRecordV2 {
        event_id: event.event_id().clone(),
        trade_id: trade_id.clone(),
        ordinal,
        role: participant.role,
        account_id: participant.account_id,
        start_position: participant.start_position,
        order_id: participant.order_id.clone(),
        twap_id: participant.twap_id,
        client_order_id: participant.client_order_id.clone(),
        fill_quantity: quantity,
        position_effect,
        block_height: event.block_height(),
    }
}

fn reject_prior_v2_facts(
    state: &StateView<'_>,
    trade_key: &StateKey,
    buyer_key: &StateKey,
    seller_key: &StateKey,
    reconciliation_key: &StateKey,
) -> Result<(), ReducerError> {
    let mut found = false;
    if let Some(bytes) = state.get(trade_key) {
        TradeStateRecordV2::decode_at(trade_key, bytes).map_err(prior_fact_reducer_error)?;
        found = true;
    }
    if let Some(bytes) = state.get(buyer_key) {
        TradeParticipantRecordV2::decode_at(buyer_key, bytes).map_err(prior_fact_reducer_error)?;
        found = true;
    }
    if let Some(bytes) = state.get(seller_key) {
        TradeParticipantRecordV2::decode_at(seller_key, bytes).map_err(prior_fact_reducer_error)?;
        found = true;
    }
    if let Some(bytes) = state.get(reconciliation_key) {
        TradeReconciliationRecordV2::decode_at(reconciliation_key, bytes)
            .map_err(prior_fact_reducer_error)?;
        found = true;
    }
    if found {
        return Err(reducer_error(
            "trade_state.trade_id_collision",
            "trade identity is already present in canonical V2 state",
        ));
    }
    Ok(())
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

fn reconciliation_v2_evidence_hash(
    record: &TradeStateRecordV2,
    buyer: &TradeParticipantRecordV2,
    seller: &TradeParticipantRecordV2,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(EVIDENCE_V2_HASH_CONTEXT);
    frame(&mut hasher, record.event_id.as_str().as_bytes());
    frame(&mut hasher, record.trade_id.as_str().as_bytes());
    frame(&mut hasher, record.market_id.as_str().as_bytes());
    frame(&mut hasher, record.price.to_string().as_bytes());
    frame(&mut hasher, record.quantity.to_string().as_bytes());
    hash_v2_participant(&mut hasher, buyer);
    hash_v2_participant(&mut hasher, seller);
    hasher.update(&record.block_height.get().to_be_bytes());
    hasher.update(&record.payload_hash);
    *hasher.finalize().as_bytes()
}

fn hash_v2_participant(hasher: &mut blake3::Hasher, participant: &TradeParticipantRecordV2) {
    hasher.update(&[participant.ordinal]);
    frame(hasher, encode_role(participant.role).as_bytes());
    hasher.update(participant.account_id.as_bytes());
    frame(hasher, participant.start_position.to_string().as_bytes());
    frame(hasher, participant.order_id.as_str().as_bytes());
    hash_optional_u64(hasher, participant.twap_id.map(TwapId::get));
    hash_optional_text(
        hasher,
        participant
            .client_order_id
            .as_ref()
            .map(ClientOrderId::as_str),
    );
    frame(hasher, participant.fill_quantity.to_string().as_bytes());
    frame(hasher, participant.position_effect.to_string().as_bytes());
}

fn hash_optional_u64(hasher: &mut blake3::Hasher, value: Option<u64>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            hasher.update(&value.to_be_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    }
}

fn hash_optional_text(hasher: &mut blake3::Hasher, value: Option<&str>) {
    match value {
        Some(value) => {
            hasher.update(&[1]);
            frame(hasher, value.as_bytes());
        }
        None => {
            hasher.update(&[0]);
        }
    }
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

fn prior_fact_reducer_error(_error: TradeStateError) -> ReducerError {
    reducer_error(
        "trade_state.prior_fact_invalid",
        "existing canonical V2 trade fact is corrupt or stored under the wrong key",
    )
}
