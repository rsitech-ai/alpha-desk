use std::collections::BTreeSet;
use std::str::FromStr;

use canonical_events::{
    BackstopLiquidation, CanonicalEventEnvelope, EventKind, EventPayload, LiquidationFill,
    LiquidationStarted, PositionSettled,
};
use domain_types::{
    Address, BlockHeight, EventId, LiquidationId, MarketId, PositionQuantity, Price, Quantity,
    QuoteAmount, RoundingMode, UsdAmount,
};
use serde::{Deserialize, Serialize};

use crate::{ApplyContext, EventReducer, ReducerError, StateKey, StateMutation, StateView};

use super::codec::{PositionStateError, decode_wire, encode_wire, require_record_bytes, state_key};
use super::episodes::{
    EpisodeCloseCauseV1, LoadedNonTradePair, NonTradeQuantityResult,
    PositionEpisodeEffectFactRecordV1, PositionEpisodeRecordV1, load_nontrade_pair,
    stage_nontrade_transition,
};
use super::quantity::PositionUnresolvedCauseFactRecordV1;

const CURRENT_NAMESPACE: &str = "liquidation-current.v1";
const CURRENT_SCHEMA: &str = "hyperliquid-alpha-desk/liquidation-current/v1";
const START_FACT_NAMESPACE: &str = "liquidation-start-fact.v1";
const START_FACT_SCHEMA: &str = "hyperliquid-alpha-desk/liquidation-start-fact/v1";
const FILL_FACT_NAMESPACE: &str = "liquidation-fill-fact.v1";
const FILL_FACT_SCHEMA: &str = "hyperliquid-alpha-desk/liquidation-fill-fact/v1";
const MARKET_FLOW_NAMESPACE: &str = "liquidation-market-flow-current.v1";
const MARKET_FLOW_SCHEMA: &str = "hyperliquid-alpha-desk/liquidation-market-flow-current/v1";
const BACKSTOP_FACT_NAMESPACE: &str = "backstop-liquidation-fact.v1";
const BACKSTOP_FACT_SCHEMA: &str = "hyperliquid-alpha-desk/backstop-liquidation-fact/v1";
const SETTLEMENT_FACT_NAMESPACE: &str = "position-settlement-fact.v1";
const SETTLEMENT_FACT_SCHEMA: &str = "hyperliquid-alpha-desk/position-settlement-fact/v1";

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalLiquidationReducerV1;

impl CanonicalLiquidationReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0";
}

impl EventReducer for CanonicalLiquidationReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        event.schema_version() == "1.0.0"
            && matches!(
                event.event_kind(),
                EventKind::LiquidationStarted
                    | EventKind::LiquidationFill
                    | EventKind::BackstopLiquidation
                    | EventKind::PositionSettled
            )
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        if !self.supports(event) {
            return Err(liquidation_error(
                "liquidation_state.unsupported_event",
                "liquidation reducer received an unsupported event",
            ));
        }
        match event.payload() {
            EventPayload::LiquidationStarted(payload) => reduce_start(state, event, payload),
            EventPayload::LiquidationFill(payload) => reduce_fill(state, event, payload),
            EventPayload::BackstopLiquidation(payload) => reduce_backstop(state, event, payload),
            EventPayload::PositionSettled(payload) => reduce_settlement(state, event, payload),
            _ => Err(liquidation_error(
                "liquidation_state.unsupported_event",
                "liquidation reducer received an unsupported payload",
            )),
        }
    }
}

fn exact_identity(
    event: &CanonicalEventEnvelope,
    kind: EventKind,
    markets: &[MarketId],
    accounts: &[Address],
) -> Result<(), ReducerError> {
    if event.event_kind() == kind
        && event.market_ids() == markets
        && event.account_addresses() == accounts
    {
        Ok(())
    } else {
        Err(liquidation_error(
            "liquidation_state.identity_mismatch",
            "liquidation envelope identity does not match its payload",
        ))
    }
}

fn reduce_start(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    payload: &LiquidationStarted,
) -> Result<Vec<StateMutation>, ReducerError> {
    exact_identity(
        event,
        EventKind::LiquidationStarted,
        &[],
        &[payload.account_id],
    )?;

    let current_key =
        LiquidationCurrentRecordV1::state_key(&payload.liquidation_id).map_err(|_| {
            liquidation_error(
                "liquidation_state.process_prior_invalid",
                "invalid process key",
            )
        })?;
    if let Some(bytes) = state.get(&current_key) {
        LiquidationCurrentRecordV1::decode_at(&current_key, bytes).map_err(|_| {
            liquidation_error(
                "liquidation_state.process_prior_invalid",
                "existing liquidation process is invalid",
            )
        })?;
        return Err(liquidation_error(
            "liquidation_state.process_identity_collision",
            "liquidation process identity already exists",
        ));
    }

    let fact = LiquidationStartFactRecordV1::try_new(
        payload.liquidation_id.clone(),
        event.event_id().clone(),
        payload.account_id,
        payload.margin_value,
        payload.maintenance_requirement,
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
        event.payload_hash(),
    )
    .map_err(|_| {
        liquidation_error(
            "liquidation_state.identity_mismatch",
            "liquidation start payload is invalid",
        )
    })?;
    let fact_key = LiquidationStartFactRecordV1::state_key(fact.liquidation_id(), fact.event_id())
        .map_err(|_| {
            liquidation_error(
                "liquidation_state.start_fact_prior_invalid",
                "invalid liquidation start fact key",
            )
        })?;
    if let Some(bytes) = state.get(&fact_key) {
        LiquidationStartFactRecordV1::decode_at(&fact_key, bytes).map_err(|_| {
            liquidation_error(
                "liquidation_state.start_fact_prior_invalid",
                "existing liquidation start fact is invalid",
            )
        })?;
        return Err(liquidation_error(
            "liquidation_state.start_fact_identity_collision",
            "liquidation start fact identity already exists",
        ));
    }

    let current = LiquidationCurrentRecordV1::try_new(
        payload.liquidation_id.clone(),
        payload.account_id,
        payload.margin_value,
        payload.maintenance_requirement,
        LiquidationObservedStatusV1::Started,
        event.event_id().clone(),
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
        None,
        event.event_id().clone(),
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
    )
    .map_err(|_| {
        liquidation_error(
            "liquidation_state.process_transition_invalid",
            "liquidation start process transition is invalid",
        )
    })?;

    Ok(vec![
        StateMutation::put(
            fact_key,
            fact.encode().map_err(|_| {
                liquidation_error(
                    "liquidation_state.start_fact_prior_invalid",
                    "could not encode liquidation start fact",
                )
            })?,
        ),
        StateMutation::put(
            current_key,
            current.encode().map_err(|_| {
                liquidation_error(
                    "liquidation_state.process_transition_invalid",
                    "could not encode liquidation process",
                )
            })?,
        ),
    ])
}

fn reduce_fill(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    payload: &LiquidationFill,
) -> Result<Vec<StateMutation>, ReducerError> {
    exact_identity(
        event,
        EventKind::LiquidationFill,
        std::slice::from_ref(&payload.market_id),
        &[payload.account_id],
    )?;
    let fact = LiquidationFillFactRecordV1::try_new(
        payload.liquidation_id.clone(),
        event.event_id().clone(),
        payload.account_id,
        payload.market_id.clone(),
        payload.price,
        payload.quantity,
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
        event.payload_hash(),
    )
    .map_err(|_| identity_mismatch())?;
    let fact_key = LiquidationFillFactRecordV1::state_key(fact.liquidation_id(), fact.event_id())
        .map_err(|_| fill_fact_prior_invalid())?;
    reject_fill_fact(state, &fact_key)?;

    let process = load_process(state, &payload.liquidation_id)?;
    validate_process_observation(&process, payload.account_id, event)?;
    let process_mutation = updated_process_mutation(&process, event, false)?;

    let flow_key = LiquidationMarketFlowCurrentRecordV1::state_key(
        &payload.liquidation_id,
        &payload.account_id,
        &payload.market_id,
    )
    .map_err(|_| flow_prior_invalid())?;
    let prior_flow = state
        .get(&flow_key)
        .map(|bytes| {
            LiquidationMarketFlowCurrentRecordV1::decode_at(&flow_key, bytes)
                .map_err(|_| flow_prior_invalid())
        })
        .transpose()?;
    if let Some(flow) = prior_flow.as_ref() {
        validate_flow_observation(flow, &process, event)?;
    }
    let loaded = load_nontrade_pair(state, &payload.account_id, &payload.market_id)?;
    let result = fill_result(&loaded, payload.quantity)?;
    let flow = updated_flow(prior_flow.as_ref(), payload, event)?;
    let mut account_mutations = stage_nontrade_transition(
        event,
        payload.account_id,
        &payload.market_id,
        loaded,
        result,
        EpisodeCloseCauseV1::LiquidationFill,
        payload.quantity.scale().max(match result {
            NonTradeQuantityResult::Exact(value) => value.scale(),
            NonTradeQuantityResult::Ambiguous => payload.quantity.scale(),
        }),
    )?;
    validate_episode_secondary_collisions(state, event, &account_mutations)?;

    let mut mutations = vec![
        StateMutation::put(
            fact_key,
            fact.encode().map_err(|_| fill_fact_prior_invalid())?,
        ),
        StateMutation::put(flow_key, flow.encode().map_err(|_| flow_prior_invalid())?),
        process_mutation,
    ];
    mutations.append(&mut account_mutations);
    ensure_unique_liquidation_keys(&mutations)?;
    Ok(mutations)
}

fn reduce_backstop(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    payload: &BackstopLiquidation,
) -> Result<Vec<StateMutation>, ReducerError> {
    exact_identity(
        event,
        EventKind::BackstopLiquidation,
        std::slice::from_ref(&payload.market_id),
        &[payload.account_id, payload.backstop_account_id],
    )?;
    let fact = BackstopLiquidationFactRecordV1::try_new(
        payload.liquidation_id.clone(),
        event.event_id().clone(),
        payload.account_id,
        payload.backstop_account_id,
        payload.market_id.clone(),
        payload.quantity,
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
        event.payload_hash(),
    )
    .map_err(|_| identity_mismatch())?;
    let fact_key =
        BackstopLiquidationFactRecordV1::state_key(fact.liquidation_id(), fact.event_id())
            .map_err(|_| backstop_fact_prior_invalid())?;
    reject_backstop_fact(state, &fact_key)?;
    let process = load_process(state, &payload.liquidation_id)?;
    validate_process_observation(&process, payload.account_id, event)?;
    let process_mutation = updated_process_mutation(&process, event, true)?;

    let accounts = [payload.account_id, payload.backstop_account_id];
    let loaded = [
        load_nontrade_pair(state, &accounts[0], &payload.market_id)?,
        load_nontrade_pair(state, &accounts[1], &payload.market_id)?,
    ];
    let mut staged_bundles = Vec::new();
    for (account, pair) in accounts.into_iter().zip(loaded) {
        staged_bundles.push(stage_nontrade_transition(
            event,
            account,
            &payload.market_id,
            pair,
            NonTradeQuantityResult::Ambiguous,
            EpisodeCloseCauseV1::BackstopInterrupted,
            payload.quantity.scale(),
        )?);
    }

    let mut bundles = Vec::new();
    for (account, staged) in accounts.into_iter().zip(staged_bundles) {
        let cause = PositionUnresolvedCauseFactRecordV1::backstop_liquidation(
            account,
            payload.market_id.clone(),
            event.event_id().clone(),
            payload.liquidation_id.clone(),
        );
        let cause_key = PositionUnresolvedCauseFactRecordV1::state_key(
            &account,
            &payload.market_id,
            event.event_id(),
            &payload.liquidation_id,
        )
        .map_err(|_| unresolved_prior_invalid())?;
        if let Some(bytes) = state.get(&cause_key) {
            PositionUnresolvedCauseFactRecordV1::decode_at(&cause_key, bytes)
                .map_err(|_| unresolved_prior_invalid())?;
            return Err(liquidation_error(
                "liquidation_state.unresolved_identity_collision",
                "position unresolved cause identity already exists",
            ));
        }
        validate_episode_secondary_collisions(state, event, &staged)?;
        let mut bundle = vec![StateMutation::put(
            cause_key,
            cause.encode().map_err(|_| unresolved_prior_invalid())?,
        )];
        bundle.extend(staged);
        bundles.push(bundle);
    }

    let mut mutations = vec![
        StateMutation::put(
            fact_key,
            fact.encode().map_err(|_| backstop_fact_prior_invalid())?,
        ),
        process_mutation,
    ];
    for mut bundle in bundles {
        mutations.append(&mut bundle);
    }
    ensure_unique_liquidation_keys(&mutations)?;
    Ok(mutations)
}

fn reduce_settlement(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    payload: &PositionSettled,
) -> Result<Vec<StateMutation>, ReducerError> {
    exact_identity(
        event,
        EventKind::PositionSettled,
        std::slice::from_ref(&payload.market_id),
        &[payload.account_id],
    )?;
    let fact = PositionSettlementFactRecordV1::try_new(
        event.event_id().clone(),
        payload.account_id,
        payload.market_id.clone(),
        payload.settlement_price,
        payload.settled_quantity,
        payload.realized_pnl,
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
        event.payload_hash(),
    )
    .map_err(|_| identity_mismatch())?;
    let fact_key = PositionSettlementFactRecordV1::state_key(
        fact.event_id(),
        &payload.account_id,
        &payload.market_id,
    )
    .map_err(|_| settlement_fact_prior_invalid())?;
    reject_settlement_fact(state, &fact_key)?;
    let loaded = load_nontrade_pair(state, &payload.account_id, &payload.market_id)?;
    let result = settlement_result(&loaded, payload.settled_quantity)?;
    let zero_scale = payload.settled_quantity.scale().max(match result {
        NonTradeQuantityResult::Exact(value) => value.scale(),
        NonTradeQuantityResult::Ambiguous => payload.settled_quantity.scale(),
    });
    let mut account_mutations = stage_nontrade_transition(
        event,
        payload.account_id,
        &payload.market_id,
        loaded,
        result,
        EpisodeCloseCauseV1::Settlement,
        zero_scale,
    )?;
    validate_episode_secondary_collisions(state, event, &account_mutations)?;
    let mut mutations = vec![StateMutation::put(
        fact_key,
        fact.encode().map_err(|_| settlement_fact_prior_invalid())?,
    )];
    mutations.append(&mut account_mutations);
    ensure_unique_liquidation_keys(&mutations)?;
    Ok(mutations)
}

fn fill_result(
    loaded: &LoadedNonTradePair,
    quantity: Quantity,
) -> Result<NonTradeQuantityResult, ReducerError> {
    if loaded.is_known_zero() {
        return Err(liquidation_error(
            "liquidation_state.fill_overrun",
            "liquidation fill overruns a known position",
        ));
    }
    match loaded.known_quantity() {
        Some(position) if position.raw() != 0 => toward_zero_result(position, quantity, true),
        _ => Ok(NonTradeQuantityResult::Ambiguous),
    }
}

fn settlement_result(
    loaded: &LoadedNonTradePair,
    quantity: Quantity,
) -> Result<NonTradeQuantityResult, ReducerError> {
    match loaded.known_quantity() {
        Some(position) if position.raw() != 0 => toward_zero_result(position, quantity, false),
        _ => Ok(NonTradeQuantityResult::Ambiguous),
    }
}

fn toward_zero_result(
    position: PositionQuantity,
    quantity: Quantity,
    reject_overrun: bool,
) -> Result<NonTradeQuantityResult, ReducerError> {
    let scale = position.scale().max(quantity.scale());
    let position = position
        .checked_rescale_up(scale)
        .map_err(|_| quantity_arithmetic())?;
    let quantity = quantity
        .rescale(scale, RoundingMode::TowardZero)
        .map_err(|_| quantity_arithmetic())?;
    let unsigned =
        PositionQuantity::from_raw(quantity.raw(), scale).map_err(|_| quantity_arithmetic())?;
    let candidate = if position.raw() > 0 {
        position.checked_sub(unsigned)
    } else {
        position.checked_add(unsigned)
    }
    .map_err(|_| quantity_arithmetic())?;
    let overrun =
        (position.raw() > 0 && candidate.raw() < 0) || (position.raw() < 0 && candidate.raw() > 0);
    if overrun {
        if reject_overrun {
            Err(liquidation_error(
                "liquidation_state.fill_overrun",
                "liquidation fill overruns a known position",
            ))
        } else {
            Ok(NonTradeQuantityResult::Ambiguous)
        }
    } else {
        Ok(NonTradeQuantityResult::Exact(candidate))
    }
}

fn load_process(
    state: &StateView<'_>,
    liquidation_id: &LiquidationId,
) -> Result<LiquidationCurrentRecordV1, ReducerError> {
    let key = LiquidationCurrentRecordV1::state_key(liquidation_id)
        .map_err(|_| process_prior_invalid())?;
    let bytes = state.get(&key).ok_or_else(|| {
        liquidation_error(
            "liquidation_state.process_missing",
            "liquidation process is missing",
        )
    })?;
    LiquidationCurrentRecordV1::decode_at(&key, bytes).map_err(|_| process_prior_invalid())
}

fn validate_process_observation(
    process: &LiquidationCurrentRecordV1,
    account_id: Address,
    event: &CanonicalEventEnvelope,
) -> Result<(), ReducerError> {
    if process.account_id() != account_id {
        return Err(liquidation_error(
            "liquidation_state.process_account_mismatch",
            "liquidation process account does not match event",
        ));
    }
    let incoming = EventPosition::new(
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
    );
    let prior = EventPosition::new(
        process.last_observation_block_height(),
        process.last_observation_transaction_index(),
        process.last_observation_canonical_event_index(),
    );
    if event.event_id() == process.last_observation_event_id() || incoming <= prior {
        return Err(liquidation_error(
            "liquidation_state.process_provenance_regression",
            "liquidation process provenance did not advance",
        ));
    }
    Ok(())
}

fn validate_flow_observation(
    flow: &LiquidationMarketFlowCurrentRecordV1,
    process: &LiquidationCurrentRecordV1,
    event: &CanonicalEventEnvelope,
) -> Result<(), ReducerError> {
    let flow_position = EventPosition::new(
        flow.last_fill_block_height(),
        flow.last_fill_transaction_index(),
        flow.last_fill_canonical_event_index(),
    );
    let process_position = EventPosition::new(
        process.last_observation_block_height(),
        process.last_observation_transaction_index(),
        process.last_observation_canonical_event_index(),
    );
    let coherent_with_process = if flow.last_fill_event_id() == process.last_observation_event_id()
    {
        flow_position == process_position
    } else {
        flow_position < process_position
    };
    let incoming_position = EventPosition::new(
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
    );
    if !coherent_with_process
        || event.event_id() == flow.last_fill_event_id()
        || incoming_position <= flow_position
    {
        return Err(flow_prior_invalid());
    }
    Ok(())
}

fn updated_process_mutation(
    process: &LiquidationCurrentRecordV1,
    event: &CanonicalEventEnvelope,
    backstop: bool,
) -> Result<StateMutation, ReducerError> {
    let first_backstop = if backstop && process.first_backstop_event_id().is_none() {
        Some((
            event.event_id().clone(),
            event.block_height(),
            event.transaction_index(),
            event.canonical_event_index(),
        ))
    } else {
        process.first_backstop_event_id().cloned().map(|event_id| {
            (
                event_id,
                process
                    .first_backstop_block_height()
                    .expect("validated backstop position"),
                process
                    .first_backstop_transaction_index()
                    .expect("validated backstop position"),
                process
                    .first_backstop_canonical_event_index()
                    .expect("validated backstop position"),
            )
        })
    };
    let status = if backstop {
        LiquidationObservedStatusV1::BackstopObserved
    } else {
        process.observed_status()
    };
    let updated = LiquidationCurrentRecordV1::try_new(
        process.liquidation_id().clone(),
        process.account_id(),
        process.start_margin_value(),
        process.start_maintenance_requirement(),
        status,
        process.start_event_id().clone(),
        process.start_block_height(),
        process.start_transaction_index(),
        process.start_canonical_event_index(),
        first_backstop,
        event.event_id().clone(),
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
    )
    .map_err(|_| {
        liquidation_error(
            "liquidation_state.process_transition_invalid",
            "liquidation process transition is invalid",
        )
    })?;
    let key = LiquidationCurrentRecordV1::state_key(updated.liquidation_id())
        .map_err(|_| process_prior_invalid())?;
    Ok(StateMutation::put(
        key,
        updated.encode().map_err(|_| process_prior_invalid())?,
    ))
}

fn updated_flow(
    prior: Option<&LiquidationMarketFlowCurrentRecordV1>,
    payload: &LiquidationFill,
    event: &CanonicalEventEnvelope,
) -> Result<LiquidationMarketFlowCurrentRecordV1, ReducerError> {
    let (quantity, first_event, first_height, first_transaction, first_index) = match prior {
        Some(prior) => {
            let scale = prior
                .observed_filled_quantity()
                .scale()
                .max(payload.quantity.scale());
            let left = prior
                .observed_filled_quantity()
                .rescale(scale, RoundingMode::TowardZero)
                .map_err(|_| flow_arithmetic())?;
            let right = payload
                .quantity
                .rescale(scale, RoundingMode::TowardZero)
                .map_err(|_| flow_arithmetic())?;
            (
                left.checked_add(right).map_err(|_| flow_arithmetic())?,
                prior.first_fill_event_id().clone(),
                prior.first_fill_block_height(),
                prior.first_fill_transaction_index(),
                prior.first_fill_canonical_event_index(),
            )
        }
        None => (
            payload.quantity,
            event.event_id().clone(),
            event.block_height(),
            event.transaction_index(),
            event.canonical_event_index(),
        ),
    };
    LiquidationMarketFlowCurrentRecordV1::try_new(
        payload.liquidation_id.clone(),
        payload.account_id,
        payload.market_id.clone(),
        quantity,
        first_event,
        first_height,
        first_transaction,
        first_index,
        event.event_id().clone(),
        event.block_height(),
        event.transaction_index(),
        event.canonical_event_index(),
    )
    .map_err(|_| flow_arithmetic())
}

fn reject_fill_fact(state: &StateView<'_>, key: &StateKey) -> Result<(), ReducerError> {
    if let Some(bytes) = state.get(key) {
        LiquidationFillFactRecordV1::decode_at(key, bytes)
            .map_err(|_| fill_fact_prior_invalid())?;
        return Err(liquidation_error(
            "liquidation_state.fill_fact_identity_collision",
            "liquidation fill fact identity already exists",
        ));
    }
    Ok(())
}

fn reject_backstop_fact(state: &StateView<'_>, key: &StateKey) -> Result<(), ReducerError> {
    if let Some(bytes) = state.get(key) {
        BackstopLiquidationFactRecordV1::decode_at(key, bytes)
            .map_err(|_| backstop_fact_prior_invalid())?;
        return Err(liquidation_error(
            "liquidation_state.backstop_fact_identity_collision",
            "backstop fact identity already exists",
        ));
    }
    Ok(())
}

fn reject_settlement_fact(state: &StateView<'_>, key: &StateKey) -> Result<(), ReducerError> {
    if let Some(bytes) = state.get(key) {
        PositionSettlementFactRecordV1::decode_at(key, bytes)
            .map_err(|_| settlement_fact_prior_invalid())?;
        return Err(liquidation_error(
            "liquidation_state.settlement_fact_identity_collision",
            "position settlement fact identity already exists",
        ));
    }
    Ok(())
}

fn validate_episode_secondary_collisions(
    state: &StateView<'_>,
    event: &CanonicalEventEnvelope,
    mutations: &[StateMutation],
) -> Result<(), ReducerError> {
    for mutation in mutations {
        let key = mutation.key();
        if key.namespace() == "position-episode-effect-fact.v1" {
            let Some(existing) = state.get(key) else {
                continue;
            };
            PositionEpisodeEffectFactRecordV1::decode_at(key, existing)
                .map_err(|_| episode_effect_prior_invalid())?;
            return Err(liquidation_error(
                "liquidation_state.episode_effect_identity_collision",
                "position episode effect identity already exists",
            ));
        }
        if key.namespace() != "position-episode.v1" {
            continue;
        }
        let Some(proposed_bytes) = mutation.value() else {
            continue;
        };
        let proposed = PositionEpisodeRecordV1::decode_at(key, proposed_bytes)
            .map_err(|_| episode_prior_invalid())?;
        if proposed.opening_anchor_event_id() != event.event_id()
            || proposed.opening_leg_ordinal() != 1
        {
            continue;
        }
        let Some(existing) = state.get(key) else {
            continue;
        };
        PositionEpisodeRecordV1::decode_at(key, existing).map_err(|_| episode_prior_invalid())?;
        return Err(liquidation_error(
            "liquidation_state.episode_identity_collision",
            "position episode identity already exists",
        ));
    }
    Ok(())
}

fn ensure_unique_liquidation_keys(mutations: &[StateMutation]) -> Result<(), ReducerError> {
    let mut keys = BTreeSet::new();
    if mutations.iter().all(|mutation| keys.insert(mutation.key())) {
        Ok(())
    } else {
        Err(liquidation_error(
            "liquidation_state.duplicate_mutation_key",
            "liquidation reducer emitted duplicate mutation keys",
        ))
    }
}

fn identity_mismatch() -> ReducerError {
    liquidation_error(
        "liquidation_state.identity_mismatch",
        "liquidation payload is invalid",
    )
}

fn fill_fact_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.fill_fact_prior_invalid",
        "prior liquidation fill fact is invalid",
    )
}

fn backstop_fact_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.backstop_fact_prior_invalid",
        "prior backstop fact is invalid",
    )
}

fn settlement_fact_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.settlement_fact_prior_invalid",
        "prior settlement fact is invalid",
    )
}

fn process_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.process_prior_invalid",
        "prior liquidation process is invalid",
    )
}

fn flow_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.flow_prior_invalid",
        "prior liquidation flow is invalid",
    )
}

fn quantity_arithmetic() -> ReducerError {
    liquidation_error(
        "liquidation_state.quantity_arithmetic",
        "liquidation position arithmetic failed",
    )
}

fn flow_arithmetic() -> ReducerError {
    liquidation_error(
        "liquidation_state.flow_arithmetic",
        "liquidation flow arithmetic failed",
    )
}

fn unresolved_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.unresolved_prior_invalid",
        "prior unresolved cause fact is invalid",
    )
}

fn episode_effect_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.episode_effect_prior_invalid",
        "prior position episode effect is invalid",
    )
}

fn episode_prior_invalid() -> ReducerError {
    liquidation_error(
        "liquidation_state.episode_prior_invalid",
        "prior position episode is invalid",
    )
}

fn liquidation_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidationObservedStatusV1 {
    Started,
    BackstopObserved,
}

impl LiquidationObservedStatusV1 {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::Started => "started",
            Self::BackstopObserved => "backstop_observed",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "started" => Ok(Self::Started),
            "backstop_observed" => Ok(Self::BackstopObserved),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiquidationSourceValueResolutionV1 {
    UnavailableFromSource,
}

impl LiquidationSourceValueResolutionV1 {
    const fn as_wire(self) -> &'static str {
        match self {
            Self::UnavailableFromSource => "unavailable_from_source",
        }
    }

    fn parse(value: &str) -> Result<Self, PositionStateError> {
        match value {
            "unavailable_from_source" => Ok(Self::UnavailableFromSource),
            _ => Err(PositionStateError::InvalidRecord),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct EventPosition {
    block_height: BlockHeight,
    transaction_index: u32,
    canonical_event_index: u32,
}

impl EventPosition {
    const fn new(
        block_height: BlockHeight,
        transaction_index: u32,
        canonical_event_index: u32,
    ) -> Self {
        Self {
            block_height,
            transaction_index,
            canonical_event_index,
        }
    }
}

fn validate_identity_position_pair(
    first_id: &EventId,
    first: EventPosition,
    last_id: &EventId,
    last: EventPosition,
) -> Result<(), PositionStateError> {
    if (first_id == last_id && first == last) || (first_id != last_id && first < last) {
        Ok(())
    } else {
        Err(PositionStateError::InvalidRecord)
    }
}

fn parse_payload_blake3(value: &str) -> Result<[u8; 32], PositionStateError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !(byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    {
        return Err(PositionStateError::InvalidRecord);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| PositionStateError::InvalidRecord)?;
    Ok(hash)
}

fn encode_payload_blake3(value: [u8; 32]) -> String {
    hex::encode(value)
}

fn validate_rule_version(value: &str) -> Result<(), PositionStateError> {
    if value == CanonicalLiquidationReducerV1::VERSION {
        Ok(())
    } else {
        Err(PositionStateError::InvalidRecord)
    }
}

fn validate_start_margin(
    margin_value: UsdAmount,
    maintenance_requirement: UsdAmount,
) -> Result<(), PositionStateError> {
    if margin_value.raw() < 0
        || maintenance_requirement.raw() < 0
        || margin_value.scale() != maintenance_requirement.scale()
        || margin_value.raw() >= maintenance_requirement.raw()
    {
        Err(PositionStateError::InvalidRecord)
    } else {
        Ok(())
    }
}

macro_rules! provenance_getters {
    () => {
        #[must_use]
        pub const fn block_height(&self) -> BlockHeight {
            self.position.block_height
        }

        #[must_use]
        pub const fn transaction_index(&self) -> u32 {
            self.position.transaction_index
        }

        #[must_use]
        pub const fn canonical_event_index(&self) -> u32 {
            self.position.canonical_event_index
        }

        #[must_use]
        pub fn rule_version(&self) -> &str {
            &self.rule_version
        }
    };
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidationCurrentRecordV1 {
    liquidation_id: LiquidationId,
    account_id: Address,
    start_margin_value: UsdAmount,
    start_maintenance_requirement: UsdAmount,
    observed_status: LiquidationObservedStatusV1,
    start_event_id: EventId,
    start_position: EventPosition,
    first_backstop_event_id: Option<EventId>,
    first_backstop_position: Option<EventPosition>,
    last_observation_event_id: EventId,
    last_observation_position: EventPosition,
    rule_version: String,
}

impl LiquidationCurrentRecordV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        liquidation_id: LiquidationId,
        account_id: Address,
        start_margin_value: UsdAmount,
        start_maintenance_requirement: UsdAmount,
        observed_status: LiquidationObservedStatusV1,
        start_event_id: EventId,
        start_block_height: BlockHeight,
        start_transaction_index: u32,
        start_canonical_event_index: u32,
        first_backstop: Option<(EventId, BlockHeight, u32, u32)>,
        last_observation_event_id: EventId,
        last_observation_block_height: BlockHeight,
        last_observation_transaction_index: u32,
        last_observation_canonical_event_index: u32,
    ) -> Result<Self, PositionStateError> {
        let (first_backstop_event_id, first_backstop_position) = match first_backstop {
            Some((event_id, height, transaction_index, canonical_event_index)) => (
                Some(event_id),
                Some(EventPosition::new(
                    height,
                    transaction_index,
                    canonical_event_index,
                )),
            ),
            None => (None, None),
        };
        let record = Self {
            liquidation_id,
            account_id,
            start_margin_value,
            start_maintenance_requirement,
            observed_status,
            start_event_id,
            start_position: EventPosition::new(
                start_block_height,
                start_transaction_index,
                start_canonical_event_index,
            ),
            first_backstop_event_id,
            first_backstop_position,
            last_observation_event_id,
            last_observation_position: EventPosition::new(
                last_observation_block_height,
                last_observation_transaction_index,
                last_observation_canonical_event_index,
            ),
            rule_version: CanonicalLiquidationReducerV1::VERSION.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(liquidation_id: &LiquidationId) -> Result<StateKey, PositionStateError> {
        state_key(CURRENT_NAMESPACE, &[liquidation_id.as_str().as_bytes()])
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: LiquidationCurrentWire = decode_wire(bytes)?;
        if wire.schema != CURRENT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let first_backstop_position = optional_position(
            wire.first_backstop_block_height,
            wire.first_backstop_transaction_index,
            wire.first_backstop_canonical_event_index,
        )?;
        let record = Self {
            liquidation_id: LiquidationId::new(wire.liquidation_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            start_margin_value: UsdAmount::from_str(&wire.start_margin_value)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            start_maintenance_requirement: UsdAmount::from_str(&wire.start_maintenance_requirement)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            observed_status: LiquidationObservedStatusV1::parse(&wire.observed_status)?,
            start_event_id: EventId::new(wire.start_event_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            start_position: EventPosition::new(
                BlockHeight::new(wire.start_block_height),
                wire.start_transaction_index,
                wire.start_canonical_event_index,
            ),
            first_backstop_event_id: wire
                .first_backstop_event_id
                .map(EventId::new)
                .transpose()
                .map_err(|_| PositionStateError::InvalidRecord)?,
            first_backstop_position,
            last_observation_event_id: EventId::new(wire.last_observation_event_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            last_observation_position: EventPosition::new(
                BlockHeight::new(wire.last_observation_block_height),
                wire.last_observation_transaction_index,
                wire.last_observation_canonical_event_index,
            ),
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.liquidation_id)? == *key {
            Ok(record)
        } else {
            Err(PositionStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&LiquidationCurrentWire {
            schema: CURRENT_SCHEMA.to_owned(),
            liquidation_id: self.liquidation_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            start_margin_value: self.start_margin_value.to_string(),
            start_maintenance_requirement: self.start_maintenance_requirement.to_string(),
            observed_status: self.observed_status.as_wire().to_owned(),
            start_event_id: self.start_event_id.as_str().to_owned(),
            start_block_height: self.start_position.block_height.get(),
            start_transaction_index: self.start_position.transaction_index,
            start_canonical_event_index: self.start_position.canonical_event_index,
            first_backstop_event_id: self
                .first_backstop_event_id
                .as_ref()
                .map(|value| value.as_str().to_owned()),
            first_backstop_block_height: self
                .first_backstop_position
                .map(|value| value.block_height.get()),
            first_backstop_transaction_index: self
                .first_backstop_position
                .map(|value| value.transaction_index),
            first_backstop_canonical_event_index: self
                .first_backstop_position
                .map(|value| value.canonical_event_index),
            last_observation_event_id: self.last_observation_event_id.as_str().to_owned(),
            last_observation_block_height: self.last_observation_position.block_height.get(),
            last_observation_transaction_index: self.last_observation_position.transaction_index,
            last_observation_canonical_event_index: self
                .last_observation_position
                .canonical_event_index,
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        validate_rule_version(&self.rule_version)?;
        validate_start_margin(self.start_margin_value, self.start_maintenance_requirement)?;
        validate_identity_position_pair(
            &self.start_event_id,
            self.start_position,
            &self.last_observation_event_id,
            self.last_observation_position,
        )?;
        match (
            self.observed_status,
            self.first_backstop_event_id.as_ref(),
            self.first_backstop_position,
        ) {
            (LiquidationObservedStatusV1::Started, None, None) => Ok(()),
            (
                LiquidationObservedStatusV1::BackstopObserved,
                Some(backstop_id),
                Some(backstop_position),
            ) => {
                if self.start_event_id == *backstop_id || self.start_position >= backstop_position {
                    return Err(PositionStateError::InvalidRecord);
                }
                validate_identity_position_pair(
                    backstop_id,
                    backstop_position,
                    &self.last_observation_event_id,
                    self.last_observation_position,
                )
            }
            _ => Err(PositionStateError::InvalidRecord),
        }
    }

    #[must_use]
    pub const fn observed_status(&self) -> LiquidationObservedStatusV1 {
        self.observed_status
    }

    #[must_use]
    pub const fn start_margin_value(&self) -> UsdAmount {
        self.start_margin_value
    }

    #[must_use]
    pub const fn start_maintenance_requirement(&self) -> UsdAmount {
        self.start_maintenance_requirement
    }

    #[must_use]
    pub const fn first_backstop_event_id(&self) -> Option<&EventId> {
        self.first_backstop_event_id.as_ref()
    }

    #[must_use]
    pub const fn liquidation_id(&self) -> &LiquidationId {
        &self.liquidation_id
    }

    #[must_use]
    pub const fn account_id(&self) -> Address {
        self.account_id
    }

    #[must_use]
    pub const fn start_event_id(&self) -> &EventId {
        &self.start_event_id
    }

    #[must_use]
    pub const fn start_block_height(&self) -> BlockHeight {
        self.start_position.block_height
    }

    #[must_use]
    pub const fn start_transaction_index(&self) -> u32 {
        self.start_position.transaction_index
    }

    #[must_use]
    pub const fn start_canonical_event_index(&self) -> u32 {
        self.start_position.canonical_event_index
    }

    #[must_use]
    pub const fn first_backstop_block_height(&self) -> Option<BlockHeight> {
        match self.first_backstop_position {
            Some(value) => Some(value.block_height),
            None => None,
        }
    }

    #[must_use]
    pub const fn first_backstop_transaction_index(&self) -> Option<u32> {
        match self.first_backstop_position {
            Some(value) => Some(value.transaction_index),
            None => None,
        }
    }

    #[must_use]
    pub const fn first_backstop_canonical_event_index(&self) -> Option<u32> {
        match self.first_backstop_position {
            Some(value) => Some(value.canonical_event_index),
            None => None,
        }
    }

    #[must_use]
    pub const fn last_observation_event_id(&self) -> &EventId {
        &self.last_observation_event_id
    }

    #[must_use]
    pub const fn last_observation_block_height(&self) -> BlockHeight {
        self.last_observation_position.block_height
    }

    #[must_use]
    pub const fn last_observation_transaction_index(&self) -> u32 {
        self.last_observation_position.transaction_index
    }

    #[must_use]
    pub const fn last_observation_canonical_event_index(&self) -> u32 {
        self.last_observation_position.canonical_event_index
    }

    #[must_use]
    pub fn rule_version(&self) -> &str {
        &self.rule_version
    }
}

fn optional_position(
    block_height: Option<u64>,
    transaction_index: Option<u32>,
    canonical_event_index: Option<u32>,
) -> Result<Option<EventPosition>, PositionStateError> {
    match (block_height, transaction_index, canonical_event_index) {
        (None, None, None) => Ok(None),
        (Some(height), Some(transaction), Some(event)) => Ok(Some(EventPosition::new(
            BlockHeight::new(height),
            transaction,
            event,
        ))),
        _ => Err(PositionStateError::InvalidRecord),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidationStartFactRecordV1 {
    liquidation_id: LiquidationId,
    event_id: EventId,
    account_id: Address,
    margin_value: UsdAmount,
    maintenance_requirement: UsdAmount,
    position: EventPosition,
    payload_blake3: [u8; 32],
    rule_version: String,
}

impl LiquidationStartFactRecordV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        liquidation_id: LiquidationId,
        event_id: EventId,
        account_id: Address,
        margin_value: UsdAmount,
        maintenance_requirement: UsdAmount,
        block_height: BlockHeight,
        transaction_index: u32,
        canonical_event_index: u32,
        payload_blake3: [u8; 32],
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            liquidation_id,
            event_id,
            account_id,
            margin_value,
            maintenance_requirement,
            position: EventPosition::new(block_height, transaction_index, canonical_event_index),
            payload_blake3,
            rule_version: CanonicalLiquidationReducerV1::VERSION.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(
        liquidation_id: &LiquidationId,
        event_id: &EventId,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            START_FACT_NAMESPACE,
            &[
                liquidation_id.as_str().as_bytes(),
                event_id.as_str().as_bytes(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: LiquidationStartFactWire = decode_wire(bytes)?;
        if wire.schema != START_FACT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            liquidation_id: LiquidationId::new(wire.liquidation_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            event_id: EventId::new(wire.event_id).map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            margin_value: UsdAmount::from_str(&wire.margin_value)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            maintenance_requirement: UsdAmount::from_str(&wire.maintenance_requirement)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            position: EventPosition::new(
                BlockHeight::new(wire.block_height),
                wire.transaction_index,
                wire.canonical_event_index,
            ),
            payload_blake3: parse_payload_blake3(&wire.payload_blake3)?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.liquidation_id, &record.event_id)? == *key {
            Ok(record)
        } else {
            Err(PositionStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&LiquidationStartFactWire {
            schema: START_FACT_SCHEMA.to_owned(),
            liquidation_id: self.liquidation_id.as_str().to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            margin_value: self.margin_value.to_string(),
            maintenance_requirement: self.maintenance_requirement.to_string(),
            block_height: self.position.block_height.get(),
            transaction_index: self.position.transaction_index,
            canonical_event_index: self.position.canonical_event_index,
            payload_blake3: encode_payload_blake3(self.payload_blake3),
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        validate_rule_version(&self.rule_version)?;
        validate_start_margin(self.margin_value, self.maintenance_requirement)
    }

    #[must_use]
    pub const fn payload_blake3(&self) -> &[u8; 32] {
        &self.payload_blake3
    }

    #[must_use]
    pub const fn liquidation_id(&self) -> &LiquidationId {
        &self.liquidation_id
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
    pub const fn margin_value(&self) -> UsdAmount {
        self.margin_value
    }

    #[must_use]
    pub const fn maintenance_requirement(&self) -> UsdAmount {
        self.maintenance_requirement
    }

    provenance_getters!();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidationFillFactRecordV1 {
    liquidation_id: LiquidationId,
    event_id: EventId,
    account_id: Address,
    market_id: MarketId,
    price: Price,
    quantity: Quantity,
    position: EventPosition,
    payload_blake3: [u8; 32],
    rule_version: String,
}

impl LiquidationFillFactRecordV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        liquidation_id: LiquidationId,
        event_id: EventId,
        account_id: Address,
        market_id: MarketId,
        price: Price,
        quantity: Quantity,
        block_height: BlockHeight,
        transaction_index: u32,
        canonical_event_index: u32,
        payload_blake3: [u8; 32],
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            liquidation_id,
            event_id,
            account_id,
            market_id,
            price,
            quantity,
            position: EventPosition::new(block_height, transaction_index, canonical_event_index),
            payload_blake3,
            rule_version: CanonicalLiquidationReducerV1::VERSION.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(
        liquidation_id: &LiquidationId,
        event_id: &EventId,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            FILL_FACT_NAMESPACE,
            &[
                liquidation_id.as_str().as_bytes(),
                event_id.as_str().as_bytes(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: LiquidationFillFactWire = decode_wire(bytes)?;
        if wire.schema != FILL_FACT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            liquidation_id: LiquidationId::new(wire.liquidation_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            event_id: EventId::new(wire.event_id).map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            price: Price::from_str(&wire.price).map_err(|_| PositionStateError::InvalidRecord)?,
            quantity: Quantity::from_str(&wire.quantity)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            position: EventPosition::new(
                BlockHeight::new(wire.block_height),
                wire.transaction_index,
                wire.canonical_event_index,
            ),
            payload_blake3: parse_payload_blake3(&wire.payload_blake3)?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.liquidation_id, &record.event_id)? == *key {
            Ok(record)
        } else {
            Err(PositionStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&LiquidationFillFactWire {
            schema: FILL_FACT_SCHEMA.to_owned(),
            liquidation_id: self.liquidation_id.as_str().to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            price: self.price.to_string(),
            quantity: self.quantity.to_string(),
            block_height: self.position.block_height.get(),
            transaction_index: self.position.transaction_index,
            canonical_event_index: self.position.canonical_event_index,
            payload_blake3: encode_payload_blake3(self.payload_blake3),
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        validate_rule_version(&self.rule_version)?;
        if self.price.raw() <= 0 || self.quantity.raw() <= 0 {
            Err(PositionStateError::InvalidRecord)
        } else {
            Ok(())
        }
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
    pub const fn liquidation_id(&self) -> &LiquidationId {
        &self.liquidation_id
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
    pub const fn payload_blake3(&self) -> &[u8; 32] {
        &self.payload_blake3
    }

    provenance_getters!();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidationMarketFlowCurrentRecordV1 {
    liquidation_id: LiquidationId,
    account_id: Address,
    market_id: MarketId,
    observed_filled_quantity: Quantity,
    first_fill_event_id: EventId,
    first_fill_position: EventPosition,
    last_fill_event_id: EventId,
    last_fill_position: EventPosition,
    rule_version: String,
}

impl LiquidationMarketFlowCurrentRecordV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        liquidation_id: LiquidationId,
        account_id: Address,
        market_id: MarketId,
        observed_filled_quantity: Quantity,
        first_fill_event_id: EventId,
        first_fill_block_height: BlockHeight,
        first_fill_transaction_index: u32,
        first_fill_canonical_event_index: u32,
        last_fill_event_id: EventId,
        last_fill_block_height: BlockHeight,
        last_fill_transaction_index: u32,
        last_fill_canonical_event_index: u32,
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            liquidation_id,
            account_id,
            market_id,
            observed_filled_quantity,
            first_fill_event_id,
            first_fill_position: EventPosition::new(
                first_fill_block_height,
                first_fill_transaction_index,
                first_fill_canonical_event_index,
            ),
            last_fill_event_id,
            last_fill_position: EventPosition::new(
                last_fill_block_height,
                last_fill_transaction_index,
                last_fill_canonical_event_index,
            ),
            rule_version: CanonicalLiquidationReducerV1::VERSION.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(
        liquidation_id: &LiquidationId,
        account_id: &Address,
        market_id: &MarketId,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            MARKET_FLOW_NAMESPACE,
            &[
                liquidation_id.as_str().as_bytes(),
                account_id.as_bytes(),
                market_id.as_str().as_bytes(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: LiquidationMarketFlowWire = decode_wire(bytes)?;
        if wire.schema != MARKET_FLOW_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            liquidation_id: LiquidationId::new(wire.liquidation_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            observed_filled_quantity: Quantity::from_str(&wire.observed_filled_quantity)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            first_fill_event_id: EventId::new(wire.first_fill_event_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            first_fill_position: EventPosition::new(
                BlockHeight::new(wire.first_fill_block_height),
                wire.first_fill_transaction_index,
                wire.first_fill_canonical_event_index,
            ),
            last_fill_event_id: EventId::new(wire.last_fill_event_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            last_fill_position: EventPosition::new(
                BlockHeight::new(wire.last_fill_block_height),
                wire.last_fill_transaction_index,
                wire.last_fill_canonical_event_index,
            ),
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(
            &record.liquidation_id,
            &record.account_id,
            &record.market_id,
        )? == *key
        {
            Ok(record)
        } else {
            Err(PositionStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&LiquidationMarketFlowWire {
            schema: MARKET_FLOW_SCHEMA.to_owned(),
            liquidation_id: self.liquidation_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            observed_filled_quantity: self.observed_filled_quantity.to_string(),
            first_fill_event_id: self.first_fill_event_id.as_str().to_owned(),
            first_fill_block_height: self.first_fill_position.block_height.get(),
            first_fill_transaction_index: self.first_fill_position.transaction_index,
            first_fill_canonical_event_index: self.first_fill_position.canonical_event_index,
            last_fill_event_id: self.last_fill_event_id.as_str().to_owned(),
            last_fill_block_height: self.last_fill_position.block_height.get(),
            last_fill_transaction_index: self.last_fill_position.transaction_index,
            last_fill_canonical_event_index: self.last_fill_position.canonical_event_index,
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        validate_rule_version(&self.rule_version)?;
        if self.observed_filled_quantity.raw() <= 0 {
            return Err(PositionStateError::InvalidRecord);
        }
        validate_identity_position_pair(
            &self.first_fill_event_id,
            self.first_fill_position,
            &self.last_fill_event_id,
            self.last_fill_position,
        )
    }

    #[must_use]
    pub const fn observed_filled_quantity(&self) -> Quantity {
        self.observed_filled_quantity
    }

    #[must_use]
    pub const fn liquidation_id(&self) -> &LiquidationId {
        &self.liquidation_id
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
    pub const fn first_fill_event_id(&self) -> &EventId {
        &self.first_fill_event_id
    }

    #[must_use]
    pub const fn first_fill_block_height(&self) -> BlockHeight {
        self.first_fill_position.block_height
    }

    #[must_use]
    pub const fn first_fill_transaction_index(&self) -> u32 {
        self.first_fill_position.transaction_index
    }

    #[must_use]
    pub const fn first_fill_canonical_event_index(&self) -> u32 {
        self.first_fill_position.canonical_event_index
    }

    #[must_use]
    pub const fn last_fill_event_id(&self) -> &EventId {
        &self.last_fill_event_id
    }

    #[must_use]
    pub const fn last_fill_block_height(&self) -> BlockHeight {
        self.last_fill_position.block_height
    }

    #[must_use]
    pub const fn last_fill_transaction_index(&self) -> u32 {
        self.last_fill_position.transaction_index
    }

    #[must_use]
    pub const fn last_fill_canonical_event_index(&self) -> u32 {
        self.last_fill_position.canonical_event_index
    }

    #[must_use]
    pub fn rule_version(&self) -> &str {
        &self.rule_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackstopLiquidationFactRecordV1 {
    liquidation_id: LiquidationId,
    event_id: EventId,
    account_id: Address,
    backstop_account_id: Address,
    market_id: MarketId,
    quantity: Quantity,
    transfer_price_resolution: LiquidationSourceValueResolutionV1,
    entry_price_resolution: LiquidationSourceValueResolutionV1,
    position: EventPosition,
    payload_blake3: [u8; 32],
    rule_version: String,
}

impl BackstopLiquidationFactRecordV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        liquidation_id: LiquidationId,
        event_id: EventId,
        account_id: Address,
        backstop_account_id: Address,
        market_id: MarketId,
        quantity: Quantity,
        block_height: BlockHeight,
        transaction_index: u32,
        canonical_event_index: u32,
        payload_blake3: [u8; 32],
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            liquidation_id,
            event_id,
            account_id,
            backstop_account_id,
            market_id,
            quantity,
            transfer_price_resolution: LiquidationSourceValueResolutionV1::UnavailableFromSource,
            entry_price_resolution: LiquidationSourceValueResolutionV1::UnavailableFromSource,
            position: EventPosition::new(block_height, transaction_index, canonical_event_index),
            payload_blake3,
            rule_version: CanonicalLiquidationReducerV1::VERSION.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(
        liquidation_id: &LiquidationId,
        event_id: &EventId,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            BACKSTOP_FACT_NAMESPACE,
            &[
                liquidation_id.as_str().as_bytes(),
                event_id.as_str().as_bytes(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: BackstopLiquidationFactWire = decode_wire(bytes)?;
        if wire.schema != BACKSTOP_FACT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            liquidation_id: LiquidationId::new(wire.liquidation_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            event_id: EventId::new(wire.event_id).map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            backstop_account_id: Address::parse_api(&wire.backstop_account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            quantity: Quantity::from_str(&wire.quantity)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            transfer_price_resolution: LiquidationSourceValueResolutionV1::parse(
                &wire.transfer_price_resolution,
            )?,
            entry_price_resolution: LiquidationSourceValueResolutionV1::parse(
                &wire.entry_price_resolution,
            )?,
            position: EventPosition::new(
                BlockHeight::new(wire.block_height),
                wire.transaction_index,
                wire.canonical_event_index,
            ),
            payload_blake3: parse_payload_blake3(&wire.payload_blake3)?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.liquidation_id, &record.event_id)? == *key {
            Ok(record)
        } else {
            Err(PositionStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&BackstopLiquidationFactWire {
            schema: BACKSTOP_FACT_SCHEMA.to_owned(),
            liquidation_id: self.liquidation_id.as_str().to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            backstop_account_id: self.backstop_account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            quantity: self.quantity.to_string(),
            transfer_price_resolution: self.transfer_price_resolution.as_wire().to_owned(),
            entry_price_resolution: self.entry_price_resolution.as_wire().to_owned(),
            block_height: self.position.block_height.get(),
            transaction_index: self.position.transaction_index,
            canonical_event_index: self.position.canonical_event_index,
            payload_blake3: encode_payload_blake3(self.payload_blake3),
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        validate_rule_version(&self.rule_version)?;
        if self.account_id == self.backstop_account_id
            || self.quantity.raw() <= 0
            || self.transfer_price_resolution
                != LiquidationSourceValueResolutionV1::UnavailableFromSource
            || self.entry_price_resolution
                != LiquidationSourceValueResolutionV1::UnavailableFromSource
        {
            Err(PositionStateError::InvalidRecord)
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub const fn transfer_price_resolution(&self) -> LiquidationSourceValueResolutionV1 {
        self.transfer_price_resolution
    }

    #[must_use]
    pub const fn entry_price_resolution(&self) -> LiquidationSourceValueResolutionV1 {
        self.entry_price_resolution
    }

    #[must_use]
    pub const fn liquidation_id(&self) -> &LiquidationId {
        &self.liquidation_id
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
    pub const fn backstop_account_id(&self) -> Address {
        self.backstop_account_id
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
    pub const fn payload_blake3(&self) -> &[u8; 32] {
        &self.payload_blake3
    }

    provenance_getters!();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PositionSettlementFactRecordV1 {
    event_id: EventId,
    account_id: Address,
    market_id: MarketId,
    settlement_price: Price,
    settled_quantity: Quantity,
    realized_pnl: QuoteAmount,
    position: EventPosition,
    payload_blake3: [u8; 32],
    rule_version: String,
}

impl PositionSettlementFactRecordV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        event_id: EventId,
        account_id: Address,
        market_id: MarketId,
        settlement_price: Price,
        settled_quantity: Quantity,
        realized_pnl: QuoteAmount,
        block_height: BlockHeight,
        transaction_index: u32,
        canonical_event_index: u32,
        payload_blake3: [u8; 32],
    ) -> Result<Self, PositionStateError> {
        let record = Self {
            event_id,
            account_id,
            market_id,
            settlement_price,
            settled_quantity,
            realized_pnl,
            position: EventPosition::new(block_height, transaction_index, canonical_event_index),
            payload_blake3,
            rule_version: CanonicalLiquidationReducerV1::VERSION.to_owned(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn state_key(
        event_id: &EventId,
        account_id: &Address,
        market_id: &MarketId,
    ) -> Result<StateKey, PositionStateError> {
        state_key(
            SETTLEMENT_FACT_NAMESPACE,
            &[
                event_id.as_str().as_bytes(),
                account_id.as_bytes(),
                market_id.as_str().as_bytes(),
            ],
        )
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, PositionStateError> {
        let wire: PositionSettlementFactWire = decode_wire(bytes)?;
        if wire.schema != SETTLEMENT_FACT_SCHEMA {
            return Err(PositionStateError::InvalidRecord);
        }
        let record = Self {
            event_id: EventId::new(wire.event_id).map_err(|_| PositionStateError::InvalidRecord)?,
            account_id: Address::parse_api(&wire.account_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            market_id: MarketId::new(wire.market_id)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            settlement_price: Price::from_str(&wire.settlement_price)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            settled_quantity: Quantity::from_str(&wire.settled_quantity)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            realized_pnl: QuoteAmount::from_str(&wire.realized_pnl)
                .map_err(|_| PositionStateError::InvalidRecord)?,
            position: EventPosition::new(
                BlockHeight::new(wire.block_height),
                wire.transaction_index,
                wire.canonical_event_index,
            ),
            payload_blake3: parse_payload_blake3(&wire.payload_blake3)?,
            rule_version: wire.rule_version,
        };
        record.validate()?;
        require_record_bytes(&record.encode()?, bytes)?;
        Ok(record)
    }

    pub fn decode_at(key: &StateKey, bytes: &[u8]) -> Result<Self, PositionStateError> {
        let record = Self::decode(bytes)?;
        if Self::state_key(&record.event_id, &record.account_id, &record.market_id)? == *key {
            Ok(record)
        } else {
            Err(PositionStateError::KeyMismatch)
        }
    }

    pub(super) fn encode(&self) -> Result<Vec<u8>, PositionStateError> {
        self.validate()?;
        encode_wire(&PositionSettlementFactWire {
            schema: SETTLEMENT_FACT_SCHEMA.to_owned(),
            event_id: self.event_id.as_str().to_owned(),
            account_id: self.account_id.to_api_string(),
            market_id: self.market_id.as_str().to_owned(),
            settlement_price: self.settlement_price.to_string(),
            settled_quantity: self.settled_quantity.to_string(),
            realized_pnl: self.realized_pnl.to_string(),
            block_height: self.position.block_height.get(),
            transaction_index: self.position.transaction_index,
            canonical_event_index: self.position.canonical_event_index,
            payload_blake3: encode_payload_blake3(self.payload_blake3),
            rule_version: self.rule_version.clone(),
        })
    }

    fn validate(&self) -> Result<(), PositionStateError> {
        validate_rule_version(&self.rule_version)?;
        if self.settlement_price.raw() < 0 || self.settled_quantity.raw() <= 0 {
            Err(PositionStateError::InvalidRecord)
        } else {
            Ok(())
        }
    }

    #[must_use]
    pub const fn settlement_price(&self) -> Price {
        self.settlement_price
    }

    #[must_use]
    pub const fn settled_quantity(&self) -> Quantity {
        self.settled_quantity
    }

    #[must_use]
    pub const fn realized_pnl(&self) -> QuoteAmount {
        self.realized_pnl
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
    pub const fn payload_blake3(&self) -> &[u8; 32] {
        &self.payload_blake3
    }

    provenance_getters!();
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiquidationCurrentWire {
    schema: String,
    liquidation_id: String,
    account_id: String,
    start_margin_value: String,
    start_maintenance_requirement: String,
    observed_status: String,
    start_event_id: String,
    start_block_height: u64,
    start_transaction_index: u32,
    start_canonical_event_index: u32,
    first_backstop_event_id: Option<String>,
    first_backstop_block_height: Option<u64>,
    first_backstop_transaction_index: Option<u32>,
    first_backstop_canonical_event_index: Option<u32>,
    last_observation_event_id: String,
    last_observation_block_height: u64,
    last_observation_transaction_index: u32,
    last_observation_canonical_event_index: u32,
    rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiquidationStartFactWire {
    schema: String,
    liquidation_id: String,
    event_id: String,
    account_id: String,
    margin_value: String,
    maintenance_requirement: String,
    block_height: u64,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: String,
    rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiquidationFillFactWire {
    schema: String,
    liquidation_id: String,
    event_id: String,
    account_id: String,
    market_id: String,
    price: String,
    quantity: String,
    block_height: u64,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: String,
    rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LiquidationMarketFlowWire {
    schema: String,
    liquidation_id: String,
    account_id: String,
    market_id: String,
    observed_filled_quantity: String,
    first_fill_event_id: String,
    first_fill_block_height: u64,
    first_fill_transaction_index: u32,
    first_fill_canonical_event_index: u32,
    last_fill_event_id: String,
    last_fill_block_height: u64,
    last_fill_transaction_index: u32,
    last_fill_canonical_event_index: u32,
    rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BackstopLiquidationFactWire {
    schema: String,
    liquidation_id: String,
    event_id: String,
    account_id: String,
    backstop_account_id: String,
    market_id: String,
    quantity: String,
    transfer_price_resolution: String,
    entry_price_resolution: String,
    block_height: u64,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: String,
    rule_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionSettlementFactWire {
    schema: String,
    event_id: String,
    account_id: String,
    market_id: String,
    settlement_price: String,
    settled_quantity: String,
    realized_pnl: String,
    block_height: u64,
    transaction_index: u32,
    canonical_event_index: u32,
    payload_blake3: String,
    rule_version: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: Address = Address::from_bytes([0x11; 20]);
    const BACKSTOP: Address = Address::from_bytes([0x22; 20]);

    fn liquidation() -> LiquidationId {
        LiquidationId::new("liq-constructor").unwrap()
    }

    fn market() -> MarketId {
        MarketId::new("perp:BTC").unwrap()
    }

    fn event(value: &str) -> EventId {
        EventId::new(value).unwrap()
    }

    #[test]
    fn every_validated_constructor_round_trips_through_exact_key_and_codec() {
        let current = LiquidationCurrentRecordV1::try_new(
            liquidation(),
            ACCOUNT,
            UsdAmount::from_str("9.00").unwrap(),
            UsdAmount::from_str("10.00").unwrap(),
            LiquidationObservedStatusV1::BackstopObserved,
            event("evt-start"),
            BlockHeight::new(10),
            2,
            0,
            Some((event("evt-backstop"), BlockHeight::new(10), 2, 2)),
            event("evt-fill-after-backstop"),
            BlockHeight::new(10),
            2,
            3,
        )
        .unwrap();
        let current_key = LiquidationCurrentRecordV1::state_key(current.liquidation_id()).unwrap();
        assert_eq!(
            LiquidationCurrentRecordV1::decode_at(&current_key, &current.encode().unwrap())
                .unwrap(),
            current
        );

        let start = LiquidationStartFactRecordV1::try_new(
            liquidation(),
            event("evt-start"),
            ACCOUNT,
            UsdAmount::from_str("9.00").unwrap(),
            UsdAmount::from_str("10.00").unwrap(),
            BlockHeight::new(10),
            2,
            0,
            [1; 32],
        )
        .unwrap();
        let start_key =
            LiquidationStartFactRecordV1::state_key(start.liquidation_id(), start.event_id())
                .unwrap();
        assert_eq!(
            LiquidationStartFactRecordV1::decode_at(&start_key, &start.encode().unwrap()).unwrap(),
            start
        );

        let fill = LiquidationFillFactRecordV1::try_new(
            liquidation(),
            event("evt-fill"),
            ACCOUNT,
            market(),
            Price::from_str("100.0").unwrap(),
            Quantity::from_str("0.25").unwrap(),
            BlockHeight::new(10),
            2,
            1,
            [2; 32],
        )
        .unwrap();
        let fill_key =
            LiquidationFillFactRecordV1::state_key(fill.liquidation_id(), fill.event_id()).unwrap();
        assert_eq!(
            LiquidationFillFactRecordV1::decode_at(&fill_key, &fill.encode().unwrap()).unwrap(),
            fill
        );

        let flow = LiquidationMarketFlowCurrentRecordV1::try_new(
            liquidation(),
            ACCOUNT,
            market(),
            Quantity::from_str("0.375").unwrap(),
            event("evt-fill"),
            BlockHeight::new(10),
            2,
            1,
            event("evt-fill-2"),
            BlockHeight::new(10),
            2,
            3,
        )
        .unwrap();
        let flow_key = LiquidationMarketFlowCurrentRecordV1::state_key(
            flow.liquidation_id(),
            &flow.account_id(),
            flow.market_id(),
        )
        .unwrap();
        assert_eq!(
            LiquidationMarketFlowCurrentRecordV1::decode_at(&flow_key, &flow.encode().unwrap())
                .unwrap(),
            flow
        );

        let backstop = BackstopLiquidationFactRecordV1::try_new(
            liquidation(),
            event("evt-backstop"),
            ACCOUNT,
            BACKSTOP,
            market(),
            Quantity::from_str("0.125").unwrap(),
            BlockHeight::new(10),
            2,
            2,
            [3; 32],
        )
        .unwrap();
        let backstop_key = BackstopLiquidationFactRecordV1::state_key(
            backstop.liquidation_id(),
            backstop.event_id(),
        )
        .unwrap();
        assert_eq!(
            BackstopLiquidationFactRecordV1::decode_at(&backstop_key, &backstop.encode().unwrap())
                .unwrap(),
            backstop
        );

        let settlement = PositionSettlementFactRecordV1::try_new(
            event("evt-settlement"),
            ACCOUNT,
            market(),
            Price::from_str("0").unwrap(),
            Quantity::from_str("1").unwrap(),
            QuoteAmount::from_str("-2.5").unwrap(),
            BlockHeight::new(10),
            2,
            4,
            [4; 32],
        )
        .unwrap();
        let settlement_key = PositionSettlementFactRecordV1::state_key(
            settlement.event_id(),
            &settlement.account_id(),
            settlement.market_id(),
        )
        .unwrap();
        assert_eq!(
            PositionSettlementFactRecordV1::decode_at(
                &settlement_key,
                &settlement.encode().unwrap()
            )
            .unwrap(),
            settlement
        );
    }

    #[test]
    fn constructors_reject_partial_status_invalid_order_and_invalid_amounts() {
        assert!(
            LiquidationCurrentRecordV1::try_new(
                liquidation(),
                ACCOUNT,
                UsdAmount::from_str("9.00").unwrap(),
                UsdAmount::from_str("10.00").unwrap(),
                LiquidationObservedStatusV1::BackstopObserved,
                event("evt-start"),
                BlockHeight::new(10),
                2,
                0,
                None,
                event("evt-start"),
                BlockHeight::new(10),
                2,
                0,
            )
            .is_err()
        );
        assert!(
            LiquidationCurrentRecordV1::try_new(
                liquidation(),
                ACCOUNT,
                UsdAmount::from_str("10.00").unwrap(),
                UsdAmount::from_str("10.00").unwrap(),
                LiquidationObservedStatusV1::Started,
                event("evt-start"),
                BlockHeight::new(10),
                2,
                0,
                None,
                event("evt-start"),
                BlockHeight::new(10),
                2,
                0,
            )
            .is_err()
        );
        assert!(
            LiquidationFillFactRecordV1::try_new(
                liquidation(),
                event("evt-fill"),
                ACCOUNT,
                market(),
                Price::from_str("0").unwrap(),
                Quantity::from_str("1").unwrap(),
                BlockHeight::new(10),
                2,
                1,
                [0; 32],
            )
            .is_err()
        );
        assert!(
            LiquidationMarketFlowCurrentRecordV1::try_new(
                liquidation(),
                ACCOUNT,
                market(),
                Quantity::from_str("-1").unwrap(),
                event("evt-fill"),
                BlockHeight::new(10),
                2,
                1,
                event("evt-fill"),
                BlockHeight::new(10),
                2,
                1,
            )
            .is_err()
        );
        assert!(
            BackstopLiquidationFactRecordV1::try_new(
                liquidation(),
                event("evt-backstop"),
                ACCOUNT,
                ACCOUNT,
                market(),
                Quantity::from_str("1").unwrap(),
                BlockHeight::new(10),
                2,
                2,
                [0; 32],
            )
            .is_err()
        );
        assert!(
            PositionSettlementFactRecordV1::try_new(
                event("evt-settlement"),
                ACCOUNT,
                market(),
                Price::from_str("-1").unwrap(),
                Quantity::from_str("1").unwrap(),
                QuoteAmount::from_str("0").unwrap(),
                BlockHeight::new(10),
                2,
                3,
                [0; 32],
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_mutation_keys_are_rejected_by_the_complete_vector_guard() {
        let key = StateKey::try_new("liquidation-test.v1", vec![1]).unwrap();
        let mutations = vec![
            StateMutation::put(key.clone(), vec![1]),
            StateMutation::put(key, vec![2]),
        ];
        let error = ensure_unique_liquidation_keys(&mutations).unwrap_err();
        assert_eq!(
            error.reason_code(),
            "liquidation_state.duplicate_mutation_key"
        );
    }
}
