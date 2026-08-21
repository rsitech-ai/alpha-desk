//! Official WS observations stay provisional. Committed node facts win.
//!
//! ponytail: this lane is not a canonical reducer. Snapshot slots are
//! replace-state in process memory. T11 session hashes are also process-local,
//! so a restart re-classifies the same bytes as `SnapshotReplace`; this lane
//! still replaces the slot instead of appending events. Persist hashes in T13/T14
//! if a committed reducer needs them across process death.

use std::collections::BTreeMap;

use hl_protocol::ws::{WsFamily, family_by_identifier};

use crate::bus::Subject;
use crate::ws_session::InboundClass;

pub const PROVISIONAL_FEATURE_SCOPE: &str = "provisional";
pub const DEFAULT_UNMATCHED_TTL_MILLIS: u64 = 60_000;

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
    unmatched: BTreeMap<String, UnmatchedProvisional>,
    snapshots: BTreeMap<String, [u8; 32]>,
    findings: Vec<ReconciliationFinding>,
    source_red: bool,
}

impl ProvisionalWsLane {
    #[must_use]
    pub fn new(ttl_millis: u64) -> Self {
        Self {
            ttl_millis,
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
        self.unmatched.len()
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
            InboundClass::DuplicateSnapshot => self.observe_snapshot(observation, true),
            InboundClass::SnapshotReplace => self.observe_snapshot(observation, false),
            InboundClass::IncrementalEvent => self.observe_incremental(observation),
        }
    }

    pub fn observe_committed(&mut self, fact: &CommittedFact, _now_millis: u64) -> LaneDecision {
        let Some(pending) = self.unmatched.remove(fact.match_key()) else {
            return LaneDecision::Ignored;
        };
        if pending.content_hash == fact.content_hash() {
            return LaneDecision::Confirmed {
                key: fact.match_key().to_owned(),
            };
        }
        let finding = conflict_finding(
            fact.match_key(),
            &pending.family,
            pending.content_hash,
            fact.content_hash(),
        );
        self.findings.push(finding.clone());
        LaneDecision::Conflict { finding }
    }

    pub fn expire(&mut self, now_millis: u64) -> Vec<LaneDecision> {
        let ttl = self.ttl_millis;
        let expired_keys: Vec<String> = self
            .unmatched
            .iter()
            .filter(|(_, pending)| now_millis.saturating_sub(pending.received_at_millis) >= ttl)
            .map(|(key, _)| key.clone())
            .collect();
        let mut decisions = Vec::with_capacity(expired_keys.len());
        for key in expired_keys {
            self.unmatched.remove(&key);
            decisions.push(LaneDecision::Expired { key });
        }
        decisions
    }

    fn observe_snapshot(
        &mut self,
        observation: &WsLaneObservation,
        classified_duplicate: bool,
    ) -> LaneDecision {
        let subject = snapshot_subject_for_family(&observation.family);
        if classified_duplicate
            && self.snapshots.get(&observation.match_key) == Some(&observation.content_hash)
        {
            return LaneDecision::DuplicateSnapshot {
                key: observation.match_key.clone(),
            };
        }
        let previous = self
            .snapshots
            .insert(observation.match_key.clone(), observation.content_hash);
        if previous == Some(observation.content_hash) {
            return LaneDecision::DuplicateSnapshot {
                key: observation.match_key.clone(),
            };
        }
        if self.source_red {
            return LaneDecision::Suppressed {
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
        self.unmatched.insert(
            observation.match_key.clone(),
            UnmatchedProvisional {
                content_hash: observation.content_hash,
                family: observation.family.clone(),
                received_at_millis: observation.received_at_millis,
            },
        );
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
