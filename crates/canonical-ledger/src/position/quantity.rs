use std::collections::BTreeSet;
use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload, TradeParticipantRoleV1};
use domain_types::{
    Address, BlockHeight, EventId, ExactQuoteNotional, LiquidationId, MarketId, PositionQuantity,
    Price, Quantity, RoundingMode, TradeId,
};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyContext, EventReducer, MarketCurrentRecordV1, MarketMetadataResolutionV1, ReducerError,
    StateKey, StateMutation, StateView,
    position::codec::{
        PositionStateError, decode_wire, encode_wire, require_record_bytes, state_key,
    },
};

const CURRENT_NAMESPACE: &str = "position-quantity-current.v1";
const EFFECT_NAMESPACE: &str = "position-effect-fact.v1";
const UNRESOLVED_NAMESPACE: &str = "position-unresolved-cause-fact.v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/position-quantity-current/v1";
const EFFECT_SCHEMA: &str = "hyperliquid-alpha-desk/position-effect-fact/v1";
const UNRESOLVED_SCHEMA: &str = "hyperliquid-alpha-desk/position-unresolved-cause-fact/v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalPositionReducerV1;

impl CanonicalPositionReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-position@1.0.0";
}

impl EventReducer for CanonicalPositionReducerV1 {
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
        if !self.supports(event) {
            return Err(reducer_error(
                "position_state.unsupported_event",
                "position reducer received an unsupported event",
            ));
        }
        let EventPayload::TradeMatched(trade) = event.payload() else {
            return Err(reducer_error(
                "position_state.unsupported_event",
                "position reducer received an unsupported event",
            ));
        };
        let trade_id = trade.trade_id.as_ref().ok_or_else(identity_mismatch)?;
        let market_id = trade.market_id.as_ref().ok_or_else(identity_mismatch)?;
        let Some([buyer, seller]) = trade.participants.as_deref() else {
            return Err(identity_mismatch());
        };
        if buyer.role != TradeParticipantRoleV1::Buyer
            || seller.role != TradeParticipantRoleV1::Seller
            || buyer.account_id == seller.account_id
            || event.market_ids() != std::slice::from_ref(market_id)
            || event.account_addresses() != [buyer.account_id, seller.account_id]
        {
            return Err(identity_mismatch());
        }

        let market = load_market(state, market_id)?;
        let (price_scale, quantity_scale, tick_size, lot_size) = exact_market_contract(&market)?;
        let price = normalize_price(trade.price, price_scale)?;
        let fill = normalize_quantity(trade.quantity, quantity_scale)?;
        if price.raw() <= 0 || price.raw() % tick_size.raw() != 0 {
            return Err(reducer_error(
                "position_state.price_tick_misaligned",
                "trade price is not positive and tick aligned",
            ));
        }
        if fill.raw() <= 0 || fill.raw() % lot_size.raw() != 0 {
            return Err(quantity_lot_misaligned());
        }

        let legs = [
            (
                TradeParticipantRoleV1::Buyer,
                buyer.account_id,
                buyer.start_position,
            ),
            (
                TradeParticipantRoleV1::Seller,
                seller.account_id,
                seller.start_position,
            ),
        ];
        let mut effect_mutations = Vec::with_capacity(2);
        let mut current_mutations = Vec::with_capacity(2);
        for (role, account_id, source_start) in legs {
            let start = normalize_position(source_start, quantity_scale)?;
            if start.raw() % lot_size.raw() != 0 {
                return Err(quantity_lot_misaligned());
            }
            let effect = PositionQuantity::from_raw(fill.raw(), quantity_scale)
                .map_err(|_| position_arithmetic())?;
            let result = match role {
                TradeParticipantRoleV1::Buyer => start
                    .checked_add(effect)
                    .map_err(|_| position_arithmetic())?,
                TradeParticipantRoleV1::Seller => start
                    .checked_sub(effect)
                    .map_err(|_| position_arithmetic())?,
            };
            if result.raw() % lot_size.raw() != 0 {
                return Err(quantity_lot_misaligned());
            }

            let current_key = PositionQuantityCurrentRecordV1::state_key(&account_id, market_id)
                .map_err(codec_reducer_error)?;
            let (anchor_transition, first_anchor_event_id) =
                match load_current(state, &current_key)? {
                    None => (
                        PositionAnchorTransitionV1::FirstObservation,
                        event.event_id().clone(),
                    ),
                    Some(current) => match current.known_quantity {
                        Some(known) => {
                            let known = normalize_position(known, quantity_scale)?;
                            if known != start {
                                return Err(reducer_error(
                                    "position_state.start_position_mismatch",
                                    "source start position does not match known current position",
                                ));
                            }
                            (
                                PositionAnchorTransitionV1::Continued,
                                current
                                    .first_anchor_event_id
                                    .ok_or_else(current_record_invalid)?,
                            )
                        }
                        None => (
                            PositionAnchorTransitionV1::ReanchoredFromUnresolved,
                            current
                                .first_anchor_event_id
                                .unwrap_or_else(|| event.event_id().clone()),
                        ),
                    },
                };

            let effect_key = PositionEffectFactRecordV1::state_key(trade_id, role)
                .map_err(codec_reducer_error)?;
            reject_prior_effect(state, &effect_key)?;
            let effect_record = PositionEffectFactRecordV1 {
                event_id: event.event_id().clone(),
                trade_id: trade_id.clone(),
                account_id,
                market_id: market_id.clone(),
                role,
                anchor_transition,
                start_position: start,
                fill_quantity: fill,
                result_position: result,
                rule_version: Self::VERSION.to_owned(),
            };
            let current_record = PositionQuantityCurrentRecordV1::try_new(
                account_id,
                market_id.clone(),
                Some(result),
                Some(first_anchor_event_id),
                event.event_id().clone(),
                event.block_height(),
            )
            .map_err(codec_reducer_error)?;
            effect_mutations.push(StateMutation::put(
                effect_key,
                effect_record.encode().map_err(codec_reducer_error)?,
            ));
            current_mutations.push(StateMutation::put(
                current_key,
                current_record.encode().map_err(codec_reducer_error)?,
            ));
        }
        // This validation deliberately follows normalization/alignment of
        // price, fill, both starts, and both results. Trade V2 retains source
        // price and analytical episodes own persisted notionals.
        ExactQuoteNotional::checked_product(price, fill).map_err(|_| {
            reducer_error(
                "position_state.notional_arithmetic",
                "normalized exact notional cannot be represented",
            )
        })?;
        let mut mutations = effect_mutations;
        mutations.extend(current_mutations);
        ensure_unique_mutation_keys(&mutations)?;
        Ok(mutations)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionAnchorTransitionV1 {
    FirstObservation,
    Continued,
    ReanchoredFromUnresolved,
}

impl PositionAnchorTransitionV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::FirstObservation => "first_observation",
            Self::Continued => "continued",
            Self::ReanchoredFromUnresolved => "reanchored_from_unresolved",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "first_observation" => Ok(Self::FirstObservation),
            "continued" => Ok(Self::Continued),
            "reanchored_from_unresolved" => Ok(Self::ReanchoredFromUnresolved),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PositionUnresolvedCauseV1 {
    BackstopLiquidation,
}

impl PositionUnresolvedCauseV1 {
    #[allow(dead_code, reason = "used by the planned sibling liquidation reducer")]
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::BackstopLiquidation => "backstop_liquidation",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "backstop_liquidation" => Ok(Self::BackstopLiquidation),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionQuantityCurrentRecordV1 {
    account_id: Address,
    market_id: MarketId,
    known_quantity: Option<PositionQuantity>,
    first_anchor_event_id: Option<EventId>,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl PositionQuantityCurrentRecordV1 {
    pub(super) fn try_new(
        account_id: Address,
        market_id: MarketId,
        known_quantity: Option<PositionQuantity>,
        first_anchor_event_id: Option<EventId>,
        last_event_id: EventId,
        last_block_height: BlockHeight,
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            account_id,
            market_id,
            known_quantity,
            first_anchor_event_id,
            last_event_id,
            last_block_height,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(
        account_id: &Address,
        market_id: &MarketId,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            CURRENT_NAMESPACE,
            &[account_id.as_bytes(), market_id.as_str().as_bytes()],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: PositionQuantityCurrentWireV1 = decode_wire(bytes)?;
        if wire.schema != CURRENT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            known_quantity: wire
                .known_quantity
                .map(|value| PositionQuantity::from_str(&value))
                .transpose()
                .map_err(|_| PositionStateError::InvalidRecord)?,
            first_anchor_event_id: wire
                .first_anchor_event_id
                .map(EventId::new)
                .transpose()
                .map_err(|_| PositionStateError::InvalidRecord)?,
            last_event_id: EventId::new(wire.last_event_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            last_block_height: BlockHeight::new(wire.last_block_height),
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.account_id, &record.market_id)? != *key {
            return Err(PositionStateError::KeyMismatch);
        }
        Ok(record)
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&PositionQuantityCurrentWireV1 {
            schema: CURRENT_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            known_quantity: self.known_quantity.map(|value| value.to_string()),
            first_anchor_event_id: self
                .first_anchor_event_id
                .as_ref()
                .map(EventId::as_str)
                .map(str::to_owned),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        if self.known_quantity.is_some() && self.first_anchor_event_id.is_none() {
            Err(PositionStateError::InvalidRecord)
        } else {
            Ok(())
        }
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
    pub const fn known_quantity(&self) -> Option<PositionQuantity> {
        self.known_quantity
    }

    #[must_use]
    pub const fn first_anchor_event_id(&self) -> Option<&EventId> {
        self.first_anchor_event_id.as_ref()
    }

    #[must_use]
    pub const fn last_event_id(&self) -> &EventId {
        &self.last_event_id
    }

    #[must_use]
    pub const fn last_block_height(&self) -> BlockHeight {
        self.last_block_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionEffectFactRecordV1 {
    event_id: EventId,
    trade_id: TradeId,
    account_id: Address,
    market_id: MarketId,
    role: TradeParticipantRoleV1,
    anchor_transition: PositionAnchorTransitionV1,
    start_position: PositionQuantity,
    fill_quantity: Quantity,
    result_position: PositionQuantity,
    rule_version: String,
}

impl PositionEffectFactRecordV1 {
    pub fn state_key(
        trade_id: &TradeId,
        role: TradeParticipantRoleV1,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            EFFECT_NAMESPACE,
            &[trade_id.as_str().as_bytes(), encode_role(role).as_bytes()],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: PositionEffectFactWireV1 = decode_wire(bytes)?;
        if wire.schema != EFFECT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| PositionStateError::InvalidRecord)?,
            trade_id: TradeId::new(wire.trade_id).map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            role: decode_role(&wire.role)?,
            anchor_transition: PositionAnchorTransitionV1::parse(&wire.anchor_transition)?,
            start_position: PositionQuantity::from_str(&wire.start_position)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            fill_quantity: Quantity::from_str(&wire.fill_quantity)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            result_position: PositionQuantity::from_str(&wire.result_position)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.trade_id, record.role)? != *key {
            return Err(PositionStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&PositionEffectFactWireV1 {
            schema: EFFECT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            trade_id: self.trade_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            role: encode_role(self.role).to_owned(),
            anchor_transition: self.anchor_transition.as_wire_name().to_owned(),
            start_position: self.start_position.to_string(),
            fill_quantity: self.fill_quantity.to_string(),
            result_position: self.result_position.to_string(),
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        if self.rule_version != CanonicalPositionReducerV1::VERSION
            || self.fill_quantity.raw() <= 0
            || self.start_position.scale() != self.fill_quantity.scale()
            || self.result_position.scale() != self.fill_quantity.scale()
        {
            return Err(PositionStateError::InvalidRecord);
        }
        let effect =
            PositionQuantity::from_raw(self.fill_quantity.raw(), self.fill_quantity.scale())
                .map_err(|_| PositionStateError::InvalidRecord)?;
        let expected = match self.role {
            TradeParticipantRoleV1::Buyer => self.start_position.checked_add(effect),
            TradeParticipantRoleV1::Seller => self.start_position.checked_sub(effect),
        }
        .map_err(|_| PositionStateError::InvalidRecord)?;
        if expected != self.result_position {
            return Err(PositionStateError::InvalidRecord);
        }
        Ok(())
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
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn role(&self) -> TradeParticipantRoleV1 {
        self.role
    }

    #[must_use]
    pub const fn anchor_transition(&self) -> PositionAnchorTransitionV1 {
        self.anchor_transition
    }

    #[must_use]
    pub const fn start_position(&self) -> PositionQuantity {
        self.start_position
    }

    #[must_use]
    pub const fn fill_quantity(&self) -> Quantity {
        self.fill_quantity
    }

    #[must_use]
    pub const fn result_position(&self) -> PositionQuantity {
        self.result_position
    }

    #[must_use]
    pub fn rule_version(&self) -> &str {
        &self.rule_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionUnresolvedCauseFactRecordV1 {
    account_id: Address,
    market_id: MarketId,
    event_id: EventId,
    liquidation_id: LiquidationId,
    cause: PositionUnresolvedCauseV1,
}

impl PositionUnresolvedCauseFactRecordV1 {
    #[allow(dead_code, reason = "used by the planned sibling liquidation reducer")]
    pub(super) fn backstop_liquidation(
        account_id: Address,
        market_id: MarketId,
        event_id: EventId,
        liquidation_id: LiquidationId,
    ) -> Self {
        Self {
            account_id,
            market_id,
            event_id,
            liquidation_id,
            cause: PositionUnresolvedCauseV1::BackstopLiquidation,
        }
    }

    pub fn state_key(
        account_id: &Address,
        market_id: &MarketId,
        event_id: &EventId,
        liquidation_id: &LiquidationId,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            UNRESOLVED_NAMESPACE,
            &[
                account_id.as_bytes(),
                market_id.as_str().as_bytes(),
                event_id.as_str().as_bytes(),
                liquidation_id.as_str().as_bytes(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: PositionUnresolvedCauseFactWireV1 = decode_wire(bytes)?;
        if wire.schema != UNRESOLVED_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            event_id: EventId::new(wire.event_id).map_err(|_| PositionStateError::InvalidRecord)?,
            liquidation_id: LiquidationId::new(wire.liquidation_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            cause: PositionUnresolvedCauseV1::parse(&wire.cause)?,
        };
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(
            &record.account_id,
            &record.market_id,
            &record.event_id,
            &record.liquidation_id,
        )? != *key
        {
            return Err(PositionStateError::KeyMismatch);
        }
        Ok(record)
    }

    #[allow(dead_code, reason = "used by the planned sibling liquidation reducer")]
    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        encode_wire(&PositionUnresolvedCauseFactWireV1 {
            schema: UNRESOLVED_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            liquidation_id: self.liquidation_id.as_str().to_owned(),
            cause: self.cause.as_wire_name().to_owned(),
        })
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
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
    }

    #[must_use]
    pub const fn liquidation_id(&self) -> &LiquidationId {
        &self.liquidation_id
    }

    #[must_use]
    pub const fn cause(&self) -> PositionUnresolvedCauseV1 {
        self.cause
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionQuantityCurrentWireV1 {
    schema: String,
    account_id: String,
    market_id: String,
    known_quantity: Option<String>,
    first_anchor_event_id: Option<String>,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionEffectFactWireV1 {
    schema: String,
    event_id: String,
    trade_id: String,
    account_id: String,
    market_id: String,
    role: String,
    anchor_transition: String,
    start_position: String,
    fill_quantity: String,
    result_position: String,
    rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionUnresolvedCauseFactWireV1 {
    schema: String,
    account_id: String,
    market_id: String,
    event_id: String,
    liquidation_id: String,
    cause: String,
}

fn load_market(
    state: &StateView<'_>,
    market_id: &MarketId,
) -> Result<MarketCurrentRecordV1, ReducerError> {
    let key = MarketCurrentRecordV1::state_key(market_id).map_err(|_| {
        reducer_error(
            "position_state.market_prerequisite_invalid",
            "market prerequisite key construction failed",
        )
    })?;
    let bytes = state.get(&key).ok_or_else(|| {
        reducer_error(
            "position_state.market_prerequisite_missing",
            "market prerequisite is missing",
        )
    })?;
    MarketCurrentRecordV1::decode_at(&key, bytes).map_err(|_| {
        reducer_error(
            "position_state.market_prerequisite_invalid",
            "market prerequisite is corrupt or key mismatched",
        )
    })
}

fn exact_market_contract(
    market: &MarketCurrentRecordV1,
) -> Result<(u8, u8, Price, Quantity), ReducerError> {
    if market.metadata_resolution() != MarketMetadataResolutionV1::Exact {
        return Err(reducer_error(
            "position_state.market_metadata_unresolved",
            "market prerequisite metadata is unresolved",
        ));
    }
    let price_scale = market
        .price_scale()
        .and_then(|value| u8::try_from(value).ok());
    let quantity_scale = market
        .quantity_scale()
        .and_then(|value| u8::try_from(value).ok());
    match (
        price_scale,
        quantity_scale,
        market.tick_size(),
        market.lot_size(),
    ) {
        (Some(price_scale), Some(quantity_scale), Some(tick), Some(lot))
            if tick.raw() > 0
                && lot.raw() > 0
                && tick.scale() == price_scale
                && lot.scale() == quantity_scale =>
        {
            Ok((price_scale, quantity_scale, tick, lot))
        }
        _ => Err(reducer_error(
            "position_state.market_prerequisite_invalid",
            "exact market prerequisite is internally invalid",
        )),
    }
}

fn normalize_price(value: Price, target_scale: u8) -> Result<Price, ReducerError> {
    if value.scale() > target_scale {
        return Err(scale_normalization());
    }
    value
        .rescale(target_scale, RoundingMode::TowardZero)
        .map_err(|_| scale_normalization())
}

fn normalize_quantity(value: Quantity, target_scale: u8) -> Result<Quantity, ReducerError> {
    if value.scale() > target_scale {
        return Err(scale_normalization());
    }
    value
        .rescale(target_scale, RoundingMode::TowardZero)
        .map_err(|_| scale_normalization())
}

fn normalize_position(
    value: PositionQuantity,
    target_scale: u8,
) -> Result<PositionQuantity, ReducerError> {
    value
        .checked_rescale_up(target_scale)
        .map_err(|_| scale_normalization())
}

fn load_current(
    state: &StateView<'_>,
    key: &StateKey,
) -> Result<Option<PositionQuantityCurrentRecordV1>, ReducerError> {
    state
        .get(key)
        .map(|bytes| {
            PositionQuantityCurrentRecordV1::decode_at(key, bytes).map_err(|_| {
                reducer_error(
                    "position_state.current_record_invalid",
                    "position current record is corrupt or key mismatched",
                )
            })
        })
        .transpose()
}

fn reject_prior_effect(state: &StateView<'_>, key: &StateKey) -> Result<(), ReducerError> {
    let Some(bytes) = state.get(key) else {
        return Ok(());
    };
    PositionEffectFactRecordV1::decode_at(key, bytes).map_err(codec_reducer_error)?;
    Err(reducer_error(
        "position_state.effect_collision",
        "position effect identity is already present",
    ))
}

fn ensure_unique_mutation_keys(mutations: &[StateMutation]) -> Result<(), ReducerError> {
    let mut keys = BTreeSet::new();
    if mutations.iter().all(|mutation| keys.insert(mutation.key())) {
        Ok(())
    } else {
        Err(reducer_error(
            "position_state.duplicate_mutation_key",
            "position event produced duplicate mutation keys",
        ))
    }
}

const fn encode_role(role: TradeParticipantRoleV1) -> &'static str {
    match role {
        TradeParticipantRoleV1::Buyer => "buyer",
        TradeParticipantRoleV1::Seller => "seller",
    }
}

fn decode_role(value: &str) -> Result<TradeParticipantRoleV1, PositionStateError> {
    match value {
        "buyer" => Ok(TradeParticipantRoleV1::Buyer),
        "seller" => Ok(TradeParticipantRoleV1::Seller),
        _ => Err(PositionStateError::InvalidRecord),
    }
}

fn identity_mismatch() -> ReducerError {
    reducer_error(
        "position_state.identity_mismatch",
        "trade payload and envelope identities must match exactly",
    )
}

fn scale_normalization() -> ReducerError {
    reducer_error(
        "position_state.scale_normalization",
        "trade values cannot be normalized upward exactly",
    )
}

fn quantity_lot_misaligned() -> ReducerError {
    reducer_error(
        "position_state.quantity_lot_misaligned",
        "trade quantity or position is not lot aligned",
    )
}

fn position_arithmetic() -> ReducerError {
    reducer_error(
        "position_state.position_arithmetic",
        "position arithmetic cannot be represented exactly",
    )
}

fn current_record_invalid() -> ReducerError {
    reducer_error(
        "position_state.current_record_invalid",
        "position current record violates its anchor invariant",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn codec_reducer_error(error: PositionStateError) -> ReducerError {
    ReducerError::from_static(
        error.reason_code(),
        "position-state codec or key validation failed",
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use canonical_events::ConfirmationClass;

    use super::*;

    #[test]
    fn direct_participant_free_reduce_is_a_typed_unsupported_event() {
        let event = CanonicalEventEnvelope::fixture().unwrap();
        let entries = BTreeMap::new();
        let state = crate::state::view_entries(&entries);
        let context = ApplyContext::new(
            event.chain_id(),
            event.block_height(),
            event.block_time(),
            ConfirmationClass::CommittedPrimary,
        );
        let error = EventReducer::reduce(&CanonicalPositionReducerV1, &state, &event, &context)
            .unwrap_err();
        assert_eq!(error.reason_code(), "position_state.unsupported_event");
    }

    #[test]
    fn sibling_only_current_and_backstop_constructors_preserve_invariants() {
        let account = Address::from_bytes([0x11; 20]);
        let market = MarketId::new("perp:BTC").unwrap();
        let event = EventId::new("event-anchor").unwrap();
        let invalid = PositionQuantityCurrentRecordV1::try_new(
            account,
            market.clone(),
            Some(PositionQuantity::from_str("1").unwrap()),
            None,
            event.clone(),
            BlockHeight::new(1),
        );
        assert_eq!(invalid.unwrap_err(), PositionStateError::InvalidRecord);

        let current = PositionQuantityCurrentRecordV1::try_new(
            account,
            market.clone(),
            None,
            Some(event.clone()),
            event.clone(),
            BlockHeight::new(1),
        )
        .unwrap();
        let current_key = PositionQuantityCurrentRecordV1::state_key(&account, &market).unwrap();
        assert_eq!(
            PositionQuantityCurrentRecordV1::decode_at(&current_key, &current.encode().unwrap())
                .unwrap(),
            current
        );

        let cause = PositionUnresolvedCauseFactRecordV1::backstop_liquidation(
            account,
            market.clone(),
            event.clone(),
            LiquidationId::new("liq-backstop").unwrap(),
        );
        let cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
            &account,
            &market,
            &event,
            cause.liquidation_id(),
        )
        .unwrap();
        assert_eq!(
            PositionUnresolvedCauseFactRecordV1::decode_at(&cause_key, &cause.encode().unwrap())
                .unwrap(),
            cause
        );
    }
}
