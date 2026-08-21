use std::collections::{BTreeMap, BTreeSet};

use canonical_events::{CanonicalEventEnvelope, EventKind, EventPayload};
use domain_types::MarketId;
use orderbook::{BookHealth, OrderBook, TriggerKind};

use crate::{
    ApplyContext, BlockDeltaView, CanonicalAccountReducerV1, CanonicalLiquidationReducerV1,
    CanonicalMarketReducerV1, CanonicalOrderReducerV1, CanonicalPositionEpisodeReducerV1,
    CanonicalPositionReducerV1, CanonicalTradeReducerV1, CanonicalTradeReducerV2,
    CanonicalTriggerReducerV1, CanonicalTwapReducerV1, EventReducer, OrderCurrentRecordV1,
    ReducerError, StateImage, StateMutation, StateView, TriggerCurrentRecordV1,
};

const UNSUPPORTED_EVENT_REASON: &str = "canonical_state.unsupported_event";
const COMPONENT_SUPPORT_MISMATCH_REASON: &str = "canonical_state.component_support_mismatch";
const DUPLICATE_MUTATION_KEY_REASON: &str = "canonical_state.duplicate_mutation_key";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalStateComponentVersionV1 {
    name: &'static str,
    version: &'static str,
}

impl CanonicalStateComponentVersionV1 {
    const fn new(name: &'static str, version: &'static str) -> Self {
        Self { name, version }
    }

    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    #[must_use]
    pub const fn version(&self) -> &'static str {
        self.version
    }
}

const EXPECTED_COMPONENT_MANIFEST: [CanonicalStateComponentVersionV1; 10] = [
    CanonicalStateComponentVersionV1::new(
        "market",
        "hyperliquid-alpha-desk-canonical-market@1.0.0",
    ),
    CanonicalStateComponentVersionV1::new("order", "hyperliquid-alpha-desk-canonical-order@1.0.0"),
    CanonicalStateComponentVersionV1::new(
        "trade_v1",
        "hyperliquid-alpha-desk-canonical-trade@1.0.0",
    ),
    CanonicalStateComponentVersionV1::new(
        "trade_v2",
        "hyperliquid-alpha-desk-canonical-trade@2.0.0",
    ),
    CanonicalStateComponentVersionV1::new(
        "account",
        "hyperliquid-alpha-desk-canonical-account@1.0.0",
    ),
    CanonicalStateComponentVersionV1::new(
        "position_quantity",
        "hyperliquid-alpha-desk-canonical-position@1.0.0",
    ),
    CanonicalStateComponentVersionV1::new(
        "position_episode",
        "hyperliquid-alpha-desk-canonical-position-episode@1.0.0",
    ),
    CanonicalStateComponentVersionV1::new(
        "position_liquidation",
        "hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0",
    ),
    CanonicalStateComponentVersionV1::new(
        "trigger",
        "hyperliquid-alpha-desk-canonical-trigger@1.0.0",
    ),
    CanonicalStateComponentVersionV1::new("twap", "hyperliquid-alpha-desk-canonical-twap@1.0.0"),
];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CanonicalStateError {
    #[error(
        "canonical state component {component} version mismatch: expected {expected}, received {actual}"
    )]
    ComponentVersionMismatch {
        component: &'static str,
        expected: &'static str,
        actual: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum L4ProjectionError {
    #[error("l4 projection requires a committed watermark")]
    MissingWatermark,
    #[error("l4 projection order record is invalid")]
    InvalidOrder,
    #[error("l4 projection trigger record is invalid")]
    InvalidTrigger,
    #[error("l4 book is not healthy: {0}")]
    Unhealthy(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Component {
    Market,
    Order,
    TradeV1,
    TradeV2,
    Account,
    PositionQuantity,
    PositionEpisode,
    PositionLiquidation,
    Trigger,
    Twap,
}

const MARKET_OWNER: &[Component] = &[Component::Market];
const ORDER_OWNER: &[Component] = &[Component::Order];
const TRADE_V1_OWNER: &[Component] = &[Component::TradeV1];
const ENRICHED_TRADE_OWNERS: &[Component] = &[
    Component::TradeV1,
    Component::TradeV2,
    Component::PositionQuantity,
    Component::PositionEpisode,
];
const FUNDING_OWNERS: &[Component] = &[Component::Account, Component::PositionEpisode];
const ACCOUNT_OWNER: &[Component] = &[Component::Account];
const LIQUIDATION_OWNER: &[Component] = &[Component::PositionLiquidation];
const TRIGGER_OWNER: &[Component] = &[Component::Trigger];
const TWAP_OWNER: &[Component] = &[Component::Twap];
const NO_OWNERS: &[Component] = &[];
const ALL_COMPONENTS: &[Component; 10] = &[
    Component::Market,
    Component::Order,
    Component::TradeV1,
    Component::TradeV2,
    Component::Account,
    Component::PositionQuantity,
    Component::PositionEpisode,
    Component::PositionLiquidation,
    Component::Trigger,
    Component::Twap,
];

/// The sealed production reducer for canonical account state.
///
/// It cannot be constructed with [`Default`]:
///
/// ```compile_fail
/// use canonical_ledger::CanonicalStateReducerV1;
/// let _ = CanonicalStateReducerV1::default();
/// ```
///
/// Its direct children cannot be supplied through a struct literal:
///
/// ```compile_fail
/// use canonical_ledger::{CanonicalMarketReducerV1, CanonicalStateReducerV1};
/// let _ = CanonicalStateReducerV1 { market: CanonicalMarketReducerV1 };
/// ```
///
/// It has no unchecked constructor:
///
/// ```compile_fail
/// use canonical_ledger::CanonicalStateReducerV1;
/// let _ = CanonicalStateReducerV1::new();
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CanonicalStateReducerV1 {
    market: CanonicalMarketReducerV1,
    order: CanonicalOrderReducerV1,
    trade_v1: CanonicalTradeReducerV1,
    trade_v2: CanonicalTradeReducerV2,
    account: CanonicalAccountReducerV1,
    position_quantity: CanonicalPositionReducerV1,
    position_episode: CanonicalPositionEpisodeReducerV1,
    position_liquidation: CanonicalLiquidationReducerV1,
    trigger: CanonicalTriggerReducerV1,
    twap: CanonicalTwapReducerV1,
}

impl CanonicalStateReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-state@1.1.0";

    pub fn try_new() -> Result<Self, CanonicalStateError> {
        validate_component_manifest(&actual_component_manifest())?;
        Ok(Self {
            market: CanonicalMarketReducerV1,
            order: CanonicalOrderReducerV1,
            trade_v1: CanonicalTradeReducerV1,
            trade_v2: CanonicalTradeReducerV2,
            account: CanonicalAccountReducerV1,
            position_quantity: CanonicalPositionReducerV1,
            position_episode: CanonicalPositionEpisodeReducerV1,
            position_liquidation: CanonicalLiquidationReducerV1,
            trigger: CanonicalTriggerReducerV1,
            twap: CanonicalTwapReducerV1,
        })
    }

    #[must_use]
    pub const fn component_manifest(&self) -> &'static [CanonicalStateComponentVersionV1; 10] {
        &EXPECTED_COMPONENT_MANIFEST
    }

    pub fn project_l4_book(
        market_id: &MarketId,
        image: &StateImage,
    ) -> Result<OrderBook, L4ProjectionError> {
        let as_of = image
            .block_height()
            .ok_or(L4ProjectionError::MissingWatermark)?;
        let mut triggers = BTreeMap::new();
        let mut orders = Vec::new();
        for (key, bytes) in image.entries() {
            match key.namespace() {
                "trigger-current.v1" => {
                    let record = TriggerCurrentRecordV1::decode(bytes)
                        .map_err(|_| L4ProjectionError::InvalidTrigger)?;
                    if record.market_id() == market_id {
                        triggers.insert(record.order_id().clone(), record.trigger_price());
                    }
                }
                "order-current.v1" => {
                    let record = OrderCurrentRecordV1::decode(bytes)
                        .map_err(|_| L4ProjectionError::InvalidOrder)?;
                    if record.market_id() == market_id
                        && let Some(resting) = record.try_resting()
                    {
                        orders.push(resting);
                    }
                }
                _ => {}
            }
        }
        for order in &mut orders {
            if let Some(trigger_px) = triggers.get(&order.order_id) {
                *order = order.clone().with_trigger(TriggerKind::Activated {
                    trigger_px: *trigger_px,
                });
            }
        }
        orders.sort_by(|left, right| left.order_id.as_str().cmp(right.order_id.as_str()));
        let mut book = OrderBook::awaiting_snapshot(market_id.clone(), as_of);
        book.apply_snapshot(as_of.get(), as_of, orders);
        match book.health() {
            BookHealth::Healthy => Ok(book),
            BookHealth::AwaitingSnapshot { reason } | BookHealth::Red { reason } => {
                Err(L4ProjectionError::Unhealthy(reason.clone()))
            }
        }
    }

    fn child_supports(&self, component: Component, event: &CanonicalEventEnvelope) -> bool {
        match component {
            Component::Market => self.market.supports(event),
            Component::Order => self.order.supports(event),
            Component::TradeV1 => self.trade_v1.supports(event),
            Component::TradeV2 => self.trade_v2.supports(event),
            Component::Account => self.account.supports(event),
            Component::PositionQuantity => self.position_quantity.supports(event),
            Component::PositionEpisode => self.position_episode.supports(event),
            Component::PositionLiquidation => self.position_liquidation.supports(event),
            Component::Trigger => self.trigger.supports(event),
            Component::Twap => self.twap.supports(event),
        }
    }

    fn reduce_child(
        &self,
        component: Component,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        match component {
            Component::Market => self.market.reduce(state, event, context),
            Component::Order => self.order.reduce(state, event, context),
            Component::TradeV1 => self.trade_v1.reduce(state, event, context),
            Component::TradeV2 => self.trade_v2.reduce(state, event, context),
            Component::Account => self.account.reduce(state, event, context),
            Component::PositionQuantity => self.position_quantity.reduce(state, event, context),
            Component::PositionEpisode => self.position_episode.reduce(state, event, context),
            Component::PositionLiquidation => {
                self.position_liquidation.reduce(state, event, context)
            }
            Component::Trigger => self.trigger.reduce(state, event, context),
            Component::Twap => self.twap.reduce(state, event, context),
        }
    }

    fn validate_child(
        &self,
        component: Component,
        state: &StateView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        match component {
            Component::Market => self.market.validate_block(state, context),
            Component::Order => self.order.validate_block(state, context),
            Component::TradeV1 => self.trade_v1.validate_block(state, context),
            Component::TradeV2 => self.trade_v2.validate_block(state, context),
            Component::Account => self.account.validate_block(state, context),
            Component::PositionQuantity => self.position_quantity.validate_block(state, context),
            Component::PositionEpisode => self.position_episode.validate_block(state, context),
            Component::PositionLiquidation => {
                self.position_liquidation.validate_block(state, context)
            }
            Component::Trigger => self.trigger.validate_block(state, context),
            Component::Twap => self.twap.validate_block(state, context),
        }
    }

    fn validate_child_delta(
        &self,
        component: Component,
        final_state: &StateView<'_>,
        delta: &BlockDeltaView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        match component {
            Component::Market => self
                .market
                .validate_block_delta(final_state, delta, context),
            Component::Order => self.order.validate_block_delta(final_state, delta, context),
            Component::TradeV1 => self
                .trade_v1
                .validate_block_delta(final_state, delta, context),
            Component::TradeV2 => self
                .trade_v2
                .validate_block_delta(final_state, delta, context),
            Component::Account => self
                .account
                .validate_block_delta(final_state, delta, context),
            Component::PositionQuantity => {
                self.position_quantity
                    .validate_block_delta(final_state, delta, context)
            }
            Component::PositionEpisode => {
                self.position_episode
                    .validate_block_delta(final_state, delta, context)
            }
            Component::PositionLiquidation => {
                self.position_liquidation
                    .validate_block_delta(final_state, delta, context)
            }
            Component::Trigger => self
                .trigger
                .validate_block_delta(final_state, delta, context),
            Component::Twap => self.twap.validate_block_delta(final_state, delta, context),
        }
    }
}

impl EventReducer for CanonicalStateReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        !owners(event).is_empty()
    }

    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        let selected = owners(event);
        if selected.is_empty() {
            return Err(composite_error(
                UNSUPPORTED_EVENT_REASON,
                "canonical state reducer received an unsupported event",
            ));
        }
        if selected
            .iter()
            .any(|component| !self.child_supports(*component, event))
        {
            return Err(composite_error(
                COMPONENT_SUPPORT_MISMATCH_REASON,
                "frozen canonical state owner does not support the selected event",
            ));
        }

        reduce_selected_children(
            selected,
            state,
            event,
            context,
            |component, forwarded_state, forwarded_event, forwarded_context| {
                self.reduce_child(
                    component,
                    forwarded_state,
                    forwarded_event,
                    forwarded_context,
                )
            },
        )
    }

    fn validate_block(
        &self,
        state: &StateView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        fanout_all_components(
            state,
            None::<&BlockDeltaView<'_>>,
            context,
            |component, forwarded_state, _, forwarded_context| {
                self.validate_child(component, forwarded_state, forwarded_context)
            },
        )
    }

    fn validate_block_delta(
        &self,
        final_state: &StateView<'_>,
        delta: &BlockDeltaView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        fanout_all_components(
            final_state,
            Some(delta),
            context,
            |component, forwarded_state, forwarded_delta, forwarded_context| {
                self.validate_child_delta(
                    component,
                    forwarded_state,
                    forwarded_delta.expect("delta fanout always forwards the supplied delta"),
                    forwarded_context,
                )
            },
        )
    }
}

fn owners(event: &CanonicalEventEnvelope) -> &'static [Component] {
    if event.schema_version() != "1.0.0" {
        return NO_OWNERS;
    }
    match event.event_kind() {
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
        | EventKind::OutcomeResolved => MARKET_OWNER,
        EventKind::OrderAccepted
        | EventKind::OrderRested
        | EventKind::OrderModified
        | EventKind::OrderPartiallyFilled
        | EventKind::OrderFilled
        | EventKind::OrderCancelled
        | EventKind::OrderRejected => ORDER_OWNER,
        EventKind::TradeMatched => match event.payload() {
            EventPayload::TradeMatched(trade) if trade.participants.is_some() => {
                ENRICHED_TRADE_OWNERS
            }
            EventPayload::TradeMatched(_) => TRADE_V1_OWNER,
            _ => NO_OWNERS,
        },
        EventKind::FundingPaid | EventKind::FundingReceived => FUNDING_OWNERS,
        EventKind::DepositCredited
        | EventKind::WithdrawalDebited
        | EventKind::SpotTransfer
        | EventKind::PerpTransfer
        | EventKind::SubaccountTransfer
        | EventKind::VaultDeposit
        | EventKind::VaultWithdrawal
        | EventKind::FeeCharged
        | EventKind::BuilderFeeCharged
        | EventKind::ReferralReward
        | EventKind::AccountModeChanged
        | EventKind::MarginModeChanged
        | EventKind::LeverageChanged => ACCOUNT_OWNER,
        EventKind::LiquidationStarted
        | EventKind::LiquidationFill
        | EventKind::BackstopLiquidation
        | EventKind::PositionSettled => LIQUIDATION_OWNER,
        EventKind::TriggerOrderActivated => TRIGGER_OWNER,
        EventKind::TwapStarted | EventKind::TwapSliceFilled | EventKind::TwapCompleted => {
            TWAP_OWNER
        }
        EventKind::NonUserOrderCancelled
        | EventKind::InternalTransfer
        | EventKind::AccountClassTransfer
        | EventKind::VaultCreated
        | EventKind::VaultDistribution
        | EventKind::VaultLeaderCommissionPaid
        | EventKind::RewardClaimed
        | EventKind::SpotGenesisApplied
        | EventKind::StakingDeposit
        | EventKind::StakingDelegated
        | EventKind::StakingUndelegated
        | EventKind::StakingWithdrawalQueued
        | EventKind::StakingWithdrawalCompleted
        | EventKind::ValidatorRewardPaid => NO_OWNERS,
    }
}

fn validate_component_manifest(
    actual: &[CanonicalStateComponentVersionV1; 10],
) -> Result<(), CanonicalStateError> {
    for (expected, actual) in EXPECTED_COMPONENT_MANIFEST.iter().zip(actual) {
        if expected.name != actual.name || expected.version != actual.version {
            return Err(CanonicalStateError::ComponentVersionMismatch {
                component: expected.name,
                expected: expected.version,
                actual: actual.version,
            });
        }
    }
    Ok(())
}

fn merge_selected_children(
    selected: &[Component],
    mut invoke: impl FnMut(Component) -> Result<Vec<StateMutation>, ReducerError>,
) -> Result<Vec<StateMutation>, ReducerError> {
    let mut mutations = Vec::new();
    let mut keys = BTreeSet::new();
    for component in selected {
        let child_mutations = invoke(*component)?;
        for mutation in &child_mutations {
            if !keys.insert(mutation.key().clone()) {
                return Err(composite_error(
                    DUPLICATE_MUTATION_KEY_REASON,
                    "canonical state children emitted the same mutation key",
                ));
            }
        }
        mutations.extend(child_mutations);
    }
    Ok(mutations)
}

fn reduce_selected_children<'a, S, E, C>(
    selected: &[Component],
    state: &'a S,
    event: &'a E,
    context: &'a C,
    mut reduce: impl FnMut(Component, &'a S, &'a E, &'a C) -> Result<Vec<StateMutation>, ReducerError>,
) -> Result<Vec<StateMutation>, ReducerError> {
    merge_selected_children(selected, |component| {
        reduce(component, state, event, context)
    })
}

fn fanout_all_components<'a, S, D, C>(
    state: &'a S,
    delta: Option<&'a D>,
    context: &'a C,
    mut validate: impl FnMut(Component, &'a S, Option<&'a D>, &'a C) -> Result<(), ReducerError>,
) -> Result<(), ReducerError> {
    for component in ALL_COMPONENTS {
        validate(*component, state, delta, context)?;
    }
    Ok(())
}

const fn actual_component_manifest() -> [CanonicalStateComponentVersionV1; 10] {
    [
        CanonicalStateComponentVersionV1::new("market", CanonicalMarketReducerV1::VERSION),
        CanonicalStateComponentVersionV1::new("order", CanonicalOrderReducerV1::VERSION),
        CanonicalStateComponentVersionV1::new("trade_v1", CanonicalTradeReducerV1::VERSION),
        CanonicalStateComponentVersionV1::new("trade_v2", CanonicalTradeReducerV2::VERSION),
        CanonicalStateComponentVersionV1::new("account", CanonicalAccountReducerV1::VERSION),
        CanonicalStateComponentVersionV1::new(
            "position_quantity",
            CanonicalPositionReducerV1::VERSION,
        ),
        CanonicalStateComponentVersionV1::new(
            "position_episode",
            CanonicalPositionEpisodeReducerV1::VERSION,
        ),
        CanonicalStateComponentVersionV1::new(
            "position_liquidation",
            CanonicalLiquidationReducerV1::VERSION,
        ),
        CanonicalStateComponentVersionV1::new("trigger", CanonicalTriggerReducerV1::VERSION),
        CanonicalStateComponentVersionV1::new("twap", CanonicalTwapReducerV1::VERSION),
    ]
}

fn composite_error(reason_code: &'static str, message: &'static str) -> ReducerError {
    ReducerError::from_static(reason_code, message)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::str::FromStr;

    use canonical_events::{
        ConfirmationClass, EventPayload, TradeParticipantRoleV1, TradeParticipantV1,
    };
    use domain_types::{
        Address, BlockHeight, ChainId, ClientOrderId, EventId, OrderId, PositionQuantity,
        ProtocolTime, TransactionId, TwapId,
    };

    use super::*;
    use crate::{StateKey, state::view_entries};

    #[test]
    fn injected_manifest_mismatch_retains_diagnostics() {
        let mut actual = actual_component_manifest();
        actual[3] = CanonicalStateComponentVersionV1::new(
            "trade_v2",
            "hyperliquid-alpha-desk-canonical-trade@2.1.0",
        );

        assert_eq!(
            validate_component_manifest(&actual),
            Err(CanonicalStateError::ComponentVersionMismatch {
                component: "trade_v2",
                expected: "hyperliquid-alpha-desk-canonical-trade@2.0.0",
                actual: "hyperliquid-alpha-desk-canonical-trade@2.1.0",
            })
        );
    }

    #[test]
    fn same_child_duplicate_stops_before_a_later_child_error() {
        let key = StateKey::try_new("test.composite", b"duplicate".to_vec()).unwrap();
        let mut invocation_log = Vec::new();

        let error = merge_selected_children(ENRICHED_TRADE_OWNERS, |component| {
            invocation_log.push(component);
            match component {
                Component::TradeV1 => Ok(vec![
                    StateMutation::put(key.clone(), vec![1]),
                    StateMutation::put(key.clone(), vec![2]),
                ]),
                _ => Err(composite_error(
                    "test.later_child_failed",
                    "later child must not be invoked",
                )),
            }
        })
        .expect_err("same-child duplicate must fail");

        assert_eq!(error.reason_code(), DUPLICATE_MUTATION_KEY_REASON);
        assert_eq!(invocation_log, [Component::TradeV1]);
    }

    #[test]
    fn cross_child_duplicate_stops_before_a_later_child_error() {
        let key = StateKey::try_new("test.composite", b"duplicate".to_vec()).unwrap();
        let mut invocation_log = Vec::new();

        let error = merge_selected_children(ENRICHED_TRADE_OWNERS, |component| {
            invocation_log.push(component);
            match component {
                Component::TradeV1 => Ok(vec![StateMutation::put(key.clone(), vec![1])]),
                Component::TradeV2 => Ok(vec![StateMutation::put(key.clone(), vec![2])]),
                _ => Err(composite_error(
                    "test.later_child_failed",
                    "later child must not be invoked",
                )),
            }
        })
        .expect_err("cross-child duplicate must fail");

        assert_eq!(error.reason_code(), DUPLICATE_MUTATION_KEY_REASON);
        assert_eq!(invocation_log, [Component::TradeV1, Component::TradeV2]);
    }

    #[test]
    fn selected_children_receive_the_same_pre_event_references_in_manifest_order() {
        let state = 11_u8;
        let event = 22_u8;
        let context = 33_u8;
        let mut observations = Vec::new();

        let mutations = reduce_selected_children(
            ENRICHED_TRADE_OWNERS,
            &state,
            &event,
            &context,
            |component, seen_state, seen_event, seen_context| {
                observations.push((
                    component,
                    std::ptr::eq(seen_state, &state),
                    std::ptr::eq(seen_event, &event),
                    std::ptr::eq(seen_context, &context),
                ));
                Ok(Vec::new())
            },
        )
        .unwrap();

        assert!(mutations.is_empty());
        assert_eq!(
            observations,
            [
                (Component::TradeV1, true, true, true),
                (Component::TradeV2, true, true, true),
                (Component::PositionQuantity, true, true, true),
                (Component::PositionEpisode, true, true, true),
            ]
        );
    }

    #[test]
    fn validation_fanout_forwards_identical_references_and_stops_on_first_error() {
        let state = 11_u8;
        let delta = 22_u8;
        let context = 33_u8;
        let mut observations = Vec::new();

        let error = fanout_all_components(
            &state,
            Some(&delta),
            &context,
            |component, seen_state, seen_delta, seen_context| {
                observations.push((
                    component,
                    std::ptr::eq(seen_state, &state),
                    seen_delta.is_some_and(|value| std::ptr::eq(value, &delta)),
                    std::ptr::eq(seen_context, &context),
                ));
                if component == Component::Account {
                    Err(composite_error(
                        "test.validation_failed",
                        "scripted account validation failure",
                    ))
                } else {
                    Ok(())
                }
            },
        )
        .expect_err("first scripted validation error must stop fanout");

        assert_eq!(error.reason_code(), "test.validation_failed");
        assert_eq!(
            observations,
            [
                (Component::Market, true, true, true),
                (Component::Order, true, true, true),
                (Component::TradeV1, true, true, true),
                (Component::TradeV2, true, true, true),
                (Component::Account, true, true, true),
            ]
        );
    }

    #[test]
    fn direct_validation_fanout_visits_all_components_with_identical_references() {
        let state = 11_u8;
        let context = 33_u8;
        let mut observations = Vec::new();

        fanout_all_components(
            &state,
            None::<&u8>,
            &context,
            |component, seen_state, seen_delta, seen_context| {
                observations.push((
                    component,
                    std::ptr::eq(seen_state, &state),
                    seen_delta.is_none(),
                    std::ptr::eq(seen_context, &context),
                ));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            observations,
            [
                (Component::Market, true, true, true),
                (Component::Order, true, true, true),
                (Component::TradeV1, true, true, true),
                (Component::TradeV2, true, true, true),
                (Component::Account, true, true, true),
                (Component::PositionQuantity, true, true, true),
                (Component::PositionEpisode, true, true, true),
                (Component::PositionLiquidation, true, true, true),
                (Component::Trigger, true, true, true),
                (Component::Twap, true, true, true),
            ]
        );
    }

    #[test]
    fn frozen_ownership_table_covers_every_event_kind_and_enriched_trade_fanout() {
        let reducer = CanonicalStateReducerV1::try_new().unwrap();
        for payload in EventPayload::fixtures().unwrap() {
            let event = fixture_event("1.0.0", payload);
            let expected = expected_fixture_owners(event.event_kind());
            assert_eq!(
                reducer.supports(&event),
                !expected.is_empty(),
                "unexpected ownership for {:?}",
                event.event_kind()
            );
            assert_eq!(
                owners(&event),
                expected,
                "unexpected fanout for {:?}",
                event.event_kind()
            );
        }

        let enriched = enriched_trade_fixture("1.0.0");
        assert_eq!(
            owners(&enriched),
            [
                Component::TradeV1,
                Component::TradeV2,
                Component::PositionQuantity,
                Component::PositionEpisode,
            ]
        );
        assert!(!reducer.supports(&enriched_trade_fixture("1.1.0")));
    }

    fn expected_fixture_owners(kind: EventKind) -> &'static [Component] {
        match kind {
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
            | EventKind::OutcomeResolved => MARKET_OWNER,
            EventKind::OrderAccepted
            | EventKind::OrderRested
            | EventKind::OrderModified
            | EventKind::OrderPartiallyFilled
            | EventKind::OrderFilled
            | EventKind::OrderCancelled
            | EventKind::OrderRejected => ORDER_OWNER,
            EventKind::TradeMatched => TRADE_V1_OWNER,
            EventKind::FundingPaid | EventKind::FundingReceived => FUNDING_OWNERS,
            EventKind::DepositCredited
            | EventKind::WithdrawalDebited
            | EventKind::SpotTransfer
            | EventKind::PerpTransfer
            | EventKind::SubaccountTransfer
            | EventKind::VaultDeposit
            | EventKind::VaultWithdrawal
            | EventKind::FeeCharged
            | EventKind::BuilderFeeCharged
            | EventKind::ReferralReward
            | EventKind::AccountModeChanged
            | EventKind::MarginModeChanged
            | EventKind::LeverageChanged => ACCOUNT_OWNER,
            EventKind::LiquidationStarted
            | EventKind::LiquidationFill
            | EventKind::BackstopLiquidation
            | EventKind::PositionSettled => LIQUIDATION_OWNER,
            EventKind::TriggerOrderActivated => TRIGGER_OWNER,
            EventKind::TwapStarted | EventKind::TwapSliceFilled | EventKind::TwapCompleted => {
                TWAP_OWNER
            }
            EventKind::NonUserOrderCancelled
            | EventKind::InternalTransfer
            | EventKind::AccountClassTransfer
            | EventKind::VaultCreated
            | EventKind::VaultDistribution
            | EventKind::VaultLeaderCommissionPaid
            | EventKind::RewardClaimed
            | EventKind::SpotGenesisApplied
            | EventKind::StakingDeposit
            | EventKind::StakingDelegated
            | EventKind::StakingUndelegated
            | EventKind::StakingWithdrawalQueued
            | EventKind::StakingWithdrawalCompleted
            | EventKind::ValidatorRewardPaid => NO_OWNERS,
        }
    }

    #[test]
    fn direct_unsupported_reduce_returns_the_composite_reason_code() {
        let reducer = CanonicalStateReducerV1::try_new().unwrap();
        let trigger_payload = EventPayload::fixtures()
            .unwrap()
            .into_iter()
            .find(|payload| payload.kind() == EventKind::TriggerOrderActivated)
            .unwrap();
        let event = fixture_event("1.1.0", trigger_payload);
        let entries = BTreeMap::new();
        let chain_id = ChainId::new("mainnet").unwrap();
        let context = ApplyContext::new(
            &chain_id,
            BlockHeight::new(1),
            ProtocolTime::from_unix_micros(1).unwrap(),
            ConfirmationClass::CommittedPrimary,
        );

        let error = reducer
            .reduce(&view_entries(&entries), &event, &context)
            .expect_err("schema 1.1.0 trigger ownership remains unsupported");

        assert_eq!(error.reason_code(), UNSUPPORTED_EVENT_REASON);
    }

    fn fixture_event(schema: &str, payload: EventPayload) -> CanonicalEventEnvelope {
        CanonicalEventEnvelope::try_new(
            schema,
            "mainnet",
            BlockHeight::new(1),
            ProtocolTime::from_unix_micros(1).unwrap(),
            TransactionId::new(format!("tx-{}", payload.kind().as_wire_name())).unwrap(),
            0,
            0,
            EventId::new(format!("event-{}", payload.kind().as_wire_name())).unwrap(),
            Vec::new(),
            Vec::new(),
            ConfirmationClass::CommittedPrimary,
            payload,
            "composite-test@1.0.0",
        )
        .unwrap()
    }

    fn enriched_trade_fixture(schema: &str) -> CanonicalEventEnvelope {
        let mut payload = EventPayload::fixtures()
            .unwrap()
            .into_iter()
            .find(|payload| payload.kind() == EventKind::TradeMatched)
            .unwrap();
        let buyer = Address::from_bytes([0x11; 20]);
        let seller = Address::from_bytes([0x22; 20]);
        let EventPayload::TradeMatched(trade) = &mut payload else {
            unreachable!("fixture kind is TradeMatched");
        };
        trade.participants = Some(Box::new([
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Buyer,
                account_id: buyer,
                start_position: PositionQuantity::from_str("1.00000000").unwrap(),
                order_id: OrderId::new("buyer-order").unwrap(),
                twap_id: Some(TwapId::new(1)),
                client_order_id: Some(
                    ClientOrderId::new("0x11111111111111111111111111111111").unwrap(),
                ),
            },
            TradeParticipantV1 {
                role: TradeParticipantRoleV1::Seller,
                account_id: seller,
                start_position: PositionQuantity::from_str("-1.00000000").unwrap(),
                order_id: OrderId::new("seller-order").unwrap(),
                twap_id: None,
                client_order_id: None,
            },
        ]));

        CanonicalEventEnvelope::try_new(
            schema,
            "mainnet",
            BlockHeight::new(1),
            ProtocolTime::from_unix_micros(1).unwrap(),
            TransactionId::new("tx-enriched").unwrap(),
            0,
            0,
            EventId::new("event-enriched").unwrap(),
            Vec::new(),
            vec![buyer, seller],
            ConfirmationClass::CommittedPrimary,
            payload,
            "composite-test@1.0.0",
        )
        .unwrap()
    }
}
