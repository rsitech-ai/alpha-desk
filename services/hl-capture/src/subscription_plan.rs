//! Deterministic official WebSocket subscription planner.
//!
//! Allocates market-wide and priority-user connections under the documented
//! per-IP limits and always leaves reserved failover capacity unused.

use std::collections::{BTreeMap, BTreeSet};

use hl_protocol::ws::{WsSubscription, families, family_by_identifier, parse_subscription};
use serde_json::{Map, Value};

pub const OFFICIAL_WS_MAX_CONNECTIONS: u32 = 10;
pub const OFFICIAL_WS_MAX_NEW_CONNECTIONS_PER_MINUTE: u32 = 30;
pub const OFFICIAL_WS_MAX_SUBSCRIPTIONS: u32 = 1_000;
pub const OFFICIAL_WS_MAX_UNIQUE_USERS: u32 = 10;
pub const OFFICIAL_WS_MAX_OUTGOING_PER_MINUTE: u32 = 2_000;
pub const OFFICIAL_WS_MAX_INFLIGHT_POSTS: u32 = 100;

const JITTER_CONTEXT: &[u8] = b"hl.ws-reconnect-jitter.v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OfficialWsLimits {
    max_connections: u32,
    max_new_connections_per_minute: u32,
    max_subscriptions: u32,
    max_unique_users: u32,
    max_outgoing_per_minute: u32,
    max_inflight_posts: u32,
}

impl OfficialWsLimits {
    #[must_use]
    pub const fn official() -> Self {
        Self {
            max_connections: OFFICIAL_WS_MAX_CONNECTIONS,
            max_new_connections_per_minute: OFFICIAL_WS_MAX_NEW_CONNECTIONS_PER_MINUTE,
            max_subscriptions: OFFICIAL_WS_MAX_SUBSCRIPTIONS,
            max_unique_users: OFFICIAL_WS_MAX_UNIQUE_USERS,
            max_outgoing_per_minute: OFFICIAL_WS_MAX_OUTGOING_PER_MINUTE,
            max_inflight_posts: OFFICIAL_WS_MAX_INFLIGHT_POSTS,
        }
    }

    #[must_use]
    pub const fn max_connections(self) -> u32 {
        self.max_connections
    }

    #[must_use]
    pub const fn max_new_connections_per_minute(self) -> u32 {
        self.max_new_connections_per_minute
    }

    #[must_use]
    pub const fn max_subscriptions(self) -> u32 {
        self.max_subscriptions
    }

    #[must_use]
    pub const fn max_unique_users(self) -> u32 {
        self.max_unique_users
    }

    #[must_use]
    pub const fn max_outgoing_per_minute(self) -> u32 {
        self.max_outgoing_per_minute
    }

    #[must_use]
    pub const fn max_inflight_posts(self) -> u32 {
        self.max_inflight_posts
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlannerConfig {
    limits: OfficialWsLimits,
    reserved_failover_connections: u32,
    reserved_failover_users: u32,
    reconnect_base_millis: u64,
    reconnect_max_millis: u64,
}

impl PlannerConfig {
    #[must_use]
    pub const fn official() -> Self {
        Self {
            limits: OfficialWsLimits::official(),
            reserved_failover_connections: 1,
            reserved_failover_users: 1,
            reconnect_base_millis: 250,
            reconnect_max_millis: 8_000,
        }
    }

    #[must_use]
    pub const fn limits(self) -> OfficialWsLimits {
        self.limits
    }

    #[must_use]
    pub const fn reserved_failover_connections(self) -> u32 {
        self.reserved_failover_connections
    }

    #[must_use]
    pub const fn reserved_failover_users(self) -> u32 {
        self.reserved_failover_users
    }

    #[must_use]
    pub fn with_reserves(self, connections: u32, users: u32) -> Self {
        Self {
            reserved_failover_connections: connections,
            reserved_failover_users: users,
            ..self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionDemand {
    identifier: String,
    user: Option<String>,
    coin: Option<String>,
    interval: Option<String>,
    dex: Option<String>,
}

impl SubscriptionDemand {
    #[must_use]
    pub fn new(identifier: impl Into<String>) -> Self {
        Self {
            identifier: identifier.into(),
            user: None,
            coin: None,
            interval: None,
            dex: None,
        }
    }

    #[must_use]
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = Some(user.into());
        self
    }

    #[must_use]
    pub fn with_coin(mut self, coin: impl Into<String>) -> Self {
        self.coin = Some(coin.into());
        self
    }

    #[must_use]
    pub fn with_interval(mut self, interval: impl Into<String>) -> Self {
        self.interval = Some(interval.into());
        self
    }

    #[must_use]
    pub fn with_dex(mut self, dex: impl Into<String>) -> Self {
        self.dex = Some(dex.into());
        self
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceHealthHint {
    Green,
    Red,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlannerInput {
    demands: Vec<SubscriptionDemand>,
    source_health: BTreeMap<String, SourceHealthHint>,
    freshness_target_millis: BTreeMap<String, u64>,
}

impl PlannerInput {
    #[must_use]
    pub fn new(demands: Vec<SubscriptionDemand>) -> Self {
        Self {
            demands,
            source_health: BTreeMap::new(),
            freshness_target_millis: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn with_health(mut self, identifier: impl Into<String>, health: SourceHealthHint) -> Self {
        self.source_health.insert(identifier.into(), health);
        self
    }

    #[must_use]
    pub fn with_freshness(mut self, identifier: impl Into<String>, millis: u64) -> Self {
        self.freshness_target_millis
            .insert(identifier.into(), millis);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    UnknownFamily,
    SourceRed,
    UserLimit,
    SubscriptionLimit,
    ConnectionLimit,
    MissingUser,
    MissingCoin,
    MissingInterval,
}

impl RejectReason {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnknownFamily => "unknown_family",
            Self::SourceRed => "source_red",
            Self::UserLimit => "user_limit",
            Self::SubscriptionLimit => "subscription_limit",
            Self::ConnectionLimit => "connection_limit",
            Self::MissingUser => "missing_user",
            Self::MissingCoin => "missing_coin",
            Self::MissingInterval => "missing_interval",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RejectedDemand {
    demand: SubscriptionDemand,
    reason: RejectReason,
}

impl RejectedDemand {
    #[must_use]
    pub fn demand(&self) -> &SubscriptionDemand {
        &self.demand
    }

    #[must_use]
    pub const fn reason(&self) -> RejectReason {
        self.reason
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlannedConnectionKind {
    MarketWide { dex: String },
    PriorityUser { user: String },
    FailoverReserve,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedSubscription {
    identity: blake3::Hash,
    canonical_json: String,
    identifier: String,
    user: Option<String>,
    freshness_target_millis: u64,
}

impl PlannedSubscription {
    #[must_use]
    pub const fn identity(&self) -> blake3::Hash {
        self.identity
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical_json
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    #[must_use]
    pub fn user(&self) -> Option<&str> {
        self.user.as_deref()
    }

    #[must_use]
    pub const fn freshness_target_millis(&self) -> u64 {
        self.freshness_target_millis
    }

    pub fn to_ws_subscription(&self) -> Result<WsSubscription, hl_protocol::SourceError> {
        let value: Value = serde_json::from_str(&self.canonical_json).map_err(|_| {
            hl_protocol::SourceError::MalformedPayload(
                "planned subscription is not JSON".to_owned(),
            )
        })?;
        parse_subscription(&value)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedConnection {
    slot: u8,
    kind: PlannedConnectionKind,
    subscriptions: Vec<PlannedSubscription>,
    reconnect_jitter_millis: u64,
}

impl PlannedConnection {
    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.slot
    }

    #[must_use]
    pub const fn kind(&self) -> &PlannedConnectionKind {
        &self.kind
    }

    #[must_use]
    pub fn subscriptions(&self) -> &[PlannedSubscription] {
        &self.subscriptions
    }

    #[must_use]
    pub const fn reconnect_jitter_millis(&self) -> u64 {
        self.reconnect_jitter_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscriptionPlan {
    connections: Vec<PlannedConnection>,
    rejected: Vec<RejectedDemand>,
    reserved_connections: u32,
    reserved_user_slots: u32,
}

impl SubscriptionPlan {
    #[must_use]
    pub fn connections(&self) -> &[PlannedConnection] {
        &self.connections
    }

    #[must_use]
    pub fn rejected(&self) -> &[RejectedDemand] {
        &self.rejected
    }

    #[must_use]
    pub const fn reserved_connections(&self) -> u32 {
        self.reserved_connections
    }

    #[must_use]
    pub const fn reserved_user_slots(&self) -> u32 {
        self.reserved_user_slots
    }

    #[must_use]
    pub fn subscription_count(&self) -> usize {
        self.connections
            .iter()
            .map(|connection| connection.subscriptions.len())
            .sum()
    }

    #[must_use]
    pub fn unique_users(&self) -> BTreeSet<&str> {
        self.connections
            .iter()
            .flat_map(|connection| connection.subscriptions.iter())
            .filter_map(|subscription| subscription.user.as_deref())
            .collect()
    }

    #[must_use]
    pub fn canonical_json(&self) -> String {
        serde_json::to_string(&self.canonical_value()).unwrap_or_else(|_| "{}".to_owned())
    }

    fn canonical_value(&self) -> Value {
        let connections = self
            .connections
            .iter()
            .map(|connection| {
                let mut object = Map::new();
                object.insert("slot".to_owned(), Value::from(connection.slot));
                object.insert(
                    "kind".to_owned(),
                    Value::String(match &connection.kind {
                        PlannedConnectionKind::MarketWide { dex } => {
                            if dex.is_empty() {
                                "market-wide".to_owned()
                            } else {
                                format!("market-wide:{dex}")
                            }
                        }
                        PlannedConnectionKind::PriorityUser { user } => {
                            format!("priority-user:{user}")
                        }
                        PlannedConnectionKind::FailoverReserve => "failover-reserve".to_owned(),
                    }),
                );
                object.insert(
                    "reconnect_jitter_millis".to_owned(),
                    Value::from(connection.reconnect_jitter_millis),
                );
                let subscriptions = connection
                    .subscriptions
                    .iter()
                    .map(|subscription| {
                        let mut row = Map::new();
                        row.insert(
                            "identity".to_owned(),
                            Value::String(hex::encode(subscription.identity.as_bytes())),
                        );
                        row.insert(
                            "canonical".to_owned(),
                            Value::String(subscription.canonical_json.clone()),
                        );
                        row.insert(
                            "freshness_target_millis".to_owned(),
                            Value::from(subscription.freshness_target_millis),
                        );
                        Value::Object(row)
                    })
                    .collect();
                object.insert("subscriptions".to_owned(), Value::Array(subscriptions));
                Value::Object(object)
            })
            .collect();
        let rejected = self
            .rejected
            .iter()
            .map(|item| {
                let mut object = Map::new();
                object.insert(
                    "identifier".to_owned(),
                    Value::String(item.demand.identifier.clone()),
                );
                object.insert(
                    "reason".to_owned(),
                    Value::String(item.reason.as_str().to_owned()),
                );
                Value::Object(object)
            })
            .collect();
        let mut root = Map::new();
        root.insert(
            "reserved_connections".to_owned(),
            Value::from(self.reserved_connections),
        );
        root.insert(
            "reserved_user_slots".to_owned(),
            Value::from(self.reserved_user_slots),
        );
        root.insert("connections".to_owned(), Value::Array(connections));
        root.insert("rejected".to_owned(), Value::Array(rejected));
        Value::Object(root)
    }
}

#[must_use]
pub fn reconnect_jitter_millis(slot: u8, attempt: u32, base: u64, cap: u64) -> u64 {
    if cap <= base {
        return base;
    }
    let mut hasher = blake3::Hasher::new();
    hasher.update(JITTER_CONTEXT);
    hasher.update(&[slot]);
    hasher.update(&attempt.to_be_bytes());
    let hash = hasher.finalize();
    let mut pick_bytes = [0_u8; 8];
    pick_bytes.copy_from_slice(&hash.as_bytes()[..8]);
    let span = cap.saturating_sub(base);
    base + (u64::from_le_bytes(pick_bytes) % (span.saturating_add(1)))
}

#[must_use]
pub fn expand_official_demand(
    family_ids: &[String],
    users: &[String],
    coins: &[String],
    dexes: &[String],
    intervals: &[String],
) -> Vec<SubscriptionDemand> {
    let mut demands = Vec::new();
    for identifier in family_ids {
        let Some(family) = family_by_identifier(identifier) else {
            demands.push(SubscriptionDemand::new(identifier.clone()));
            continue;
        };
        match (
            family.user_scoped,
            family.coin_scoped,
            family.requires_interval,
        ) {
            (true, true, _) => {
                for user in users {
                    for coin in coins {
                        demands.push(
                            SubscriptionDemand::new(identifier.clone())
                                .with_user(user.clone())
                                .with_coin(coin.clone()),
                        );
                    }
                }
                if users.is_empty() {
                    demands.push(SubscriptionDemand::new(identifier.clone()));
                }
            }
            (true, false, _) => {
                if users.is_empty() {
                    demands.push(SubscriptionDemand::new(identifier.clone()));
                } else {
                    for user in users {
                        demands.push(
                            SubscriptionDemand::new(identifier.clone()).with_user(user.clone()),
                        );
                    }
                }
            }
            (false, true, true) => {
                let interval_set = if intervals.is_empty() {
                    vec![None]
                } else {
                    intervals.iter().cloned().map(Some).collect()
                };
                if coins.is_empty() {
                    demands.push(SubscriptionDemand::new(identifier.clone()));
                } else {
                    for coin in coins {
                        for interval in &interval_set {
                            let mut demand =
                                SubscriptionDemand::new(identifier.clone()).with_coin(coin.clone());
                            if let Some(interval) = interval {
                                demand = demand.with_interval(interval.clone());
                            }
                            demands.push(demand);
                        }
                    }
                }
            }
            (false, true, false) => {
                if coins.is_empty() {
                    demands.push(SubscriptionDemand::new(identifier.clone()));
                } else {
                    for coin in coins {
                        demands.push(
                            SubscriptionDemand::new(identifier.clone()).with_coin(coin.clone()),
                        );
                    }
                }
            }
            (false, false, _) => {
                if dexes.is_empty() {
                    demands.push(SubscriptionDemand::new(identifier.clone()));
                } else {
                    for dex in dexes {
                        demands.push(
                            SubscriptionDemand::new(identifier.clone()).with_dex(dex.clone()),
                        );
                    }
                }
            }
        }
    }
    demands
}

#[must_use]
pub fn plan_subscriptions(config: PlannerConfig, input: PlannerInput) -> SubscriptionPlan {
    let reserved_connections = config
        .reserved_failover_connections
        .min(config.limits.max_connections);
    let reserved_users = config
        .reserved_failover_users
        .min(config.limits.max_unique_users);
    let active_slots = config
        .limits
        .max_connections
        .saturating_sub(reserved_connections) as usize;
    let user_slots = config
        .limits
        .max_unique_users
        .saturating_sub(reserved_users) as usize;

    let mut rejected = Vec::new();
    let mut prepared = Vec::new();
    for demand in sorted_demands(input.demands) {
        if input.source_health.get(&demand.identifier) == Some(&SourceHealthHint::Red) {
            rejected.push(RejectedDemand {
                demand,
                reason: RejectReason::SourceRed,
            });
            continue;
        }
        match prepare_subscription(&demand, &input.freshness_target_millis) {
            Ok(prepared_row) => prepared.push((demand, prepared_row)),
            Err(reason) => rejected.push(RejectedDemand { demand, reason }),
        }
    }

    let mut admitted_users = BTreeSet::new();
    let mut kept = Vec::new();
    for (demand, subscription) in prepared {
        if let Some(user) = subscription.user.as_deref() {
            if admitted_users.contains(user) {
                kept.push((demand, subscription));
                continue;
            }
            if admitted_users.len() >= user_slots {
                rejected.push(RejectedDemand {
                    demand,
                    reason: RejectReason::UserLimit,
                });
                continue;
            }
            admitted_users.insert(user.to_owned());
        }
        kept.push((demand, subscription));
    }

    let max_subs = config.limits.max_subscriptions as usize;
    let mut admitted = Vec::new();
    for (index, (demand, subscription)) in kept.into_iter().enumerate() {
        if index >= max_subs {
            rejected.push(RejectedDemand {
                demand,
                reason: RejectReason::SubscriptionLimit,
            });
        } else {
            admitted.push((demand, subscription));
        }
    }

    let mut market_by_dex: BTreeMap<String, Vec<PlannedSubscription>> = BTreeMap::new();
    let mut user_by_addr: BTreeMap<String, Vec<PlannedSubscription>> = BTreeMap::new();
    for (demand, subscription) in admitted {
        if let Some(user) = subscription.user.clone() {
            user_by_addr.entry(user).or_default().push(subscription);
        } else {
            market_by_dex
                .entry(demand.dex.clone().unwrap_or_default())
                .or_default()
                .push(subscription);
        }
    }

    let mut kinds: Vec<PlannedConnectionKind> = Vec::new();
    if !market_by_dex.is_empty() && kinds.len() < active_slots {
        let first_dex = market_by_dex.keys().next().cloned().unwrap_or_default();
        kinds.push(PlannedConnectionKind::MarketWide { dex: first_dex });
    }
    for user in user_by_addr.keys() {
        if kinds.len() >= active_slots {
            break;
        }
        kinds.push(PlannedConnectionKind::PriorityUser { user: user.clone() });
    }
    for dex in market_by_dex.keys().skip(1) {
        if kinds.len() >= active_slots {
            break;
        }
        kinds.push(PlannedConnectionKind::MarketWide { dex: dex.clone() });
    }

    let admitted_user_kinds: BTreeSet<&str> = kinds
        .iter()
        .filter_map(|kind| match kind {
            PlannedConnectionKind::PriorityUser { user } => Some(user.as_str()),
            PlannedConnectionKind::MarketWide { .. } | PlannedConnectionKind::FailoverReserve => {
                None
            }
        })
        .collect();
    let leftover_users: Vec<String> = user_by_addr
        .keys()
        .filter(|user| !admitted_user_kinds.contains(user.as_str()))
        .cloned()
        .collect();
    for user in leftover_users {
        if let Some(subs) = user_by_addr.remove(&user) {
            for subscription in subs {
                rejected.push(RejectedDemand {
                    demand: SubscriptionDemand::new(subscription.identifier)
                        .with_user(user.clone()),
                    reason: RejectReason::ConnectionLimit,
                });
            }
        }
    }

    let mut connections: Vec<PlannedConnection> = kinds
        .into_iter()
        .enumerate()
        .map(|(index, kind)| {
            let slot = u8::try_from(index).unwrap_or(u8::MAX);
            PlannedConnection {
                slot,
                kind,
                subscriptions: Vec::new(),
                reconnect_jitter_millis: reconnect_jitter_millis(
                    slot,
                    0,
                    config.reconnect_base_millis,
                    config.reconnect_max_millis,
                ),
            }
        })
        .collect();

    let first_market = connections
        .iter()
        .position(|connection| matches!(connection.kind, PlannedConnectionKind::MarketWide { .. }));
    for (dex, mut subscriptions) in market_by_dex {
        subscriptions.sort_by(|left, right| left.canonical_json.cmp(&right.canonical_json));
        let target = connections.iter().position(|connection| {
            matches!(&connection.kind, PlannedConnectionKind::MarketWide { dex: assigned } if *assigned == dex)
        });
        match target.or(first_market) {
            Some(index) => connections[index].subscriptions.append(&mut subscriptions),
            None => {
                for subscription in subscriptions {
                    rejected.push(RejectedDemand {
                        demand: SubscriptionDemand::new(subscription.identifier),
                        reason: RejectReason::ConnectionLimit,
                    });
                }
            }
        }
    }
    for (user, mut subscriptions) in user_by_addr {
        subscriptions.sort_by(|left, right| left.canonical_json.cmp(&right.canonical_json));
        if let Some(index) = connections.iter().position(|connection| {
            matches!(&connection.kind, PlannedConnectionKind::PriorityUser { user: assigned } if *assigned == user)
        }) {
            connections[index].subscriptions.append(&mut subscriptions);
        }
    }

    let start_slot = connections.len();
    for offset in 0..reserved_connections as usize {
        let slot = u8::try_from(start_slot.saturating_add(offset)).unwrap_or(u8::MAX);
        connections.push(PlannedConnection {
            slot,
            kind: PlannedConnectionKind::FailoverReserve,
            subscriptions: Vec::new(),
            reconnect_jitter_millis: reconnect_jitter_millis(
                slot,
                0,
                config.reconnect_base_millis,
                config.reconnect_max_millis,
            ),
        });
    }

    SubscriptionPlan {
        connections,
        rejected,
        reserved_connections,
        reserved_user_slots: reserved_users,
    }
}

#[must_use]
pub fn official_family_identifiers() -> Vec<&'static str> {
    families().iter().map(|family| family.identifier).collect()
}

fn sorted_demands(mut demands: Vec<SubscriptionDemand>) -> Vec<SubscriptionDemand> {
    demands.sort_by(|left, right| {
        (
            left.identifier.as_str(),
            left.user.as_deref(),
            left.coin.as_deref(),
            left.interval.as_deref(),
            left.dex.as_deref(),
        )
            .cmp(&(
                right.identifier.as_str(),
                right.user.as_deref(),
                right.coin.as_deref(),
                right.interval.as_deref(),
                right.dex.as_deref(),
            ))
    });
    demands
}

fn prepare_subscription(
    demand: &SubscriptionDemand,
    freshness: &BTreeMap<String, u64>,
) -> Result<PlannedSubscription, RejectReason> {
    let family = family_by_identifier(&demand.identifier).ok_or(RejectReason::UnknownFamily)?;
    if family.user_scoped && demand.user.as_ref().is_none_or(|user| user.is_empty()) {
        return Err(RejectReason::MissingUser);
    }
    if family.coin_scoped && demand.coin.as_ref().is_none_or(|coin| coin.is_empty()) {
        return Err(RejectReason::MissingCoin);
    }
    if family.requires_interval
        && demand
            .interval
            .as_ref()
            .is_none_or(|interval| interval.is_empty())
    {
        return Err(RejectReason::MissingInterval);
    }
    let mut fields = Map::new();
    fields.insert("type".to_owned(), Value::String(demand.identifier.clone()));
    if let Some(user) = &demand.user {
        fields.insert("user".to_owned(), Value::String(user.clone()));
    }
    if let Some(coin) = &demand.coin {
        fields.insert("coin".to_owned(), Value::String(coin.clone()));
    }
    if let Some(interval) = &demand.interval {
        fields.insert("interval".to_owned(), Value::String(interval.clone()));
    }
    if let Some(dex) = &demand.dex {
        fields.insert("dex".to_owned(), Value::String(dex.clone()));
    }
    let parsed =
        parse_subscription(&Value::Object(fields)).map_err(|_| RejectReason::UnknownFamily)?;
    Ok(PlannedSubscription {
        identity: parsed.identity(),
        canonical_json: parsed.canonical_json().to_owned(),
        identifier: parsed.identifier().to_owned(),
        user: demand.user.clone(),
        freshness_target_millis: freshness.get(&demand.identifier).copied().unwrap_or(1_000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_plan_caps_connections_at_ten() {
        let demands = (0..20)
            .map(|index| SubscriptionDemand::new("allMids").with_dex(format!("dex{index:02}")))
            .collect();
        let plan = plan_subscriptions(PlannerConfig::official(), PlannerInput::new(demands));
        assert!(plan.connections().len() <= 10);
        assert_eq!(plan.reserved_connections(), 1);
        assert!(
            plan.connections().iter().any(|connection| matches!(
                connection.kind(),
                PlannedConnectionKind::FailoverReserve
            ))
        );
        assert!(plan.connections().iter().all(|connection| {
            !matches!(connection.kind(), PlannedConnectionKind::FailoverReserve)
                || connection.subscriptions().is_empty()
        }));
    }

    #[test]
    fn subscription_plan_caps_unique_users_at_ten() {
        let demands = (0..12)
            .map(|index| {
                SubscriptionDemand::new("userFills").with_user(format!("0x{:040x}", index + 1))
            })
            .collect();
        let plan = plan_subscriptions(
            PlannerConfig::official().with_reserves(1, 0),
            PlannerInput::new(demands),
        );
        assert!(plan.unique_users().len() <= 10);
        assert!(
            plan.rejected()
                .iter()
                .any(|item| item.reason() == RejectReason::UserLimit)
        );
    }

    #[test]
    fn subscription_plan_caps_subscriptions_at_1000() {
        let demands = (0..1_001)
            .map(|index| SubscriptionDemand::new("trades").with_coin(format!("C{index:04}")))
            .collect();
        let plan = plan_subscriptions(PlannerConfig::official(), PlannerInput::new(demands));
        assert_eq!(plan.subscription_count(), 1_000);
        assert!(
            plan.rejected()
                .iter()
                .any(|item| item.reason() == RejectReason::SubscriptionLimit)
        );
    }

    #[test]
    fn subscription_plan_output_is_deterministic() {
        let demands = vec![
            SubscriptionDemand::new("trades").with_coin("ETH"),
            SubscriptionDemand::new("allMids"),
            SubscriptionDemand::new("trades").with_coin("BTC"),
        ];
        let left = plan_subscriptions(
            PlannerConfig::official(),
            PlannerInput::new(demands.clone()),
        );
        let right = plan_subscriptions(PlannerConfig::official(), PlannerInput::new(demands));
        assert_eq!(left.canonical_json(), right.canonical_json());
        assert_eq!(
            reconnect_jitter_millis(0, 1, 250, 8_000),
            reconnect_jitter_millis(0, 1, 250, 8_000)
        );
        assert_ne!(
            reconnect_jitter_millis(0, 1, 250, 8_000),
            reconnect_jitter_millis(1, 1, 250, 8_000)
        );
    }

    #[test]
    fn subscription_plan_rejects_fast_asset_ctxs() {
        let plan = plan_subscriptions(
            PlannerConfig::official(),
            PlannerInput::new(vec![SubscriptionDemand::new("fastAssetCtxs")]),
        );
        assert_eq!(plan.subscription_count(), 0);
        assert_eq!(plan.rejected()[0].reason(), RejectReason::UnknownFamily);
    }
}
