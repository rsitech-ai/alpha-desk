//! Official `/info` REST adapter. Archive exact bytes before parsed publication.

use std::collections::{BTreeMap, BTreeSet};
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use domain_types::{ChainId, SourceId};
use hl_protocol::info::{
    EncodedInfoRequest, InfoParseContext, InfoRegistry, ParsedInfoResponse, TimePageCursor,
};
use hl_protocol::{
    ErrorDisposition, ObservationClass, ObservationError, ReceiveTimestamps, SourceAdmission,
    SourceCursor, SourceObservation,
};
use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, RootCertStore, StreamOwned};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use storage_ports::{
    ArchiveError, RawObservationArchive, RawObservationBatch, RawObservationRange,
};

use crate::config::OFFICIAL_INFO_REQUEST_URL;
use crate::egress::{EgressError, InfoHttpResponse, InfoTransport, official_info_post_url};
use crate::info_scheduler::{
    SchedulerError, TimePageCrawl, TimePageCrawlRequest, crawl_time_pages,
};
use crate::request_budget::{RequestBudget, RequestCost, SchedulePriority};

const CHECKPOINT_SCHEMA: &str = "hl.info-job-checkpoint.v2";
const MAX_HTTP_HEADER_BYTES: usize = 16 * 1024;
const INFO_SOURCE_VERSION: &str = "official-info-v1";
const INFO_PARSER_SCHEMA: &str = "info-rest-v1";
const INFO_ARCHIVE_BUILD_ID: &str = "hl-capture-info";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoFaultPoint {
    AfterArchive,
    AfterPublish,
}

pub trait InfoFaultInjector: Send + Sync {
    fn check(&self, point: InfoFaultPoint) -> Result<(), InfoCaptureError>;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct NoInfoFaults;

impl InfoFaultInjector for NoInfoFaults {
    fn check(&self, _point: InfoFaultPoint) -> Result<(), InfoCaptureError> {
        Ok(())
    }
}

pub trait CaptureClock: Send + Sync {
    fn now_millis(&self) -> u64;
}

#[derive(Debug, Default)]
pub struct FakeCaptureClock {
    now: AtomicU64,
}

impl FakeCaptureClock {
    #[must_use]
    pub const fn at(now_millis: u64) -> Self {
        Self {
            now: AtomicU64::new(now_millis),
        }
    }

    pub fn set(&self, now_millis: u64) {
        self.now.store(now_millis, Ordering::SeqCst);
    }
}

impl CaptureClock for FakeCaptureClock {
    fn now_millis(&self) -> u64 {
        self.now.load(Ordering::SeqCst)
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemCaptureClock;

impl CaptureClock for SystemCaptureClock {
    fn now_millis(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0)
    }
}

#[derive(Debug, Default)]
pub struct MemoryInfoArchive {
    bodies: BTreeMap<String, Bytes>,
    published: BTreeSet<String>,
}

impl MemoryInfoArchive {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put(&mut self, body: &[u8]) -> Result<String, InfoCaptureError> {
        InfoArchive::put(self, body)
    }

    #[must_use]
    pub fn get(&self, archive_ref: &str) -> Option<&Bytes> {
        self.bodies.get(archive_ref)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.bodies.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bodies.is_empty()
    }

    pub fn mark_published(&mut self, archive_ref: &str) {
        InfoArchive::mark_published(self, archive_ref);
    }

    #[must_use]
    pub fn was_published(&self, archive_ref: &str) -> bool {
        InfoArchive::was_published(self, archive_ref)
    }
}

pub trait InfoArchive {
    fn put(&mut self, body: &[u8]) -> Result<String, InfoCaptureError>;
    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, InfoCaptureError>;
    fn mark_published(&mut self, archive_ref: &str);
    fn was_published(&self, archive_ref: &str) -> bool;
}

impl InfoArchive for MemoryInfoArchive {
    fn put(&mut self, body: &[u8]) -> Result<String, InfoCaptureError> {
        if body.is_empty() {
            return Err(InfoCaptureError::EmptyBody);
        }
        let archive_ref = archive_ref_for(body);
        self.bodies
            .entry(archive_ref.clone())
            .or_insert_with(|| Bytes::copy_from_slice(body));
        Ok(archive_ref)
    }

    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, InfoCaptureError> {
        Ok(self.bodies.get(archive_ref).cloned())
    }

    fn mark_published(&mut self, archive_ref: &str) {
        self.published.insert(archive_ref.to_owned());
    }

    fn was_published(&self, archive_ref: &str) -> bool {
        self.published.contains(archive_ref)
    }
}

pub struct RawPortInfoArchive {
    archive: Arc<dyn RawObservationArchive>,
    chain_id: ChainId,
    source_id: SourceId,
    max_payload_bytes: usize,
    published: BTreeSet<String>,
}

impl std::fmt::Debug for RawPortInfoArchive {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RawPortInfoArchive")
            .field("chain_id", &self.chain_id)
            .field("source_id", &self.source_id)
            .finish_non_exhaustive()
    }
}

impl RawPortInfoArchive {
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
            published: BTreeSet::new(),
        }
    }

    pub fn open(
        root: impl AsRef<Path>,
        chain_id: ChainId,
        source_id: SourceId,
        max_payload_bytes: usize,
    ) -> Result<Self, InfoCaptureError> {
        let archive = LocalParquetArchive::open(
            root,
            ArchiveConfig::production(INFO_ARCHIVE_BUILD_ID)
                .map_err(|_| InfoCaptureError::Archive)?,
        )
        .map_err(|_| InfoCaptureError::Archive)?;
        Ok(Self::new(
            Arc::new(archive),
            chain_id,
            source_id,
            max_payload_bytes,
        ))
    }

    fn read_body(&self, archive_ref: &str) -> Result<Option<Bytes>, InfoCaptureError> {
        let range = RawObservationRange::try_new(archive_ref, 0, 0)
            .map_err(|_| InfoCaptureError::Archive)?;
        match self
            .archive
            .read_observations(&self.chain_id, &self.source_id, range)
        {
            Ok(mut iterator) => iterator
                .next()
                .transpose()
                .map_err(|_| InfoCaptureError::Archive)
                .map(|observation| observation.map(|item| item.payload().clone())),
            Err(ArchiveError::RangeUnavailable) => Ok(None),
            Err(_) => Err(InfoCaptureError::Archive),
        }
    }
}

impl InfoArchive for RawPortInfoArchive {
    fn put(&mut self, body: &[u8]) -> Result<String, InfoCaptureError> {
        if body.is_empty() {
            return Err(InfoCaptureError::EmptyBody);
        }
        let archive_ref = archive_ref_for(body);
        if self.read_body(&archive_ref)?.is_some() {
            return Ok(archive_ref);
        }
        let payload = Bytes::copy_from_slice(body);
        let observation = SourceObservation::new(
            self.source_id.clone(),
            INFO_SOURCE_VERSION,
            ObservationClass::Snapshot,
            SourceCursor::new(archive_ref.clone(), 0).map_err(|_| InfoCaptureError::Archive)?,
            // ponytail: content-hash epoch is the lookup key. Wall time is unused
            // on replay. Pass received_at when T11 opens a live OfficialInfo lane.
            ReceiveTimestamps::new(1, 0).map_err(|_| InfoCaptureError::Archive)?,
            INFO_PARSER_SCHEMA,
            payload,
            Vec::new(),
            self.max_payload_bytes,
        )
        .map_err(|error| match error {
            ObservationError::EmptyPayload => InfoCaptureError::EmptyBody,
            _ => InfoCaptureError::Archive,
        })?;
        let spool_hash = *blake3::hash(body).as_bytes();
        let batch = RawObservationBatch::try_new(
            self.chain_id.clone(),
            vec![observation],
            spool_hash,
            spool_hash,
        )
        .map_err(|_| InfoCaptureError::Archive)?;
        self.archive
            .append_batch(&batch)
            .map_err(|_| InfoCaptureError::Archive)?;
        Ok(archive_ref)
    }

    fn get(&self, archive_ref: &str) -> Result<Option<Bytes>, InfoCaptureError> {
        self.read_body(archive_ref)
    }

    fn mark_published(&mut self, archive_ref: &str) {
        self.published.insert(archive_ref.to_owned());
    }

    fn was_published(&self, archive_ref: &str) -> bool {
        self.published.contains(archive_ref)
    }
}

#[derive(Debug, Default)]
pub struct MemoryInfoPublisher {
    publications: Vec<String>,
}

impl MemoryInfoPublisher {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn publish(&mut self, archive_ref: &str) -> Result<(), InfoCaptureError> {
        self.publications.push(archive_ref.to_owned());
        Ok(())
    }

    #[must_use]
    pub fn publications(&self) -> &[String] {
        &self.publications
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InfoJobCheckpoint {
    schema_version: String,
    job_id: String,
    capability_id: String,
    next_start_millis: i64,
    overlap_millis: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    last_archive_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    request_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pending_publish_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    quarantine_reason: Option<String>,
}

impl InfoJobCheckpoint {
    pub fn new(
        job_id: impl Into<String>,
        capability_id: impl Into<String>,
        next_start_millis: i64,
        overlap_millis: i64,
    ) -> Self {
        Self {
            schema_version: CHECKPOINT_SCHEMA.to_owned(),
            job_id: job_id.into(),
            capability_id: capability_id.into(),
            next_start_millis,
            overlap_millis,
            last_archive_ref: None,
            request_hash: None,
            pending_publish_refs: Vec::new(),
            quarantine_reason: None,
        }
    }

    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    #[must_use]
    pub fn capability_id(&self) -> &str {
        &self.capability_id
    }

    #[must_use]
    pub const fn next_start_millis(&self) -> i64 {
        self.next_start_millis
    }

    #[must_use]
    pub fn last_archive_ref(&self) -> Option<&str> {
        self.last_archive_ref.as_deref()
    }

    #[must_use]
    pub fn request_hash(&self) -> Option<&str> {
        self.request_hash.as_deref()
    }

    #[must_use]
    pub fn pending_publish_ref(&self) -> Option<&str> {
        self.pending_publish_refs.last().map(String::as_str)
    }

    #[must_use]
    pub fn pending_publish_refs(&self) -> &[String] {
        &self.pending_publish_refs
    }

    fn queue_pending(&mut self, archive_ref: String) {
        self.last_archive_ref = Some(archive_ref.clone());
        if !self
            .pending_publish_refs
            .iter()
            .any(|queued| queued == &archive_ref)
        {
            self.pending_publish_refs.push(archive_ref);
        }
    }

    fn drop_pending(&mut self, archive_ref: &str) {
        self.pending_publish_refs
            .retain(|queued| queued != archive_ref);
    }

    #[must_use]
    pub fn quarantine_reason(&self) -> Option<&str> {
        self.quarantine_reason.as_deref()
    }

    pub fn persist_to(&self, directory: &Path) -> Result<(), InfoCaptureError> {
        std::fs::create_dir_all(directory).map_err(|_| InfoCaptureError::Checkpoint)?;
        let path = checkpoint_path(directory, &self.job_id)?;
        let temporary = directory.join(format!(
            "{}.tmp",
            path.file_name()
                .and_then(|name| name.to_str())
                .ok_or(InfoCaptureError::Checkpoint)?
        ));
        let encoded = serde_json::to_vec_pretty(self).map_err(|_| InfoCaptureError::Checkpoint)?;
        std::fs::write(&temporary, encoded).map_err(|_| InfoCaptureError::Checkpoint)?;
        std::fs::rename(&temporary, &path).map_err(|_| InfoCaptureError::Checkpoint)
    }

    pub fn load_from(directory: &Path, job_id: &str) -> Result<Option<Self>, InfoCaptureError> {
        let path = checkpoint_path(directory, job_id)?;
        match std::fs::read(&path) {
            Ok(bytes) => {
                let checkpoint: Self =
                    serde_json::from_slice(&bytes).map_err(|_| InfoCaptureError::Checkpoint)?;
                if checkpoint.schema_version != CHECKPOINT_SCHEMA || checkpoint.job_id != job_id {
                    return Err(InfoCaptureError::Checkpoint);
                }
                Ok(Some(checkpoint))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(InfoCaptureError::Checkpoint),
        }
    }

    pub fn resume_cursor(&self) -> Result<TimePageCursor, InfoCaptureError> {
        TimePageCursor::new(self.next_start_millis, self.overlap_millis)
            .map_err(|_| InfoCaptureError::Checkpoint)
    }
}

fn checkpoint_path(directory: &Path, job_id: &str) -> Result<PathBuf, InfoCaptureError> {
    if job_id.is_empty() || job_id.contains('/') || job_id.contains('\\') || job_id.contains("..") {
        return Err(InfoCaptureError::Checkpoint);
    }
    Ok(directory.join(format!("{job_id}.json")))
}

fn archive_ref_for(body: &[u8]) -> String {
    format!("info-{}", hex::encode(blake3::hash(body).as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoCaptureOutcome {
    Published,
    Duplicate,
    Quarantined,
}

pub struct InfoCaptureCoordinator<'a> {
    archive: &'a mut dyn InfoArchive,
    publisher: &'a mut MemoryInfoPublisher,
    faults: &'a dyn InfoFaultInjector,
    max_body: usize,
}

impl<'a> InfoCaptureCoordinator<'a> {
    pub fn new(
        archive: &'a mut dyn InfoArchive,
        publisher: &'a mut MemoryInfoPublisher,
        faults: &'a dyn InfoFaultInjector,
        max_body: usize,
    ) -> Self {
        Self {
            archive,
            publisher,
            faults,
            max_body,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn capture_response<T: InfoTransport>(
        &mut self,
        transport: &mut T,
        checkpoint: &mut InfoJobCheckpoint,
        registry: InfoRegistry,
        params: &BTreeMap<String, Value>,
        received_at: domain_types::KnownTime,
        request_url: &str,
        persist_dir: Option<&Path>,
    ) -> Result<InfoCaptureOutcome, InfoCaptureError> {
        let posted = post_info_bytes(
            transport,
            registry,
            &checkpoint.capability_id,
            params,
            request_url,
            self.max_body,
        )?;
        self.finish_posted(posted, checkpoint, registry, received_at, persist_dir)
    }

    pub fn replay_pending(
        &mut self,
        checkpoint: &mut InfoJobCheckpoint,
        registry: InfoRegistry,
        received_at: domain_types::KnownTime,
        persist_dir: Option<&Path>,
    ) -> Result<InfoCaptureOutcome, InfoCaptureError> {
        let pending = checkpoint.pending_publish_refs.clone();
        if pending.is_empty() {
            return Ok(InfoCaptureOutcome::Duplicate);
        }
        let mut published_any = false;
        for archive_ref in pending {
            if !checkpoint
                .pending_publish_refs
                .iter()
                .any(|queued| queued == &archive_ref)
            {
                continue;
            }
            let body = self
                .archive
                .get(&archive_ref)?
                .ok_or(InfoCaptureError::MissingArchive)?;
            let request_hash = checkpoint
                .request_hash
                .as_deref()
                .and_then(decode_hash)
                .unwrap_or_else(|| blake3::hash(&body));
            match self.publish_or_quarantine(
                &archive_ref,
                &body,
                request_hash,
                checkpoint,
                registry,
                received_at,
                persist_dir,
            )? {
                InfoCaptureOutcome::Published => published_any = true,
                InfoCaptureOutcome::Duplicate | InfoCaptureOutcome::Quarantined => {}
            }
        }
        if published_any {
            Ok(InfoCaptureOutcome::Published)
        } else {
            Ok(InfoCaptureOutcome::Duplicate)
        }
    }

    fn finish_posted(
        &mut self,
        posted: PostedInfo,
        checkpoint: &mut InfoJobCheckpoint,
        registry: InfoRegistry,
        received_at: domain_types::KnownTime,
        persist_dir: Option<&Path>,
    ) -> Result<InfoCaptureOutcome, InfoCaptureError> {
        let request_hash = posted.request_hash;
        let archive_ref = self.archive.put(&posted.body)?;
        checkpoint.request_hash = Some(hex::encode(request_hash.as_bytes()));
        checkpoint.queue_pending(archive_ref.clone());
        persist_checkpoint(checkpoint, persist_dir)?;
        self.faults.check(InfoFaultPoint::AfterArchive)?;
        self.publish_or_quarantine(
            &archive_ref,
            &posted.body,
            request_hash,
            checkpoint,
            registry,
            received_at,
            persist_dir,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn publish_or_quarantine(
        &mut self,
        archive_ref: &str,
        body: &[u8],
        request_hash: blake3::Hash,
        checkpoint: &mut InfoJobCheckpoint,
        registry: InfoRegistry,
        received_at: domain_types::KnownTime,
        persist_dir: Option<&Path>,
    ) -> Result<InfoCaptureOutcome, InfoCaptureError> {
        if self.archive.was_published(archive_ref) {
            checkpoint.drop_pending(archive_ref);
            persist_checkpoint(checkpoint, persist_dir)?;
            return Ok(InfoCaptureOutcome::Duplicate);
        }
        let endpoint = registry
            .get(&checkpoint.capability_id)
            .map_err(InfoCaptureError::Info)?;
        let parsed = match parse_archived(endpoint, body, request_hash, received_at, archive_ref) {
            Ok(parsed) => parsed,
            Err(error) => {
                let info = match error {
                    InfoCaptureError::Info(info) => info,
                    other => return Err(other),
                };
                if info.disposition() != ErrorDisposition::Quarantine {
                    return Err(InfoCaptureError::Info(info));
                }
                checkpoint.quarantine_reason = Some(info.reason_code().to_owned());
                checkpoint.drop_pending(archive_ref);
                persist_checkpoint(checkpoint, persist_dir)?;
                return Ok(InfoCaptureOutcome::Quarantined);
            }
        };
        if parsed.request_hash() != request_hash {
            return Err(InfoCaptureError::Checkpoint);
        }
        let _ = parsed.response_hash();
        self.publisher.publish(archive_ref)?;
        self.archive.mark_published(archive_ref);
        checkpoint.drop_pending(archive_ref);
        persist_checkpoint(checkpoint, persist_dir)?;
        self.faults.check(InfoFaultPoint::AfterPublish)?;
        Ok(InfoCaptureOutcome::Published)
    }
}

fn persist_checkpoint(
    checkpoint: &InfoJobCheckpoint,
    persist_dir: Option<&Path>,
) -> Result<(), InfoCaptureError> {
    if let Some(directory) = persist_dir {
        checkpoint.persist_to(directory)?;
    }
    Ok(())
}

struct PostedInfo {
    request_hash: blake3::Hash,
    body: Bytes,
}

fn post_info_bytes<T: InfoTransport>(
    transport: &mut T,
    registry: InfoRegistry,
    capability_id: &str,
    params: &BTreeMap<String, Value>,
    request_url: &str,
    max_body: usize,
) -> Result<PostedInfo, InfoCaptureError> {
    let endpoint = registry
        .get(capability_id)
        .map_err(InfoCaptureError::Info)?;
    let admission = endpoint
        .admission()
        .map_err(|_| InfoCaptureError::CommittedLane)?;
    reject_committed(admission)?;
    let encoded = endpoint.encode(params).map_err(InfoCaptureError::Info)?;
    let url = official_info_post_url(request_url).map_err(InfoCaptureError::Egress)?;
    if crate::forbids_exchange_request(encoded.identifier(), encoded.body(), &url) {
        return Err(InfoCaptureError::Egress(EgressError::ExchangeForbidden));
    }
    let response = transport
        .post_info(&url, &encoded)
        .map_err(InfoCaptureError::Egress)?;
    match response.status() {
        200 => {
            if response.body().len() > max_body {
                return Err(InfoCaptureError::Egress(EgressError::BodyTooLarge));
            }
            Ok(PostedInfo {
                request_hash: encoded.content_hash(),
                body: response.body().clone(),
            })
        }
        429 => Err(InfoCaptureError::Egress(EgressError::RateLimited)),
        status => Err(InfoCaptureError::Egress(EgressError::HttpStatus(status))),
    }
}

fn reject_committed(admission: SourceAdmission) -> Result<(), InfoCaptureError> {
    if admission.trust() != hl_protocol::SourceTrust::ReconciledSnapshot
        || admission.observation_class() != ObservationClass::Snapshot
        || admission.can_advance_committed_watermark()
    {
        return Err(InfoCaptureError::CommittedLane);
    }
    Ok(())
}

fn parse_archived(
    endpoint: &hl_protocol::info::InfoEndpoint,
    body: &[u8],
    request_hash: blake3::Hash,
    received_at: domain_types::KnownTime,
    archive_ref: &str,
) -> Result<ParsedInfoResponse<Value>, InfoCaptureError> {
    let context = InfoParseContext::new(
        request_hash,
        received_at,
        hl_protocol::info::ArchiveRef::new(archive_ref).map_err(InfoCaptureError::Info)?,
    );
    endpoint
        .parse(body, &context)
        .map_err(InfoCaptureError::Info)
}

fn decode_hash(value: &str) -> Option<blake3::Hash> {
    let bytes = hex::decode(value).ok()?;
    let bytes: [u8; 32] = bytes.try_into().ok()?;
    Some(blake3::Hash::from(bytes))
}

#[allow(clippy::too_many_arguments)]
pub fn capture_time_pages<T: InfoTransport>(
    transport: &mut T,
    budget: &mut RequestBudget,
    archive: &mut dyn InfoArchive,
    publisher: &mut MemoryInfoPublisher,
    checkpoint: &mut InfoJobCheckpoint,
    registry: InfoRegistry,
    extra_params: &BTreeMap<String, Value>,
    cursor: TimePageCursor,
    page_limit: usize,
    priority: SchedulePriority,
    now_millis: u64,
    received_at: domain_types::KnownTime,
    cost: RequestCost,
    request_url: &str,
    persist_dir: Option<&Path>,
    max_body: usize,
    faults: &dyn InfoFaultInjector,
) -> Result<TimePageCrawl, InfoCaptureError> {
    let capability_id = checkpoint.capability_id().to_owned();
    let job_id = checkpoint.job_id().to_owned();
    let pending_ref = checkpoint
        .last_archive_ref
        .clone()
        .unwrap_or_else(|| "info-pending".to_owned());
    let crawl = {
        let mut wrapping = ArchivingInfoTransport {
            inner: transport,
            archive: &mut *archive,
            checkpoint: &mut *checkpoint,
            persist_dir,
            max_body,
            last_error: None,
        };
        let crawl = crawl_time_pages(
            &mut wrapping,
            budget,
            registry,
            TimePageCrawlRequest::new(
                &capability_id,
                extra_params,
                cursor,
                page_limit,
                &job_id,
                priority,
                now_millis,
                received_at,
                &pending_ref,
                cost,
            )
            .with_request_url(request_url),
        );
        if let Some(error) = wrapping.last_error.take() {
            return Err(error);
        }
        crawl.map_err(InfoCaptureError::Scheduler)?
    };
    checkpoint.next_start_millis = crawl.cursor().next_query_start_millis();
    persist_checkpoint(checkpoint, persist_dir)?;
    faults.check(InfoFaultPoint::AfterArchive)?;
    InfoCaptureCoordinator::new(archive, publisher, faults, max_body).replay_pending(
        checkpoint,
        registry,
        received_at,
        persist_dir,
    )?;
    Ok(crawl)
}

struct ArchivingInfoTransport<'a, T> {
    inner: &'a mut T,
    archive: &'a mut dyn InfoArchive,
    checkpoint: &'a mut InfoJobCheckpoint,
    persist_dir: Option<&'a Path>,
    max_body: usize,
    last_error: Option<InfoCaptureError>,
}

impl<T: InfoTransport> InfoTransport for ArchivingInfoTransport<'_, T> {
    fn post_info(
        &mut self,
        url: &str,
        request: &EncodedInfoRequest,
    ) -> Result<InfoHttpResponse, EgressError> {
        let response = self.inner.post_info(url, request)?;
        if response.status() != 200 {
            return Ok(response);
        }
        if response.body().len() > self.max_body {
            return Err(EgressError::BodyTooLarge);
        }
        match self.archive.put(response.body()) {
            Ok(archive_ref) => {
                self.checkpoint.queue_pending(archive_ref);
                if let Err(error) = persist_checkpoint(self.checkpoint, self.persist_dir) {
                    self.last_error = Some(error);
                }
            }
            Err(error) => self.last_error = Some(error),
        }
        Ok(response)
    }
}

pub struct HttpsInfoTransport {
    client_config: Arc<ClientConfig>,
    timeout: Duration,
    max_body: usize,
}

impl HttpsInfoTransport {
    pub fn try_new(timeout: Duration, max_body: usize) -> Result<Self, EgressError> {
        if timeout.is_zero() {
            return Err(EgressError::Timeout);
        }
        if max_body == 0 {
            return Err(EgressError::BodyTooLarge);
        }
        let native = rustls_native_certs::load_native_certs();
        let mut roots = RootCertStore::empty();
        for cert in native.certs {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            return Err(EgressError::TlsRequired);
        }
        let client_config =
            ClientConfig::builder_with_provider(rustls::crypto::ring::default_provider().into())
                .with_safe_default_protocol_versions()
                .map_err(|_| EgressError::TlsRequired)?
                .with_root_certificates(roots)
                .with_no_client_auth();
        Ok(Self {
            client_config: Arc::new(client_config),
            timeout,
            max_body,
        })
    }
}

impl InfoTransport for HttpsInfoTransport {
    fn post_info(
        &mut self,
        url: &str,
        request: &EncodedInfoRequest,
    ) -> Result<InfoHttpResponse, EgressError> {
        let url = official_info_post_url(url)?;
        https_post_json(
            &self.client_config,
            &url,
            request.body(),
            self.timeout,
            self.max_body,
        )
    }
}

fn https_post_json(
    client_config: &Arc<ClientConfig>,
    url: &str,
    body: &[u8],
    timeout: Duration,
    max_body: usize,
) -> Result<InfoHttpResponse, EgressError> {
    let (host, path) = split_https_host_path(url)?;
    let server_name =
        ServerName::try_from(host.to_owned()).map_err(|_| EgressError::TlsRequired)?;
    let stream = TcpStream::connect((host, 443)).map_err(|_| EgressError::Timeout)?;
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|_| EgressError::Timeout)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|_| EgressError::Timeout)?;
    let conn = ClientConnection::new(Arc::clone(client_config), server_name)
        .map_err(|_| EgressError::TlsRequired)?;
    let mut tls = StreamOwned::new(conn, stream);
    let header = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAccept-Encoding: identity\r\n\r\n",
        body.len()
    );
    tls.write_all(header.as_bytes())
        .map_err(|_| EgressError::Timeout)?;
    tls.write_all(body).map_err(|_| EgressError::Timeout)?;
    tls.flush().map_err(|_| EgressError::Timeout)?;
    let mut raw = Vec::new();
    let mut buf = [0_u8; 8_192];
    loop {
        if raw.len() > MAX_HTTP_HEADER_BYTES.saturating_add(max_body) {
            return Err(EgressError::BodyTooLarge);
        }
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(read) => raw.extend_from_slice(&buf[..read]),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(EgressError::Timeout);
            }
            Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {
                return Err(EgressError::Timeout);
            }
            Err(_) => return Err(EgressError::Timeout),
        }
    }
    parse_http_response(&raw, max_body)
}

fn split_https_host_path(url: &str) -> Result<(&str, &str), EgressError> {
    let rest = url
        .strip_prefix("https://")
        .ok_or(EgressError::TlsRequired)?;
    match rest.split_once('/') {
        Some((host, _)) => Ok((host, &rest[host.len()..])),
        None => Ok((rest, "/info")),
    }
}

fn parse_http_response(raw: &[u8], max_body: usize) -> Result<InfoHttpResponse, EgressError> {
    let header_end = raw
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(EgressError::Timeout)?;
    if header_end > MAX_HTTP_HEADER_BYTES {
        return Err(EgressError::BodyTooLarge);
    }
    let header = std::str::from_utf8(&raw[..header_end]).map_err(|_| EgressError::TlsRequired)?;
    let mut lines = header.split("\r\n");
    let status_line = lines.next().ok_or(EgressError::TlsRequired)?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or(EgressError::TlsRequired)?;
    let mut content_length = None;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-encoding") {
            let encoding = value.trim();
            if !encoding.is_empty() && !encoding.eq_ignore_ascii_case("identity") {
                return Err(EgressError::CompressedBody);
            }
        }
        if name.eq_ignore_ascii_case("transfer-encoding")
            && value.trim().to_ascii_lowercase().contains("chunked")
        {
            return Err(EgressError::CompressedBody);
        }
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(
                value
                    .trim()
                    .parse::<usize>()
                    .map_err(|_| EgressError::BodyTooLarge)?,
            );
        }
    }
    let body = &raw[header_end + 4..];
    if body.len() > max_body {
        return Err(EgressError::BodyTooLarge);
    }
    if let Some(length) = content_length
        && length > max_body
    {
        return Err(EgressError::BodyTooLarge);
    }
    Ok(InfoHttpResponse::new(status, Bytes::copy_from_slice(body)))
}

#[must_use]
pub fn default_info_request_url() -> &'static str {
    OFFICIAL_INFO_REQUEST_URL
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum InfoCaptureError {
    #[error("info protocol error")]
    Info(hl_protocol::info::InfoError),
    #[error("info egress error")]
    Egress(EgressError),
    #[error("info scheduler error")]
    Scheduler(SchedulerError),
    #[error("official info cannot publish on the committed lane")]
    CommittedLane,
    #[error("info archive is missing a pending body")]
    MissingArchive,
    #[error("info raw observation archive failed")]
    Archive,
    #[error("info job checkpoint is invalid")]
    Checkpoint,
    #[error("info response body is empty")]
    EmptyBody,
    #[error("info capture fault injected")]
    InjectedFault(InfoFaultPoint),
}

impl InfoCaptureError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::Info(error) => error.reason_code(),
            Self::Egress(error) => error.reason_code(),
            Self::Scheduler(error) => error.reason_code(),
            Self::CommittedLane => "capture_info.committed_lane",
            Self::MissingArchive => "capture_info.missing_archive",
            Self::Archive => "capture_info.archive",
            Self::Checkpoint => "capture_info.checkpoint",
            Self::EmptyBody => "capture_info.empty_body",
            Self::InjectedFault(_) => "capture_info.injected_fault",
        }
    }
}

impl From<hl_protocol::info::InfoError> for InfoCaptureError {
    fn from(error: hl_protocol::info::InfoError) -> Self {
        Self::Info(error)
    }
}

#[cfg(test)]
mod http_parse_tests {
    use super::{parse_http_response, split_https_host_path};
    use crate::egress::EgressError;

    #[test]
    fn compressed_body_is_rejected() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\nContent-Length: 1\r\n\r\nx";
        let error = parse_http_response(raw, 1024).expect_err("gzip");
        assert!(matches!(error, EgressError::CompressedBody));
    }

    #[test]
    fn chunked_body_is_rejected() {
        let raw = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nx\r\n0\r\n\r\n";
        let error = parse_http_response(raw, 1024).expect_err("chunked");
        assert!(matches!(error, EgressError::CompressedBody));
    }

    #[test]
    fn oversized_body_is_rejected() {
        let raw = b"HTTP/1.1 200 OK\r\nContent-Length: 32\r\n\r\nxxxxxxxx";
        let error = parse_http_response(raw, 4).expect_err("limit");
        assert!(matches!(error, EgressError::BodyTooLarge));
    }

    #[test]
    fn host_path_splits_official_info() {
        let (host, path) = split_https_host_path("https://api.hyperliquid.xyz/info").expect("url");
        assert_eq!(host, "api.hyperliquid.xyz");
        assert_eq!(path, "/info");
    }
}
