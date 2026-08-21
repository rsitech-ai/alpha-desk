use std::str::FromStr;

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::{
    Address, AssetId, BlockHeight, DexId, EventId, FundingRate, MarketId, OutcomeId, Price,
    ProtocolTime, Quantity, QuoteAmount,
};
use orderbook::L2ReconcilePolicyV1;
use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView};

const FACT_NAMESPACE: &str = "market-fact.v1";
const DEX_NAMESPACE: &str = "dex-current.v1";
const ASSET_NAMESPACE: &str = "asset-context-current.v1";
const MARKET_NAMESPACE: &str = "market-current.v1";
const METADATA_NAMESPACE: &str = "market-metadata-version.v1";
const OUTCOME_NAMESPACE: &str = "market-outcome-current.v1";

const FACT_SCHEMA: &str = "hyperliquid-alpha-desk/market-fact/v1";
const DEX_SCHEMA: &str = "hyperliquid-alpha-desk/dex-current/v1";
const ASSET_SCHEMA: &str = "hyperliquid-alpha-desk/asset-context-current/v1";
const MARKET_SCHEMA: &str = "hyperliquid-alpha-desk/market-current/v1";
const METADATA_SCHEMA: &str = "hyperliquid-alpha-desk/market-metadata-version/v1";
const OUTCOME_SCHEMA: &str = "hyperliquid-alpha-desk/market-outcome-current/v1";
const CREATION_METADATA_VERSION: &str = "creation@1.0.0";
const MAX_RECORD_BYTES: usize = 16 * 1024;
const MAX_STATE_KEY_BYTES: usize = 64 * 1024;
const KEY_FRAME_BYTES: usize = size_of::<u64>();

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalMarketReducerV1;

impl CanonicalMarketReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-market@1.0.0";
}

impl EventReducer for CanonicalMarketReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0"
            && matches!(
                event.event_kind(),
                EventKind::DexCreated
                    | EventKind::AssetContextUpdated
                    | EventKind::MarketCreated
                    | EventKind::MarketMetadataChanged
                    | EventKind::MarketHalted
                    | EventKind::MarketResumed
                    | EventKind::OpenInterestCapChanged
                    | EventKind::MarginTableChanged
                    | EventKind::OracleUpdated
                    | EventKind::FundingRateUpdated
                    | EventKind::OutcomeCreated
                    | EventKind::OutcomeResolved
            )
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let fact_key =
            MarketFactRecordV1::state_key(event.event_id()).map_err(codec_reducer_error)?;
        if state.contains_key(&fact_key) {
            return Err(reducer_error(
                "market_state.event_identity_collision",
                "market event identity is already present",
            ));
        }
        let fact = MarketFactRecordV1::from_event(event)?;
        let mut mutations = vec![StateMutation::put(
            fact_key,
            fact.encode().map_err(codec_reducer_error)?,
        )];

        match event.payload() {
            EventPayload::DexCreated(payload) => {
                require_no_markets(event)?;
                require_accounts(event, std::slice::from_ref(&payload.operator_account_id))?;
                let key =
                    DexCurrentRecordV1::state_key(&payload.dex_id).map_err(codec_reducer_error)?;
                if state.contains_key(&key) {
                    return Err(reducer_error(
                        "market_state.dex_identity_collision",
                        "DEX identity is already present",
                    ));
                }
                let record = DexCurrentRecordV1 {
                    dex_id: payload.dex_id.clone(),
                    name: payload.name.clone(),
                    operator_account_id: payload.operator_account_id,
                    created_at_block: event.block_height(),
                };
                mutations.push(StateMutation::put(
                    key,
                    record.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::AssetContextUpdated(payload) => {
                require_no_markets(event)?;
                require_accounts(event, &[])?;
                let key = AssetContextCurrentRecordV1::state_key(&payload.asset_id)
                    .map_err(codec_reducer_error)?;
                if state.contains_key(&key) {
                    return Err(reducer_error(
                        "market_state.asset_identity_collision",
                        "asset identity is already present",
                    ));
                }
                let record = AssetContextCurrentRecordV1 {
                    asset_id: payload.asset_id.clone(),
                    context_version: payload.context_version.clone(),
                    context_hash: payload.context_hash,
                    updated_at_block: event.block_height(),
                };
                mutations.push(StateMutation::put(
                    key,
                    record.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::MarketCreated(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                if payload.base_asset_id == payload.quote_asset_id {
                    return Err(reducer_error(
                        "market_state.invalid_assets",
                        "market assets must be distinct",
                    ));
                }
                require_record(
                    state,
                    &DexCurrentRecordV1::state_key(&payload.dex_id).map_err(codec_reducer_error)?,
                    "market_state.missing_dex",
                    "market DEX prerequisite is missing",
                )?;
                for asset_id in [&payload.base_asset_id, &payload.quote_asset_id] {
                    require_record(
                        state,
                        &AssetContextCurrentRecordV1::state_key(asset_id)
                            .map_err(codec_reducer_error)?,
                        "market_state.missing_asset",
                        "market asset prerequisite is missing",
                    )?;
                }
                if payload.tick_size.raw() <= 0 || payload.lot_size.raw() <= 0 {
                    return Err(reducer_error(
                        "market_state.invalid_fixed_point",
                        "market tick and lot values must be positive",
                    ));
                }
                let current_key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                if state.contains_key(&current_key) {
                    return Err(reducer_error(
                        "market_state.market_identity_collision",
                        "market identity is already present",
                    ));
                }
                let metadata_key = MarketMetadataVersionRecordV1::state_key(
                    &payload.market_id,
                    CREATION_METADATA_VERSION,
                )
                .map_err(codec_reducer_error)?;
                if state.contains_key(&metadata_key) {
                    return Err(reducer_error(
                        "market_state.metadata_identity_collision",
                        "market metadata identity is already present",
                    ));
                }
                let metadata_hash = event.payload_hash();
                let current = MarketCurrentRecordV1 {
                    market_id: payload.market_id.clone(),
                    dex_id: payload.dex_id.clone(),
                    base_asset_id: payload.base_asset_id.clone(),
                    quote_asset_id: payload.quote_asset_id.clone(),
                    status: MarketStatusV1::Active,
                    metadata_resolution: MarketMetadataResolutionV1::Exact,
                    metadata_version: CREATION_METADATA_VERSION.to_owned(),
                    metadata_hash,
                    tick_size: Some(payload.tick_size),
                    lot_size: Some(payload.lot_size),
                    price_scale: Some(payload.tick_size.scale()),
                    quantity_scale: Some(payload.lot_size.scale()),
                    open_interest_cap: None,
                    margin_table_hash: None,
                    oracle_price: None,
                    oracle_source: None,
                    oracle_effective_at: None,
                    funding_rate: None,
                    funding_effective_at: None,
                    created_at_block: event.block_height(),
                    updated_at_block: event.block_height(),
                };
                let metadata = MarketMetadataVersionRecordV1 {
                    market_id: payload.market_id.clone(),
                    metadata_version: CREATION_METADATA_VERSION.to_owned(),
                    metadata_hash,
                    effective_from_block: event.block_height(),
                    effective_until_block: None,
                    resolution: MarketMetadataResolutionV1::Exact,
                    tick_size: Some(payload.tick_size),
                    lot_size: Some(payload.lot_size),
                    price_scale: Some(payload.tick_size.scale()),
                    quantity_scale: Some(payload.lot_size.scale()),
                };
                mutations.push(StateMutation::put(
                    current_key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
                mutations.push(StateMutation::put(
                    metadata_key,
                    metadata.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::MarketMetadataChanged(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let current_key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let mut current = load_market(state, &current_key)?;
                if payload.metadata_version.as_str() <= current.metadata_version.as_str()
                    || event.block_height() <= current.updated_at_block
                {
                    return Err(reducer_error(
                        "market_state.non_monotonic_metadata",
                        "market metadata versions and heights must increase",
                    ));
                }
                let prior_key = MarketMetadataVersionRecordV1::state_key(
                    &payload.market_id,
                    &current.metadata_version,
                )
                .map_err(codec_reducer_error)?;
                let mut prior = load_metadata(state, &prior_key)?;
                if prior.effective_until_block.is_some()
                    || prior.effective_from_block >= event.block_height()
                {
                    return Err(reducer_error(
                        "market_state.non_monotonic_metadata",
                        "market metadata interval cannot overlap",
                    ));
                }
                let next_key = MarketMetadataVersionRecordV1::state_key(
                    &payload.market_id,
                    &payload.metadata_version,
                )
                .map_err(codec_reducer_error)?;
                if state.contains_key(&next_key) {
                    return Err(reducer_error(
                        "market_state.metadata_identity_collision",
                        "market metadata identity is already present",
                    ));
                }
                let previous_height = event
                    .block_height()
                    .get()
                    .checked_sub(1)
                    .map(BlockHeight::new)
                    .ok_or_else(|| {
                        reducer_error(
                            "market_state.arithmetic_overflow",
                            "metadata interval height underflowed",
                        )
                    })?;
                prior.effective_until_block = Some(previous_height);
                let next = MarketMetadataVersionRecordV1 {
                    market_id: payload.market_id.clone(),
                    metadata_version: payload.metadata_version.clone(),
                    metadata_hash: payload.metadata_hash,
                    effective_from_block: event.block_height(),
                    effective_until_block: None,
                    resolution: MarketMetadataResolutionV1::Unresolved,
                    tick_size: None,
                    lot_size: None,
                    price_scale: None,
                    quantity_scale: None,
                };
                current.metadata_resolution = MarketMetadataResolutionV1::Unresolved;
                current
                    .metadata_version
                    .clone_from(&payload.metadata_version);
                current.metadata_hash = payload.metadata_hash;
                current.tick_size = None;
                current.lot_size = None;
                current.price_scale = None;
                current.quantity_scale = None;
                current.open_interest_cap = None;
                current.margin_table_hash = None;
                current.oracle_price = None;
                current.oracle_source = None;
                current.oracle_effective_at = None;
                current.funding_rate = None;
                current.funding_effective_at = None;
                current.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    prior_key,
                    prior.encode().map_err(codec_reducer_error)?,
                ));
                mutations.push(StateMutation::put(
                    next_key,
                    next.encode().map_err(codec_reducer_error)?,
                ));
                mutations.push(StateMutation::put(
                    current_key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::MarketHalted(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let mut current = load_market(state, &key)?;
                if current.status != MarketStatusV1::Active {
                    return Err(invalid_status_transition());
                }
                current.status = MarketStatusV1::Halted;
                current.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::MarketResumed(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let mut current = load_market(state, &key)?;
                if current.status != MarketStatusV1::Halted {
                    return Err(invalid_status_transition());
                }
                current.status = MarketStatusV1::Active;
                current.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::OpenInterestCapChanged(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let mut current = load_market(state, &key)?;
                require_exact_metadata(&current)?;
                if payload.previous_cap.raw() < 0
                    || payload.new_cap.raw() < 0
                    || same_fixed(payload.previous_cap, payload.new_cap)
                {
                    return Err(reducer_error(
                        "market_state.invalid_fixed_point",
                        "open-interest caps must be distinct nonnegative fixed-point values",
                    ));
                }
                if current
                    .open_interest_cap
                    .is_some_and(|value| !same_fixed(value, payload.previous_cap))
                {
                    return Err(reducer_error(
                        "market_state.previous_cap_mismatch",
                        "previous open-interest cap does not match current state",
                    ));
                }
                current.open_interest_cap = Some(payload.new_cap);
                current.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::MarginTableChanged(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let mut current = load_market(state, &key)?;
                require_exact_metadata(&current)?;
                if payload.previous_table_hash == payload.new_table_hash {
                    return Err(reducer_error(
                        "market_state.invalid_margin_table",
                        "margin table hashes must differ",
                    ));
                }
                if current
                    .margin_table_hash
                    .as_deref()
                    .is_some_and(|value| value != payload.previous_table_hash)
                {
                    return Err(reducer_error(
                        "market_state.previous_margin_table_mismatch",
                        "previous margin table does not match current state",
                    ));
                }
                current
                    .margin_table_hash
                    .clone_from(&Some(payload.new_table_hash.clone()));
                current.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::OracleUpdated(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let mut current = load_market(state, &key)?;
                require_exact_metadata(&current)?;
                if payload.oracle_price.raw() <= 0 {
                    return Err(reducer_error(
                        "market_state.invalid_fixed_point",
                        "oracle price must be positive",
                    ));
                }
                if current
                    .oracle_effective_at
                    .is_some_and(|value| payload.effective_at < value)
                {
                    return Err(reducer_error(
                        "market_state.stale_oracle_time",
                        "oracle effective time cannot regress",
                    ));
                }
                current.oracle_price = Some(payload.oracle_price);
                current.oracle_source = Some(payload.source.clone());
                current.oracle_effective_at = Some(payload.effective_at);
                current.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::FundingRateUpdated(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let mut current = load_market(state, &key)?;
                require_exact_metadata(&current)?;
                if current
                    .funding_effective_at
                    .is_some_and(|value| payload.effective_at < value)
                {
                    return Err(reducer_error(
                        "market_state.stale_funding_time",
                        "funding effective time cannot regress",
                    ));
                }
                current.funding_rate = Some(payload.funding_rate);
                current.funding_effective_at = Some(payload.effective_at);
                current.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    key,
                    current.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::OutcomeCreated(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let market_key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                load_market(state, &market_key)?;
                let key =
                    OutcomeCurrentRecordV1::state_key(&payload.market_id, &payload.outcome_id)
                        .map_err(codec_reducer_error)?;
                if state.contains_key(&key) {
                    return Err(reducer_error(
                        "market_state.outcome_identity_collision",
                        "outcome identity is already present",
                    ));
                }
                let outcome = OutcomeCurrentRecordV1 {
                    market_id: payload.market_id.clone(),
                    outcome_id: payload.outcome_id.clone(),
                    description: payload.description.clone(),
                    settlement_value: None,
                    resolved_at: None,
                    created_at_block: event.block_height(),
                    updated_at_block: event.block_height(),
                };
                mutations.push(StateMutation::put(
                    key,
                    outcome.encode().map_err(codec_reducer_error)?,
                ));
            }
            EventPayload::OutcomeResolved(payload) => {
                require_market(event, &payload.market_id)?;
                require_accounts(event, &[])?;
                let market_key = MarketCurrentRecordV1::state_key(&payload.market_id)
                    .map_err(codec_reducer_error)?;
                let current = load_market(state, &market_key)?;
                require_exact_metadata(&current)?;
                let key =
                    OutcomeCurrentRecordV1::state_key(&payload.market_id, &payload.outcome_id)
                        .map_err(codec_reducer_error)?;
                let bytes = state.get(&key).ok_or_else(|| {
                    reducer_error(
                        "market_state.missing_outcome",
                        "outcome prerequisite is missing",
                    )
                })?;
                let mut outcome =
                    OutcomeCurrentRecordV1::decode_at(&key, bytes).map_err(codec_reducer_error)?;
                if outcome.resolved_at.is_some() {
                    return Err(reducer_error(
                        "market_state.outcome_already_resolved",
                        "outcome resolution is immutable",
                    ));
                }
                if payload.settlement_value.raw() < 0 {
                    return Err(reducer_error(
                        "market_state.invalid_fixed_point",
                        "outcome settlement value must be nonnegative",
                    ));
                }
                outcome.settlement_value = Some(payload.settlement_value);
                outcome.resolved_at = Some(payload.resolved_at);
                outcome.updated_at_block = event.block_height();
                mutations.push(StateMutation::put(
                    key,
                    outcome.encode().map_err(codec_reducer_error)?,
                ));
            }
            _ => {
                return Err(reducer_error(
                    "market_state.invalid_event",
                    "market reducer received a non-market payload",
                ));
            }
        }
        Ok(mutations)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketStatusV1 {
    Active,
    Halted,
}

impl MarketStatusV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Halted => "halted",
        }
    }

    fn parse(value: &str) -> Result<Self, MarketStateError> {
        match value {
            "active" => Ok(Self::Active),
            "halted" => Ok(Self::Halted),
            _ => Err(MarketStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MarketMetadataResolutionV1 {
    Exact,
    Unresolved,
}

impl MarketMetadataResolutionV1 {
    const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Unresolved => "unresolved",
        }
    }

    fn parse(value: &str) -> Result<Self, MarketStateError> {
        match value {
            "exact" => Ok(Self::Exact),
            "unresolved" => Ok(Self::Unresolved),
            _ => Err(MarketStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketFactRecordV1 {
    event_id: EventId,
    event_kind: EventKind,
    market_id: Option<MarketId>,
    dex_id: Option<DexId>,
    asset_id: Option<AssetId>,
    outcome_id: Option<OutcomeId>,
    block_height: BlockHeight,
    payload_hash: [u8; 32],
    rule_version: String,
}

impl MarketFactRecordV1 {
    pub fn state_key(event_id: &EventId) -> Result<StateKey, MarketStateError> {
        single_key(FACT_NAMESPACE, event_id.as_str().as_bytes())
    }

    fn from_event(event: &CanonicalEventEnvelope) -> Result<Self, ReducerError> {
        let (market_id, dex_id, asset_id, outcome_id) = match event.payload() {
            EventPayload::DexCreated(value) => (None, Some(value.dex_id.clone()), None, None),
            EventPayload::AssetContextUpdated(value) => {
                (None, None, Some(value.asset_id.clone()), None)
            }
            EventPayload::MarketCreated(value) => (
                Some(value.market_id.clone()),
                Some(value.dex_id.clone()),
                None,
                None,
            ),
            EventPayload::MarketMetadataChanged(value) => {
                (Some(value.market_id.clone()), None, None, None)
            }
            EventPayload::MarketHalted(value) => (Some(value.market_id.clone()), None, None, None),
            EventPayload::MarketResumed(value) => (Some(value.market_id.clone()), None, None, None),
            EventPayload::OpenInterestCapChanged(value) => {
                (Some(value.market_id.clone()), None, None, None)
            }
            EventPayload::MarginTableChanged(value) => {
                (Some(value.market_id.clone()), None, None, None)
            }
            EventPayload::OracleUpdated(value) => (Some(value.market_id.clone()), None, None, None),
            EventPayload::FundingRateUpdated(value) => {
                (Some(value.market_id.clone()), None, None, None)
            }
            EventPayload::OutcomeCreated(value) => (
                Some(value.market_id.clone()),
                None,
                None,
                Some(value.outcome_id.clone()),
            ),
            EventPayload::OutcomeResolved(value) => (
                Some(value.market_id.clone()),
                None,
                None,
                Some(value.outcome_id.clone()),
            ),
            _ => {
                return Err(reducer_error(
                    "market_state.invalid_event",
                    "market reducer received a non-market payload",
                ));
            }
        };
        Ok(Self {
            event_id: event.event_id().clone(),
            event_kind: event.event_kind(),
            market_id,
            dex_id,
            asset_id,
            outcome_id,
            block_height: event.block_height(),
            payload_hash: event.payload_hash(),
            rule_version: CanonicalMarketReducerV1::VERSION.to_owned(),
        })
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MarketStateError> {
        let wire: FactWire = decode_canonical(bytes)?;
        if wire.schema != FACT_SCHEMA || wire.rule_version != CanonicalMarketReducerV1::VERSION {
            return Err(MarketStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| MarketStateError::InvalidRecord)?,
            event_kind: EventKind::try_from(wire.event_kind.as_str())
                .map_err(|_| MarketStateError::InvalidRecord)?,
            market_id: wire
                .market_id
                .map(MarketId::new)
                .transpose()
                .map_err(|_| MarketStateError::InvalidRecord)?,
            dex_id: wire
                .dex_id
                .map(DexId::new)
                .transpose()
                .map_err(|_| MarketStateError::InvalidRecord)?,
            asset_id: wire
                .asset_id
                .map(AssetId::new)
                .transpose()
                .map_err(|_| MarketStateError::InvalidRecord)?,
            outcome_id: wire
                .outcome_id
                .map(OutcomeId::new)
                .transpose()
                .map_err(|_| MarketStateError::InvalidRecord)?,
            block_height: BlockHeight::new(wire.block_height),
            payload_hash: decode_hash(&wire.payload_blake3)?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, MarketStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.event_id)? != *key {
            return Err(MarketStateError::KeyMismatch);
        }
        Ok(record)
    }

    pub fn encode(&self) -> Result<Vec<u8>, MarketStateError> {
        self.validate()?;
        encode_canonical(&FactWire {
            schema: FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            event_kind: self.event_kind.as_wire_name().to_owned(),
            market_id: self
                .market_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            dex_id: self.dex_id.as_ref().map(|value| value.as_str().to_owned()),
            asset_id: self
                .asset_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            outcome_id: self
                .outcome_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            block_height: self.block_height.get(),
            payload_blake3: hex::encode(self.payload_hash),
            rule_version: self.rule_version.clone(),
        })
    }

    #[must_use]
    pub const fn event_kind(&self) -> EventKind {
        self.event_kind
    }

    fn validate(&self) -> Result<(), MarketStateError> {
        if self.rule_version != CanonicalMarketReducerV1::VERSION {
            return Err(MarketStateError::InvalidRecord);
        }
        let valid_identity = match self.event_kind {
            EventKind::DexCreated => {
                self.market_id.is_none()
                    && self.dex_id.is_some()
                    && self.asset_id.is_none()
                    && self.outcome_id.is_none()
            }
            EventKind::AssetContextUpdated => {
                self.market_id.is_none()
                    && self.dex_id.is_none()
                    && self.asset_id.is_some()
                    && self.outcome_id.is_none()
            }
            EventKind::MarketCreated => {
                self.market_id.is_some()
                    && self.dex_id.is_some()
                    && self.asset_id.is_none()
                    && self.outcome_id.is_none()
            }
            EventKind::MarketMetadataChanged
            | EventKind::MarketHalted
            | EventKind::MarketResumed
            | EventKind::OpenInterestCapChanged
            | EventKind::MarginTableChanged
            | EventKind::OracleUpdated
            | EventKind::FundingRateUpdated => {
                self.market_id.is_some()
                    && self.dex_id.is_none()
                    && self.asset_id.is_none()
                    && self.outcome_id.is_none()
            }
            EventKind::OutcomeCreated | EventKind::OutcomeResolved => {
                self.market_id.is_some()
                    && self.dex_id.is_none()
                    && self.asset_id.is_none()
                    && self.outcome_id.is_some()
            }
            _ => false,
        };
        if valid_identity {
            Ok(())
        } else {
            Err(MarketStateError::InvalidRecord)
        }
    }

    #[must_use]
    pub const fn event_id(&self) -> &EventId {
        &self.event_id
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
pub struct DexCurrentRecordV1 {
    dex_id: DexId,
    name: String,
    operator_account_id: Address,
    created_at_block: BlockHeight,
}

impl DexCurrentRecordV1 {
    pub fn state_key(dex_id: &DexId) -> Result<StateKey, MarketStateError> {
        single_key(DEX_NAMESPACE, dex_id.as_str().as_bytes())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MarketStateError> {
        let wire: DexWire = decode_canonical(bytes)?;
        if wire.schema != DEX_SCHEMA {
            return Err(MarketStateError::InvalidRecord);
        }
        let record = Self {
            dex_id: DexId::new(wire.dex_id).map_err(|_| MarketStateError::InvalidRecord)?,
            name: wire.name,
            operator_account_id: Address::parse_api(&wire.operator_account_id)
                .map_err(|_| MarketStateError::InvalidRecord)?,
            created_at_block: BlockHeight::new(wire.created_at_block),
        };
        if !valid_text(&record.name, 256) {
            return Err(MarketStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, MarketStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.dex_id)? != *key {
            return Err(MarketStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, MarketStateError> {
        encode_canonical(&DexWire {
            schema: DEX_SCHEMA.to_owned(),
            dex_id: self.dex_id.as_str().to_owned(),
            name: self.name.clone(),
            operator_account_id: self.operator_account_id.to_api_string(),
            created_at_block: self.created_at_block.get(),
        })
    }

    #[must_use]
    pub const fn dex_id(&self) -> &DexId {
        &self.dex_id
    }

    #[must_use]
    pub const fn operator_account_id(&self) -> Address {
        self.operator_account_id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn created_at_block(&self) -> BlockHeight {
        self.created_at_block
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetContextCurrentRecordV1 {
    asset_id: AssetId,
    context_version: String,
    context_hash: [u8; 32],
    updated_at_block: BlockHeight,
}

impl AssetContextCurrentRecordV1 {
    pub fn state_key(asset_id: &AssetId) -> Result<StateKey, MarketStateError> {
        single_key(ASSET_NAMESPACE, asset_id.as_str().as_bytes())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MarketStateError> {
        let wire: AssetWire = decode_canonical(bytes)?;
        if wire.schema != ASSET_SCHEMA {
            return Err(MarketStateError::InvalidRecord);
        }
        let record = Self {
            asset_id: AssetId::new(wire.asset_id).map_err(|_| MarketStateError::InvalidRecord)?,
            context_version: wire.context_version,
            context_hash: decode_hash(&wire.context_blake3)?,
            updated_at_block: BlockHeight::new(wire.updated_at_block),
        };
        if !valid_text(&record.context_version, 128) {
            return Err(MarketStateError::InvalidRecord);
        }
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, MarketStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.asset_id)? != *key {
            return Err(MarketStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, MarketStateError> {
        encode_canonical(&AssetWire {
            schema: ASSET_SCHEMA.to_owned(),
            asset_id: self.asset_id.as_str().to_owned(),
            context_version: self.context_version.clone(),
            context_blake3: hex::encode(self.context_hash),
            updated_at_block: self.updated_at_block.get(),
        })
    }

    #[must_use]
    pub const fn asset_id(&self) -> &AssetId {
        &self.asset_id
    }

    #[must_use]
    pub fn context_version(&self) -> &str {
        &self.context_version
    }

    #[must_use]
    pub const fn context_hash(&self) -> [u8; 32] {
        self.context_hash
    }

    #[must_use]
    pub const fn updated_at_block(&self) -> BlockHeight {
        self.updated_at_block
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketCurrentRecordV1 {
    market_id: MarketId,
    dex_id: DexId,
    base_asset_id: AssetId,
    quote_asset_id: AssetId,
    status: MarketStatusV1,
    metadata_resolution: MarketMetadataResolutionV1,
    metadata_version: String,
    metadata_hash: [u8; 32],
    tick_size: Option<Price>,
    lot_size: Option<Quantity>,
    price_scale: Option<u8>,
    quantity_scale: Option<u8>,
    open_interest_cap: Option<QuoteAmount>,
    margin_table_hash: Option<String>,
    oracle_price: Option<Price>,
    oracle_source: Option<String>,
    oracle_effective_at: Option<ProtocolTime>,
    funding_rate: Option<FundingRate>,
    funding_effective_at: Option<ProtocolTime>,
    created_at_block: BlockHeight,
    updated_at_block: BlockHeight,
}

impl MarketCurrentRecordV1 {
    pub fn state_key(market_id: &MarketId) -> Result<StateKey, MarketStateError> {
        single_key(MARKET_NAMESPACE, market_id.as_str().as_bytes())
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MarketStateError> {
        let wire: MarketWire = decode_canonical(bytes)?;
        if wire.schema != MARKET_SCHEMA {
            return Err(MarketStateError::InvalidRecord);
        }
        let record = Self {
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| MarketStateError::InvalidRecord)?,
            dex_id: DexId::new(wire.dex_id).map_err(|_| MarketStateError::InvalidRecord)?,
            base_asset_id: AssetId::new(wire.base_asset_id)
                .map_err(|_| MarketStateError::InvalidRecord)?,
            quote_asset_id: AssetId::new(wire.quote_asset_id)
                .map_err(|_| MarketStateError::InvalidRecord)?,
            status: MarketStatusV1::parse(&wire.status)?,
            metadata_resolution: MarketMetadataResolutionV1::parse(&wire.metadata_resolution)?,
            metadata_version: wire.metadata_version,
            metadata_hash: decode_hash(&wire.metadata_blake3)?,
            tick_size: parse_optional(&wire.tick_size)?,
            lot_size: parse_optional(&wire.lot_size)?,
            price_scale: wire.price_scale,
            quantity_scale: wire.quantity_scale,
            open_interest_cap: parse_optional(&wire.open_interest_cap)?,
            margin_table_hash: wire.margin_table_hash,
            oracle_price: parse_optional(&wire.oracle_price)?,
            oracle_source: wire.oracle_source,
            oracle_effective_at: parse_optional_time(wire.oracle_effective_at_micros)?,
            funding_rate: parse_optional(&wire.funding_rate)?,
            funding_effective_at: parse_optional_time(wire.funding_effective_at_micros)?,
            created_at_block: BlockHeight::new(wire.created_at_block),
            updated_at_block: BlockHeight::new(wire.updated_at_block),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, MarketStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.market_id)? != *key {
            return Err(MarketStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, MarketStateError> {
        self.validate()?;
        encode_canonical(&MarketWire {
            schema: MARKET_SCHEMA.to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            dex_id: self.dex_id.as_str().to_owned(),
            base_asset_id: self.base_asset_id.as_str().to_owned(),
            quote_asset_id: self.quote_asset_id.as_str().to_owned(),
            status: self.status.as_wire_name().to_owned(),
            metadata_resolution: self.metadata_resolution.as_wire_name().to_owned(),
            metadata_version: self.metadata_version.clone(),
            metadata_blake3: hex::encode(self.metadata_hash),
            tick_size: display_optional(self.tick_size),
            lot_size: display_optional(self.lot_size),
            price_scale: self.price_scale,
            quantity_scale: self.quantity_scale,
            open_interest_cap: display_optional(self.open_interest_cap),
            margin_table_hash: self.margin_table_hash.clone(),
            oracle_price: display_optional(self.oracle_price),
            oracle_source: self.oracle_source.clone(),
            oracle_effective_at_micros: self.oracle_effective_at.map(ProtocolTime::unix_micros),
            funding_rate: display_optional(self.funding_rate),
            funding_effective_at_micros: self.funding_effective_at.map(ProtocolTime::unix_micros),
            created_at_block: self.created_at_block.get(),
            updated_at_block: self.updated_at_block.get(),
        })
    }

    fn validate(&self) -> Result<(), MarketStateError> {
        if self.base_asset_id == self.quote_asset_id
            || self.updated_at_block < self.created_at_block
            || !valid_text(&self.metadata_version, 128)
            || self.open_interest_cap.is_some_and(|value| value.raw() < 0)
            || self
                .margin_table_hash
                .as_deref()
                .is_some_and(|value| !valid_text(value, 256))
            || self
                .oracle_source
                .as_deref()
                .is_some_and(|value| !valid_text(value, 256))
        {
            return Err(MarketStateError::InvalidRecord);
        }
        let oracle_fields = [
            self.oracle_price.is_some(),
            self.oracle_source.is_some(),
            self.oracle_effective_at.is_some(),
        ];
        if oracle_fields.iter().any(|present| *present)
            && !oracle_fields.iter().all(|present| *present)
            || self.oracle_price.is_some_and(|value| value.raw() <= 0)
            || self.funding_rate.is_some() != self.funding_effective_at.is_some()
        {
            return Err(MarketStateError::InvalidRecord);
        }
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => {
                let (Some(tick), Some(lot), Some(price_scale), Some(quantity_scale)) = (
                    self.tick_size,
                    self.lot_size,
                    self.price_scale,
                    self.quantity_scale,
                ) else {
                    return Err(MarketStateError::InvalidRecord);
                };
                if tick.raw() <= 0
                    || lot.raw() <= 0
                    || tick.scale() != price_scale
                    || lot.scale() != quantity_scale
                {
                    return Err(MarketStateError::InvalidRecord);
                }
            }
            MarketMetadataResolutionV1::Unresolved => {
                if self.tick_size.is_some()
                    || self.lot_size.is_some()
                    || self.price_scale.is_some()
                    || self.quantity_scale.is_some()
                {
                    return Err(MarketStateError::InvalidRecord);
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn dex_id(&self) -> &DexId {
        &self.dex_id
    }

    #[must_use]
    pub const fn base_asset_id(&self) -> &AssetId {
        &self.base_asset_id
    }

    #[must_use]
    pub const fn quote_asset_id(&self) -> &AssetId {
        &self.quote_asset_id
    }

    #[must_use]
    pub const fn status(&self) -> MarketStatusV1 {
        self.status
    }

    #[must_use]
    pub const fn metadata_resolution(&self) -> MarketMetadataResolutionV1 {
        self.metadata_resolution
    }

    #[must_use]
    pub fn metadata_version(&self) -> &str {
        &self.metadata_version
    }

    #[must_use]
    pub const fn tick_size(&self) -> Option<Price> {
        self.tick_size
    }

    #[must_use]
    pub const fn lot_size(&self) -> Option<Quantity> {
        self.lot_size
    }

    pub fn l2_reconcile_policy_v1(&self) -> Result<L2ReconcilePolicyV1, MarketStateError> {
        match (self.tick_size, self.lot_size) {
            (Some(tick), Some(lot)) if tick.raw() > 0 && lot.raw() > 0 => {
                Ok(L2ReconcilePolicyV1::for_market(tick, lot))
            }
            _ => Err(MarketStateError::InvalidRecord),
        }
    }

    #[must_use]
    pub const fn price_scale(&self) -> Option<u32> {
        match self.price_scale {
            Some(scale) => Some(scale as u32),
            None => None,
        }
    }

    #[must_use]
    pub const fn quantity_scale(&self) -> Option<u32> {
        match self.quantity_scale {
            Some(scale) => Some(scale as u32),
            None => None,
        }
    }

    #[must_use]
    pub const fn open_interest_cap(&self) -> Option<QuoteAmount> {
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => self.open_interest_cap,
            MarketMetadataResolutionV1::Unresolved => None,
        }
    }

    #[must_use]
    pub fn margin_table_hash(&self) -> Option<&str> {
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => self.margin_table_hash.as_deref(),
            MarketMetadataResolutionV1::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn oracle_price(&self) -> Option<Price> {
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => self.oracle_price,
            MarketMetadataResolutionV1::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn funding_rate(&self) -> Option<FundingRate> {
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => self.funding_rate,
            MarketMetadataResolutionV1::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn metadata_hash(&self) -> [u8; 32] {
        self.metadata_hash
    }

    #[must_use]
    pub fn oracle_source(&self) -> Option<&str> {
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => self.oracle_source.as_deref(),
            MarketMetadataResolutionV1::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn oracle_effective_at(&self) -> Option<ProtocolTime> {
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => self.oracle_effective_at,
            MarketMetadataResolutionV1::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn funding_effective_at(&self) -> Option<ProtocolTime> {
        match self.metadata_resolution {
            MarketMetadataResolutionV1::Exact => self.funding_effective_at,
            MarketMetadataResolutionV1::Unresolved => None,
        }
    }

    #[must_use]
    pub const fn created_at_block(&self) -> BlockHeight {
        self.created_at_block
    }

    #[must_use]
    pub const fn updated_at_block(&self) -> BlockHeight {
        self.updated_at_block
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketMetadataVersionRecordV1 {
    market_id: MarketId,
    metadata_version: String,
    metadata_hash: [u8; 32],
    effective_from_block: BlockHeight,
    effective_until_block: Option<BlockHeight>,
    resolution: MarketMetadataResolutionV1,
    tick_size: Option<Price>,
    lot_size: Option<Quantity>,
    price_scale: Option<u8>,
    quantity_scale: Option<u8>,
}

impl MarketMetadataVersionRecordV1 {
    pub fn state_key(
        market_id: &MarketId,
        metadata_version: &str,
    ) -> Result<StateKey, MarketStateError> {
        compound_key(
            METADATA_NAMESPACE,
            &[market_id.as_str().as_bytes(), metadata_version.as_bytes()],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MarketStateError> {
        let wire: MetadataWire = decode_canonical(bytes)?;
        if wire.schema != METADATA_SCHEMA {
            return Err(MarketStateError::InvalidRecord);
        }
        let record = Self {
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| MarketStateError::InvalidRecord)?,
            metadata_version: wire.metadata_version,
            metadata_hash: decode_hash(&wire.metadata_blake3)?,
            effective_from_block: BlockHeight::new(wire.effective_from_block),
            effective_until_block: wire.effective_until_block.map(BlockHeight::new),
            resolution: MarketMetadataResolutionV1::parse(&wire.resolution)?,
            tick_size: parse_optional(&wire.tick_size)?,
            lot_size: parse_optional(&wire.lot_size)?,
            price_scale: wire.price_scale,
            quantity_scale: wire.quantity_scale,
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, MarketStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.market_id, &record.metadata_version)? != *key {
            return Err(MarketStateError::KeyMismatch);
        }
        Ok(record)
    }

    fn encode(&self) -> Result<Vec<u8>, MarketStateError> {
        self.validate()?;
        encode_canonical(&MetadataWire {
            schema: METADATA_SCHEMA.to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            metadata_version: self.metadata_version.clone(),
            metadata_blake3: hex::encode(self.metadata_hash),
            effective_from_block: self.effective_from_block.get(),
            effective_until_block: self.effective_until_block.map(BlockHeight::get),
            resolution: self.resolution.as_wire_name().to_owned(),
            tick_size: display_optional(self.tick_size),
            lot_size: display_optional(self.lot_size),
            price_scale: self.price_scale,
            quantity_scale: self.quantity_scale,
        })
    }

    fn validate(&self) -> Result<(), MarketStateError> {
        if !valid_text(&self.metadata_version, 128)
            || self
                .effective_until_block
                .is_some_and(|until| until < self.effective_from_block)
        {
            return Err(MarketStateError::InvalidRecord);
        }
        match self.resolution {
            MarketMetadataResolutionV1::Exact => {
                let (Some(tick), Some(lot), Some(price_scale), Some(quantity_scale)) = (
                    self.tick_size,
                    self.lot_size,
                    self.price_scale,
                    self.quantity_scale,
                ) else {
                    return Err(MarketStateError::InvalidRecord);
                };
                if tick.raw() <= 0
                    || lot.raw() <= 0
                    || tick.scale() != price_scale
                    || lot.scale() != quantity_scale
                {
                    return Err(MarketStateError::InvalidRecord);
                }
            }
            MarketMetadataResolutionV1::Unresolved => {
                if self.tick_size.is_some()
                    || self.lot_size.is_some()
                    || self.price_scale.is_some()
                    || self.quantity_scale.is_some()
                {
                    return Err(MarketStateError::InvalidRecord);
                }
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn effective_from_block(&self) -> BlockHeight {
        self.effective_from_block
    }

    #[must_use]
    pub const fn effective_until_block(&self) -> Option<BlockHeight> {
        self.effective_until_block
    }

    #[must_use]
    pub const fn resolution(&self) -> MarketMetadataResolutionV1 {
        self.resolution
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub fn metadata_version(&self) -> &str {
        &self.metadata_version
    }

    #[must_use]
    pub const fn metadata_hash(&self) -> [u8; 32] {
        self.metadata_hash
    }

    #[must_use]
    pub const fn tick_size(&self) -> Option<Price> {
        self.tick_size
    }

    #[must_use]
    pub const fn lot_size(&self) -> Option<Quantity> {
        self.lot_size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutcomeCurrentRecordV1 {
    market_id: MarketId,
    outcome_id: OutcomeId,
    description: String,
    settlement_value: Option<Price>,
    resolved_at: Option<ProtocolTime>,
    created_at_block: BlockHeight,
    updated_at_block: BlockHeight,
}

impl OutcomeCurrentRecordV1 {
    pub fn state_key(
        market_id: &MarketId,
        outcome_id: &OutcomeId,
    ) -> Result<StateKey, MarketStateError> {
        compound_key(
            OUTCOME_NAMESPACE,
            &[
                market_id.as_str().as_bytes(),
                outcome_id.as_str().as_bytes(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, MarketStateError> {
        let wire: OutcomeWire = decode_canonical(bytes)?;
        if wire.schema != OUTCOME_SCHEMA {
            return Err(MarketStateError::InvalidRecord);
        }
        let record = Self {
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| MarketStateError::InvalidRecord)?,
            outcome_id: OutcomeId::new(wire.outcome_id)
                .map_err(|_| MarketStateError::InvalidRecord)?,
            description: wire.description,
            settlement_value: parse_optional(&wire.settlement_value)?,
            resolved_at: parse_optional_time(wire.resolved_at_micros)?,
            created_at_block: BlockHeight::new(wire.created_at_block),
            updated_at_block: BlockHeight::new(wire.updated_at_block),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, MarketStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.market_id, &record.outcome_id)? != *key {
            return Err(MarketStateError::KeyMismatch);
        }
        Ok(record)
    }

    pub fn encode(&self) -> Result<Vec<u8>, MarketStateError> {
        self.validate()?;
        encode_canonical(&OutcomeWire {
            schema: OUTCOME_SCHEMA.to_owned(),
            market_id: self.market_id.as_str().to_owned(),
            outcome_id: self.outcome_id.as_str().to_owned(),
            description: self.description.clone(),
            settlement_value: display_optional(self.settlement_value),
            resolved_at_micros: self.resolved_at.map(ProtocolTime::unix_micros),
            created_at_block: self.created_at_block.get(),
            updated_at_block: self.updated_at_block.get(),
        })
    }

    #[must_use]
    pub const fn settlement_value(&self) -> Option<Price> {
        self.settlement_value
    }

    #[must_use]
    pub const fn resolved_at(&self) -> Option<ProtocolTime> {
        self.resolved_at
    }

    fn validate(&self) -> Result<(), MarketStateError> {
        if !valid_text(&self.description, 2_048)
            || self.settlement_value.is_some() != self.resolved_at.is_some()
            || self.settlement_value.is_some_and(|value| value.raw() < 0)
            || self.updated_at_block < self.created_at_block
        {
            Err(MarketStateError::InvalidRecord)
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn outcome_id(&self) -> &OutcomeId {
        &self.outcome_id
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MarketStateError {
    #[error("market-state key is invalid")]
    InvalidKey,
    #[error("market-state record cannot be decoded")]
    Codec,
    #[error("market-state record bytes are not canonical")]
    NonCanonical,
    #[error("market-state record is invalid")]
    InvalidRecord,
    #[error("market-state record identity does not match its key")]
    KeyMismatch,
    #[error("market-state record exceeds its deterministic bound")]
    LimitExceeded,
}

impl MarketStateError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidKey => "market_state.codec.invalid_key",
            Self::Codec => "market_state.codec.decode",
            Self::NonCanonical => "market_state.codec.noncanonical",
            Self::InvalidRecord => "market_state.codec.invalid_record",
            Self::KeyMismatch => "market_state.codec.key_mismatch",
            Self::LimitExceeded => "market_state.codec.limit_exceeded",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct FactWire {
    schema: String,
    event_id: String,
    event_kind: String,
    market_id: Option<String>,
    dex_id: Option<String>,
    asset_id: Option<String>,
    outcome_id: Option<String>,
    block_height: u64,
    payload_blake3: String,
    rule_version: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DexWire {
    schema: String,
    dex_id: String,
    name: String,
    operator_account_id: String,
    created_at_block: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AssetWire {
    schema: String,
    asset_id: String,
    context_version: String,
    context_blake3: String,
    updated_at_block: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarketWire {
    schema: String,
    market_id: String,
    dex_id: String,
    base_asset_id: String,
    quote_asset_id: String,
    status: String,
    metadata_resolution: String,
    metadata_version: String,
    metadata_blake3: String,
    tick_size: Option<String>,
    lot_size: Option<String>,
    price_scale: Option<u8>,
    quantity_scale: Option<u8>,
    open_interest_cap: Option<String>,
    margin_table_hash: Option<String>,
    oracle_price: Option<String>,
    oracle_source: Option<String>,
    oracle_effective_at_micros: Option<i64>,
    funding_rate: Option<String>,
    funding_effective_at_micros: Option<i64>,
    created_at_block: u64,
    updated_at_block: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MetadataWire {
    schema: String,
    market_id: String,
    metadata_version: String,
    metadata_blake3: String,
    effective_from_block: u64,
    effective_until_block: Option<u64>,
    resolution: String,
    tick_size: Option<String>,
    lot_size: Option<String>,
    price_scale: Option<u8>,
    quantity_scale: Option<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct OutcomeWire {
    schema: String,
    market_id: String,
    outcome_id: String,
    description: String,
    settlement_value: Option<String>,
    resolved_at_micros: Option<i64>,
    created_at_block: u64,
    updated_at_block: u64,
}

fn require_no_markets(event: &CanonicalEventEnvelope) -> Result<(), ReducerError> {
    if event.market_ids().is_empty() {
        Ok(())
    } else {
        Err(reducer_error(
            "market_state.invalid_market_identity",
            "event must not carry envelope markets",
        ))
    }
}

fn require_market(
    event: &CanonicalEventEnvelope,
    market_id: &MarketId,
) -> Result<(), ReducerError> {
    if event.market_ids() == std::slice::from_ref(market_id) {
        Ok(())
    } else {
        Err(reducer_error(
            "market_state.invalid_market_identity",
            "payload and envelope market must match exactly",
        ))
    }
}

fn require_accounts(
    event: &CanonicalEventEnvelope,
    expected: &[Address],
) -> Result<(), ReducerError> {
    if event.account_addresses() == expected {
        Ok(())
    } else {
        Err(reducer_error(
            "market_state.invalid_account_identity",
            "payload and envelope accounts must match exactly",
        ))
    }
}

fn require_record(
    state: &StateView<'_>,
    key: &StateKey,
    reason_code: &'static str,
    message: &'static str,
) -> Result<(), ReducerError> {
    if state.contains_key(key) {
        Ok(())
    } else {
        Err(reducer_error(reason_code, message))
    }
}

fn load_market(
    state: &StateView<'_>,
    key: &StateKey,
) -> Result<MarketCurrentRecordV1, ReducerError> {
    let bytes = state.get(key).ok_or_else(|| {
        reducer_error(
            "market_state.missing_market",
            "market prerequisite is missing",
        )
    })?;
    MarketCurrentRecordV1::decode_at(key, bytes).map_err(codec_reducer_error)
}

fn load_metadata(
    state: &StateView<'_>,
    key: &StateKey,
) -> Result<MarketMetadataVersionRecordV1, ReducerError> {
    let bytes = state.get(key).ok_or_else(|| {
        reducer_error(
            "market_state.missing_metadata",
            "market metadata prerequisite is missing",
        )
    })?;
    MarketMetadataVersionRecordV1::decode_at(key, bytes).map_err(codec_reducer_error)
}

fn require_exact_metadata(current: &MarketCurrentRecordV1) -> Result<(), ReducerError> {
    if current.metadata_resolution == MarketMetadataResolutionV1::Unresolved {
        Err(metadata_unresolved())
    } else {
        Ok(())
    }
}

fn same_fixed(left: QuoteAmount, right: QuoteAmount) -> bool {
    left.raw() == right.raw() && left.scale() == right.scale()
}

fn invalid_status_transition() -> ReducerError {
    reducer_error(
        "market_state.invalid_status_transition",
        "market status transition is not allowed",
    )
}

fn single_key(namespace: &str, identity: &[u8]) -> Result<StateKey, MarketStateError> {
    compound_key(namespace, &[identity])
}

fn compound_key(namespace: &str, identities: &[&[u8]]) -> Result<StateKey, MarketStateError> {
    let framed_len = identities.iter().try_fold(0_usize, |total, identity| {
        if identity.is_empty() {
            return Err(MarketStateError::InvalidKey);
        }
        total
            .checked_add(KEY_FRAME_BYTES)
            .and_then(|size| size.checked_add(identity.len()))
            .ok_or(MarketStateError::InvalidKey)
    })?;
    if framed_len > MAX_STATE_KEY_BYTES {
        return Err(MarketStateError::InvalidKey);
    }

    let mut key = Vec::new();
    key.try_reserve_exact(framed_len)
        .map_err(|_| MarketStateError::InvalidKey)?;
    for identity in identities {
        let length = u64::try_from(identity.len()).map_err(|_| MarketStateError::InvalidKey)?;
        key.extend_from_slice(&length.to_be_bytes());
        key.extend_from_slice(identity);
    }
    StateKey::try_new(namespace, key).map_err(|_| MarketStateError::InvalidKey)
}

fn encode_canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, MarketStateError> {
    let bytes = serde_json::to_vec(value).map_err(|_| MarketStateError::Codec)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(MarketStateError::LimitExceeded);
    }
    Ok(bytes)
}

fn decode_canonical<T>(bytes: &[u8]) -> Result<T, MarketStateError>
where
    T: DeserializeOwned + Serialize,
{
    if bytes.is_empty() || bytes.len() > MAX_RECORD_BYTES {
        return Err(MarketStateError::LimitExceeded);
    }
    let value = serde_json::from_slice(bytes).map_err(|_| MarketStateError::Codec)?;
    if encode_canonical(&value)? != bytes {
        return Err(MarketStateError::NonCanonical);
    }
    Ok(value)
}

fn decode_hash(value: &str) -> Result<[u8; 32], MarketStateError> {
    if value.len() != 64 || value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(MarketStateError::InvalidRecord);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| MarketStateError::InvalidRecord)?;
    Ok(hash)
}

fn parse_optional<T: FromStr>(value: &Option<String>) -> Result<Option<T>, MarketStateError> {
    value
        .as_deref()
        .map(str::parse)
        .transpose()
        .map_err(|_| MarketStateError::InvalidRecord)
}

fn parse_optional_time(value: Option<i64>) -> Result<Option<ProtocolTime>, MarketStateError> {
    value
        .map(ProtocolTime::from_unix_micros)
        .transpose()
        .map_err(|_| MarketStateError::InvalidRecord)
}

fn display_optional<T: ToString>(value: Option<T>) -> Option<String> {
    value.map(|item| item.to_string())
}

fn valid_text(value: &str, max_bytes: usize) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= max_bytes
        && !value.chars().any(char::is_control)
}

fn metadata_unresolved() -> ReducerError {
    reducer_error(
        "market_state.metadata_unresolved",
        "market metadata values are unresolved",
    )
}

fn reducer_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

fn codec_reducer_error(error: MarketStateError) -> ReducerError {
    match error {
        MarketStateError::InvalidKey => reducer_error(
            "market_state.codec_invalid_key",
            "market state key encoding failed",
        ),
        MarketStateError::LimitExceeded => reducer_error(
            "market_state.codec_limit_exceeded",
            "market state record exceeds its deterministic bound",
        ),
        MarketStateError::Codec
        | MarketStateError::NonCanonical
        | MarketStateError::InvalidRecord
        | MarketStateError::KeyMismatch => reducer_error(
            "market_state.codec_failed",
            "market state record encoding failed",
        ),
    }
}
