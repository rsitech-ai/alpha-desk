use std::str::FromStr;

use domain_types::{
    Address, BlockHeight, EventId, ExactQuoteNotional, MarketId, PositionEpisodeId,
    PositionQuantity, Quantity, QuoteAmount,
};
use serde::{Deserialize, Serialize};

use crate::{StateKey, StateView};

use super::codec::{PositionStateError, decode_wire, encode_wire, require_record_bytes, state_key};

const EPISODE_NAMESPACE: &str = "position-episode.v1";
const EPISODE_SCHEMA: &str = "hyperliquid-alpha-desk/position-episode/v1";
const CURRENT_NAMESPACE: &str = "position-episode-current.v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/position-episode-current/v1";
const EFFECT_NAMESPACE: &str = "position-episode-effect-fact.v1";
const EFFECT_SCHEMA: &str = "hyperliquid-alpha-desk/position-episode-effect-fact/v1";
const EPISODE_ID_CONTEXT: &str = "hyperliquid-alpha-desk/position-episode-id/v1";
const EPISODE_ID_PREFIX: &str = "pos_ep_";
const MAX_IDENTITY_BYTES: usize = 64 * 1024;
const FRAME_BYTES: usize = size_of::<u64>();
const MAX_LEG_ORDINAL: u8 = 1;
const EPISODE_RULE_VERSION: &str = "hyperliquid-alpha-desk-canonical-position-episode@1.0.0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeCompletenessV1 {
    CompleteFromFlat,
    PartialFromFirstObservation,
}

impl EpisodeCompletenessV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::CompleteFromFlat => "complete_from_flat",
            Self::PartialFromFirstObservation => "partial_from_first_observation",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "complete_from_flat" => Ok(Self::CompleteFromFlat),
            "partial_from_first_observation" => Ok(Self::PartialFromFirstObservation),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeCloseCauseV1 {
    TradeFlat,
    TradeReversal,
    LiquidationFill,
    Settlement,
    BackstopInterrupted,
}

impl EpisodeCloseCauseV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::TradeFlat => "trade_flat",
            Self::TradeReversal => "trade_reversal",
            Self::LiquidationFill => "liquidation_fill",
            Self::Settlement => "settlement",
            Self::BackstopInterrupted => "backstop_interrupted",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "trade_flat" => Ok(Self::TradeFlat),
            "trade_reversal" => Ok(Self::TradeReversal),
            "liquidation_fill" => Ok(Self::LiquidationFill),
            "settlement" => Ok(Self::Settlement),
            "backstop_interrupted" => Ok(Self::BackstopInterrupted),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeStatusV1 {
    Open,
    Closed,
    Interrupted,
}

impl EpisodeStatusV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "open" => Ok(Self::Open),
            "closed" => Ok(Self::Closed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeAttributionResolutionV1 {
    NoOpenEpisode,
    Resolved,
    Interrupted,
}

impl EpisodeAttributionResolutionV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::NoOpenEpisode => "no_open_episode",
            Self::Resolved => "resolved",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "no_open_episode" => Ok(Self::NoOpenEpisode),
            "resolved" => Ok(Self::Resolved),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeEffectKindV1 {
    Opened,
    Updated,
    Closed,
    Interrupted,
}

impl EpisodeEffectKindV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Opened => "opened",
            Self::Updated => "updated",
            Self::Closed => "closed",
            Self::Interrupted => "interrupted",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "opened" => Ok(Self::Opened),
            "updated" => Ok(Self::Updated),
            "closed" => Ok(Self::Closed),
            "interrupted" => Ok(Self::Interrupted),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

pub fn derive_position_episode_id(
    account_id: &Address,
    market_id: &MarketId,
    opening_anchor_event_id: &EventId,
    opening_leg_ordinal: u8,
) -> Result<PositionEpisodeId, PositionStateError> {
    require_ordinal(opening_leg_ordinal)?;
    let identities = [
        account_id.as_bytes().as_slice(),
        market_id.as_str().as_bytes(),
        opening_anchor_event_id.as_str().as_bytes(),
    ];
    let encoded_len = identities.iter().try_fold(1_usize, |total, identity| {
        total
            .checked_add(FRAME_BYTES)
            .and_then(|value| value.checked_add(identity.len()))
            .ok_or(PositionStateError::InvalidKey)
    })?;
    if encoded_len > MAX_IDENTITY_BYTES {
        return Err(PositionStateError::InvalidKey);
    }

    let mut hasher = blake3::Hasher::new_derive_key(EPISODE_ID_CONTEXT);
    for identity in identities {
        let length = u64::try_from(identity.len()).map_err(|_| PositionStateError::InvalidKey)?;
        hasher.update(&length.to_be_bytes());
        hasher.update(identity);
    }
    hasher.update(&[opening_leg_ordinal]);
    PositionEpisodeId::new(format!("{EPISODE_ID_PREFIX}{}", hasher.finalize().to_hex()))
        .map_err(|_| PositionStateError::InvalidRecord)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionEpisodeRecordV1 {
    episode_id: PositionEpisodeId,
    account_id: Address,
    market_id: MarketId,
    opening_anchor_event_id: EventId,
    opening_leg_ordinal: u8,
    opening_position: PositionQuantity,
    close_event_id: Option<EventId>,
    close_cause: Option<EpisodeCloseCauseV1>,
    completeness: EpisodeCompletenessV1,
    buy_quantity: Quantity,
    buy_notional: ExactQuoteNotional,
    sell_quantity: Quantity,
    sell_notional: ExactQuoteNotional,
    funding_paid: QuoteAmount,
    funding_received: QuoteAmount,
    status: EpisodeStatusV1,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl PositionEpisodeRecordV1 {
    #[allow(dead_code, reason = "used by the planned episode reducer")]
    #[allow(
        clippy::too_many_arguments,
        reason = "the episode snapshot is explicit"
    )]
    pub(super) fn try_new(
        episode_id: PositionEpisodeId,
        account_id: Address,
        market_id: MarketId,
        opening_anchor_event_id: EventId,
        opening_leg_ordinal: u8,
        opening_position: PositionQuantity,
        close_event_id: Option<EventId>,
        close_cause: Option<EpisodeCloseCauseV1>,
        completeness: EpisodeCompletenessV1,
        buy_quantity: Quantity,
        buy_notional: ExactQuoteNotional,
        sell_quantity: Quantity,
        sell_notional: ExactQuoteNotional,
        funding_paid: QuoteAmount,
        funding_received: QuoteAmount,
        status: EpisodeStatusV1,
        last_event_id: EventId,
        last_block_height: BlockHeight,
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            episode_id,
            account_id,
            market_id,
            opening_anchor_event_id,
            opening_leg_ordinal,
            opening_position,
            close_event_id,
            close_cause,
            completeness,
            buy_quantity,
            buy_notional,
            sell_quantity,
            sell_notional,
            funding_paid,
            funding_received,
            status,
            last_event_id,
            last_block_height,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(episode_id: &PositionEpisodeId) -> Result<StateKey, PositionStateError> {
        require_episode_id_shape(episode_id)?;
        state_key(EPISODE_NAMESPACE, &[episode_id.as_str().as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: PositionEpisodeWireV1 = decode_wire(bytes)?;
        if wire.schema != EPISODE_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            episode_id: PositionEpisodeId::new(wire.episode_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            opening_anchor_event_id: EventId::new(wire.opening_anchor_event_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            opening_leg_ordinal: wire.opening_leg_ordinal,
            opening_position: PositionQuantity::from_str(&wire.opening_position)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            close_event_id: wire
                .close_event_id
                .map(EventId::new)
                .transpose()
                .map_err(|_| PositionStateError::InvalidRecord)?,
            close_cause: wire
                .close_cause
                .as_deref()
                .map(EpisodeCloseCauseV1::parse)
                .transpose()?,
            completeness: EpisodeCompletenessV1::parse(&wire.completeness)?,
            buy_quantity: Quantity::from_str(&wire.buy_quantity)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            buy_notional: ExactQuoteNotional::from_str(&wire.buy_notional)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            sell_quantity: Quantity::from_str(&wire.sell_quantity)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            sell_notional: ExactQuoteNotional::from_str(&wire.sell_notional)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            funding_paid: QuoteAmount::from_str(&wire.funding_paid)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            funding_received: QuoteAmount::from_str(&wire.funding_received)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            status: EpisodeStatusV1::parse(&wire.status)?,
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
        if Self::state_key(&record.episode_id)? != *key {
            return Err(PositionStateError::KeyMismatch);
        }
        Ok(record)
    }

    #[allow(dead_code, reason = "used by the planned episode reducer")]
    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&PositionEpisodeWireV1 {
            schema: EPISODE_SCHEMA.to_owned(),
            episode_id: self.episode_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            opening_anchor_event_id: self.opening_anchor_event_id.as_str().to_owned(),
            opening_leg_ordinal: self.opening_leg_ordinal,
            opening_position: self.opening_position.to_string(),
            close_event_id: self
                .close_event_id
                .as_ref()
                .map(EventId::as_str)
                .map(str::to_owned),
            close_cause: self
                .close_cause
                .map(EpisodeCloseCauseV1::as_wire_name)
                .map(str::to_owned),
            completeness: self.completeness.as_wire_name().to_owned(),
            buy_quantity: self.buy_quantity.to_string(),
            buy_notional: self.buy_notional.to_string(),
            sell_quantity: self.sell_quantity.to_string(),
            sell_notional: self.sell_notional.to_string(),
            funding_paid: self.funding_paid.to_string(),
            funding_received: self.funding_received.to_string(),
            status: self.status.as_wire_name().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        require_ordinal(self.opening_leg_ordinal)?;
        require_episode_id_shape(&self.episode_id)?;
        let expected = derive_position_episode_id(
            &self.account_id,
            &self.market_id,
            &self.opening_anchor_event_id,
            self.opening_leg_ordinal,
        )?;
        if expected != self.episode_id {
            return Err(PositionStateError::InvalidRecord);
        }
        match self.completeness {
            EpisodeCompletenessV1::CompleteFromFlat if self.opening_position.raw() != 0 => {
                return Err(PositionStateError::InvalidRecord);
            }
            EpisodeCompletenessV1::PartialFromFirstObservation
                if self.opening_position.raw() == 0 =>
            {
                return Err(PositionStateError::InvalidRecord);
            }
            _ => {}
        }
        validate_close_matrix(self.status, self.close_event_id.as_ref(), self.close_cause)?;
        validate_amounts(
            self.buy_quantity,
            &self.buy_notional,
            self.sell_quantity,
            &self.sell_notional,
            self.funding_paid,
            self.funding_received,
        )
    }

    #[must_use]
    pub const fn episode_id(&self) -> &PositionEpisodeId {
        &self.episode_id
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
    pub const fn opening_anchor_event_id(&self) -> &EventId {
        &self.opening_anchor_event_id
    }
    #[must_use]
    pub const fn opening_leg_ordinal(&self) -> u8 {
        self.opening_leg_ordinal
    }
    #[must_use]
    pub const fn opening_position(&self) -> PositionQuantity {
        self.opening_position
    }
    #[must_use]
    pub const fn close_event_id(&self) -> Option<&EventId> {
        self.close_event_id.as_ref()
    }
    #[must_use]
    pub const fn close_cause(&self) -> Option<EpisodeCloseCauseV1> {
        self.close_cause
    }
    #[must_use]
    pub const fn completeness(&self) -> EpisodeCompletenessV1 {
        self.completeness
    }
    #[must_use]
    pub const fn buy_quantity(&self) -> Quantity {
        self.buy_quantity
    }
    #[must_use]
    pub const fn buy_notional(&self) -> &ExactQuoteNotional {
        &self.buy_notional
    }
    #[must_use]
    pub const fn sell_quantity(&self) -> Quantity {
        self.sell_quantity
    }
    #[must_use]
    pub const fn sell_notional(&self) -> &ExactQuoteNotional {
        &self.sell_notional
    }
    #[must_use]
    pub const fn funding_paid(&self) -> QuoteAmount {
        self.funding_paid
    }
    #[must_use]
    pub const fn funding_received(&self) -> QuoteAmount {
        self.funding_received
    }
    #[must_use]
    pub const fn status(&self) -> EpisodeStatusV1 {
        self.status
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
pub struct PositionEpisodeCurrentRecordV1 {
    account_id: Address,
    market_id: MarketId,
    episode_id: Option<PositionEpisodeId>,
    attribution_resolution: EpisodeAttributionResolutionV1,
    last_event_id: EventId,
    last_block_height: BlockHeight,
}

impl PositionEpisodeCurrentRecordV1 {
    #[allow(dead_code, reason = "used by the planned episode reducer")]
    pub(super) fn try_new(
        account_id: Address,
        market_id: MarketId,
        episode_id: Option<PositionEpisodeId>,
        attribution_resolution: EpisodeAttributionResolutionV1,
        last_event_id: EventId,
        last_block_height: BlockHeight,
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            account_id,
            market_id,
            episode_id,
            attribution_resolution,
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
        let wire: PositionEpisodeCurrentWireV1 = decode_wire(bytes)?;
        if wire.schema != CURRENT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            episode_id: wire
                .episode_id
                .map(PositionEpisodeId::new)
                .transpose()
                .map_err(|_| PositionStateError::InvalidRecord)?,
            attribution_resolution: EpisodeAttributionResolutionV1::parse(
                &wire.attribution_resolution,
            )?,
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

    pub fn validate_reference(&self, state: &StateView<'_>) -> Result<(), PositionStateError> {
        self.validate()?;
        match (&self.attribution_resolution, &self.episode_id) {
            (EpisodeAttributionResolutionV1::Resolved, Some(episode_id)) => {
                let key = PositionEpisodeRecordV1::state_key(episode_id)?;
                let bytes = state.get(&key).ok_or(PositionStateError::InvalidRecord)?;
                let episode = PositionEpisodeRecordV1::decode_at(&key, bytes)?;
                if episode.account_id != self.account_id
                    || episode.market_id != self.market_id
                    || episode.status != EpisodeStatusV1::Open
                {
                    return Err(PositionStateError::InvalidRecord);
                }
                Ok(())
            }
            (EpisodeAttributionResolutionV1::NoOpenEpisode, None)
            | (EpisodeAttributionResolutionV1::Interrupted, None) => Ok(()),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }

    #[allow(dead_code, reason = "used by the planned episode reducer")]
    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&PositionEpisodeCurrentWireV1 {
            schema: CURRENT_SCHEMA.to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            episode_id: self
                .episode_id
                .as_ref()
                .map(PositionEpisodeId::as_str)
                .map(str::to_owned),
            attribution_resolution: self.attribution_resolution.as_wire_name().to_owned(),
            last_event_id: self.last_event_id.as_str().to_owned(),
            last_block_height: self.last_block_height.get(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        match (&self.attribution_resolution, &self.episode_id) {
            (EpisodeAttributionResolutionV1::Resolved, Some(episode_id)) => {
                require_episode_id_shape(episode_id)
            }
            (EpisodeAttributionResolutionV1::NoOpenEpisode, None)
            | (EpisodeAttributionResolutionV1::Interrupted, None) => Ok(()),
            _ => Err(PositionStateError::InvalidRecord),
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
    pub const fn episode_id(&self) -> Option<&PositionEpisodeId> {
        self.episode_id.as_ref()
    }
    #[must_use]
    pub const fn attribution_resolution(&self) -> EpisodeAttributionResolutionV1 {
        self.attribution_resolution
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
pub struct PositionEpisodeEffectFactRecordV1 {
    event_id: EventId,
    account_id: Address,
    market_id: MarketId,
    leg_ordinal: u8,
    episode_id: PositionEpisodeId,
    effect_kind: EpisodeEffectKindV1,
    buy_quantity_delta: Quantity,
    buy_notional_delta: ExactQuoteNotional,
    sell_quantity_delta: Quantity,
    sell_notional_delta: ExactQuoteNotional,
    funding_paid_delta: QuoteAmount,
    funding_received_delta: QuoteAmount,
    close_cause: Option<EpisodeCloseCauseV1>,
    rule_version: String,
}

impl PositionEpisodeEffectFactRecordV1 {
    pub const RULE_VERSION: &'static str = EPISODE_RULE_VERSION;

    #[allow(dead_code, reason = "used by the planned episode reducer")]
    #[allow(
        clippy::too_many_arguments,
        reason = "the immutable effect fact is explicit"
    )]
    pub(super) fn try_new(
        event_id: EventId,
        account_id: Address,
        market_id: MarketId,
        leg_ordinal: u8,
        episode_id: PositionEpisodeId,
        effect_kind: EpisodeEffectKindV1,
        buy_quantity_delta: Quantity,
        buy_notional_delta: ExactQuoteNotional,
        sell_quantity_delta: Quantity,
        sell_notional_delta: ExactQuoteNotional,
        funding_paid_delta: QuoteAmount,
        funding_received_delta: QuoteAmount,
        close_cause: Option<EpisodeCloseCauseV1>,
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            event_id,
            account_id,
            market_id,
            leg_ordinal,
            episode_id,
            effect_kind,
            buy_quantity_delta,
            buy_notional_delta,
            sell_quantity_delta,
            sell_notional_delta,
            funding_paid_delta,
            funding_received_delta,
            close_cause,
            rule_version: Self::RULE_VERSION.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(
        event_id: &EventId,
        account_id: &Address,
        market_id: &MarketId,
        leg_ordinal: u8,
    ) -> Result<StateKey, PositionStateError> {
        require_ordinal(leg_ordinal)?;
        state_key(
            EFFECT_NAMESPACE,
            &[
                event_id.as_str().as_bytes(),
                account_id.as_bytes(),
                market_id.as_str().as_bytes(),
                &[leg_ordinal],
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: PositionEpisodeEffectFactWireV1 = decode_wire(bytes)?;
        if wire.schema != EFFECT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            leg_ordinal: wire.leg_ordinal,
            episode_id: PositionEpisodeId::new(wire.episode_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            effect_kind: EpisodeEffectKindV1::parse(&wire.effect_kind)?,
            buy_quantity_delta: Quantity::from_str(&wire.buy_quantity_delta)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            buy_notional_delta: ExactQuoteNotional::from_str(&wire.buy_notional_delta)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            sell_quantity_delta: Quantity::from_str(&wire.sell_quantity_delta)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            sell_notional_delta: ExactQuoteNotional::from_str(&wire.sell_notional_delta)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            funding_paid_delta: QuoteAmount::from_str(&wire.funding_paid_delta)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            funding_received_delta: QuoteAmount::from_str(&wire.funding_received_delta)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            close_cause: wire
                .close_cause
                .as_deref()
                .map(EpisodeCloseCauseV1::parse)
                .transpose()?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(
            &record.event_id,
            &record.account_id,
            &record.market_id,
            record.leg_ordinal,
        )? != *key
        {
            return Err(PositionStateError::KeyMismatch);
        }
        Ok(record)
    }

    #[allow(dead_code, reason = "used by the planned episode reducer")]
    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&PositionEpisodeEffectFactWireV1 {
            schema: EFFECT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            leg_ordinal: self.leg_ordinal,
            episode_id: self.episode_id.as_str().to_owned(),
            effect_kind: self.effect_kind.as_wire_name().to_owned(),
            buy_quantity_delta: self.buy_quantity_delta.to_string(),
            buy_notional_delta: self.buy_notional_delta.to_string(),
            sell_quantity_delta: self.sell_quantity_delta.to_string(),
            sell_notional_delta: self.sell_notional_delta.to_string(),
            funding_paid_delta: self.funding_paid_delta.to_string(),
            funding_received_delta: self.funding_received_delta.to_string(),
            close_cause: self
                .close_cause
                .map(EpisodeCloseCauseV1::as_wire_name)
                .map(str::to_owned),
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        require_ordinal(self.leg_ordinal)?;
        require_episode_id_shape(&self.episode_id)?;
        if self.rule_version != EPISODE_RULE_VERSION {
            return Err(PositionStateError::InvalidRecord);
        }
        validate_effect_close_matrix(self.effect_kind, self.close_cause)?;
        validate_amounts(
            self.buy_quantity_delta,
            &self.buy_notional_delta,
            self.sell_quantity_delta,
            &self.sell_notional_delta,
            self.funding_paid_delta,
            self.funding_received_delta,
        )
    }

    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
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
    pub const fn leg_ordinal(&self) -> u8 {
        self.leg_ordinal
    }
    #[must_use]
    pub const fn episode_id(&self) -> &PositionEpisodeId {
        &self.episode_id
    }
    #[must_use]
    pub const fn effect_kind(&self) -> EpisodeEffectKindV1 {
        self.effect_kind
    }
    #[must_use]
    pub const fn buy_quantity_delta(&self) -> Quantity {
        self.buy_quantity_delta
    }
    #[must_use]
    pub const fn buy_notional_delta(&self) -> &ExactQuoteNotional {
        &self.buy_notional_delta
    }
    #[must_use]
    pub const fn sell_quantity_delta(&self) -> Quantity {
        self.sell_quantity_delta
    }
    #[must_use]
    pub const fn sell_notional_delta(&self) -> &ExactQuoteNotional {
        &self.sell_notional_delta
    }
    #[must_use]
    pub const fn funding_paid_delta(&self) -> QuoteAmount {
        self.funding_paid_delta
    }
    #[must_use]
    pub const fn funding_received_delta(&self) -> QuoteAmount {
        self.funding_received_delta
    }
    #[must_use]
    pub const fn close_cause(&self) -> Option<EpisodeCloseCauseV1> {
        self.close_cause
    }
    #[must_use]
    pub fn rule_version(&self) -> &str {
        &self.rule_version
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionEpisodeWireV1 {
    schema: String,
    episode_id: String,
    account_id: String,
    market_id: String,
    opening_anchor_event_id: String,
    opening_leg_ordinal: u8,
    opening_position: String,
    close_event_id: Option<String>,
    close_cause: Option<String>,
    completeness: String,
    buy_quantity: String,
    buy_notional: String,
    sell_quantity: String,
    sell_notional: String,
    funding_paid: String,
    funding_received: String,
    status: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionEpisodeCurrentWireV1 {
    schema: String,
    account_id: String,
    market_id: String,
    episode_id: Option<String>,
    attribution_resolution: String,
    last_event_id: String,
    last_block_height: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionEpisodeEffectFactWireV1 {
    schema: String,
    event_id: String,
    account_id: String,
    market_id: String,
    leg_ordinal: u8,
    episode_id: String,
    effect_kind: String,
    buy_quantity_delta: String,
    buy_notional_delta: String,
    sell_quantity_delta: String,
    sell_notional_delta: String,
    funding_paid_delta: String,
    funding_received_delta: String,
    close_cause: Option<String>,
    rule_version: String,
}

fn require_ordinal(value: u8) -> Result<(), PositionStateError> {
    if value <= MAX_LEG_ORDINAL {
        Ok(())
    } else {
        Err(PositionStateError::InvalidRecord)
    }
}

fn require_episode_id_shape(value: &PositionEpisodeId) -> Result<(), PositionStateError> {
    let text = value.as_str();
    if text.len() != EPISODE_ID_PREFIX.len() + 64
        || !text.starts_with(EPISODE_ID_PREFIX)
        || !text
            .bytes()
            .skip(EPISODE_ID_PREFIX.len())
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(PositionStateError::InvalidRecord);
    }
    Ok(())
}

fn validate_close_matrix(
    status: EpisodeStatusV1,
    close_event_id: Option<&EventId>,
    close_cause: Option<EpisodeCloseCauseV1>,
) -> Result<(), PositionStateError> {
    match (status, close_event_id, close_cause) {
        (EpisodeStatusV1::Open, None, None) => Ok(()),
        (
            EpisodeStatusV1::Closed,
            Some(_),
            Some(EpisodeCloseCauseV1::TradeFlat | EpisodeCloseCauseV1::TradeReversal),
        ) => Ok(()),
        (
            EpisodeStatusV1::Interrupted,
            Some(_),
            Some(
                EpisodeCloseCauseV1::LiquidationFill
                | EpisodeCloseCauseV1::Settlement
                | EpisodeCloseCauseV1::BackstopInterrupted,
            ),
        ) => Ok(()),
        _ => Err(PositionStateError::InvalidRecord),
    }
}

fn validate_effect_close_matrix(
    effect_kind: EpisodeEffectKindV1,
    close_cause: Option<EpisodeCloseCauseV1>,
) -> Result<(), PositionStateError> {
    match (effect_kind, close_cause) {
        (EpisodeEffectKindV1::Opened | EpisodeEffectKindV1::Updated, None) => Ok(()),
        (
            EpisodeEffectKindV1::Closed,
            Some(EpisodeCloseCauseV1::TradeFlat | EpisodeCloseCauseV1::TradeReversal),
        ) => Ok(()),
        (
            EpisodeEffectKindV1::Interrupted,
            Some(
                EpisodeCloseCauseV1::LiquidationFill
                | EpisodeCloseCauseV1::Settlement
                | EpisodeCloseCauseV1::BackstopInterrupted,
            ),
        ) => Ok(()),
        _ => Err(PositionStateError::InvalidRecord),
    }
}

fn validate_amounts(
    buy_quantity: Quantity,
    buy_notional: &ExactQuoteNotional,
    sell_quantity: Quantity,
    sell_notional: &ExactQuoteNotional,
    funding_paid: QuoteAmount,
    funding_received: QuoteAmount,
) -> Result<(), PositionStateError> {
    let buy_notional_text = buy_notional.to_string();
    let sell_notional_text = sell_notional.to_string();
    if buy_quantity.raw() < 0
        || sell_quantity.raw() < 0
        || funding_paid.raw() < 0
        || funding_received.raw() < 0
        || buy_notional_text.starts_with('-')
        || sell_notional_text.starts_with('-')
        || (buy_quantity.raw() == 0) != (buy_notional_text == "0")
        || (sell_quantity.raw() == 0) != (sell_notional_text == "0")
    {
        return Err(PositionStateError::InvalidRecord);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::state::view_entries;

    const ACCOUNT: Address = Address::from_bytes([0x11; 20]);
    const OTHER_ACCOUNT: Address = Address::from_bytes([0x22; 20]);

    fn market() -> MarketId {
        MarketId::new("perp:BTC").unwrap()
    }

    fn event(value: &str) -> EventId {
        EventId::new(value).unwrap()
    }

    fn zero_notional() -> ExactQuoteNotional {
        ExactQuoteNotional::from_str("0").unwrap()
    }

    fn open_episode(
        account_id: Address,
        market_id: MarketId,
        status: EpisodeStatusV1,
    ) -> PositionEpisodeRecordV1 {
        let opening = event("evt-open");
        let id = derive_position_episode_id(&account_id, &market_id, &opening, 0).unwrap();
        let (close_event_id, close_cause, last_event_id) = match status {
            EpisodeStatusV1::Open => (None, None, opening.clone()),
            EpisodeStatusV1::Closed => (
                Some(event("evt-close")),
                Some(EpisodeCloseCauseV1::TradeFlat),
                event("evt-close"),
            ),
            EpisodeStatusV1::Interrupted => (
                Some(event("evt-interrupt")),
                Some(EpisodeCloseCauseV1::Settlement),
                event("evt-interrupt"),
            ),
        };
        PositionEpisodeRecordV1::try_new(
            id,
            account_id,
            market_id,
            opening,
            0,
            PositionQuantity::from_raw(0, 8).unwrap(),
            close_event_id,
            close_cause,
            EpisodeCompletenessV1::CompleteFromFlat,
            Quantity::from_raw(1, 8).unwrap(),
            ExactQuoteNotional::from_str("1").unwrap(),
            Quantity::from_raw(0, 8).unwrap(),
            zero_notional(),
            QuoteAmount::from_raw(0, 6).unwrap(),
            QuoteAmount::from_raw(0, 6).unwrap(),
            status,
            last_event_id,
            BlockHeight::new(7),
        )
        .unwrap()
    }

    fn resolved_current(episode: &PositionEpisodeRecordV1) -> PositionEpisodeCurrentRecordV1 {
        PositionEpisodeCurrentRecordV1::try_new(
            episode.account_id(),
            episode.market_id().clone(),
            Some(episode.episode_id().clone()),
            EpisodeAttributionResolutionV1::Resolved,
            event("evt-current"),
            BlockHeight::new(8),
        )
        .unwrap()
    }

    #[test]
    fn state_aware_reference_validation_rejects_missing_corrupt_and_non_open_targets() {
        let episode = open_episode(ACCOUNT, market(), EpisodeStatusV1::Open);
        let current = resolved_current(&episode);
        let key = PositionEpisodeRecordV1::state_key(episode.episode_id()).unwrap();

        let mut entries = BTreeMap::new();
        entries.insert(key.clone(), episode.encode().unwrap());
        assert_eq!(current.validate_reference(&view_entries(&entries)), Ok(()));

        assert_eq!(
            current.validate_reference(&view_entries(&BTreeMap::new())),
            Err(PositionStateError::InvalidRecord)
        );

        entries.insert(key.clone(), b"corrupt".to_vec());
        assert!(current.validate_reference(&view_entries(&entries)).is_err());

        for status in [EpisodeStatusV1::Closed, EpisodeStatusV1::Interrupted] {
            let terminal = open_episode(ACCOUNT, market(), status);
            let terminal_current = resolved_current(&terminal);
            let terminal_key = PositionEpisodeRecordV1::state_key(terminal.episode_id()).unwrap();
            let terminal_entries = BTreeMap::from([(terminal_key, terminal.encode().unwrap())]);
            assert_eq!(
                terminal_current.validate_reference(&view_entries(&terminal_entries)),
                Err(PositionStateError::InvalidRecord)
            );
        }
    }

    #[test]
    fn state_aware_reference_validation_rejects_key_or_identity_mismatch() {
        let expected = open_episode(ACCOUNT, market(), EpisodeStatusV1::Open);
        let current = resolved_current(&expected);
        let expected_key = PositionEpisodeRecordV1::state_key(expected.episode_id()).unwrap();

        for other in [
            open_episode(OTHER_ACCOUNT, market(), EpisodeStatusV1::Open),
            open_episode(
                ACCOUNT,
                MarketId::new("perp:ETH").unwrap(),
                EpisodeStatusV1::Open,
            ),
        ] {
            let entries = BTreeMap::from([(expected_key.clone(), other.encode().unwrap())]);
            assert!(current.validate_reference(&view_entries(&entries)).is_err());
        }
    }

    #[test]
    fn non_resolved_current_records_require_no_episode_reference() {
        for resolution in [
            EpisodeAttributionResolutionV1::NoOpenEpisode,
            EpisodeAttributionResolutionV1::Interrupted,
        ] {
            let current = PositionEpisodeCurrentRecordV1::try_new(
                ACCOUNT,
                market(),
                None,
                resolution,
                event("evt-current"),
                BlockHeight::new(8),
            )
            .unwrap();
            assert_eq!(
                current.validate_reference(&view_entries(&BTreeMap::new())),
                Ok(())
            );
        }
    }
}
