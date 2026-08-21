use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Mutex;

use bytes::Bytes;
use domain_types::{ChainId, KnownTime, SourceId};
use hl_capture::{
    InboundClass, MemoryWsFanout, NoWsFaults, OfficialWsLimits, PlannerConfig, PlannerInput,
    ProcessIpBudget, RawPortWsArchive, SessionState, SubscriptionDemand, WsArchive,
    WsCaptureCoordinator, WsCaptureError, WsFaultInjector, WsFaultPoint, WsSession,
    WsSessionCheckpoint, classify_inbound, guard_ws_url, plan_subscriptions,
    replay_official_ws_fixtures, ws_request_hash,
};
use hl_protocol::ObservationClass;
use hl_protocol::ws::parse_ws_message;

fn now() -> KnownTime {
    KnownTime::from_unix_micros(1_700_000_000_000_000).expect("time")
}

struct MemoryWsStore {
    bodies: BTreeMap<String, Bytes>,
    holes: BTreeSet<String>,
}

impl MemoryWsStore {
    fn new() -> Self {
        Self {
            bodies: BTreeMap::new(),
            holes: BTreeSet::new(),
        }
    }

    fn hide(&mut self, archive_ref: &str) {
        self.holes.insert(archive_ref.to_owned());
    }
}

impl WsArchive for MemoryWsStore {
    fn put(
        &mut self,
        body: &[u8],
        received_at: KnownTime,
        request_hash: blake3::Hash,
        observation_class: ObservationClass,
    ) -> Result<String, WsCaptureError> {
        if observation_class == ObservationClass::CommittedBlock {
            return Err(WsCaptureError::CommittedLane);
        }
        if received_at.unix_micros() == 1 {
            return Err(WsCaptureError::Archive);
        }
        if request_hash == blake3::hash(body) {
            return Err(WsCaptureError::RequestIdentity);
        }
        let archive_ref = format!("ws-{}", hex::encode(blake3::hash(body).as_bytes()));
        self.bodies
            .entry(archive_ref.clone())
            .or_insert_with(|| Bytes::copy_from_slice(body));
        Ok(archive_ref)
    }

    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, WsCaptureError> {
        if self.holes.contains(archive_ref) {
            return Ok(None);
        }
        Ok(self.bodies.get(archive_ref).cloned())
    }
}

struct OneShotWsFault {
    point: Mutex<Option<WsFaultPoint>>,
}

impl OneShotWsFault {
    fn new(point: WsFaultPoint) -> Self {
        Self {
            point: Mutex::new(Some(point)),
        }
    }
}

impl WsFaultInjector for OneShotWsFault {
    fn check(&self, point: WsFaultPoint) -> Result<(), WsCaptureError> {
        let mut selected = self.point.lock().expect("fault lock");
        if selected.as_ref() == Some(&point) {
            selected.take();
            Err(WsCaptureError::InjectedFault(point))
        } else {
            Ok(())
        }
    }
}

struct AlwaysWsFault {
    point: WsFaultPoint,
}

impl WsFaultInjector for AlwaysWsFault {
    fn check(&self, point: WsFaultPoint) -> Result<(), WsCaptureError> {
        if point == self.point {
            Err(WsCaptureError::InjectedFault(point))
        } else {
            Ok(())
        }
    }
}

fn notification_session() -> WsSession {
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
        .find(|connection| {
            !matches!(
                connection.kind(),
                hl_capture::PlannedConnectionKind::FailoverReserve
            )
        })
        .cloned()
        .expect("active");
    WsSession::open(
        connection,
        OfficialWsLimits::official(),
        ProcessIpBudget::official(),
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
fn non_allowlisted_and_plaintext_ws_urls_fail() {
    assert_eq!(
        guard_ws_url("ws://api.hyperliquid.xyz/ws"),
        Err(WsCaptureError::TlsRequired)
    );
    assert_eq!(
        guard_ws_url("wss://example.com/ws"),
        Err(WsCaptureError::HostNotAllowlisted)
    );
    assert_eq!(
        guard_ws_url("wss://api.hyperliquid.xyz/exchange"),
        Err(WsCaptureError::ExchangeForbidden)
    );
    assert_eq!(
        guard_ws_url("wss://api.hyperliquid.xyz/ws"),
        Ok("wss://api.hyperliquid.xyz/ws".to_owned())
    );
    assert_eq!(
        guard_ws_url("wss://api.hyperliquid-testnet.xyz/ws"),
        Ok("wss://api.hyperliquid-testnet.xyz/ws".to_owned())
    );
}

#[test]
fn n8_request_hash_is_not_the_body_hash() {
    let body = br#"{"channel":"notification","data":{"notification":"a"}}"#;
    let first = ws_request_hash(blake3::hash(b"sub-a"), 0, 1, now().unix_micros());
    let second = ws_request_hash(blake3::hash(b"sub-a"), 0, 2, now().unix_micros());
    assert_ne!(first, blake3::hash(body));
    assert_ne!(second, blake3::hash(body));
    assert_ne!(first, second);
}

#[test]
fn snapshot_flagged_incremental_is_applied_as_replace() {
    let payload = Bytes::from_static(
        br#"{"channel":"notification","data":{"notification":"hi","isSnapshot":true}}"#,
    );
    let observation = parse_ws_message(payload).expect("parse");
    assert_eq!(
        classify_inbound(&observation, None),
        InboundClass::SnapshotReplace
    );
    assert_ne!(
        observation.observation_class(),
        ObservationClass::CommittedBlock
    );
}

#[test]
fn crash_after_archive_replays_from_pending() {
    let mut archive = MemoryWsStore::new();
    let mut fanout = MemoryWsFanout::new(8);
    let mut session = notification_session();
    let mut checkpoint = WsSessionCheckpoint::new(session.slot());
    let faults = OneShotWsFault::new(WsFaultPoint::AfterArchive);
    let payload =
        Bytes::from_static(br#"{"channel":"notification","data":{"notification":"keep"}}"#);
    let planned = session_subscription(&session);
    let error = WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &faults,
        None,
    )
    .ingest(payload.clone(), Some(&planned), now())
    .expect_err("fault");
    assert_eq!(
        error,
        WsCaptureError::InjectedFault(WsFaultPoint::AfterArchive)
    );
    assert_eq!(checkpoint.pending().len(), 1);
    assert!(fanout.items().is_empty());
    let class = WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .replay_pending(now())
    .expect("replay");
    assert_eq!(class, InboundClass::IncrementalEvent);
    assert_eq!(fanout.items().len(), 1);
    assert!(checkpoint.pending().is_empty());
    let published = &fanout.items()[0];
    assert_ne!(published.request_hash(), blake3::hash(&payload));
}

#[test]
fn bounded_backlog_does_not_drop_archived_messages() {
    let mut archive = MemoryWsStore::new();
    let mut fanout = MemoryWsFanout::new(1);
    let mut session = notification_session();
    let mut checkpoint = WsSessionCheckpoint::new(session.slot());
    let planned = session_subscription(&session);
    WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .ingest(
        Bytes::from_static(br#"{"channel":"notification","data":{"notification":"one"}}"#),
        Some(&planned),
        now(),
    )
    .expect("first");
    let error = WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .ingest(
        Bytes::from_static(br#"{"channel":"notification","data":{"notification":"two"}}"#),
        Some(&planned),
        now(),
    )
    .expect_err("full");
    assert_eq!(error, WsCaptureError::BacklogFull);
    assert_eq!(checkpoint.pending().len(), 1);
    fanout.pop_front();
    WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .replay_pending(now())
    .expect("drain");
    assert_eq!(fanout.items().len(), 1);
    assert!(checkpoint.pending().is_empty());
}

#[test]
fn snapshot_backlog_retry_is_replace_not_duplicate() {
    let mut archive = MemoryWsStore::new();
    let mut fanout = MemoryWsFanout::new(1);
    let mut session = notification_session();
    let mut checkpoint = WsSessionCheckpoint::new(session.slot());
    let planned = session_subscription(&session);
    WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .ingest(
        Bytes::from_static(br#"{"channel":"notification","data":{"notification":"fill"}}"#),
        Some(&planned),
        now(),
    )
    .expect("fill");
    let snapshot = Bytes::from_static(
        br#"{"channel":"notification","data":{"notification":"hi","isSnapshot":true}}"#,
    );
    let error = WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .ingest(snapshot, Some(&planned), now())
    .expect_err("full");
    assert_eq!(error, WsCaptureError::BacklogFull);
    assert_eq!(checkpoint.pending().len(), 1);
    fanout.pop_front();
    let class = WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .replay_pending(now())
    .expect("drain");
    assert_eq!(class, InboundClass::SnapshotReplace);
    assert_eq!(fanout.items().len(), 1);
    assert_eq!(fanout.items()[0].class(), InboundClass::SnapshotReplace);
    assert!(checkpoint.pending().is_empty());
}

#[test]
fn durable_ws_archive_uses_real_received_at() {
    let directory = tempfile::tempdir().expect("temp");
    let mut archive = RawPortWsArchive::open(
        directory.path().join("raw"),
        ChainId::new("mainnet").expect("chain"),
        SourceId::new("official-ws").expect("source"),
        1_048_576,
    )
    .expect("open");
    let body = br#"{"channel":"pong"}"#;
    let request = ws_request_hash(blake3::hash(b"pong"), 0, 1, now().unix_micros());
    let archive_ref = archive
        .put(body, now(), request, ObservationClass::ProvisionalFeed)
        .expect("put");
    let stored = archive.get(&archive_ref).expect("get").expect("body");
    assert_eq!(stored.as_ref(), body);
}

#[test]
fn official_ws_fixtures_never_classify_as_committed() {
    let fixtures =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/hyperliquid/official-ws");
    let classes = replay_official_ws_fixtures(&fixtures).expect("replay");
    assert!(!classes.is_empty());
    assert!(classes.contains(&InboundClass::Quarantine));
}

#[test]
fn schema_drift_unknown_state_affecting_is_quarantine() {
    let payload = Bytes::from_static(br#"{"channel":"mysteryWalletFeed","data":{"fills":[]}}"#);
    let error = parse_ws_message(payload).expect_err("drift");
    assert_eq!(error.reason_code(), "source.schema_drift");
}

#[test]
fn orderly_session_shutdown_is_terminal() {
    let mut session = notification_session();
    session.shutdown(1).expect("shutdown");
    assert_eq!(session.state(), SessionState::Shutdown);
}

#[test]
#[ignore]
fn live_official_all_mids_requires_opt_in() {
    let url = guard_ws_url("wss://api.hyperliquid.xyz/ws").expect("allowlisted");
    assert_eq!(url, "wss://api.hyperliquid.xyz/ws");
}

#[test]
fn missing_archive_does_not_strand_later_pending() {
    let mut archive = MemoryWsStore::new();
    let mut fanout = MemoryWsFanout::new(8);
    let mut session = notification_session();
    let mut checkpoint = WsSessionCheckpoint::new(session.slot());
    let faults = AlwaysWsFault {
        point: WsFaultPoint::AfterArchive,
    };
    let planned = session_subscription(&session);
    let first =
        Bytes::from_static(br#"{"channel":"notification","data":{"notification":"first"}}"#);
    let second =
        Bytes::from_static(br#"{"channel":"notification","data":{"notification":"second"}}"#);
    WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &faults,
        None,
    )
    .ingest(first, Some(&planned), now())
    .expect_err("fault first");
    WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &faults,
        None,
    )
    .ingest(second, Some(&planned), now())
    .expect_err("fault second");
    assert_eq!(checkpoint.pending().len(), 2);
    let missing = checkpoint.pending()[0].archive_ref().to_owned();
    archive.hide(&missing);
    let class = WsCaptureCoordinator::new(
        &mut archive,
        &mut fanout,
        &mut session,
        &mut checkpoint,
        &NoWsFaults,
        None,
    )
    .replay_pending(now())
    .expect("drain");
    assert_eq!(class, InboundClass::IncrementalEvent);
    assert_eq!(fanout.items().len(), 1);
    assert!(checkpoint.pending().is_empty());
}

fn session_subscription(_session: &WsSession) -> hl_capture::PlannedSubscription {
    let plan = plan_subscriptions(
        PlannerConfig::official(),
        PlannerInput::new(vec![
            SubscriptionDemand::new("notification")
                .with_user("0x0000000000000000000000000000000000000001"),
        ]),
    );
    plan.connections()
        .iter()
        .find(|connection| !connection.subscriptions().is_empty())
        .and_then(|connection| connection.subscriptions().first())
        .cloned()
        .expect("planned subscription")
}
