//! Official public WebSocket adapter. Archive exact bytes before fan-out.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use domain_types::{ChainId, KnownTime, SourceId};
use hl_protocol::ws::WsObservation;
use hl_protocol::{
    ObservationClass, ObservationError, ReceiveTimestamps, SourceCursor, SourceError,
    SourceObservation,
};
use serde::{Deserialize, Serialize};
use storage_ports::{
    ArchiveError, RawObservationArchive, RawObservationBatch, RawObservationRange,
};

use crate::config::OFFICIAL_WS_URLS;
use crate::subscription_plan::{OfficialWsLimits, PlannedSubscription};
use crate::ws_session::{AppliedInbound, InboundClass, WsSession, WsSessionError};

const CHECKPOINT_SCHEMA: &str = "hl.ws-session-checkpoint.v1";
const WS_SOURCE_VERSION: &str = "official-ws-v1";
const WS_PARSER_SCHEMA: &str = "ws-capture-v1";
const WS_ARCHIVE_BUILD_ID: &str = "hl-capture-ws";
const REQUEST_CONTEXT: &[u8] = b"hl.ws-request.v1\0";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsFaultPoint {
    AfterArchive,
    AfterFanout,
}

pub trait WsFaultInjector: Send + Sync {
    fn check(&self, point: WsFaultPoint) -> Result<(), WsCaptureError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoWsFaults;

impl WsFaultInjector for NoWsFaults {
    fn check(&self, _point: WsFaultPoint) -> Result<(), WsCaptureError> {
        Ok(())
    }
}

pub fn guard_ws_url(url: &str) -> Result<String, WsCaptureError> {
    if url.is_empty() || url.contains('@') {
        return Err(WsCaptureError::HostNotAllowlisted);
    }
    if url.contains("/exchange") {
        return Err(WsCaptureError::ExchangeForbidden);
    }
    if url.starts_with("ws://") || url.starts_with("http://") {
        return Err(WsCaptureError::TlsRequired);
    }
    if !url.starts_with("wss://") {
        return Err(WsCaptureError::TlsRequired);
    }
    let without_query = url.split(['?', '#']).next().unwrap_or(url);
    let trimmed = without_query
        .strip_suffix('/')
        .filter(|stripped| *stripped != "wss:/")
        .unwrap_or(without_query);
    let canonical = if trimmed.ends_with("/ws") {
        trimmed.to_owned()
    } else if OFFICIAL_WS_URLS
        .iter()
        .any(|allowed| allowed.trim_end_matches("/ws") == trimmed)
    {
        format!("{trimmed}/ws")
    } else {
        trimmed.to_owned()
    };
    if !OFFICIAL_WS_URLS.contains(&canonical.as_str()) && !OFFICIAL_WS_URLS.contains(&trimmed) {
        return Err(WsCaptureError::HostNotAllowlisted);
    }
    Ok(canonical)
}

#[must_use]
pub fn ws_request_hash(
    subscription_identity: blake3::Hash,
    connection_slot: u8,
    inbound_seq: u64,
    received_at_micros: i64,
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(REQUEST_CONTEXT);
    hasher.update(subscription_identity.as_bytes());
    hasher.update(&[connection_slot]);
    hasher.update(&inbound_seq.to_be_bytes());
    hasher.update(&received_at_micros.to_be_bytes());
    hasher.finalize()
}

fn archive_ref_for(body: &[u8]) -> String {
    format!("ws-{}", hex::encode(blake3::hash(body).as_bytes()))
}

pub trait WsArchive {
    fn put(
        &mut self,
        body: &[u8],
        received_at: KnownTime,
        request_hash: blake3::Hash,
        observation_class: ObservationClass,
    ) -> Result<String, WsCaptureError>;
    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, WsCaptureError>;
}

pub struct RawPortWsArchive {
    archive: Arc<dyn RawObservationArchive>,
    chain_id: ChainId,
    source_id: SourceId,
    max_payload_bytes: usize,
}

impl std::fmt::Debug for RawPortWsArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RawPortWsArchive")
            .field("chain_id", &self.chain_id)
            .field("source_id", &self.source_id)
            .finish_non_exhaustive()
    }
}

impl RawPortWsArchive {
    #[must_use]
    pub fn new(
        archive: Arc<dyn RawObservationArchive>,
        chain_id: ChainId,
        source_id: SourceId,
        max_payload_bytes: usize,
    ) -> Self {
        Self {
            archive,
            chain_id,
            source_id,
            max_payload_bytes,
        }
    }

    pub fn open(
        root: impl AsRef<Path>,
        chain_id: ChainId,
        source_id: SourceId,
        max_payload_bytes: usize,
    ) -> Result<Self, WsCaptureError> {
        let archive = LocalParquetArchive::open(
            root,
            ArchiveConfig::production(WS_ARCHIVE_BUILD_ID).map_err(|_| WsCaptureError::Archive)?,
        )
        .map_err(|_| WsCaptureError::Archive)?;
        Ok(Self::new(
            Arc::new(archive),
            chain_id,
            source_id,
            max_payload_bytes,
        ))
    }

    fn read_body(&self, archive_ref: &str) -> Result<Option<Bytes>, WsCaptureError> {
        let range =
            RawObservationRange::try_new(archive_ref, 0, 0).map_err(|_| WsCaptureError::Archive)?;
        match self
            .archive
            .read_observations(&self.chain_id, &self.source_id, range)
        {
            Ok(mut iterator) => iterator
                .next()
                .transpose()
                .map_err(|_| WsCaptureError::Archive)
                .map(|observation| observation.map(|item| item.payload().clone())),
            Err(ArchiveError::RangeUnavailable) => Ok(None),
            Err(_) => Err(WsCaptureError::Archive),
        }
    }
}

impl WsArchive for RawPortWsArchive {
    fn put(
        &mut self,
        body: &[u8],
        received_at: KnownTime,
        request_hash: blake3::Hash,
        observation_class: ObservationClass,
    ) -> Result<String, WsCaptureError> {
        if body.is_empty() {
            return Err(WsCaptureError::EmptyBody);
        }
        if observation_class == ObservationClass::CommittedBlock {
            return Err(WsCaptureError::CommittedLane);
        }
        if request_hash == blake3::hash(body) {
            return Err(WsCaptureError::RequestIdentity);
        }
        let archive_ref = archive_ref_for(body);
        if self.read_body(&archive_ref)?.is_some() {
            return Ok(archive_ref);
        }
        let payload = Bytes::copy_from_slice(body);
        let observation = SourceObservation::new(
            self.source_id.clone(),
            WS_SOURCE_VERSION,
            observation_class,
            SourceCursor::new(archive_ref.clone(), 0).map_err(|_| WsCaptureError::Archive)?,
            ReceiveTimestamps::new(received_at.unix_micros(), 0)
                .map_err(|_| WsCaptureError::Archive)?,
            WS_PARSER_SCHEMA,
            payload,
            Vec::new(),
            self.max_payload_bytes,
        )
        .map_err(|error| match error {
            ObservationError::EmptyPayload => WsCaptureError::EmptyBody,
            _ => WsCaptureError::Archive,
        })?;
        let spool_hash = *blake3::hash(body).as_bytes();
        let batch = RawObservationBatch::try_new(
            self.chain_id.clone(),
            vec![observation],
            spool_hash,
            spool_hash,
        )
        .map_err(|_| WsCaptureError::Archive)?;
        self.archive
            .append_batch(&batch)
            .map_err(|_| WsCaptureError::Archive)?;
        Ok(archive_ref)
    }

    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, WsCaptureError> {
        self.read_body(archive_ref)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsPublished {
    archive_ref: String,
    request_hash: blake3::Hash,
    class: InboundClass,
    observation_class: ObservationClass,
    payload: Bytes,
}

impl WsPublished {
    #[must_use]
    pub fn archive_ref(&self) -> &str {
        &self.archive_ref
    }

    #[must_use]
    pub const fn request_hash(&self) -> blake3::Hash {
        self.request_hash
    }

    #[must_use]
    pub const fn class(&self) -> InboundClass {
        self.class
    }

    #[must_use]
    pub const fn observation_class(&self) -> ObservationClass {
        self.observation_class
    }

    #[must_use]
    pub fn payload(&self) -> &Bytes {
        &self.payload
    }
}

pub trait WsFanout {
    fn push(&mut self, item: WsPublished) -> Result<(), WsCaptureError>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[derive(Debug, Default)]
pub struct MemoryWsFanout {
    items: Vec<WsPublished>,
    capacity: usize,
}

impl MemoryWsFanout {
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            items: Vec::new(),
            capacity,
        }
    }

    #[must_use]
    pub fn items(&self) -> &[WsPublished] {
        &self.items
    }

    pub fn pop_front(&mut self) -> Option<WsPublished> {
        if self.items.is_empty() {
            None
        } else {
            Some(self.items.remove(0))
        }
    }
}

impl WsFanout for MemoryWsFanout {
    fn push(&mut self, item: WsPublished) -> Result<(), WsCaptureError> {
        if self.items.len() >= self.capacity {
            return Err(WsCaptureError::BacklogFull);
        }
        self.items.push(item);
        Ok(())
    }

    fn len(&self) -> usize {
        self.items.len()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WsPendingItem {
    archive_ref: String,
    request_hash: String,
    inbound_seq: u64,
    subscription_identity: String,
}

impl WsPendingItem {
    #[must_use]
    pub fn archive_ref(&self) -> &str {
        &self.archive_ref
    }

    #[must_use]
    pub fn request_hash(&self) -> &str {
        &self.request_hash
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WsSessionCheckpoint {
    schema_version: String,
    connection_slot: u8,
    inbound_seq: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending: Vec<WsPendingItem>,
}

impl WsSessionCheckpoint {
    #[must_use]
    pub fn new(connection_slot: u8) -> Self {
        Self {
            schema_version: CHECKPOINT_SCHEMA.to_owned(),
            connection_slot,
            inbound_seq: 0,
            pending: Vec::new(),
        }
    }

    #[must_use]
    pub fn pending(&self) -> &[WsPendingItem] {
        &self.pending
    }

    #[must_use]
    pub const fn inbound_seq(&self) -> u64 {
        self.inbound_seq
    }

    pub fn persist_to(&self, directory: &Path) -> Result<(), WsCaptureError> {
        std::fs::create_dir_all(directory).map_err(|_| WsCaptureError::Checkpoint)?;
        let path = checkpoint_path(directory, self.connection_slot)?;
        let temporary = directory.join(format!(
            "{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or(WsCaptureError::Checkpoint)?
        ));
        let encoded = serde_json::to_vec_pretty(self).map_err(|_| WsCaptureError::Checkpoint)?;
        std::fs::write(&temporary, encoded).map_err(|_| WsCaptureError::Checkpoint)?;
        std::fs::rename(&temporary, &path).map_err(|_| WsCaptureError::Checkpoint)
    }

    pub fn load_from(
        directory: &Path,
        connection_slot: u8,
    ) -> Result<Option<Self>, WsCaptureError> {
        let path = checkpoint_path(directory, connection_slot)?;
        match std::fs::read(&path) {
            Ok(bytes) => {
                let checkpoint: Self =
                    serde_json::from_slice(&bytes).map_err(|_| WsCaptureError::Checkpoint)?;
                if checkpoint.schema_version != CHECKPOINT_SCHEMA
                    || checkpoint.connection_slot != connection_slot
                {
                    return Err(WsCaptureError::Checkpoint);
                }
                Ok(Some(checkpoint))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(WsCaptureError::Checkpoint),
        }
    }
}

fn checkpoint_path(directory: &Path, slot: u8) -> Result<PathBuf, WsCaptureError> {
    Ok(directory.join(format!("ws-slot-{slot}.json")))
}

pub struct WsCaptureCoordinator<'a, A, F, I> {
    archive: &'a mut A,
    fanout: &'a mut F,
    session: &'a mut WsSession,
    checkpoint: &'a mut WsSessionCheckpoint,
    faults: &'a I,
    persist_dir: Option<&'a Path>,
}

impl<'a, A, F, I> WsCaptureCoordinator<'a, A, F, I>
where
    A: WsArchive,
    F: WsFanout,
    I: WsFaultInjector,
{
    pub fn new(
        archive: &'a mut A,
        fanout: &'a mut F,
        session: &'a mut WsSession,
        checkpoint: &'a mut WsSessionCheckpoint,
        faults: &'a I,
        persist_dir: Option<&'a Path>,
    ) -> Self {
        Self {
            archive,
            fanout,
            session,
            checkpoint,
            faults,
            persist_dir,
        }
    }

    pub fn ingest(
        &mut self,
        payload: Bytes,
        subscription: Option<&PlannedSubscription>,
        received_at: KnownTime,
    ) -> Result<InboundClass, WsCaptureError> {
        let identity = subscription
            .map(PlannedSubscription::identity)
            .unwrap_or_else(|| blake3::hash(b"unassigned"));
        let inbound_seq = self.checkpoint.inbound_seq.saturating_add(1);
        let request_hash = ws_request_hash(
            identity,
            self.session.slot(),
            inbound_seq,
            received_at.unix_micros(),
        );
        if request_hash == blake3::hash(&payload) {
            return Err(WsCaptureError::RequestIdentity);
        }
        self.checkpoint.inbound_seq = inbound_seq;
        let parsed_class = match parse_observation_class(&payload) {
            Ok(class) => class,
            Err(SourceError::SchemaDrift(_)) => ObservationClass::ProvisionalFeed,
            Err(_) => ObservationClass::ProvisionalFeed,
        };
        if parsed_class == ObservationClass::CommittedBlock {
            return Err(WsCaptureError::CommittedLane);
        }
        let archive_ref = self
            .archive
            .put(&payload, received_at, request_hash, parsed_class)?;
        self.checkpoint.pending.push(WsPendingItem {
            archive_ref: archive_ref.clone(),
            request_hash: hex::encode(request_hash.as_bytes()),
            inbound_seq,
            subscription_identity: hex::encode(identity.as_bytes()),
        });
        persist(self.checkpoint, self.persist_dir)?;
        self.faults.check(WsFaultPoint::AfterArchive)?;
        self.fan_out_pending(received_at)
    }

    pub fn replay_pending(
        &mut self,
        received_at: KnownTime,
    ) -> Result<InboundClass, WsCaptureError> {
        self.fan_out_pending(received_at)
    }

    fn fan_out_pending(&mut self, received_at: KnownTime) -> Result<InboundClass, WsCaptureError> {
        let pending = self.checkpoint.pending.clone();
        if pending.is_empty() {
            return Ok(InboundClass::Ack);
        }
        let mut last = InboundClass::Ack;
        for item in pending {
            if !self
                .checkpoint
                .pending
                .iter()
                .any(|queued| queued.request_hash == item.request_hash)
            {
                continue;
            }
            let body = self
                .archive
                .get(&item.archive_ref)?
                .ok_or(WsCaptureError::MissingArchive)?;
            let now_millis = u64::try_from(received_at.unix_micros() / 1_000).unwrap_or(0);
            let applied = self.session.ingest(body.clone(), now_millis);
            last = applied.class();
            let observation_class = observation_class_of(&applied);
            if observation_class == ObservationClass::CommittedBlock {
                return Err(WsCaptureError::CommittedLane);
            }
            let request_hash = decode_hash(&item.request_hash).ok_or(WsCaptureError::Checkpoint)?;
            match self.fanout.push(WsPublished {
                archive_ref: item.archive_ref.clone(),
                request_hash,
                class: applied.class(),
                observation_class,
                payload: body,
            }) {
                Ok(()) => {
                    self.checkpoint
                        .pending
                        .retain(|queued| queued.request_hash != item.request_hash);
                    persist(self.checkpoint, self.persist_dir)?;
                    self.faults.check(WsFaultPoint::AfterFanout)?;
                }
                Err(WsCaptureError::BacklogFull) => return Err(WsCaptureError::BacklogFull),
                Err(error) => return Err(error),
            }
        }
        Ok(last)
    }
}

fn persist(
    checkpoint: &WsSessionCheckpoint,
    directory: Option<&Path>,
) -> Result<(), WsCaptureError> {
    if let Some(directory) = directory {
        checkpoint.persist_to(directory)?;
    }
    Ok(())
}

fn parse_observation_class(payload: &[u8]) -> Result<ObservationClass, SourceError> {
    Ok(hl_protocol::ws::parse_ws_message(Bytes::copy_from_slice(payload))?.observation_class())
}

fn observation_class_of(applied: &AppliedInbound) -> ObservationClass {
    applied
        .observation()
        .map(WsObservation::observation_class)
        .unwrap_or(ObservationClass::ProvisionalFeed)
}

fn decode_hash(value: &str) -> Option<blake3::Hash> {
    let bytes = hex::decode(value).ok()?;
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(blake3::Hash::from(bytes))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum WsCaptureError {
    #[error("websocket host is not allowlisted")]
    HostNotAllowlisted,
    #[error("websocket URL must use wss")]
    TlsRequired,
    #[error("websocket capture refused an /exchange URL")]
    ExchangeForbidden,
    #[error("websocket archive failed")]
    Archive,
    #[error("websocket body is empty")]
    EmptyBody,
    #[error("websocket checkpoint failed")]
    Checkpoint,
    #[error("websocket archive ref is missing")]
    MissingArchive,
    #[error("websocket fan-out backlog is full")]
    BacklogFull,
    #[error("websocket request identity is invalid")]
    RequestIdentity,
    #[error("websocket capture cannot use the committed lane")]
    CommittedLane,
    #[error("websocket session failed")]
    Session(WsSessionError),
    #[error("injected websocket fault")]
    InjectedFault(WsFaultPoint),
}

impl WsCaptureError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::HostNotAllowlisted => "capture_ws.host_not_allowlisted",
            Self::TlsRequired => "capture_ws.tls_required",
            Self::ExchangeForbidden => "capture_ws.exchange_forbidden",
            Self::Archive => "capture_ws.archive",
            Self::EmptyBody => "capture_ws.empty_body",
            Self::Checkpoint => "capture_ws.checkpoint",
            Self::MissingArchive => "capture_ws.missing_archive",
            Self::BacklogFull => "capture_ws.backlog_full",
            Self::RequestIdentity => "capture_ws.request_identity",
            Self::CommittedLane => "capture_ws.committed_lane",
            Self::Session(error) => error.reason_code(),
            Self::InjectedFault(WsFaultPoint::AfterArchive) => "capture_ws.fault_after_archive",
            Self::InjectedFault(WsFaultPoint::AfterFanout) => "capture_ws.fault_after_fanout",
        }
    }
}

#[derive(Serialize)]
struct WsPlanStatusDoc<'a> {
    schema_version: &'static str,
    connections: usize,
    reserved_connections: u32,
    max_connections: u32,
    subscriptions: usize,
    max_subscriptions: u32,
    unique_users: usize,
    max_unique_users: u32,
    plan_hash: String,
    limits: OfficialWsLimitsDoc,
    red_subscriptions: &'a [String],
}

#[derive(Serialize)]
struct OfficialWsLimitsDoc {
    max_new_connections_per_minute: u32,
    max_outgoing_per_minute: u32,
    max_inflight_posts: u32,
}

pub fn encode_ws_plan_status(
    plan: &crate::subscription_plan::SubscriptionPlan,
    red_subscriptions: &[String],
) -> Result<Vec<u8>, WsCaptureError> {
    let limits = OfficialWsLimits::official();
    let canonical = plan.canonical_json();
    serde_json::to_vec(&WsPlanStatusDoc {
        schema_version: "hl.capture.ws-plan.v1",
        connections: plan.connections().len(),
        reserved_connections: plan.reserved_connections(),
        max_connections: limits.max_connections(),
        subscriptions: plan.subscription_count(),
        max_subscriptions: limits.max_subscriptions(),
        unique_users: plan.unique_users().len(),
        max_unique_users: limits.max_unique_users(),
        plan_hash: hex::encode(blake3::hash(canonical.as_bytes()).as_bytes()),
        limits: OfficialWsLimitsDoc {
            max_new_connections_per_minute: limits.max_new_connections_per_minute(),
            max_outgoing_per_minute: limits.max_outgoing_per_minute(),
            max_inflight_posts: limits.max_inflight_posts(),
        },
        red_subscriptions,
    })
    .map_err(|_| WsCaptureError::Checkpoint)
}

#[must_use]
pub fn ws_plan_status_path(status_path: &Path) -> PathBuf {
    status_path.with_file_name("ws-plan.json")
}

pub fn write_ws_plan_snapshot(status_path: &Path, body: &[u8]) -> Result<(), WsCaptureError> {
    let path = ws_plan_status_path(status_path);
    let parent = path.parent().ok_or(WsCaptureError::Checkpoint)?;
    let temporary = parent.join("ws-plan.json.tmp");
    std::fs::write(&temporary, body).map_err(|_| WsCaptureError::Checkpoint)?;
    std::fs::rename(&temporary, &path).map_err(|_| WsCaptureError::Checkpoint)
}

pub fn replay_official_ws_fixtures(
    fixtures_dir: &Path,
) -> Result<Vec<InboundClass>, WsCaptureError> {
    let mut classes = Vec::new();
    let mut names: Vec<_> = std::fs::read_dir(fixtures_dir)
        .map_err(|_| WsCaptureError::Archive)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    names.sort();
    for path in names {
        let payload = std::fs::read(&path).map_err(|_| WsCaptureError::Archive)?;
        match hl_protocol::ws::parse_ws_message(Bytes::from(payload)) {
            Ok(observation) => {
                if observation.observation_class() == ObservationClass::CommittedBlock {
                    return Err(WsCaptureError::CommittedLane);
                }
                classes.push(crate::ws_session::classify_inbound(&observation, None));
            }
            Err(SourceError::SchemaDrift(_)) => classes.push(InboundClass::Quarantine),
            Err(_) => classes.push(InboundClass::Quarantine),
        }
    }
    Ok(classes)
}
