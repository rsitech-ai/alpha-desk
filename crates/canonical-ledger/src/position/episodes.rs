use std::collections::BTreeSet;
use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload, TradeParticipantRoleV1};
use domain_types::{
    Address, BlockHeight, EventId, ExactQuoteNotional, MarketId, PositionEpisodeId,
    PositionQuantity, Quantity, QuoteAmount, RoundingMode,
};
use serde::{Deserialize, Serialize};

use crate::{
    ApplyContext, BlockDeltaView, EventReducer, ReducerError, StateKey, StateMutation, StateView,
};

use super::codec::{
    PositionStateError, decode_account_market_key, decode_wire, encode_wire, require_record_bytes,
    state_key,
};
use super::quantity::{
    NormalizedTradeLeg, PositionQuantityCurrentRecordV1, TradeValidationError, ValidatedTrade,
    ValidatedTradeLeg, finish_prepared_enriched_trade, normalize_prerequisite_trade_leg,
    prepare_enriched_trade, validate_enriched_trade_prerequisites, validate_exact_market,
};

const EPISODE_NAMESPACE: &str = "position-episode.v1";
const EPISODE_SCHEMA: &str = "hyperliquid-alpha-desk/position-episode/v1";
const QUANTITY_CURRENT_NAMESPACE: &str = "position-quantity-current.v1";
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalPositionEpisodeReducerV1;

impl CanonicalPositionEpisodeReducerV1 {
    pub const VERSION: &'static str = EPISODE_RULE_VERSION;
}

impl EventReducer for CanonicalPositionEpisodeReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        if event.schema_version() != "1.0.0" {
            return false;
        }
        match event.payload() {
            EventPayload::TradeMatched(trade) => trade.participants.is_some(),
            EventPayload::FundingPaid(_) | EventPayload::FundingReceived(_) => true,
            _ => false,
        }
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if !self.supports(event) {
            return Err(episode_error(
                "position_episode.unsupported_event",
                "position episode reducer received an unsupported event",
            ));
        }
        match event.payload() {
            EventPayload::TradeMatched(_) => reduce_trade(state, event),
            EventPayload::FundingPaid(funding) => reduce_funding(
                state,
                event,
                funding.account_id,
                &funding.market_id,
                funding.amount,
                true,
            ),
            EventPayload::FundingReceived(funding) => reduce_funding(
                state,
                event,
                funding.account_id,
                &funding.market_id,
                funding.amount,
                false,
            ),
            _ => Err(episode_error(
                "position_episode.unsupported_event",
                "position episode reducer received an unsupported event",
            )),
        }
    }

    fn validate_block_delta(
        &self,
        final_state: &StateView<'_>,
        delta: &BlockDeltaView<'_>,
        _context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        validate_episode_block_delta(final_state, delta)
    }
}

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

    /// Returns the trade-only signed notional delta for a fully observed
    /// episode. This is analytical evidence, not source-provided realized PnL.
    pub fn observed_signed_trade_notional_delta(
        &self,
    ) -> Result<Option<ExactQuoteNotional>, PositionStateError> {
        if self.completeness != EpisodeCompletenessV1::CompleteFromFlat
            || self.status != EpisodeStatusV1::Closed
            || !matches!(
                self.close_cause,
                Some(EpisodeCloseCauseV1::TradeFlat | EpisodeCloseCauseV1::TradeReversal)
            )
        {
            return Ok(None);
        }
        self.sell_notional
            .checked_sub(&self.buy_notional)
            .map(Some)
            .map_err(|_| PositionStateError::InvalidRecord)
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

#[derive(Debug, Clone)]
enum LoadedEpisodePair {
    Absent,
    NoOpenEpisode,
    Resolved {
        episode: Box<PositionEpisodeRecordV1>,
        known_quantity: PositionQuantity,
    },
    Interrupted,
}

fn reduce_trade(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
) -> Result<Vec<StateMutation>, ReducerError> {
    let prerequisites = validate_enriched_trade_prerequisites(state, event)
        .map_err(map_trade_validation_for_episode)?;
    let loaded = [
        load_episode_pair(
            state,
            &prerequisites.legs[0].account_id,
            &prerequisites.market_id,
        )?,
        load_episode_pair(
            state,
            &prerequisites.legs[1].account_id,
            &prerequisites.market_id,
        )?,
    ];
    for (index, loaded_pair) in loaded.iter().enumerate() {
        let leg = normalize_prerequisite_trade_leg(&prerequisites, index)
            .map_err(map_trade_validation_for_episode)?;
        validate_loaded_trade_start(loaded_pair, leg, prerequisites.quantity_scale)?;
    }
    let prepared =
        prepare_enriched_trade(prerequisites).map_err(map_trade_validation_for_episode)?;
    let validated =
        finish_prepared_enriched_trade(prepared).map_err(map_trade_validation_for_episode)?;
    let mut mutations = Vec::new();
    for (leg, loaded) in validated.legs.into_iter().zip(loaded) {
        mutations.extend(stage_trade_leg(state, event, &validated, leg, loaded)?);
    }
    ensure_unique_episode_mutation_keys(&mutations)?;
    Ok(mutations)
}

fn validate_loaded_trade_start(
    loaded: &LoadedEpisodePair,
    leg: NormalizedTradeLeg,
    quantity_scale: u8,
) -> Result<(), ReducerError> {
    match loaded {
        LoadedEpisodePair::NoOpenEpisode if leg.start.raw() != 0 => {
            return Err(start_position_mismatch());
        }
        LoadedEpisodePair::Resolved { known_quantity, .. } => {
            let known = known_quantity
                .checked_rescale_up(quantity_scale)
                .map_err(|_| quantity_arithmetic())?;
            if known != leg.start {
                return Err(start_position_mismatch());
            }
        }
        LoadedEpisodePair::Absent
        | LoadedEpisodePair::NoOpenEpisode
        | LoadedEpisodePair::Interrupted => {}
    }
    Ok(())
}

fn stage_trade_leg(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    trade: &ValidatedTrade,
    leg: ValidatedTradeLeg,
    loaded: LoadedEpisodePair,
) -> Result<Vec<StateMutation>, ReducerError> {
    let (mut episode, is_new) = match loaded {
        LoadedEpisodePair::Absent => (
            new_trade_episode(event, trade, leg, completeness_for_start(leg.start), 0)?,
            true,
        ),
        LoadedEpisodePair::NoOpenEpisode => {
            if leg.start.raw() != 0 {
                return Err(start_position_mismatch());
            }
            (
                new_trade_episode(
                    event,
                    trade,
                    leg,
                    EpisodeCompletenessV1::CompleteFromFlat,
                    0,
                )?,
                true,
            )
        }
        LoadedEpisodePair::Interrupted => (
            new_trade_episode(event, trade, leg, completeness_for_start(leg.start), 0)?,
            true,
        ),
        LoadedEpisodePair::Resolved {
            episode: existing,
            known_quantity,
        } => {
            let known = known_quantity
                .checked_rescale_up(trade.quantity_scale)
                .map_err(|_| quantity_arithmetic())?;
            if known != leg.start {
                return Err(start_position_mismatch());
            }
            (*existing, false)
        }
    };

    let start_sign = leg.start.raw().signum();
    let result_sign = leg.result.raw().signum();
    let reversal = start_sign != 0 && result_sign != 0 && start_sign != result_sign;

    let mut effects = Vec::with_capacity(if reversal { 2 } else { 1 });
    let mut episodes = Vec::with_capacity(if reversal { 2 } else { 1 });
    let current = if reversal {
        let magnitude = checked_position_magnitude(leg.start)?;
        let closed_raw = magnitude.min(trade.fill.raw());
        let residual_raw = trade
            .fill
            .raw()
            .checked_sub(closed_raw)
            .ok_or_else(quantity_arithmetic)?;
        if closed_raw <= 0 || residual_raw <= 0 {
            return Err(quantity_arithmetic());
        }
        let closed_quantity = Quantity::from_raw(closed_raw, trade.quantity_scale)
            .map_err(|_| quantity_arithmetic())?;
        let residual_quantity = Quantity::from_raw(residual_raw, trade.quantity_scale)
            .map_err(|_| quantity_arithmetic())?;
        let closed_notional = ExactQuoteNotional::checked_product(trade.price, closed_quantity)
            .map_err(|_| notional_arithmetic())?;
        let residual_notional = ExactQuoteNotional::checked_product(trade.price, residual_quantity)
            .map_err(|_| notional_arithmetic())?;
        if closed_quantity
            .checked_add(residual_quantity)
            .map_err(|_| quantity_arithmetic())?
            != trade.fill
            || closed_notional
                .checked_add(&residual_notional)
                .map_err(|_| notional_arithmetic())?
                != trade.full_notional
        {
            return Err(notional_arithmetic());
        }

        apply_trade_delta(
            &mut episode,
            leg.role,
            closed_quantity,
            &closed_notional,
            event,
            Some(EpisodeCloseCauseV1::TradeReversal),
        )?;
        let old_effect = trade_effect(
            event,
            &leg,
            &trade.market_id,
            0,
            episode.episode_id.clone(),
            EpisodeEffectKindV1::Closed,
            closed_quantity,
            closed_notional,
            Some(EpisodeCloseCauseV1::TradeReversal),
        )?;
        effects.push((0, old_effect));
        episodes.push((is_new, episode));

        let mut residual_episode = new_trade_episode(
            event,
            trade,
            ValidatedTradeLeg {
                start: PositionQuantity::from_raw(0, trade.quantity_scale)
                    .map_err(|_| quantity_arithmetic())?,
                ..leg
            },
            EpisodeCompletenessV1::CompleteFromFlat,
            1,
        )?;
        apply_trade_delta(
            &mut residual_episode,
            leg.role,
            residual_quantity,
            &residual_notional,
            event,
            None,
        )?;
        let residual_effect = trade_effect(
            event,
            &leg,
            &trade.market_id,
            1,
            residual_episode.episode_id.clone(),
            EpisodeEffectKindV1::Opened,
            residual_quantity,
            residual_notional,
            None,
        )?;
        effects.push((1, residual_effect));
        let residual_id = residual_episode.episode_id.clone();
        episodes.push((true, residual_episode));
        PositionEpisodeCurrentRecordV1::try_new(
            leg.account_id,
            trade.market_id.clone(),
            Some(residual_id),
            EpisodeAttributionResolutionV1::Resolved,
            event.event_id().clone(),
            event.block_height(),
        )
        .map_err(|_| current_pair_mismatch())?
    } else {
        let close = (leg.result.raw() == 0).then_some(EpisodeCloseCauseV1::TradeFlat);
        apply_trade_delta(
            &mut episode,
            leg.role,
            trade.fill,
            &trade.full_notional,
            event,
            close,
        )?;
        let kind = if close.is_some() {
            EpisodeEffectKindV1::Closed
        } else if is_new {
            EpisodeEffectKindV1::Opened
        } else {
            EpisodeEffectKindV1::Updated
        };
        let effect = trade_effect(
            event,
            &leg,
            &trade.market_id,
            0,
            episode.episode_id.clone(),
            kind,
            trade.fill,
            trade.full_notional.clone(),
            close,
        )?;
        effects.push((0, effect));
        let open_id = (close.is_none()).then(|| episode.episode_id.clone());
        episodes.push((is_new, episode));
        PositionEpisodeCurrentRecordV1::try_new(
            leg.account_id,
            trade.market_id.clone(),
            open_id,
            if close.is_some() {
                EpisodeAttributionResolutionV1::NoOpenEpisode
            } else {
                EpisodeAttributionResolutionV1::Resolved
            },
            event.event_id().clone(),
            event.block_height(),
        )
        .map_err(|_| current_pair_mismatch())?
    };
    validate_proposed_pair(
        Some(leg.result),
        &current,
        episodes.iter().map(|(_, record)| record),
    )?;

    let mut mutations = Vec::with_capacity(effects.len() + episodes.len() + 1);
    for (ordinal, effect) in effects {
        let key = PositionEpisodeEffectFactRecordV1::state_key(
            event.event_id(),
            &leg.account_id,
            &trade.market_id,
            ordinal,
        )
        .map_err(|_| effect_prior_invalid())?;
        reject_prior_episode_effect(state, &key)?;
        mutations.push(StateMutation::put(
            key,
            effect.encode().map_err(|_| effect_prior_invalid())?,
        ));
    }
    for (new_identity, record) in episodes {
        let key = PositionEpisodeRecordV1::state_key(record.episode_id())
            .map_err(|_| episode_prior_invalid())?;
        if new_identity {
            reject_prior_episode(state, &key)?;
        }
        mutations.push(StateMutation::put(
            key,
            record.encode().map_err(|_| episode_prior_invalid())?,
        ));
    }
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&leg.account_id, &trade.market_id)
        .map_err(|_| episode_current_invalid())?;
    mutations.push(StateMutation::put(
        current_key,
        current.encode().map_err(|_| episode_current_invalid())?,
    ));
    Ok(mutations)
}

fn new_trade_episode(
    event: &CanonicalEventEnvelope,
    trade: &ValidatedTrade,
    leg: ValidatedTradeLeg,
    completeness: EpisodeCompletenessV1,
    opening_leg_ordinal: u8,
) -> Result<PositionEpisodeRecordV1, ReducerError> {
    let episode_id = derive_position_episode_id(
        &leg.account_id,
        &trade.market_id,
        event.event_id(),
        opening_leg_ordinal,
    )
    .map_err(|_| episode_prior_invalid())?;
    PositionEpisodeRecordV1::try_new(
        episode_id,
        leg.account_id,
        trade.market_id.clone(),
        event.event_id().clone(),
        opening_leg_ordinal,
        leg.start,
        None,
        None,
        completeness,
        zero_quantity(trade.quantity_scale)?,
        zero_notional(),
        zero_quantity(trade.quantity_scale)?,
        zero_notional(),
        zero_quote(0)?,
        zero_quote(0)?,
        EpisodeStatusV1::Open,
        event.event_id().clone(),
        event.block_height(),
    )
    .map_err(|_| episode_prior_invalid())
}

fn apply_trade_delta(
    episode: &mut PositionEpisodeRecordV1,
    role: TradeParticipantRoleV1,
    quantity: Quantity,
    notional: &ExactQuoteNotional,
    event: &CanonicalEventEnvelope,
    close_cause: Option<EpisodeCloseCauseV1>,
) -> Result<(), ReducerError> {
    match role {
        TradeParticipantRoleV1::Buyer => {
            episode.buy_quantity = add_quantities(episode.buy_quantity, quantity)?;
            episode.buy_notional = episode
                .buy_notional
                .checked_add(notional)
                .map_err(|_| notional_arithmetic())?;
        }
        TradeParticipantRoleV1::Seller => {
            episode.sell_quantity = add_quantities(episode.sell_quantity, quantity)?;
            episode.sell_notional = episode
                .sell_notional
                .checked_add(notional)
                .map_err(|_| notional_arithmetic())?;
        }
    }
    episode.close_event_id = close_cause.map(|_| event.event_id().clone());
    episode.close_cause = close_cause;
    episode.status = if close_cause.is_some() {
        EpisodeStatusV1::Closed
    } else {
        EpisodeStatusV1::Open
    };
    episode.last_event_id = event.event_id().clone();
    episode.last_block_height = event.block_height();
    episode.validate().map_err(|_| quantity_arithmetic())
}

#[allow(clippy::too_many_arguments, reason = "effect facts are explicit")]
fn trade_effect(
    event: &CanonicalEventEnvelope,
    leg: &ValidatedTradeLeg,
    market_id: &MarketId,
    ordinal: u8,
    episode_id: PositionEpisodeId,
    kind: EpisodeEffectKindV1,
    quantity: Quantity,
    notional: ExactQuoteNotional,
    close_cause: Option<EpisodeCloseCauseV1>,
) -> Result<PositionEpisodeEffectFactRecordV1, ReducerError> {
    let zero_quantity = zero_quantity(quantity.scale())?;
    let (buy_quantity, buy_notional, sell_quantity, sell_notional) = match leg.role {
        TradeParticipantRoleV1::Buyer => (quantity, notional, zero_quantity, zero_notional()),
        TradeParticipantRoleV1::Seller => (zero_quantity, zero_notional(), quantity, notional),
    };
    PositionEpisodeEffectFactRecordV1::try_new(
        event.event_id().clone(),
        leg.account_id,
        market_id.clone(),
        ordinal,
        episode_id,
        kind,
        buy_quantity,
        buy_notional,
        sell_quantity,
        sell_notional,
        zero_quote(0)?,
        zero_quote(0)?,
        close_cause,
    )
    .map_err(|_| effect_prior_invalid())
}

fn reduce_funding(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    account_id: Address,
    market_id: &MarketId,
    amount: QuoteAmount,
    paid: bool,
) -> Result<Vec<StateMutation>, ReducerError> {
    if event.event_kind()
        != if paid {
            EventKind::FundingPaid
        } else {
            EventKind::FundingReceived
        }
        || event.market_ids() != std::slice::from_ref(market_id)
        || event.account_addresses() != [account_id]
        || amount.raw() <= 0
    {
        return Err(identity_mismatch());
    }
    validate_exact_market(state, market_id).map_err(map_funding_market_validation)?;
    let LoadedEpisodePair::Resolved {
        episode,
        known_quantity,
    } = load_episode_pair(state, &account_id, market_id)?
    else {
        return Ok(Vec::new());
    };
    let mut episode = *episode;
    let (paid_total, received_total, incoming) =
        align_funding_amounts(episode.funding_paid, episode.funding_received, amount)?;
    let zero_delta = zero_quote(incoming.scale())?;
    let (funding_paid_delta, funding_received_delta) = if paid {
        episode.funding_paid = paid_total
            .checked_add(incoming)
            .map_err(|_| funding_arithmetic())?;
        episode.funding_received = received_total;
        (incoming, zero_delta)
    } else {
        episode.funding_paid = paid_total;
        episode.funding_received = received_total
            .checked_add(incoming)
            .map_err(|_| funding_arithmetic())?;
        (zero_delta, incoming)
    };
    episode.last_event_id = event.event_id().clone();
    episode.last_block_height = event.block_height();
    episode.validate().map_err(|_| funding_arithmetic())?;

    let zero_quantity = zero_quantity(
        episode
            .buy_quantity
            .scale()
            .max(episode.sell_quantity.scale()),
    )?;
    let effect = PositionEpisodeEffectFactRecordV1::try_new(
        event.event_id().clone(),
        account_id,
        market_id.clone(),
        0,
        episode.episode_id.clone(),
        EpisodeEffectKindV1::Updated,
        zero_quantity,
        zero_notional(),
        zero_quantity,
        zero_notional(),
        funding_paid_delta,
        funding_received_delta,
        None,
    )
    .map_err(|_| effect_prior_invalid())?;
    let effect_key =
        PositionEpisodeEffectFactRecordV1::state_key(event.event_id(), &account_id, market_id, 0)
            .map_err(|_| effect_prior_invalid())?;
    reject_prior_episode_effect(state, &effect_key)?;
    let episode_key = PositionEpisodeRecordV1::state_key(episode.episode_id())
        .map_err(|_| episode_prior_invalid())?;
    let current = PositionEpisodeCurrentRecordV1::try_new(
        account_id,
        market_id.clone(),
        Some(episode.episode_id.clone()),
        EpisodeAttributionResolutionV1::Resolved,
        event.event_id().clone(),
        event.block_height(),
    )
    .map_err(|_| episode_current_invalid())?;
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&account_id, market_id)
        .map_err(|_| episode_current_invalid())?;
    validate_proposed_pair(Some(known_quantity), &current, std::iter::once(&episode))?;
    let mutations = vec![
        StateMutation::put(
            effect_key,
            effect.encode().map_err(|_| effect_prior_invalid())?,
        ),
        StateMutation::put(
            episode_key,
            episode.encode().map_err(|_| episode_prior_invalid())?,
        ),
        StateMutation::put(
            current_key,
            current.encode().map_err(|_| episode_current_invalid())?,
        ),
    ];
    ensure_unique_episode_mutation_keys(&mutations)?;
    Ok(mutations)
}

fn load_episode_pair(
    state: &StateView<'_>,
    account_id: &Address,
    market_id: &MarketId,
) -> Result<LoadedEpisodePair, ReducerError> {
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(account_id, market_id)
        .map_err(|_| quantity_current_invalid())?;
    let episode_key = PositionEpisodeCurrentRecordV1::state_key(account_id, market_id)
        .map_err(|_| episode_current_invalid())?;
    let quantity = state
        .get(&quantity_key)
        .map(|bytes| {
            PositionQuantityCurrentRecordV1::decode_at(&quantity_key, bytes)
                .map_err(|_| quantity_current_invalid())
        })
        .transpose()?;
    let current = state
        .get(&episode_key)
        .map(|bytes| {
            PositionEpisodeCurrentRecordV1::decode_at(&episode_key, bytes)
                .map_err(|_| episode_current_invalid())
        })
        .transpose()?;
    match (quantity, current) {
        (None, None) => Ok(LoadedEpisodePair::Absent),
        (Some(quantity), Some(current)) => match (
            quantity.known_quantity(),
            current.attribution_resolution(),
            current.episode_id(),
        ) {
            (Some(value), EpisodeAttributionResolutionV1::NoOpenEpisode, None)
                if value.raw() == 0 =>
            {
                Ok(LoadedEpisodePair::NoOpenEpisode)
            }
            (None, EpisodeAttributionResolutionV1::Interrupted, None) => {
                Ok(LoadedEpisodePair::Interrupted)
            }
            (Some(value), EpisodeAttributionResolutionV1::Resolved, Some(target))
                if value.raw() != 0 =>
            {
                let target_key = PositionEpisodeRecordV1::state_key(target)
                    .map_err(|_| episode_reference_invalid())?;
                let bytes = state
                    .get(&target_key)
                    .ok_or_else(episode_reference_invalid)?;
                let episode = PositionEpisodeRecordV1::decode_at(&target_key, bytes)
                    .map_err(|_| episode_reference_invalid())?;
                if episode.account_id != *account_id
                    || episode.market_id != *market_id
                    || episode.status != EpisodeStatusV1::Open
                {
                    return Err(episode_reference_invalid());
                }
                Ok(LoadedEpisodePair::Resolved {
                    episode: Box::new(episode),
                    known_quantity: value,
                })
            }
            _ => Err(current_pair_mismatch()),
        },
        _ => Err(current_pair_mismatch()),
    }
}

fn validate_episode_block_delta(
    final_state: &StateView<'_>,
    delta: &BlockDeltaView<'_>,
) -> Result<(), ReducerError> {
    let mut touched = std::collections::BTreeMap::new();

    for entry in delta {
        if entry.key().namespace() != QUANTITY_CURRENT_NAMESPACE {
            continue;
        }
        let (account, market) =
            decode_account_market_key(entry.key()).map_err(|_| quantity_current_invalid())?;
        for value in [entry.block_start_value(), entry.block_final_value()]
            .into_iter()
            .flatten()
        {
            PositionQuantityCurrentRecordV1::decode_at(entry.key(), value)
                .map_err(|_| quantity_current_invalid())?;
        }
        insert_touched_pair(&mut touched, account, market)?;
    }

    for entry in delta {
        if entry.key().namespace() != CURRENT_NAMESPACE {
            continue;
        }
        let (account, market) =
            decode_account_market_key(entry.key()).map_err(|_| episode_current_invalid())?;
        for value in [entry.block_start_value(), entry.block_final_value()]
            .into_iter()
            .flatten()
        {
            PositionEpisodeCurrentRecordV1::decode_at(entry.key(), value)
                .map_err(|_| episode_current_invalid())?;
        }
        insert_touched_pair(&mut touched, account, market)?;
    }

    for entry in delta {
        if entry.key().namespace() != EPISODE_NAMESPACE {
            continue;
        }
        let mut identified = false;
        for value in [entry.block_start_value(), entry.block_final_value()]
            .into_iter()
            .flatten()
        {
            let episode = PositionEpisodeRecordV1::decode_at(entry.key(), value)
                .map_err(|_| episode_prior_invalid())?;
            insert_touched_pair(
                &mut touched,
                episode.account_id(),
                episode.market_id().clone(),
            )?;
            identified = true;
        }
        if !identified {
            return Err(episode_prior_invalid());
        }
    }

    for (_, (account, market)) in touched {
        validate_final_episode_pair(final_state, account, &market)?;
    }
    Ok(())
}

fn insert_touched_pair(
    touched: &mut std::collections::BTreeMap<StateKey, (Address, MarketId)>,
    account: Address,
    market: MarketId,
) -> Result<(), ReducerError> {
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&account, &market)
        .map_err(|_| quantity_current_invalid())?;
    touched.entry(quantity_key).or_insert((account, market));
    Ok(())
}

fn validate_final_episode_pair(
    final_state: &StateView<'_>,
    account: Address,
    market: &MarketId,
) -> Result<(), ReducerError> {
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&account, market)
        .map_err(|_| quantity_current_invalid())?;
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&account, market)
        .map_err(|_| episode_current_invalid())?;
    let quantity = final_state
        .get(&quantity_key)
        .map(|bytes| {
            PositionQuantityCurrentRecordV1::decode_at(&quantity_key, bytes)
                .map_err(|_| quantity_current_invalid())
        })
        .transpose()?;
    let current = final_state
        .get(&current_key)
        .map(|bytes| {
            PositionEpisodeCurrentRecordV1::decode_at(&current_key, bytes)
                .map_err(|_| episode_current_invalid())
        })
        .transpose()?;

    if let Some(current) = current.as_ref() {
        current
            .validate_reference(final_state)
            .map_err(|_| episode_reference_invalid())?;
    }

    match (quantity, current) {
        (None, None) => Ok(()),
        (Some(quantity), Some(current)) => match (
            quantity.known_quantity(),
            current.attribution_resolution(),
            current.episode_id(),
        ) {
            (Some(value), EpisodeAttributionResolutionV1::NoOpenEpisode, None)
                if value.raw() == 0 =>
            {
                Ok(())
            }
            (Some(value), EpisodeAttributionResolutionV1::Resolved, Some(_))
                if value.raw() != 0 =>
            {
                Ok(())
            }
            (None, EpisodeAttributionResolutionV1::Interrupted, None) => Ok(()),
            _ => Err(current_pair_mismatch()),
        },
        _ => Err(current_pair_mismatch()),
    }
}

#[derive(Debug, Clone)]
pub(super) struct LoadedNonTradePair {
    first_anchor_event_id: Option<EventId>,
    state: LoadedNonTradePairState,
}

#[derive(Debug, Clone)]
enum LoadedNonTradePairState {
    Absent,
    NoOpenEpisode,
    Interrupted,
    Resolved {
        episode: Box<PositionEpisodeRecordV1>,
        known_quantity: PositionQuantity,
    },
}

impl LoadedNonTradePair {
    pub(super) fn known_quantity(&self) -> Option<PositionQuantity> {
        match &self.state {
            LoadedNonTradePairState::Resolved { known_quantity, .. } => Some(*known_quantity),
            LoadedNonTradePairState::NoOpenEpisode => {
                Some(PositionQuantity::from_raw(0, 0).expect("canonical zero"))
            }
            LoadedNonTradePairState::Absent | LoadedNonTradePairState::Interrupted => None,
        }
    }

    pub(super) fn is_known_zero(&self) -> bool {
        matches!(self.state, LoadedNonTradePairState::NoOpenEpisode)
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) enum NonTradeQuantityResult {
    Exact(PositionQuantity),
    Ambiguous,
}

pub(super) fn load_nontrade_pair(
    state: &StateView<'_>,
    account_id: &Address,
    market_id: &MarketId,
) -> Result<LoadedNonTradePair, ReducerError> {
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(account_id, market_id)
        .map_err(|_| liquidation_quantity_current_invalid())?;
    let episode_key = PositionEpisodeCurrentRecordV1::state_key(account_id, market_id)
        .map_err(|_| liquidation_episode_current_invalid())?;
    let quantity = state
        .get(&quantity_key)
        .map(|bytes| {
            PositionQuantityCurrentRecordV1::decode_at(&quantity_key, bytes)
                .map_err(|_| liquidation_quantity_current_invalid())
        })
        .transpose()?;
    let current = state
        .get(&episode_key)
        .map(|bytes| {
            PositionEpisodeCurrentRecordV1::decode_at(&episode_key, bytes)
                .map_err(|_| liquidation_episode_current_invalid())
        })
        .transpose()?;
    match (quantity, current) {
        (None, None) => Ok(LoadedNonTradePair {
            first_anchor_event_id: None,
            state: LoadedNonTradePairState::Absent,
        }),
        (Some(quantity), Some(current)) => {
            let first_anchor_event_id = quantity.first_anchor_event_id().cloned();
            let pair = match (
                quantity.known_quantity(),
                current.attribution_resolution(),
                current.episode_id(),
            ) {
                (Some(value), EpisodeAttributionResolutionV1::NoOpenEpisode, None)
                    if value.raw() == 0 =>
                {
                    LoadedNonTradePairState::NoOpenEpisode
                }
                (None, EpisodeAttributionResolutionV1::Interrupted, None) => {
                    LoadedNonTradePairState::Interrupted
                }
                (Some(value), EpisodeAttributionResolutionV1::Resolved, Some(target))
                    if value.raw() != 0 =>
                {
                    let target_key = PositionEpisodeRecordV1::state_key(target)
                        .map_err(|_| liquidation_episode_reference_invalid())?;
                    let bytes = state
                        .get(&target_key)
                        .ok_or_else(liquidation_episode_reference_invalid)?;
                    let episode = PositionEpisodeRecordV1::decode_at(&target_key, bytes)
                        .map_err(|_| liquidation_episode_reference_invalid())?;
                    if episode.account_id != *account_id
                        || episode.market_id != *market_id
                        || episode.status != EpisodeStatusV1::Open
                    {
                        return Err(liquidation_episode_reference_invalid());
                    }
                    LoadedNonTradePairState::Resolved {
                        episode: Box::new(episode),
                        known_quantity: value,
                    }
                }
                _ => return Err(liquidation_current_pair_mismatch()),
            };
            Ok(LoadedNonTradePair {
                first_anchor_event_id,
                state: pair,
            })
        }
        _ => Err(liquidation_current_pair_mismatch()),
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn stage_nontrade_transition(
    event: &CanonicalEventEnvelope,
    account_id: Address,
    market_id: &MarketId,
    loaded: LoadedNonTradePair,
    result: NonTradeQuantityResult,
    cause: EpisodeCloseCauseV1,
    zero_quantity_scale: u8,
) -> Result<Vec<StateMutation>, ReducerError> {
    let zero_quantity = Quantity::from_raw(0, zero_quantity_scale)
        .map_err(|_| liquidation_quantity_arithmetic())?;
    let zero_notional =
        ExactQuoteNotional::from_str("0").map_err(|_| liquidation_quantity_arithmetic())?;
    let zero_funding =
        QuoteAmount::from_raw(0, 0).map_err(|_| liquidation_quantity_arithmetic())?;

    let known_quantity = match result {
        NonTradeQuantityResult::Exact(value) => Some(value),
        NonTradeQuantityResult::Ambiguous => None,
    };
    let quantity = PositionQuantityCurrentRecordV1::try_new(
        account_id,
        market_id.clone(),
        known_quantity,
        loaded.first_anchor_event_id.clone(),
        event.event_id().clone(),
        event.block_height(),
    )
    .map_err(|_| liquidation_proposed_pair_invalid())?;
    let quantity_key = PositionQuantityCurrentRecordV1::state_key(&account_id, market_id)
        .map_err(|_| liquidation_proposed_pair_invalid())?;

    let mut effects = Vec::new();
    let mut episodes = Vec::new();
    let mut proposed_open = None;
    if let LoadedNonTradePairState::Resolved { episode, .. } = loaded.state {
        let mut interrupted = *episode;
        interrupted.close_event_id = Some(event.event_id().clone());
        interrupted.close_cause = Some(cause);
        interrupted.status = EpisodeStatusV1::Interrupted;
        interrupted.last_event_id = event.event_id().clone();
        interrupted.last_block_height = event.block_height();
        interrupted
            .validate()
            .map_err(|_| liquidation_proposed_pair_invalid())?;
        let effect = PositionEpisodeEffectFactRecordV1::try_new(
            event.event_id().clone(),
            account_id,
            market_id.clone(),
            0,
            interrupted.episode_id.clone(),
            EpisodeEffectKindV1::Interrupted,
            zero_quantity,
            zero_notional.clone(),
            zero_quantity,
            zero_notional.clone(),
            zero_funding,
            zero_funding,
            Some(cause),
        )
        .map_err(|_| liquidation_proposed_pair_invalid())?;
        effects.push(effect);
        episodes.push(interrupted);
    } else if matches!(result, NonTradeQuantityResult::Exact(_)) {
        return Err(liquidation_current_pair_mismatch());
    }

    if let NonTradeQuantityResult::Exact(value) = result
        && value.raw() != 0
    {
        let episode_id = derive_position_episode_id(&account_id, market_id, event.event_id(), 1)
            .map_err(|_| liquidation_proposed_pair_invalid())?;
        let opened = PositionEpisodeRecordV1::try_new(
            episode_id.clone(),
            account_id,
            market_id.clone(),
            event.event_id().clone(),
            1,
            value,
            None,
            None,
            EpisodeCompletenessV1::PartialFromFirstObservation,
            zero_quantity,
            zero_notional.clone(),
            zero_quantity,
            zero_notional.clone(),
            zero_funding,
            zero_funding,
            EpisodeStatusV1::Open,
            event.event_id().clone(),
            event.block_height(),
        )
        .map_err(|_| liquidation_proposed_pair_invalid())?;
        let effect = PositionEpisodeEffectFactRecordV1::try_new(
            event.event_id().clone(),
            account_id,
            market_id.clone(),
            1,
            episode_id.clone(),
            EpisodeEffectKindV1::Opened,
            zero_quantity,
            zero_notional.clone(),
            zero_quantity,
            zero_notional,
            zero_funding,
            zero_funding,
            None,
        )
        .map_err(|_| liquidation_proposed_pair_invalid())?;
        effects.push(effect);
        episodes.push(opened);
        proposed_open = Some(episode_id);
    }

    let resolution = match result {
        NonTradeQuantityResult::Exact(value) if value.raw() == 0 => {
            EpisodeAttributionResolutionV1::NoOpenEpisode
        }
        NonTradeQuantityResult::Exact(_) => EpisodeAttributionResolutionV1::Resolved,
        NonTradeQuantityResult::Ambiguous => EpisodeAttributionResolutionV1::Interrupted,
    };
    let current = PositionEpisodeCurrentRecordV1::try_new(
        account_id,
        market_id.clone(),
        proposed_open,
        resolution,
        event.event_id().clone(),
        event.block_height(),
    )
    .map_err(|_| liquidation_proposed_pair_invalid())?;
    let current_key = PositionEpisodeCurrentRecordV1::state_key(&account_id, market_id)
        .map_err(|_| liquidation_proposed_pair_invalid())?;
    validate_proposed_pair(known_quantity, &current, episodes.iter())
        .map_err(|_| liquidation_proposed_pair_invalid())?;

    let mut mutations = Vec::new();
    for effect in effects {
        let key = PositionEpisodeEffectFactRecordV1::state_key(
            effect.event_id(),
            &account_id,
            market_id,
            effect.leg_ordinal(),
        )
        .map_err(|_| liquidation_episode_effect_prior_invalid())?;
        mutations.push(StateMutation::put(
            key,
            effect
                .encode()
                .map_err(|_| liquidation_episode_effect_prior_invalid())?,
        ));
    }
    for episode in episodes {
        let key = PositionEpisodeRecordV1::state_key(episode.episode_id())
            .map_err(|_| liquidation_episode_prior_invalid())?;
        mutations.push(StateMutation::put(
            key,
            episode
                .encode()
                .map_err(|_| liquidation_episode_prior_invalid())?,
        ));
    }
    mutations.push(StateMutation::put(
        quantity_key,
        quantity
            .encode()
            .map_err(|_| liquidation_proposed_pair_invalid())?,
    ));
    mutations.push(StateMutation::put(
        current_key,
        current
            .encode()
            .map_err(|_| liquidation_proposed_pair_invalid())?,
    ));
    Ok(mutations)
}

fn liquidation_quantity_current_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.quantity_current_invalid",
        "position quantity current is invalid",
    )
}

fn liquidation_episode_current_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.episode_current_invalid",
        "position episode current is invalid",
    )
}

fn liquidation_episode_reference_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.episode_reference_invalid",
        "position episode reference is invalid",
    )
}

fn liquidation_current_pair_mismatch() -> ReducerError {
    liquidation_error(
        "liquidation_state.current_pair_mismatch",
        "position quantity and episode currents do not form a valid pair",
    )
}

fn liquidation_quantity_arithmetic() -> ReducerError {
    liquidation_error(
        "liquidation_state.quantity_arithmetic",
        "non-trade position quantity arithmetic failed",
    )
}

fn liquidation_episode_effect_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.episode_effect_prior_invalid",
        "prior position episode effect is invalid",
    )
}

fn liquidation_episode_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.episode_prior_invalid",
        "prior position episode is invalid",
    )
}

fn liquidation_proposed_pair_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.proposed_pair_invalid",
        "proposed position quantity and episode pair is invalid",
    )
}

fn liquidation_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn validate_proposed_pair<'a>(
    known_quantity: Option<PositionQuantity>,
    current: &PositionEpisodeCurrentRecordV1,
    episodes: impl IntoIterator<Item = &'a PositionEpisodeRecordV1>,
) -> Result<(), ReducerError> {
    let episodes: Vec<_> = episodes.into_iter().collect();
    match (
        known_quantity,
        current.attribution_resolution,
        current.episode_id.as_ref(),
    ) {
        (Some(quantity), EpisodeAttributionResolutionV1::NoOpenEpisode, None)
            if quantity.raw() == 0 =>
        {
            Ok(())
        }
        (None, EpisodeAttributionResolutionV1::Interrupted, None) => Ok(()),
        (Some(quantity), EpisodeAttributionResolutionV1::Resolved, Some(target))
            if quantity.raw() != 0 =>
        {
            let Some(episode) = episodes
                .iter()
                .copied()
                .find(|episode| episode.episode_id == *target)
            else {
                return Err(current_pair_mismatch());
            };
            if episode.account_id != current.account_id
                || episode.market_id != current.market_id
                || episode.status != EpisodeStatusV1::Open
            {
                return Err(current_pair_mismatch());
            }
            Ok(())
        }
        _ => Err(current_pair_mismatch()),
    }
}

fn completeness_for_start(start: PositionQuantity) -> EpisodeCompletenessV1 {
    if start.raw() == 0 {
        EpisodeCompletenessV1::CompleteFromFlat
    } else {
        EpisodeCompletenessV1::PartialFromFirstObservation
    }
}

fn checked_position_magnitude(position: PositionQuantity) -> Result<i128, ReducerError> {
    if position.raw() < 0 {
        position.raw().checked_neg().ok_or_else(quantity_arithmetic)
    } else {
        Ok(position.raw())
    }
}

fn add_quantities(left: Quantity, right: Quantity) -> Result<Quantity, ReducerError> {
    let scale = left.scale().max(right.scale());
    let left = left
        .rescale(scale, RoundingMode::TowardZero)
        .map_err(|_| quantity_arithmetic())?;
    let right = right
        .rescale(scale, RoundingMode::TowardZero)
        .map_err(|_| quantity_arithmetic())?;
    left.checked_add(right).map_err(|_| quantity_arithmetic())
}

fn align_funding_amounts(
    paid: QuoteAmount,
    received: QuoteAmount,
    incoming: QuoteAmount,
) -> Result<(QuoteAmount, QuoteAmount, QuoteAmount), ReducerError> {
    let scale = paid.scale().max(received.scale()).max(incoming.scale());
    let paid = paid
        .rescale(scale, RoundingMode::TowardZero)
        .map_err(|_| funding_arithmetic())?;
    let received = received
        .rescale(scale, RoundingMode::TowardZero)
        .map_err(|_| funding_arithmetic())?;
    let incoming = incoming
        .rescale(scale, RoundingMode::TowardZero)
        .map_err(|_| funding_arithmetic())?;
    Ok((paid, received, incoming))
}

fn zero_quantity(scale: u8) -> Result<Quantity, ReducerError> {
    Quantity::from_raw(0, scale).map_err(|_| quantity_arithmetic())
}

fn zero_quote(scale: u8) -> Result<QuoteAmount, ReducerError> {
    QuoteAmount::from_raw(0, scale).map_err(|_| funding_arithmetic())
}

fn zero_notional() -> ExactQuoteNotional {
    ExactQuoteNotional::from_str("0").expect("literal zero notional is valid")
}

fn reject_prior_episode_effect(state: &StateView<'_>, key: &StateKey) -> Result<(), ReducerError> {
    let Some(bytes) = state.get(key) else {
        return Ok(());
    };
    PositionEpisodeEffectFactRecordV1::decode_at(key, bytes).map_err(|_| effect_prior_invalid())?;
    Err(episode_error(
        "position_episode.effect_identity_collision",
        "position episode effect identity is already present",
    ))
}

fn reject_prior_episode(state: &StateView<'_>, key: &StateKey) -> Result<(), ReducerError> {
    let Some(bytes) = state.get(key) else {
        return Ok(());
    };
    PositionEpisodeRecordV1::decode_at(key, bytes).map_err(|_| episode_prior_invalid())?;
    Err(episode_error(
        "position_episode.episode_identity_collision",
        "position episode identity is already present",
    ))
}

fn ensure_unique_episode_mutation_keys(mutations: &[StateMutation]) -> Result<(), ReducerError> {
    let mut keys = BTreeSet::new();
    if mutations.iter().all(|mutation| keys.insert(mutation.key())) {
        Ok(())
    } else {
        Err(episode_error(
            "position_episode.duplicate_mutation_key",
            "position episode event produced duplicate mutation keys",
        ))
    }
}

fn map_trade_validation_for_episode(error: TradeValidationError) -> ReducerError {
    match error {
        TradeValidationError::Identity => identity_mismatch(),
        TradeValidationError::MarketMissing => episode_error(
            "position_episode.market_prerequisite_missing",
            "market prerequisite is missing",
        ),
        TradeValidationError::MarketInvalid => episode_error(
            "position_episode.market_prerequisite_invalid",
            "market prerequisite is corrupt, key mismatched, or internally invalid",
        ),
        TradeValidationError::MarketUnresolved => episode_error(
            "position_episode.market_prerequisite_unresolved",
            "market prerequisite metadata is unresolved",
        ),
        TradeValidationError::NotionalArithmetic => notional_arithmetic(),
        TradeValidationError::ScaleNormalization
        | TradeValidationError::PriceTick
        | TradeValidationError::QuantityLot
        | TradeValidationError::PositionArithmetic => quantity_arithmetic(),
    }
}

fn map_funding_market_validation(error: TradeValidationError) -> ReducerError {
    match error {
        TradeValidationError::MarketMissing => episode_error(
            "position_episode.market_prerequisite_missing",
            "market prerequisite is missing",
        ),
        TradeValidationError::MarketInvalid => episode_error(
            "position_episode.market_prerequisite_invalid",
            "market prerequisite is corrupt, key mismatched, or internally invalid",
        ),
        TradeValidationError::MarketUnresolved => episode_error(
            "position_episode.market_prerequisite_unresolved",
            "market prerequisite metadata is unresolved",
        ),
        _ => episode_error(
            "position_episode.market_prerequisite_invalid",
            "market prerequisite validation failed",
        ),
    }
}

fn identity_mismatch() -> ReducerError {
    episode_error(
        "position_episode.identity_mismatch",
        "payload and envelope identities must match exactly",
    )
}

fn quantity_current_invalid() -> ReducerError {
    episode_error(
        "position_episode.quantity_current_invalid",
        "position quantity current is corrupt or key mismatched",
    )
}

fn episode_current_invalid() -> ReducerError {
    episode_error(
        "position_episode.episode_current_invalid",
        "position episode current is corrupt or key mismatched",
    )
}

fn episode_reference_invalid() -> ReducerError {
    episode_error(
        "position_episode.episode_reference_invalid",
        "resolved position episode reference is missing or invalid",
    )
}

fn current_pair_mismatch() -> ReducerError {
    episode_error(
        "position_episode.current_pair_mismatch",
        "position quantity and episode currents disagree",
    )
}

fn start_position_mismatch() -> ReducerError {
    episode_error(
        "position_episode.start_position_mismatch",
        "source start position does not match known current position",
    )
}

fn quantity_arithmetic() -> ReducerError {
    episode_error(
        "position_episode.quantity_arithmetic",
        "position episode quantity arithmetic failed",
    )
}

fn notional_arithmetic() -> ReducerError {
    episode_error(
        "position_episode.notional_arithmetic",
        "position episode notional arithmetic failed",
    )
}

fn funding_arithmetic() -> ReducerError {
    episode_error(
        "position_episode.funding_arithmetic",
        "position episode funding arithmetic failed",
    )
}

fn effect_prior_invalid() -> ReducerError {
    episode_error(
        "position_episode.effect_prior_invalid",
        "prior position episode effect is invalid",
    )
}

fn episode_prior_invalid() -> ReducerError {
    episode_error(
        "position_episode.episode_prior_invalid",
        "prior position episode is invalid",
    )
}

fn episode_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
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

    #[test]
    fn reducer_local_duplicate_mutation_keys_are_rejected() {
        let key = StateKey::try_new("position-episode-test.v1", b"same".to_vec()).unwrap();
        let error = ensure_unique_episode_mutation_keys(&[
            StateMutation::put(key.clone(), vec![1]),
            StateMutation::put(key, vec![2]),
        ])
        .expect_err("duplicate reducer-local keys must fail");
        assert_eq!(
            error.reason_code(),
            "position_episode.duplicate_mutation_key"
        );
    }
}
