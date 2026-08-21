//! Official WS observations stay provisional. Committed node facts win.
//!
//! Snapshot slots are replace-state. Session and lane hashes persist through
//! `SnapshotHashStore` so a process restart can classify DuplicateSnapshot
//! instead of replaying replace-state. WS still cannot advance a committed
//! watermark.

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use hl_protocol::ws::{WsFamily, family_by_identifier};

use crate::bus::Subject;
use crate::ws_session::InboundClass;

pub const PROVISIONAL_FEATURE_SCOPE: &str = "provisional";
pub const DEFAULT_UNMATCHED_TTL_MILLIS: u64 = 60_000;
pub const DEFAULT_UNMATCHED_LIMIT: usize = 4_096;
pub const DEFAULT_UNMATCHED_PER_KEY: usize = 8;

const FINDING_CONTEXT: &[u8] = b"hl.finding.v1\0";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WsLaneObservation {
    match_key: String,
    family: String,
    content_hash: [u8; 32],
    class: InboundClass,
    received_at_millis: u64,
}

impl WsLaneObservation {
    pub fn try_new(
        match_key: impl Into<String>,
        family: impl Into<String>,
        content_hash: [u8; 32],
        class: InboundClass,
        received_at_millis: u64,
    ) -> Result<Self, LaneError> {
        let match_key = match_key.into();
        let family = family.into();
        if match_key.is_empty() || match_key.trim() != match_key || family.is_empty() {
            return Err(LaneError::InvalidIdentity);
        }
        Ok(Self {
            match_key,
            family,
            content_hash,
            class,
            received_at_millis,
        })
    }

    #[must_use]
    pub fn match_key(&self) -> &str {
        &self.match_key
    }

    #[must_use]
    pub fn family(&self) -> &str {
        &self.family
    }

    #[must_use]
    pub const fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }

    #[must_use]
    pub const fn class(&self) -> InboundClass {
        self.class
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedFact {
    match_key: String,
    content_hash: [u8; 32],
}

impl CommittedFact {
    pub fn try_new(
        match_key: impl Into<String>,
        content_hash: [u8; 32],
    ) -> Result<Self, LaneError> {
        let match_key = match_key.into();
        if match_key.is_empty() || match_key.trim() != match_key {
            return Err(LaneError::InvalidIdentity);
        }
        Ok(Self {
            match_key,
            content_hash,
        })
    }

    #[must_use]
    pub fn match_key(&self) -> &str {
        &self.match_key
    }

    #[must_use]
    pub const fn content_hash(&self) -> [u8; 32] {
        self.content_hash
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingStatus {
    Open,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconciliationFinding {
    finding_id: String,
    domain: String,
    subject: String,
    expected_hash: [u8; 32],
    observed_hash: [u8; 32],
    status: FindingStatus,
}

impl ReconciliationFinding {
    #[must_use]
    pub fn finding_id(&self) -> &str {
        &self.finding_id
    }

    #[must_use]
    pub fn domain(&self) -> &str {
        &self.domain
    }

    #[must_use]
    pub fn subject(&self) -> &str {
        &self.subject
    }

    #[must_use]
    pub const fn expected_hash(&self) -> [u8; 32] {
        self.expected_hash
    }

    #[must_use]
    pub const fn observed_hash(&self) -> [u8; 32] {
        self.observed_hash
    }

    #[must_use]
    pub const fn status(&self) -> FindingStatus {
        self.status
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LaneDecision {
    Ignored,
    SnapshotReplace { key: String, subject: Subject },
    DuplicateSnapshot { key: String },
    ProvisionalOpen { key: String, subject: Subject },
    Confirmed { key: String },
    Expired { key: String },
    Conflict { finding: ReconciliationFinding },
    Suppressed { key: String },
    UnmatchedRefused { key: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LaneError {
    #[error("provisional lane identity is empty or padded")]
    InvalidIdentity,
}

impl LaneError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidIdentity => "capture_ws.lane_identity",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SnapshotHashStore {
    path: PathBuf,
    hashes: BTreeMap<String, [u8; 32]>,
}

impl SnapshotHashStore {
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let hashes = match fs::read_to_string(&path) {
            Ok(body) => parse_snapshot_hash_file(&body)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => BTreeMap::new(),
            Err(error) => return Err(error),
        };
        Ok(Self { path, hashes })
    }

    #[must_use]
    pub fn get(&self, key: &str) -> Option<[u8; 32]> {
        self.hashes.get(key).copied()
    }

    #[must_use]
    pub fn hashes(&self) -> &BTreeMap<String, [u8; 32]> {
        &self.hashes
    }

    pub fn upsert(&mut self, key: impl Into<String>, hash: [u8; 32]) -> io::Result<()> {
        self.extend([(key.into(), hash)])
    }

    pub fn extend(
        &mut self,
        hashes: impl IntoIterator<Item = (String, [u8; 32])>,
    ) -> io::Result<()> {
        for (key, hash) in hashes {
            if key.is_empty() || key.contains([' ', '\n', '\t']) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "snapshot hash key is empty or contains whitespace",
                ));
            }
            self.hashes.insert(key, hash);
        }
        self.flush()
    }

    pub fn flush(&self) -> io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut body = String::new();
        for (key, hash) in &self.hashes {
            body.push_str(key);
            body.push(' ');
            body.push_str(&hex::encode(hash));
            body.push('\n');
        }
        let tmp = self.path.with_extension("tmp");
        let mut file = fs::File::create(&tmp)?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

fn parse_snapshot_hash_file(body: &str) -> io::Result<BTreeMap<String, [u8; 32]>> {
    let mut hashes = BTreeMap::new();
    for line in body.lines() {
        if line.is_empty() {
            continue;
        }
        let Some((key, hex_hash)) = line.split_once(' ') else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "snapshot hash line is missing a key/hash split",
            ));
        };
        let decoded = hex::decode(hex_hash)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.to_string()))?;
        let hash: [u8; 32] = decoded.try_into().map_err(|_| {
            io::Error::new(io::ErrorKind::InvalidData, "snapshot hash must be 32 bytes")
        })?;
        hashes.insert(key.to_owned(), hash);
    }
    Ok(hashes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UnmatchedProvisional {
    content_hash: [u8; 32],
    family: String,
    received_at_millis: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLaneHealth {
    red: bool,
    reason_code: &'static str,
}

impl SourceLaneHealth {
    #[must_use]
    pub const fn red(&self) -> bool {
        self.red
    }

    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    #[must_use]
    pub const fn suppress_provisional_features(&self) -> bool {
        self.red
    }

    #[must_use]
    pub fn suppresses(&self) -> &'static [&'static str] {
        if self.red {
            &[PROVISIONAL_FEATURE_SCOPE]
        } else {
            &[]
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProvisionalWsLane {
    ttl_millis: u64,
    unmatched_limit: usize,
    per_key_limit: usize,
    unmatched: BTreeMap<String, Vec<UnmatchedProvisional>>,
    snapshots: BTreeMap<String, [u8; 32]>,
    findings: Vec<ReconciliationFinding>,
    source_red: bool,
}

impl ProvisionalWsLane {
    #[must_use]
    pub fn new(ttl_millis: u64) -> Self {
        Self::with_limits(
            ttl_millis,
            DEFAULT_UNMATCHED_LIMIT,
            DEFAULT_UNMATCHED_PER_KEY,
        )
    }

    #[must_use]
    pub fn with_limits(ttl_millis: u64, unmatched_limit: usize, per_key_limit: usize) -> Self {
        Self {
            ttl_millis,
            unmatched_limit,
            per_key_limit,
            unmatched: BTreeMap::new(),
            snapshots: BTreeMap::new(),
            findings: Vec::new(),
            source_red: false,
        }
    }

    #[must_use]
    pub fn official() -> Self {
        Self::new(DEFAULT_UNMATCHED_TTL_MILLIS)
    }

    #[must_use]
    pub const fn advances_committed_watermark(&self) -> bool {
        false
    }

    #[must_use]
    pub fn unmatched_count(&self) -> usize {
        self.unmatched.values().map(Vec::len).sum()
    }

    #[must_use]
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    #[must_use]
    pub fn findings(&self) -> &[ReconciliationFinding] {
        &self.findings
    }

    #[must_use]
    pub fn source_health(&self) -> SourceLaneHealth {
        SourceLaneHealth {
            red: self.source_red,
            reason_code: if self.source_red {
                "capture_ws.source_red"
            } else {
                "healthy"
            },
        }
    }

    #[must_use]
    pub fn provisional_features_admitted(&self) -> bool {
        !self.source_red
    }

    pub fn set_source_red(&mut self, red: bool) {
        self.source_red = red;
    }

    pub fn observe_ws(&mut self, observation: &WsLaneObservation) -> LaneDecision {
        match observation.class {
            InboundClass::Ack | InboundClass::Heartbeat | InboundClass::Unknown => {
                LaneDecision::Ignored
            }
            InboundClass::Quarantine => LaneDecision::Ignored,
            InboundClass::DuplicateSnapshot | InboundClass::SnapshotReplace => {
                self.observe_snapshot(observation)
            }
            InboundClass::IncrementalEvent => self.observe_incremental(observation),
        }
    }

    pub fn observe_committed(&mut self, fact: &CommittedFact, _now_millis: u64) -> LaneDecision {
        let Some(pending) = self.unmatched.get_mut(fact.match_key()) else {
            return LaneDecision::Ignored;
        };
        if let Some(index) = pending
            .iter()
            .position(|row| row.content_hash == fact.content_hash())
        {
            pending.remove(index);
            if pending.is_empty() {
                self.unmatched.remove(fact.match_key());
            }
            return LaneDecision::Confirmed {
                key: fact.match_key().to_owned(),
            };
        }
        let oldest = pending
            .first()
            .expect("unmatched key has at least one row")
            .clone();
        self.unmatched.remove(fact.match_key());
        let finding = conflict_finding(
            fact.match_key(),
            &oldest.family,
            oldest.content_hash,
            fact.content_hash(),
        );
        self.findings.push(finding.clone());
        LaneDecision::Conflict { finding }
    }

    pub fn expire(&mut self, now_millis: u64) -> Vec<LaneDecision> {
        let ttl = self.ttl_millis;
        let mut decisions = Vec::new();
        self.unmatched.retain(|key, rows| {
            let before = rows.len();
            rows.retain(|pending| now_millis.saturating_sub(pending.received_at_millis) < ttl);
            if rows.len() != before {
                decisions.push(LaneDecision::Expired { key: key.clone() });
            }
            !rows.is_empty()
        });
        decisions
    }

    pub fn restore_from(&mut self, store: &SnapshotHashStore) {
        self.snapshots.extend(
            store
                .hashes()
                .iter()
                .map(|(key, hash)| (key.clone(), *hash)),
        );
    }

    pub fn persist_into(&self, store: &mut SnapshotHashStore) -> io::Result<()> {
        store.extend(
            self.snapshots
                .iter()
                .map(|(key, hash)| (key.clone(), *hash)),
        )
    }

    fn observe_snapshot(&mut self, observation: &WsLaneObservation) -> LaneDecision {
        if self.source_red {
            return LaneDecision::Suppressed {
                key: observation.match_key.clone(),
            };
        }
        let subject = snapshot_subject_for_family(&observation.family);
        let previous = self
            .snapshots
            .insert(observation.match_key.clone(), observation.content_hash);
        if previous == Some(observation.content_hash) {
            return LaneDecision::DuplicateSnapshot {
                key: observation.match_key.clone(),
            };
        }
        LaneDecision::SnapshotReplace {
            key: observation.match_key.clone(),
            subject,
        }
    }

    fn observe_incremental(&mut self, observation: &WsLaneObservation) -> LaneDecision {
        if self.source_red {
            return LaneDecision::Suppressed {
                key: observation.match_key.clone(),
            };
        }
        if self
            .unmatched
            .get(&observation.match_key)
            .is_some_and(|rows| {
                rows.iter()
                    .any(|row| row.content_hash == observation.content_hash)
            })
        {
            return LaneDecision::ProvisionalOpen {
                key: observation.match_key.clone(),
                subject: snapshot_subject_for_family(&observation.family),
            };
        }
        if self
            .unmatched
            .get(&observation.match_key)
            .is_some_and(|rows| rows.len() >= self.per_key_limit)
            || self.unmatched_count() >= self.unmatched_limit
        {
            return LaneDecision::UnmatchedRefused {
                key: observation.match_key.clone(),
            };
        }
        self.unmatched
            .entry(observation.match_key.clone())
            .or_default()
            .push(UnmatchedProvisional {
                content_hash: observation.content_hash,
                family: observation.family.clone(),
                received_at_millis: observation.received_at_millis,
            });
        LaneDecision::ProvisionalOpen {
            key: observation.match_key.clone(),
            subject: snapshot_subject_for_family(&observation.family),
        }
    }
}

#[must_use]
pub fn snapshot_subject_for_family(identifier: &str) -> Subject {
    match family_by_identifier(identifier) {
        Some(family) => snapshot_subject(family),
        None => Subject::SnapshotEcosystem,
    }
}

#[must_use]
pub fn snapshot_subject(family: &WsFamily) -> Subject {
    if family.user_scoped {
        Subject::SnapshotAccount
    } else if family.coin_scoped {
        Subject::SnapshotMarket
    } else {
        Subject::SnapshotEcosystem
    }
}

fn conflict_finding(
    match_key: &str,
    family: &str,
    expected_hash: [u8; 32],
    observed_hash: [u8; 32],
) -> ReconciliationFinding {
    let mut hasher = blake3::Hasher::new();
    hasher.update(FINDING_CONTEXT);
    hasher.update(match_key.as_bytes());
    hasher.update(&expected_hash);
    hasher.update(&observed_hash);
    ReconciliationFinding {
        finding_id: hex::encode(hasher.finalize().as_bytes()),
        domain: family.to_owned(),
        subject: snapshot_subject_for_family(family).as_str().to_owned(),
        expected_hash,
        observed_hash,
        status: FindingStatus::Open,
    }
}
