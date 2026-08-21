//! Official WebSocket session lifecycle.
//!
//! Clock-driven ping, inactivity, staleness, reconnect jitter, and snapshot
//! apply policy. Transport bytes are injected by the adapter.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use hl_protocol::ws::{WsObservation, encode_subscribe, encode_unsubscribe, parse_ws_message};
use serde_json::Value;

use crate::subscription_plan::{
    OfficialWsLimits, PlannedConnection, PlannedConnectionKind, PlannedSubscription,
    reconnect_jitter_millis,
};

const PING_FRAME: &[u8] = br#"{"method":"ping"}"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionState {
    Idle,
    Connecting,
    Active,
    Reconnecting { attempt: u32 },
    Unsubscribing,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubscriptionHealth {
    Green,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InboundClass {
    Ack,
    Heartbeat,
    SnapshotReplace,
    DuplicateSnapshot,
    IncrementalEvent,
    Unknown,
    Quarantine,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AppliedInbound {
    class: InboundClass,
    observation: Option<WsObservation>,
    subscription_identity: Option<blake3::Hash>,
    payload: Bytes,
    content_hash: blake3::Hash,
}

impl AppliedInbound {
    #[must_use]
    pub const fn class(&self) -> InboundClass {
        self.class
    }

    #[must_use]
    pub const fn observation(&self) -> Option<&WsObservation> {
        self.observation.as_ref()
    }

    #[must_use]
    pub const fn subscription_identity(&self) -> Option<blake3::Hash> {
        self.subscription_identity
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        self.content_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WsSessionError {
    #[error("websocket connect rate exceeded 30 per minute")]
    ConnectRateLimited,
    #[error("websocket outgoing message rate exceeded 2000 per minute")]
    OutgoingRateLimited,
    #[error("websocket in-flight posts exceeded 100")]
    InflightPostsExceeded,
    #[error("websocket post is forbidden in read-only capture")]
    PostsForbidden,
    #[error("websocket session is shut down")]
    Shutdown,
}

impl WsSessionError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::ConnectRateLimited => "capture_ws.connect_rate",
            Self::OutgoingRateLimited => "capture_ws.outgoing_rate",
            Self::InflightPostsExceeded => "capture_ws.inflight_posts",
            Self::PostsForbidden => "capture_ws.posts_forbidden",
            Self::Shutdown => "capture_ws.shutdown",
        }
    }
}

#[derive(Debug, Clone)]
pub struct MinuteWindow {
    stamps: VecDeque<u64>,
    max_per_minute: u32,
}

impl MinuteWindow {
    #[must_use]
    pub fn new(max_per_minute: u32) -> Self {
        Self {
            stamps: VecDeque::new(),
            max_per_minute,
        }
    }

    pub fn try_add(&mut self, now_millis: u64) -> bool {
        while let Some(front) = self.stamps.front() {
            if now_millis.saturating_sub(*front) < 60_000 {
                break;
            }
            self.stamps.pop_front();
        }
        if self.stamps.len() as u32 >= self.max_per_minute {
            return false;
        }
        self.stamps.push_back(now_millis);
        true
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.stamps.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.stamps.is_empty()
    }
}

/// Process-wide connect and outgoing windows for one official WS host.
///
/// ponytail: one process. A cross-process fleet limiter needs a shared store.
#[derive(Debug, Clone)]
pub struct ProcessIpBudget {
    connect: Arc<Mutex<MinuteWindow>>,
    outgoing: Arc<Mutex<MinuteWindow>>,
}

impl ProcessIpBudget {
    #[must_use]
    pub fn new(max_connect_per_minute: u32, max_outgoing_per_minute: u32) -> Self {
        Self {
            connect: Arc::new(Mutex::new(MinuteWindow::new(max_connect_per_minute))),
            outgoing: Arc::new(Mutex::new(MinuteWindow::new(max_outgoing_per_minute))),
        }
    }

    #[must_use]
    pub fn from_limits(limits: OfficialWsLimits) -> Self {
        Self::new(
            limits.max_new_connections_per_minute(),
            limits.max_outgoing_per_minute(),
        )
    }

    #[must_use]
    pub fn official() -> Self {
        Self::from_limits(OfficialWsLimits::official())
    }

    fn try_connect(&self, now_millis: u64) -> bool {
        self.connect
            .lock()
            .map(|mut window| window.try_add(now_millis))
            .unwrap_or(false)
    }

    fn try_outgoing(&self, now_millis: u64) -> bool {
        self.outgoing
            .lock()
            .map(|mut window| window.try_add(now_millis))
            .unwrap_or(false)
    }
}

#[derive(Debug, Clone)]
struct TrackedSubscription {
    planned: PlannedSubscription,
    health: SubscriptionHealth,
    last_data_millis: Option<u64>,
    snapshot_hash: Option<blake3::Hash>,
    stale_after_millis: u64,
}

#[derive(Debug, Clone)]
pub struct WsSession {
    connection: PlannedConnection,
    state: SessionState,
    last_rx_millis: u64,
    last_ping_millis: Option<u64>,
    awaiting_pong: bool,
    ping_interval_millis: u64,
    inactivity_timeout_millis: u64,
    reconnect_base_millis: u64,
    reconnect_max_millis: u64,
    subscriptions: BTreeMap<String, TrackedSubscription>,
    outgoing: VecDeque<Bytes>,
    budget: ProcessIpBudget,
    opened_at_millis: u64,
    inflight_posts: u32,
    max_inflight_posts: u32,
    attempt: u32,
}

impl WsSession {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        connection: PlannedConnection,
        limits: OfficialWsLimits,
        budget: ProcessIpBudget,
        now_millis: u64,
        ping_interval_millis: u64,
        inactivity_timeout_millis: u64,
        reconnect_base_millis: u64,
        reconnect_max_millis: u64,
        stale_after_millis: u64,
    ) -> Result<Self, WsSessionError> {
        if matches!(connection.kind(), PlannedConnectionKind::FailoverReserve) {
            return Ok(Self {
                connection,
                state: SessionState::Idle,
                last_rx_millis: now_millis,
                last_ping_millis: None,
                awaiting_pong: false,
                ping_interval_millis,
                inactivity_timeout_millis,
                reconnect_base_millis,
                reconnect_max_millis,
                subscriptions: BTreeMap::new(),
                outgoing: VecDeque::new(),
                budget,
                opened_at_millis: now_millis,
                inflight_posts: 0,
                max_inflight_posts: limits.max_inflight_posts(),
                attempt: 0,
            });
        }
        budget
            .try_connect(now_millis)
            .then_some(())
            .ok_or(WsSessionError::ConnectRateLimited)?;
        let mut session = Self {
            subscriptions: connection
                .subscriptions()
                .iter()
                .map(|planned| {
                    (
                        hex::encode(planned.identity().as_bytes()),
                        TrackedSubscription {
                            planned: planned.clone(),
                            health: SubscriptionHealth::Green,
                            last_data_millis: None,
                            snapshot_hash: None,
                            stale_after_millis: planned
                                .freshness_target_millis()
                                .max(stale_after_millis),
                        },
                    )
                })
                .collect(),
            connection,
            state: SessionState::Connecting,
            last_rx_millis: now_millis,
            last_ping_millis: None,
            awaiting_pong: false,
            ping_interval_millis,
            inactivity_timeout_millis,
            reconnect_base_millis,
            reconnect_max_millis,
            outgoing: VecDeque::new(),
            budget,
            opened_at_millis: now_millis,
            inflight_posts: 0,
            max_inflight_posts: limits.max_inflight_posts(),
            attempt: 0,
        };
        session.enqueue_subscribes(now_millis)?;
        session.state = SessionState::Active;
        Ok(session)
    }

    #[must_use]
    pub const fn state(&self) -> SessionState {
        self.state
    }

    #[must_use]
    pub const fn slot(&self) -> u8 {
        self.connection.slot()
    }

    #[must_use]
    pub fn health(&self, identity_hex: &str) -> Option<SubscriptionHealth> {
        self.subscriptions.get(identity_hex).map(|row| row.health)
    }

    #[must_use]
    pub fn red_subscriptions(&self) -> Vec<String> {
        self.subscriptions
            .iter()
            .filter(|(_, row)| row.health == SubscriptionHealth::Red)
            .map(|(key, _)| key.clone())
            .collect()
    }

    pub fn drain_outgoing(&mut self) -> Vec<Bytes> {
        self.outgoing.drain(..).collect()
    }

    pub fn post(&mut self, _now_millis: u64) -> Result<(), WsSessionError> {
        if self.inflight_posts >= self.max_inflight_posts {
            return Err(WsSessionError::InflightPostsExceeded);
        }
        Err(WsSessionError::PostsForbidden)
    }

    pub fn on_clock(&mut self, now_millis: u64) -> Result<(), WsSessionError> {
        self.fail_if_shutdown()?;
        for row in self.subscriptions.values_mut() {
            let last = row.last_data_millis.unwrap_or(self.opened_at_millis);
            if now_millis.saturating_sub(last) >= row.stale_after_millis {
                row.health = SubscriptionHealth::Red;
            }
        }
        if matches!(
            self.state,
            SessionState::Idle | SessionState::Unsubscribing | SessionState::Shutdown
        ) {
            return Ok(());
        }
        if self.awaiting_pong {
            if let Some(ping_at) = self.last_ping_millis
                && now_millis.saturating_sub(ping_at) >= self.ping_interval_millis
            {
                self.begin_reconnect(now_millis)?;
            }
            return Ok(());
        }
        if now_millis.saturating_sub(self.last_rx_millis) >= self.inactivity_timeout_millis {
            self.queue_outgoing(Bytes::copy_from_slice(PING_FRAME), now_millis)?;
            self.last_ping_millis = Some(now_millis);
            self.awaiting_pong = true;
        }
        Ok(())
    }

    pub fn observe(&mut self, payload: Bytes, now_millis: u64) -> AppliedInbound {
        self.last_rx_millis = now_millis;
        match parse_ws_message(payload.clone()) {
            Err(_) => {
                let content_hash = blake3::hash(&payload);
                AppliedInbound {
                    class: InboundClass::Quarantine,
                    observation: None,
                    subscription_identity: None,
                    payload,
                    content_hash,
                }
            }
            Ok(observation) => self.classify_observation(observation, payload),
        }
    }

    pub fn ingest(&mut self, payload: Bytes, now_millis: u64) -> AppliedInbound {
        let applied = self.observe(payload, now_millis);
        self.commit_applied(&applied, now_millis);
        applied
    }

    pub fn commit_applied(&mut self, applied: &AppliedInbound, now_millis: u64) {
        let Some(identity) = applied.subscription_identity else {
            return;
        };
        let key = hex::encode(identity.as_bytes());
        let Some(row) = self.subscriptions.get_mut(&key) else {
            return;
        };
        row.last_data_millis = Some(now_millis);
        row.health = SubscriptionHealth::Green;
        if matches!(
            applied.class,
            InboundClass::SnapshotReplace | InboundClass::DuplicateSnapshot
        ) {
            row.snapshot_hash = Some(applied.content_hash);
        }
    }

    pub fn restore_snapshot_hashes(&mut self, hashes: &BTreeMap<String, [u8; 32]>) {
        for (key, row) in &mut self.subscriptions {
            if let Some(hash) = hashes.get(key) {
                row.snapshot_hash = Some(blake3::Hash::from_bytes(*hash));
            }
        }
    }

    #[must_use]
    pub fn snapshot_hashes(&self) -> BTreeMap<String, [u8; 32]> {
        self.subscriptions
            .iter()
            .filter_map(|(key, row)| {
                row.snapshot_hash
                    .map(|hash| (key.clone(), *hash.as_bytes()))
            })
            .collect()
    }

    pub fn reconnect_delay_millis(&self) -> u64 {
        reconnect_jitter_millis(
            self.connection.slot(),
            self.attempt,
            self.reconnect_base_millis,
            self.reconnect_max_millis,
        )
    }

    pub fn begin_reconnect(&mut self, now_millis: u64) -> Result<(), WsSessionError> {
        self.fail_if_shutdown()?;
        if !self.budget.try_connect(now_millis) {
            return Err(WsSessionError::ConnectRateLimited);
        }
        self.attempt = self.attempt.saturating_add(1);
        self.state = SessionState::Reconnecting {
            attempt: self.attempt,
        };
        self.awaiting_pong = false;
        self.last_ping_millis = None;
        self.outgoing.clear();
        self.enqueue_subscribes(now_millis)?;
        self.state = SessionState::Active;
        Ok(())
    }

    pub fn shutdown(&mut self, now_millis: u64) -> Result<(), WsSessionError> {
        if self.state == SessionState::Shutdown {
            return Ok(());
        }
        self.state = SessionState::Unsubscribing;
        self.outgoing.clear();
        let frames: Vec<Bytes> = self
            .subscriptions
            .values()
            .filter_map(|row| row.planned.to_ws_subscription().ok())
            .filter_map(|subscription| encode_unsubscribe(&subscription).ok())
            .collect();
        for frame in frames {
            let _ = self.queue_outgoing(frame, now_millis);
        }
        self.state = SessionState::Shutdown;
        Ok(())
    }

    fn classify_observation(
        &mut self,
        observation: WsObservation,
        payload: Bytes,
    ) -> AppliedInbound {
        let identity = match_subscription(&self.subscriptions, &observation);
        if matches!(observation, WsObservation::Heartbeat(_)) {
            self.awaiting_pong = false;
        }
        let previous = identity.and_then(|id| {
            self.subscriptions
                .get(&hex::encode(id.as_bytes()))
                .and_then(|row| row.snapshot_hash)
        });
        let class = classify_inbound(&observation, previous);
        AppliedInbound {
            class,
            content_hash: observation.content_hash(),
            observation: Some(observation),
            subscription_identity: identity,
            payload,
        }
    }

    fn enqueue_subscribes(&mut self, now_millis: u64) -> Result<(), WsSessionError> {
        let planned: Vec<PlannedSubscription> = self
            .subscriptions
            .values()
            .map(|row| row.planned.clone())
            .collect();
        for planned in planned {
            let subscription = planned
                .to_ws_subscription()
                .map_err(|_| WsSessionError::Shutdown)?;
            let frame = encode_subscribe(&subscription).map_err(|_| WsSessionError::Shutdown)?;
            self.queue_outgoing(frame, now_millis)?;
        }
        Ok(())
    }

    fn queue_outgoing(&mut self, frame: Bytes, now_millis: u64) -> Result<(), WsSessionError> {
        if !self.budget.try_outgoing(now_millis) {
            return Err(WsSessionError::OutgoingRateLimited);
        }
        self.outgoing.push_back(frame);
        Ok(())
    }

    fn fail_if_shutdown(&self) -> Result<(), WsSessionError> {
        if self.state == SessionState::Shutdown {
            Err(WsSessionError::Shutdown)
        } else {
            Ok(())
        }
    }
}

/// Snapshot-flagged incrementals are replace, not new events (T10 M1).
#[must_use]
pub fn classify_inbound(
    observation: &WsObservation,
    previous_snapshot: Option<blake3::Hash>,
) -> InboundClass {
    match observation {
        WsObservation::Ack(_) => InboundClass::Ack,
        WsObservation::Heartbeat(_) => InboundClass::Heartbeat,
        WsObservation::Unknown(_) => InboundClass::Unknown,
        WsObservation::Snapshot(_) => classify_snapshot(observation, previous_snapshot),
        WsObservation::Incremental(incremental) if incremental.flagged_is_snapshot() => {
            classify_snapshot(observation, previous_snapshot)
        }
        WsObservation::Incremental(_) => InboundClass::IncrementalEvent,
    }
}

fn classify_snapshot(observation: &WsObservation, previous: Option<blake3::Hash>) -> InboundClass {
    match previous {
        Some(hash) if hash == observation.content_hash() => InboundClass::DuplicateSnapshot,
        Some(_) | None => InboundClass::SnapshotReplace,
    }
}

fn match_subscription(
    subscriptions: &BTreeMap<String, TrackedSubscription>,
    observation: &WsObservation,
) -> Option<blake3::Hash> {
    match observation {
        WsObservation::Ack(ack) => Some(ack.subscription().identity()),
        WsObservation::Snapshot(_) | WsObservation::Incremental(_) => {
            let family = observation.identifier()?;
            let extracted = extract_instance_fields(observation.payload());
            let mut matched = None;
            for row in subscriptions.values() {
                if row.planned.identifier() != family {
                    continue;
                }
                if !instance_compatible(&row.planned, &extracted) {
                    continue;
                }
                if matched.is_some() {
                    return None;
                }
                matched = Some(row.planned.identity());
            }
            matched
        }
        WsObservation::Heartbeat(_) | WsObservation::Unknown(_) => None,
    }
}

#[derive(Default)]
struct InstanceFields {
    coin: Option<String>,
    user: Option<String>,
    interval: Option<String>,
}

fn extract_instance_fields(payload: &Bytes) -> InstanceFields {
    let Ok(value) = serde_json::from_slice::<Value>(payload) else {
        return InstanceFields::default();
    };
    let Some(data) = value.get("data") else {
        return InstanceFields::default();
    };
    InstanceFields {
        coin: first_string_field(data, &["coin", "s"]),
        user: first_string_field(data, &["user"]),
        interval: first_string_field(data, &["interval", "i"]),
    }
}

fn first_string_field(data: &Value, keys: &[&str]) -> Option<String> {
    match data {
        Value::Object(object) => keys
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str).map(str::to_owned)),
        Value::Array(items) => {
            let mut found = None;
            for item in items {
                let Some(value) = keys
                    .iter()
                    .find_map(|key| item.get(*key).and_then(Value::as_str))
                else {
                    continue;
                };
                match &found {
                    None => found = Some(value.to_owned()),
                    Some(existing) if existing == value => {}
                    Some(_) => return None,
                }
            }
            found
        }
        _ => None,
    }
}

fn instance_compatible(planned: &PlannedSubscription, extracted: &InstanceFields) -> bool {
    extracted
        .coin
        .as_deref()
        .is_none_or(|coin| planned.coin() == Some(coin))
        && extracted
            .user
            .as_deref()
            .is_none_or(|user| planned.user() == Some(user))
        && extracted
            .interval
            .as_deref()
            .is_none_or(|interval| planned.interval() == Some(interval))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription_plan::{
        PlannedConnection, PlannedSubscription, PlannerConfig, PlannerInput, SubscriptionDemand,
        plan_subscriptions,
    };
    use bytes::Bytes;

    fn active_session() -> WsSession {
        open_session(
            active_connection(vec![
                SubscriptionDemand::new("notification")
                    .with_user("0x0000000000000000000000000000000000000001"),
            ]),
            ProcessIpBudget::official(),
            0,
        )
    }

    fn active_connection(demands: Vec<SubscriptionDemand>) -> PlannedConnection {
        let plan = plan_subscriptions(PlannerConfig::official(), PlannerInput::new(demands));
        plan.connections()
            .iter()
            .find(|connection| !matches!(connection.kind(), PlannedConnectionKind::FailoverReserve))
            .cloned()
            .expect("active connection")
    }

    fn open_session(
        connection: PlannedConnection,
        budget: ProcessIpBudget,
        now_millis: u64,
    ) -> WsSession {
        WsSession::open(
            connection,
            OfficialWsLimits::official(),
            budget,
            now_millis,
            1_000,
            5_000,
            250,
            8_000,
            10_000,
        )
        .expect("session")
    }

    fn identity_hex(planned: &PlannedSubscription) -> String {
        hex::encode(planned.identity().as_bytes())
    }

    #[test]
    fn snapshot_flagged_incremental_is_replace_not_event() {
        let payload = Bytes::from_static(
            br#"{"channel":"notification","data":{"notification":"hi","isSnapshot":true}}"#,
        );
        let observation = parse_ws_message(payload).expect("parse");
        assert!(matches!(observation, WsObservation::Incremental(_)));
        assert!(
            observation
                .identifier()
                .is_some_and(|identifier| identifier == "notification")
        );
        if let WsObservation::Incremental(incremental) = &observation {
            assert!(incremental.flagged_is_snapshot());
        }
        assert_eq!(
            classify_inbound(&observation, None),
            InboundClass::SnapshotReplace
        );
        assert_eq!(
            classify_inbound(&observation, Some(observation.content_hash())),
            InboundClass::DuplicateSnapshot
        );
    }

    #[test]
    fn reconnect_duplicate_snapshot_is_not_a_new_event() {
        let mut session = active_session();
        let payload = Bytes::from_static(
            br#"{"channel":"notification","data":{"notification":"hi","isSnapshot":true}}"#,
        );
        let first = session.ingest(payload.clone(), 10);
        assert_eq!(first.class(), InboundClass::SnapshotReplace);
        session.begin_reconnect(20).expect("reconnect");
        let second = session.ingest(payload, 30);
        assert_eq!(second.class(), InboundClass::DuplicateSnapshot);
    }

    #[test]
    fn inactivity_sends_ping_and_stale_becomes_red() {
        let mut session = active_session();
        session.on_clock(5_000).expect("clock");
        let outgoing = session.drain_outgoing();
        assert!(outgoing.iter().any(|frame| frame.as_ref() == PING_FRAME));
        session.ingest(Bytes::from_static(br#"{"channel":"pong"}"#), 5_001);
        session.ingest(
            Bytes::from_static(br#"{"channel":"notification","data":{"notification":"x"}}"#),
            5_002,
        );
        session.on_clock(20_000).expect("stale");
        assert!(!session.red_subscriptions().is_empty());
    }

    #[test]
    fn orderly_shutdown_unsubscribes() {
        let mut session = active_session();
        let _ = session.drain_outgoing();
        session.shutdown(1).expect("shutdown");
        let outgoing = session.drain_outgoing();
        assert!(outgoing.iter().any(|frame| {
            frame
                .windows(b"unsubscribe".len())
                .any(|window| window == b"unsubscribe")
        }));
        assert_eq!(session.state(), SessionState::Shutdown);
    }

    #[test]
    fn connect_rate_rejects_the_31st_connection_in_a_minute() {
        let mut window = MinuteWindow::new(30);
        for index in 0_u32..30 {
            assert!(window.try_add(u64::from(index)));
        }
        assert!(!window.try_add(29));
        let mut later = MinuteWindow::new(30);
        for index in 0_u32..30 {
            assert!(later.try_add(u64::from(index)));
        }
        assert!(later.try_add(60_000));
    }

    #[test]
    fn two_sessions_share_the_process_connect_budget() {
        let budget = ProcessIpBudget::new(30, 2_000);
        let first = active_connection(vec![
            SubscriptionDemand::new("notification")
                .with_user("0x0000000000000000000000000000000000000001"),
        ]);
        let second = active_connection(vec![
            SubscriptionDemand::new("notification")
                .with_user("0x0000000000000000000000000000000000000002"),
        ]);
        let mut left = open_session(first, budget.clone(), 0);
        let mut right = open_session(second, budget, 0);
        let mut accepted = 2_u32;
        for tick in 1_u64..60 {
            if left.begin_reconnect(tick).is_ok() {
                accepted += 1;
            }
            if right.begin_reconnect(tick).is_ok() {
                accepted += 1;
            }
        }
        assert_eq!(accepted, 30);
        assert_eq!(
            left.begin_reconnect(59),
            Err(WsSessionError::ConnectRateLimited)
        );
    }

    #[test]
    fn dead_second_trades_subscription_goes_red() {
        let connection = active_connection(vec![
            SubscriptionDemand::new("trades").with_coin("BTC"),
            SubscriptionDemand::new("trades").with_coin("ETH"),
        ]);
        let btc = identity_hex(
            connection
                .subscriptions()
                .iter()
                .find(|planned| planned.coin() == Some("BTC"))
                .expect("btc"),
        );
        let eth = identity_hex(
            connection
                .subscriptions()
                .iter()
                .find(|planned| planned.coin() == Some("ETH"))
                .expect("eth"),
        );
        let mut session = open_session(connection, ProcessIpBudget::official(), 0);
        session.ingest(
            Bytes::from_static(
                br#"{"channel":"trades","data":[{"coin":"BTC","side":"B","px":"1","sz":"1","hash":"0xabc","time":1,"tid":1,"users":["0x0000000000000000000000000000000000000001","0x0000000000000000000000000000000000000002"]}]}"#,
            ),
            100,
        );
        session.on_clock(10_000).expect("stale");
        assert_eq!(session.health(&btc), Some(SubscriptionHealth::Green));
        assert_eq!(session.health(&eth), Some(SubscriptionHealth::Red));
        assert_eq!(session.red_subscriptions(), vec![eth]);
    }

    #[test]
    fn snapshot_hashes_are_per_subscription() {
        let connection = active_connection(vec![
            SubscriptionDemand::new("l2Book").with_coin("BTC"),
            SubscriptionDemand::new("l2Book").with_coin("ETH"),
        ]);
        let mut session = open_session(connection, ProcessIpBudget::official(), 0);
        let btc = Bytes::from_static(
            br#"{"channel":"l2Book","data":{"coin":"BTC","time":1,"levels":[[{"px":"1","sz":"1","n":1}],[{"px":"2","sz":"1","n":1}]]}}"#,
        );
        let eth = Bytes::from_static(
            br#"{"channel":"l2Book","data":{"coin":"ETH","time":1,"levels":[[{"px":"1","sz":"1","n":1}],[{"px":"2","sz":"1","n":1}]]}}"#,
        );
        assert_eq!(
            session.ingest(btc.clone(), 10).class(),
            InboundClass::SnapshotReplace
        );
        assert_eq!(
            session.ingest(eth.clone(), 11).class(),
            InboundClass::SnapshotReplace
        );
        session.begin_reconnect(20).expect("reconnect");
        assert_eq!(
            session.ingest(btc, 30).class(),
            InboundClass::DuplicateSnapshot
        );
        assert_eq!(
            session.ingest(eth, 31).class(),
            InboundClass::DuplicateSnapshot
        );
    }

    #[test]
    fn restored_snapshot_hashes_survive_a_new_session() {
        let connection =
            active_connection(vec![SubscriptionDemand::new("l2Book").with_coin("BTC")]);
        let mut session = open_session(connection.clone(), ProcessIpBudget::official(), 0);
        let btc = Bytes::from_static(
            br#"{"channel":"l2Book","data":{"coin":"BTC","time":1,"levels":[[{"px":"1","sz":"1","n":1}],[{"px":"2","sz":"1","n":1}]]}}"#,
        );
        assert_eq!(
            session.ingest(btc.clone(), 10).class(),
            InboundClass::SnapshotReplace
        );
        let hashes = session.snapshot_hashes();
        let mut restarted = open_session(connection, ProcessIpBudget::official(), 0);
        restarted.restore_snapshot_hashes(&hashes);
        assert_eq!(
            restarted.ingest(btc, 20).class(),
            InboundClass::DuplicateSnapshot
        );
    }

    #[test]
    fn posts_are_forbidden() {
        let mut session = active_session();
        assert_eq!(session.post(0), Err(WsSessionError::PostsForbidden));
    }
}
