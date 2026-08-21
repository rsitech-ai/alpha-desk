//! Official WebSocket session lifecycle.
//!
//! Clock-driven ping, inactivity, staleness, reconnect jitter, and snapshot
//! apply policy. Transport bytes are injected by the adapter.

use std::collections::{BTreeMap, VecDeque};

use bytes::Bytes;
use hl_protocol::ws::{WsObservation, encode_subscribe, encode_unsubscribe, parse_ws_message};

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
    connect_window: MinuteWindow,
    outgoing_window: MinuteWindow,
    inflight_posts: u32,
    max_inflight_posts: u32,
    attempt: u32,
}

impl WsSession {
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        connection: PlannedConnection,
        limits: OfficialWsLimits,
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
                connect_window: MinuteWindow::new(limits.max_new_connections_per_minute()),
                outgoing_window: MinuteWindow::new(limits.max_outgoing_per_minute()),
                inflight_posts: 0,
                max_inflight_posts: limits.max_inflight_posts(),
                attempt: 0,
            });
        }
        let mut connect_window = MinuteWindow::new(limits.max_new_connections_per_minute());
        connect_window
            .try_add(now_millis)
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
            connect_window,
            outgoing_window: MinuteWindow::new(limits.max_outgoing_per_minute()),
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
            if let Some(last) = row.last_data_millis
                && now_millis.saturating_sub(last) >= row.stale_after_millis
            {
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

    pub fn ingest(&mut self, payload: Bytes, now_millis: u64) -> AppliedInbound {
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
            Ok(observation) => self.apply_observation(observation, payload, now_millis),
        }
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
        if !self.connect_window.try_add(now_millis) {
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

    fn apply_observation(
        &mut self,
        observation: WsObservation,
        payload: Bytes,
        now_millis: u64,
    ) -> AppliedInbound {
        let identity = match &observation {
            WsObservation::Ack(ack) => Some(ack.subscription().identity()),
            WsObservation::Snapshot(_) | WsObservation::Incremental(_) => self
                .subscriptions
                .iter()
                .find(|(_, row)| row.planned.identifier() == observation.identifier().unwrap_or(""))
                .and_then(|(key, _)| decode_identity(key)),
            WsObservation::Heartbeat(_) | WsObservation::Unknown(_) => None,
        };
        if matches!(observation, WsObservation::Heartbeat(_)) {
            self.awaiting_pong = false;
        }
        if let Some(identity) = identity {
            let key = hex::encode(identity.as_bytes());
            if let Some(row) = self.subscriptions.get_mut(&key) {
                row.last_data_millis = Some(now_millis);
                if row.health != SubscriptionHealth::Red {
                    row.health = SubscriptionHealth::Green;
                }
            }
        }
        let previous = identity.and_then(|id| {
            self.subscriptions
                .get(&hex::encode(id.as_bytes()))
                .and_then(|row| row.snapshot_hash)
        });
        let class = classify_inbound(&observation, previous);
        if matches!(
            class,
            InboundClass::SnapshotReplace | InboundClass::DuplicateSnapshot
        ) && let Some(identity) = identity
        {
            let key = hex::encode(identity.as_bytes());
            if let Some(row) = self.subscriptions.get_mut(&key) {
                row.snapshot_hash = Some(observation.content_hash());
            }
        }
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
        if !self.outgoing_window.try_add(now_millis) {
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

fn decode_identity(hex_value: &str) -> Option<blake3::Hash> {
    let bytes = hex::decode(hex_value).ok()?;
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(blake3::Hash::from(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription_plan::{
        PlannerConfig, PlannerInput, SubscriptionDemand, plan_subscriptions,
    };
    use bytes::Bytes;

    fn active_session() -> WsSession {
        let plan = plan_subscriptions(
            PlannerConfig::official(),
            PlannerInput::new(vec![
                SubscriptionDemand::new("notification")
                    .with_user("0x0000000000000000000000000000000000000001"),
            ]),
        );
        let connection = plan
            .connections()
            .iter()
            .find(|connection| !matches!(connection.kind(), PlannedConnectionKind::FailoverReserve))
            .cloned()
            .expect("active connection");
        WsSession::open(
            connection,
            OfficialWsLimits::official(),
            0,
            1_000,
            5_000,
            250,
            8_000,
            10_000,
        )
        .expect("session")
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
    fn posts_are_forbidden() {
        let mut session = active_session();
        assert_eq!(session.post(0), Err(WsSessionError::PostsForbidden));
    }
}
