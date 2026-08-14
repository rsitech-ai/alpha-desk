use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use domain_types::{BlockHeight, ChainId, KnownTime};
use serde::{Deserialize, Serialize};

const STATUS_SCHEMA_V4: &str = "hl.capture.status.v4";
const STATUS_SCHEMA_V5: &str = "hl.capture.status.v5";
const MAX_STATUS_BYTES: usize = 16 * 1024;
const MAX_STATUS_TEXT_BYTES: usize = 512;
const MAX_AUXILIARY_SOURCES: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureHealth {
    Green,
    Yellow,
    Red,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CommittedSourceClass {
    LocallyVerifiedCommitted,
    IndependentCommitted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureSourceHealth {
    Starting,
    Healthy,
    RangeUnavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuxiliarySourceHealth {
    Starting,
    Healthy,
    Quarantined,
    Latched,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AuxiliaryQualificationState {
    Unqualified,
    Qualified,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartReconstruction {
    #[default]
    NotRequired,
    Incomplete,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuxiliarySourceStatus {
    source_id: String,
    health: AuxiliarySourceHealth,
    qualification: AuxiliaryQualificationState,
    #[serde(skip_serializing_if = "Option::is_none")]
    cursor_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tail_cursor_epoch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    durable_offset: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_sequence: Option<u64>,
    spool_records: u64,
    unarchived_records: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    unread_bytes: Option<u64>,
    partial_line: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_durable_wall_micros: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    quarantine_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error_reason: Option<String>,
    #[serde(default)]
    restart_reconstruction: RestartReconstruction,
}

impl AuxiliarySourceStatus {
    pub(crate) fn starting(source_id: impl Into<String>) -> Self {
        Self {
            source_id: source_id.into(),
            health: AuxiliarySourceHealth::Starting,
            qualification: AuxiliaryQualificationState::Unqualified,
            cursor_epoch: None,
            tail_cursor_epoch: None,
            durable_offset: None,
            local_sequence: None,
            spool_records: 0,
            unarchived_records: 0,
            unread_bytes: None,
            partial_line: false,
            last_durable_wall_micros: None,
            quarantine_reason: None,
            last_error_reason: None,
            restart_reconstruction: RestartReconstruction::NotRequired,
        }
    }

    pub(crate) fn record_recovered(
        &mut self,
        cursor_epoch: impl Into<String>,
        durable_offset: u64,
        local_sequence: u64,
        last_durable_wall_micros: i64,
        quarantine_reason: Option<&str>,
    ) {
        let quarantined = quarantine_reason.is_some();
        self.health = if quarantined {
            AuxiliarySourceHealth::Quarantined
        } else {
            AuxiliarySourceHealth::Healthy
        };
        self.cursor_epoch = Some(cursor_epoch.into());
        self.tail_cursor_epoch = self.cursor_epoch.clone();
        self.durable_offset = Some(durable_offset);
        self.local_sequence = Some(local_sequence);
        self.spool_records = local_sequence;
        self.unarchived_records = 0;
        self.unread_bytes = None;
        self.partial_line = false;
        self.last_durable_wall_micros = Some(last_durable_wall_micros);
        self.quarantine_reason = quarantine_reason.map(ToOwned::to_owned);
        self.last_error_reason = None;
        self.restart_reconstruction = RestartReconstruction::Incomplete;
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn record_durable(
        &mut self,
        cursor_epoch: impl Into<String>,
        tail_cursor_epoch: impl Into<String>,
        durable_offset: u64,
        local_sequence: u64,
        unread_bytes: u64,
        partial_line: bool,
        last_durable_wall_micros: i64,
        quarantine_reason: Option<&str>,
    ) {
        if let Some(reason) = quarantine_reason {
            self.quarantine_reason = Some(reason.to_owned());
        }
        let quarantined = self.quarantine_reason.is_some();
        self.health = if quarantined {
            AuxiliarySourceHealth::Quarantined
        } else {
            AuxiliarySourceHealth::Healthy
        };
        self.cursor_epoch = Some(cursor_epoch.into());
        self.tail_cursor_epoch = Some(tail_cursor_epoch.into());
        self.durable_offset = Some(durable_offset);
        self.local_sequence = Some(local_sequence);
        self.spool_records = local_sequence;
        self.unarchived_records = 0;
        self.unread_bytes = Some(unread_bytes);
        self.partial_line = partial_line;
        self.last_durable_wall_micros = Some(last_durable_wall_micros);
        self.last_error_reason = None;
        self.bind_live_tail();
    }

    fn bind_live_tail(&mut self) {
        if self.restart_reconstruction == RestartReconstruction::Incomplete {
            self.restart_reconstruction = RestartReconstruction::Complete;
        }
    }

    pub(crate) fn record_tail(
        &mut self,
        tail_cursor_epoch: impl Into<String>,
        unread_bytes: u64,
        partial_line: bool,
    ) {
        self.tail_cursor_epoch = Some(tail_cursor_epoch.into());
        self.unread_bytes = Some(unread_bytes);
        self.partial_line = partial_line;
        self.bind_live_tail();
    }

    pub(crate) fn record_buffered(
        &mut self,
        tail_cursor_epoch: impl Into<String>,
        spool_records: u64,
        unarchived_records: u64,
        unread_bytes: u64,
        partial_line: bool,
    ) {
        self.tail_cursor_epoch = Some(tail_cursor_epoch.into());
        self.spool_records = spool_records;
        self.unarchived_records = unarchived_records;
        self.unread_bytes = Some(unread_bytes);
        self.partial_line = partial_line;
        self.bind_live_tail();
    }

    pub(crate) fn latch(&mut self, reason_code: &str) {
        self.health = AuxiliarySourceHealth::Latched;
        self.last_error_reason = Some(reason_code.to_owned());
    }

    pub(crate) fn retrying(&mut self, reason_code: &str) {
        if self.quarantine_reason.is_none() {
            self.health = AuxiliarySourceHealth::Starting;
        } else {
            self.health = AuxiliarySourceHealth::Quarantined;
        }
        self.last_error_reason = Some(reason_code.to_owned());
    }

    pub(crate) fn retry_recovered(&mut self) {
        if self.health == AuxiliarySourceHealth::Latched {
            return;
        }
        self.health = if self.quarantine_reason.is_some() {
            AuxiliarySourceHealth::Quarantined
        } else if self.local_sequence.is_some() {
            AuxiliarySourceHealth::Healthy
        } else {
            AuxiliarySourceHealth::Starting
        };
        self.last_error_reason = None;
    }

    #[must_use]
    pub fn source_id(&self) -> &str {
        &self.source_id
    }

    #[must_use]
    pub const fn health(&self) -> AuxiliarySourceHealth {
        self.health
    }

    #[must_use]
    pub const fn qualification(&self) -> AuxiliaryQualificationState {
        self.qualification
    }

    #[must_use]
    pub fn cursor_epoch(&self) -> Option<&str> {
        self.cursor_epoch.as_deref()
    }

    #[must_use]
    pub fn tail_cursor_epoch(&self) -> Option<&str> {
        self.tail_cursor_epoch.as_deref()
    }

    #[must_use]
    pub const fn durable_offset(&self) -> Option<u64> {
        self.durable_offset
    }

    #[must_use]
    pub const fn local_sequence(&self) -> Option<u64> {
        self.local_sequence
    }

    #[must_use]
    pub const fn spool_records(&self) -> u64 {
        self.spool_records
    }

    #[must_use]
    pub const fn unarchived_records(&self) -> u64 {
        self.unarchived_records
    }

    #[must_use]
    pub const fn unread_bytes(&self) -> Option<u64> {
        self.unread_bytes
    }

    #[must_use]
    pub const fn partial_line(&self) -> bool {
        self.partial_line
    }

    #[must_use]
    pub const fn last_durable_wall_micros(&self) -> Option<i64> {
        self.last_durable_wall_micros
    }

    #[must_use]
    pub fn quarantine_reason(&self) -> Option<&str> {
        self.quarantine_reason.as_deref()
    }

    #[must_use]
    pub fn last_error_reason(&self) -> Option<&str> {
        self.last_error_reason.as_deref()
    }

    #[must_use]
    pub const fn restart_reconstruction(&self) -> RestartReconstruction {
        self.restart_reconstruction
    }

    fn validate(&self) -> Result<(), StatusError> {
        validate_status_text(&self.source_id)?;
        if let Some(epoch) = &self.cursor_epoch {
            validate_status_text(epoch)?;
        }
        if let Some(epoch) = &self.tail_cursor_epoch {
            validate_status_text(epoch)?;
        }
        if let Some(reason) = &self.last_error_reason {
            validate_reason_code(reason)?;
        }
        if let Some(reason) = &self.quarantine_reason {
            validate_reason_code(reason)?;
        }
        let durable_fields = [
            self.cursor_epoch.is_some(),
            self.durable_offset.is_some(),
            self.local_sequence.is_some(),
            self.last_durable_wall_micros.is_some(),
        ];
        let durable_sequence = self.local_sequence.unwrap_or(0);
        if durable_fields
            .iter()
            .any(|present| *present != durable_fields[0])
            || self
                .local_sequence
                .is_some_and(|sequence| sequence == 0 || sequence > self.spool_records)
            || self.spool_records.checked_sub(durable_sequence) != Some(self.unarchived_records)
            || self.last_durable_wall_micros.is_some_and(|value| value < 0)
            || matches!(
                self.health,
                AuxiliarySourceHealth::Healthy | AuxiliarySourceHealth::Quarantined
            ) && !durable_fields[0]
            || self.health == AuxiliarySourceHealth::Quarantined && self.quarantine_reason.is_none()
            || self.health == AuxiliarySourceHealth::Latched && self.last_error_reason.is_none()
            || matches!(
                self.restart_reconstruction,
                RestartReconstruction::Incomplete | RestartReconstruction::Complete
            ) && !durable_fields[0]
        {
            return Err(StatusError::InvalidField);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureMaintenanceStatus {
    enabled: bool,
    kill_switch: bool,
    health: CaptureHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason_code: Option<String>,
    pending_pack_manifest_count: u64,
    packed_range_count: u64,
    logical_manifest_count: u64,
    physical_data_object_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_scrub_at_micros: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_pack_index_at_micros: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_pack_data_at_micros: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_retention_at_micros: Option<i64>,
    retention_authorized: bool,
}

impl CaptureMaintenanceStatus {
    #[must_use]
    pub fn idle(enabled: bool, kill_switch: bool) -> Self {
        Self {
            enabled,
            kill_switch,
            health: CaptureHealth::Green,
            reason_code: None,
            pending_pack_manifest_count: 0,
            packed_range_count: 0,
            logical_manifest_count: 0,
            physical_data_object_count: 0,
            last_scrub_at_micros: None,
            last_pack_index_at_micros: None,
            last_pack_data_at_micros: None,
            last_retention_at_micros: None,
            retention_authorized: false,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), StatusError> {
        if let Some(reason) = &self.reason_code {
            validate_reason_code(reason)?;
        }
        if (self.health == CaptureHealth::Green) != self.reason_code.is_none() {
            return Err(StatusError::InvalidField);
        }
        Ok(())
    }

    #[must_use]
    pub const fn enabled(&self) -> bool {
        self.enabled
    }

    #[must_use]
    pub const fn kill_switch(&self) -> bool {
        self.kill_switch
    }

    #[must_use]
    pub const fn health(&self) -> CaptureHealth {
        self.health
    }

    #[must_use]
    pub fn reason_code(&self) -> Option<&str> {
        self.reason_code.as_deref()
    }

    #[must_use]
    pub const fn retention_authorized(&self) -> bool {
        self.retention_authorized
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureStatus {
    schema_version: String,
    snapshot_at_micros: i64,
    build_id: String,
    chain_id: String,
    health: CaptureHealth,
    ready: bool,
    active_committed_source: CommittedSourceClass,
    primary_source_health: CaptureSourceHealth,
    #[serde(skip_serializing_if = "Option::is_none")]
    independent_source_health: Option<CaptureSourceHealth>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failover_height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failover_reason: Option<crate::FailoverReason>,
    #[serde(skip_serializing_if = "Option::is_none")]
    durable_height: Option<u64>,
    pending_blocks: u64,
    #[serde(default)]
    capture_backlog_records: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    oldest_pending_capture_height: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    disk_free_basis_points: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    archive_manifest_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error_reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    throughput_records_per_sec: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    throughput_blocks_per_sec: Option<u32>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    auxiliary_sources: Vec<AuxiliarySourceStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    maintenance: Option<CaptureMaintenanceStatus>,
}

impl CaptureStatus {
    #[must_use]
    pub fn new(
        snapshot_at: KnownTime,
        build_id: impl Into<String>,
        chain_id: ChainId,
        health: CaptureHealth,
    ) -> Self {
        Self {
            schema_version: STATUS_SCHEMA_V5.to_owned(),
            snapshot_at_micros: snapshot_at.unix_micros(),
            build_id: build_id.into(),
            chain_id: chain_id.to_string(),
            health,
            ready: false,
            active_committed_source: CommittedSourceClass::LocallyVerifiedCommitted,
            primary_source_health: CaptureSourceHealth::Starting,
            independent_source_health: None,
            failover_height: None,
            failover_reason: None,
            durable_height: None,
            pending_blocks: 0,
            capture_backlog_records: 0,
            oldest_pending_capture_height: None,
            disk_free_basis_points: None,
            archive_manifest_id: None,
            last_error_reason: None,
            throughput_records_per_sec: None,
            throughput_blocks_per_sec: None,
            auxiliary_sources: Vec::new(),
            maintenance: Some(CaptureMaintenanceStatus::idle(false, false)),
        }
    }

    #[must_use]
    pub const fn with_readiness(mut self, ready: bool) -> Self {
        self.ready = ready;
        self
    }

    #[must_use]
    pub fn with_durable_height(mut self, height: Option<BlockHeight>) -> Self {
        self.durable_height = height.map(BlockHeight::get);
        self
    }

    #[must_use]
    pub fn with_source_state(
        mut self,
        active_source: CommittedSourceClass,
        primary_health: CaptureSourceHealth,
        independent_health: Option<CaptureSourceHealth>,
        failover_height: Option<BlockHeight>,
        failover_reason: Option<crate::FailoverReason>,
    ) -> Self {
        self.active_committed_source = active_source;
        self.primary_source_health = primary_health;
        self.independent_source_health = independent_health;
        self.failover_height = failover_height.map(BlockHeight::get);
        self.failover_reason = failover_reason;
        self
    }

    #[must_use]
    pub const fn with_pending_blocks(mut self, pending_blocks: u64) -> Self {
        self.pending_blocks = pending_blocks;
        self
    }

    #[must_use]
    pub fn with_capture_capacity(
        mut self,
        backlog_records: u64,
        oldest_pending_height: Option<BlockHeight>,
        disk_free_basis_points: Option<u16>,
    ) -> Self {
        self.capture_backlog_records = backlog_records;
        self.oldest_pending_capture_height = oldest_pending_height.map(BlockHeight::get);
        self.disk_free_basis_points = disk_free_basis_points;
        self
    }

    #[must_use]
    pub fn with_archive_manifest_id(mut self, manifest_id: Option<String>) -> Self {
        self.archive_manifest_id = manifest_id;
        self
    }

    #[must_use]
    pub fn with_last_error_reason(mut self, reason: Option<String>) -> Self {
        self.last_error_reason = reason;
        self
    }

    #[must_use]
    pub const fn with_throughput(self, records_per_sec: u32, blocks_per_sec: u32) -> Self {
        self.with_optional_throughput(Some(records_per_sec), Some(blocks_per_sec))
    }

    #[must_use]
    pub(crate) const fn with_optional_throughput(
        mut self,
        records_per_sec: Option<u32>,
        blocks_per_sec: Option<u32>,
    ) -> Self {
        self.throughput_records_per_sec = records_per_sec;
        self.throughput_blocks_per_sec = blocks_per_sec;
        self
    }

    #[must_use]
    pub fn with_auxiliary_sources(mut self, sources: Vec<AuxiliarySourceStatus>) -> Self {
        self.auxiliary_sources = sources;
        self
    }

    #[must_use]
    pub fn with_maintenance(mut self, maintenance: CaptureMaintenanceStatus) -> Self {
        self.maintenance = Some(maintenance);
        self.schema_version = STATUS_SCHEMA_V5.to_owned();
        self
    }

    #[must_use]
    pub const fn maintenance(&self) -> Option<&CaptureMaintenanceStatus> {
        self.maintenance.as_ref()
    }

    /// True when this snapshot is v5 with fail-closed `maintenance` present.
    /// Idle maintenance (`enabled: false`, `retention_authorized: false`) counts.
    #[must_use]
    pub fn has_fail_closed_maintenance(&self) -> bool {
        self.schema_version == STATUS_SCHEMA_V5 && self.maintenance.is_some()
    }

    /// True when `/healthz` may return HTTP 200: v5 with fail-closed
    /// `maintenance` and `ready: true`. Leftover v4 and not-ready v5 are false.
    #[must_use]
    pub fn live_ready(&self) -> bool {
        self.has_fail_closed_maintenance() && self.ready
    }

    #[must_use]
    pub const fn health(&self) -> CaptureHealth {
        self.health
    }

    #[must_use]
    pub const fn ready(&self) -> bool {
        self.ready
    }

    #[must_use]
    pub const fn throughput_records_per_sec(&self) -> Option<u32> {
        self.throughput_records_per_sec
    }

    #[must_use]
    pub const fn throughput_blocks_per_sec(&self) -> Option<u32> {
        self.throughput_blocks_per_sec
    }

    #[must_use]
    pub fn into_terminal(
        mut self,
        snapshot_at: KnownTime,
        health: CaptureHealth,
        last_error_reason: Option<String>,
    ) -> Self {
        self.snapshot_at_micros = snapshot_at.unix_micros();
        self.health = health;
        self.ready = false;
        self.last_error_reason = last_error_reason;
        if self.maintenance.is_none() {
            self.maintenance = Some(CaptureMaintenanceStatus::idle(false, false));
        }
        self.schema_version = STATUS_SCHEMA_V5.to_owned();
        self
    }

    pub(crate) fn belongs_to(&self, build_id: &str, chain_id: &ChainId) -> bool {
        self.build_id == build_id && self.chain_id == chain_id.as_str()
    }

    fn validate(&self) -> Result<(), StatusError> {
        match (self.schema_version.as_str(), self.maintenance.as_ref()) {
            (STATUS_SCHEMA_V4, None) => {}
            (STATUS_SCHEMA_V5, Some(maintenance)) => maintenance.validate()?,
            _ => return Err(StatusError::InvalidSchema),
        }
        validate_status_text(&self.build_id)?;
        validate_status_text(&self.chain_id)?;
        if let Some(manifest_id) = &self.archive_manifest_id {
            validate_status_text(manifest_id)?;
        }
        if let Some(reason) = &self.last_error_reason {
            validate_reason_code(reason)?;
        }
        if self.auxiliary_sources.len() > MAX_AUXILIARY_SOURCES {
            return Err(StatusError::InvalidField);
        }
        let mut previous = None;
        for source in &self.auxiliary_sources {
            source.validate()?;
            if previous.is_some_and(|value: &str| value >= source.source_id()) {
                return Err(StatusError::InvalidField);
            }
            previous = Some(source.source_id());
        }
        if self
            .disk_free_basis_points
            .is_some_and(|basis_points| basis_points > 10_000)
            || (self.capture_backlog_records == 0 && self.oldest_pending_capture_height.is_some())
            || (self.capture_backlog_records > 0 && self.oldest_pending_capture_height.is_none())
        {
            return Err(StatusError::InvalidField);
        }
        match self.active_committed_source {
            CommittedSourceClass::LocallyVerifiedCommitted
                if self.failover_height.is_some() || self.failover_reason.is_some() =>
            {
                return Err(StatusError::InvalidField);
            }
            CommittedSourceClass::IndependentCommitted
                if self.independent_source_health.is_none()
                    || self.failover_height.is_none()
                    || self.failover_reason.is_none()
                    || self.health == CaptureHealth::Green =>
            {
                return Err(StatusError::InvalidField);
            }
            _ => {}
        }
        let active_source_health = match self.active_committed_source {
            CommittedSourceClass::LocallyVerifiedCommitted => self.primary_source_health,
            CommittedSourceClass::IndependentCommitted => self
                .independent_source_health
                .ok_or(StatusError::InvalidField)?,
        };
        if (self.ready || self.health == CaptureHealth::Green)
            && active_source_health != CaptureSourceHealth::Healthy
        {
            return Err(StatusError::InvalidField);
        }
        Ok(())
    }
}

pub fn read_status(path: &Path) -> Result<CaptureStatus, StatusError> {
    let (status, _) = read_status_document(path)?;
    Ok(status)
}

pub(crate) fn read_status_snapshot_bytes(path: &Path) -> Result<Vec<u8>, StatusError> {
    let (_, bytes) = read_status_document(path)?;
    Ok(bytes)
}

fn read_status_document(path: &Path) -> Result<(CaptureStatus, Vec<u8>), StatusError> {
    validate_status_path(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|_| StatusError::Io)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > u64::try_from(MAX_STATUS_BYTES).expect("status bound fits u64")
    {
        return Err(StatusError::UnsafePath);
    }
    let bytes = fs::read(path).map_err(|_| StatusError::Io)?;
    if bytes.len() > MAX_STATUS_BYTES {
        return Err(StatusError::TooLarge);
    }
    let status: CaptureStatus =
        serde_json::from_slice(&bytes).map_err(|_| StatusError::Serialization)?;
    status.validate()?;
    Ok((status, bytes))
}

#[derive(Debug)]
pub struct StatusWriter {
    path: PathBuf,
}

impl StatusWriter {
    pub fn new(path: PathBuf) -> Result<Self, StatusError> {
        validate_status_path(&path)?;
        let parent = path.parent().ok_or(StatusError::UnsafePath)?;
        fs::create_dir_all(parent).map_err(|_| StatusError::Io)?;
        let metadata = fs::symlink_metadata(parent).map_err(|_| StatusError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(StatusError::UnsafePath);
        }
        if let Ok(existing) = fs::symlink_metadata(&path)
            && (existing.file_type().is_symlink() || !existing.is_file())
        {
            return Err(StatusError::UnsafePath);
        }
        Ok(Self { path })
    }

    pub fn write(&self, status: &CaptureStatus) -> Result<(), StatusError> {
        status.validate()?;
        if let Ok(existing) = fs::symlink_metadata(&self.path)
            && (existing.file_type().is_symlink() || !existing.is_file())
        {
            return Err(StatusError::UnsafePath);
        }
        let bytes = serde_json::to_vec(status).map_err(|_| StatusError::Serialization)?;
        if bytes.len() > MAX_STATUS_BYTES {
            return Err(StatusError::TooLarge);
        }
        let parent = self.path.parent().ok_or(StatusError::UnsafePath)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| StatusError::Io)?;
        temporary.write_all(&bytes).map_err(|_| StatusError::Io)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| StatusError::Io)?;
        temporary.persist(&self.path).map_err(|_| StatusError::Io)?;
        File::open(parent)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| StatusError::Io)
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum StatusError {
    #[error("capture status path is unsafe")]
    UnsafePath,
    #[error("capture status contains an invalid field")]
    InvalidField,
    #[error("capture status schema version is unsupported")]
    InvalidSchema,
    #[error("capture status exceeds its size limit")]
    TooLarge,
    #[error("capture status serialization failed")]
    Serialization,
    #[error("capture status filesystem operation failed")]
    Io,
}

impl StatusError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::UnsafePath => "capture_status.unsafe_path",
            Self::InvalidField => "capture_status.invalid_field",
            Self::InvalidSchema => "capture_status.invalid_schema",
            Self::TooLarge => "capture_status.too_large",
            Self::Serialization => "capture_status.serialization",
            Self::Io => "capture_status.io",
        }
    }
}

fn validate_status_path(path: &Path) -> Result<(), StatusError> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        Err(StatusError::UnsafePath)
    } else {
        Ok(())
    }
}

fn validate_status_text(value: &str) -> Result<(), StatusError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_STATUS_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        Err(StatusError::InvalidField)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_reason_code(value: &str) -> Result<(), StatusError> {
    validate_status_text(value)?;
    if value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
    }) {
        Ok(())
    } else {
        Err(StatusError::InvalidField)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use domain_types::{ChainId, KnownTime};
    use tempfile::TempDir;

    use super::{
        AuxiliaryQualificationState, AuxiliarySourceHealth, AuxiliarySourceStatus, CaptureHealth,
        CaptureMaintenanceStatus, CaptureSourceHealth, CaptureStatus, CommittedSourceClass,
        RestartReconstruction, StatusError, read_status,
    };

    #[test]
    fn auxiliary_status_exposes_durable_cursor_lag_quarantine_and_qualification() {
        let mut auxiliary = AuxiliarySourceStatus::starting("node-misc-events");
        auxiliary.record_durable(
            "node-file-v1:epoch",
            "node-file-v1:epoch",
            47,
            3,
            11,
            true,
            1_000,
            Some("source.schema_drift"),
        );
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        )
        .with_auxiliary_sources(vec![auxiliary]);

        status.validate().unwrap();
        let value = serde_json::to_value(status).unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(value["maintenance"]["enabled"], false);
        assert_eq!(value["maintenance"]["retention_authorized"], false);
        assert!(value.get("throughput_records_per_sec").is_none());
        assert!(value.get("throughput_blocks_per_sec").is_none());
        assert_eq!(
            value["auxiliary_sources"][0]["source_id"],
            "node-misc-events"
        );
        assert_eq!(value["auxiliary_sources"][0]["health"], "quarantined");
        assert_eq!(
            value["auxiliary_sources"][0]["qualification"],
            "unqualified"
        );
        assert_eq!(
            value["auxiliary_sources"][0]["cursor_epoch"],
            "node-file-v1:epoch"
        );
        assert_eq!(
            value["auxiliary_sources"][0]["tail_cursor_epoch"],
            "node-file-v1:epoch"
        );
        assert_eq!(value["auxiliary_sources"][0]["durable_offset"], 47);
        assert_eq!(value["auxiliary_sources"][0]["local_sequence"], 3);
        assert_eq!(value["auxiliary_sources"][0]["spool_records"], 3);
        assert_eq!(value["auxiliary_sources"][0]["unarchived_records"], 0);
        assert_eq!(value["auxiliary_sources"][0]["unread_bytes"], 11);
        assert_eq!(value["auxiliary_sources"][0]["partial_line"], true);
        assert_eq!(
            value["auxiliary_sources"][0]["quarantine_reason"],
            "source.schema_drift"
        );
        assert_eq!(
            value["auxiliary_sources"][0]["restart_reconstruction"],
            "not-required"
        );
        assert!(value["auxiliary_sources"][0]["last_error_reason"].is_null());
    }

    #[test]
    fn auxiliary_status_rejects_unsorted_duplicate_or_inconsistent_sources() {
        let duplicate = vec![
            AuxiliarySourceStatus::starting("node-fills"),
            AuxiliarySourceStatus::starting("node-fills"),
        ];
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        )
        .with_auxiliary_sources(duplicate);
        assert_eq!(status.validate(), Err(StatusError::InvalidField));

        let mut missing_durable_epoch = AuxiliarySourceStatus::starting("node-fills");
        missing_durable_epoch.tail_cursor_epoch = Some("tail-epoch".to_owned());
        missing_durable_epoch.durable_offset = Some(47);
        missing_durable_epoch.local_sequence = Some(1);
        missing_durable_epoch.spool_records = 1;
        missing_durable_epoch.last_durable_wall_micros = Some(1_000);
        assert_eq!(
            missing_durable_epoch.validate(),
            Err(StatusError::InvalidField)
        );

        let mut invalid_tail_epoch = AuxiliarySourceStatus::starting("node-fills");
        invalid_tail_epoch.tail_cursor_epoch = Some("bad\nepoch".to_owned());
        assert_eq!(
            invalid_tail_epoch.validate(),
            Err(StatusError::InvalidField)
        );

        let mut inconsistent_lag = AuxiliarySourceStatus::starting("node-fills");
        inconsistent_lag.spool_records = 2;
        inconsistent_lag.unarchived_records = 1;
        assert_eq!(inconsistent_lag.validate(), Err(StatusError::InvalidField));

        let mut invalid = AuxiliarySourceStatus::starting("node-fills");
        invalid.health = AuxiliarySourceHealth::Healthy;
        invalid.qualification = AuxiliaryQualificationState::Qualified;
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        )
        .with_auxiliary_sources(vec![invalid]);
        assert_eq!(status.validate(), Err(StatusError::InvalidField));
    }

    #[test]
    fn recovered_auxiliary_status_is_incomplete_until_the_live_tail_binds() {
        let mut auxiliary = AuxiliarySourceStatus::starting("node-fills");
        auxiliary.record_recovered("node-file-v1:epoch-a", 47, 3, 1_000, None);
        assert_eq!(
            auxiliary.restart_reconstruction(),
            RestartReconstruction::Incomplete
        );
        assert_eq!(auxiliary.cursor_epoch(), Some("node-file-v1:epoch-a"));
        assert_eq!(auxiliary.tail_cursor_epoch(), Some("node-file-v1:epoch-a"));
        auxiliary.validate().unwrap();

        auxiliary.record_tail("node-file-v1:epoch-b", 0, false);
        assert_eq!(
            auxiliary.restart_reconstruction(),
            RestartReconstruction::Complete
        );
        assert_eq!(auxiliary.tail_cursor_epoch(), Some("node-file-v1:epoch-b"));
        auxiliary.validate().unwrap();
    }

    #[test]
    fn status_reader_rejects_json_that_erases_a_quarantine_cause() {
        let mut auxiliary = AuxiliarySourceStatus::starting("node-misc-events");
        auxiliary.record_durable(
            "node-file-v1:epoch",
            "node-file-v1:epoch",
            47,
            1,
            0,
            false,
            1_000,
            Some("source.schema_drift"),
        );
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        )
        .with_auxiliary_sources(vec![auxiliary]);
        let mut value = serde_json::to_value(status).unwrap();
        value["auxiliary_sources"][0]
            .as_object_mut()
            .unwrap()
            .remove("quarantine_reason");
        value["auxiliary_sources"][0]["last_error_reason"] =
            serde_json::Value::String("source.temporary_disconnect".to_owned());
        let root = TempDir::new().unwrap();
        let path = root.path().join("status.json");
        fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();

        assert_eq!(read_status(&path), Err(StatusError::InvalidField));
    }

    #[test]
    fn v5_status_exposes_windowed_throughput_without_qualification() {
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        )
        .with_throughput(3, 1);
        status.validate().unwrap();
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(value["throughput_records_per_sec"], 3);
        assert_eq!(value["throughput_blocks_per_sec"], 1);
        assert_eq!(value["maintenance"]["enabled"], false);
        assert_eq!(value["maintenance"]["retention_authorized"], false);
        assert!(value.get("qualification").is_none());
        assert_eq!(status.throughput_records_per_sec(), Some(3));
        assert_eq!(status.throughput_blocks_per_sec(), Some(1));
    }

    #[test]
    fn status_reader_rejects_invalid_throughput_and_reconstruction_values() {
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        )
        .with_throughput(3, 1);
        let root = TempDir::new().unwrap();
        let path = root.path().join("status.json");

        let mut negative = serde_json::to_value(&status).unwrap();
        negative["throughput_records_per_sec"] = serde_json::json!(-1);
        fs::write(&path, serde_json::to_vec(&negative).unwrap()).unwrap();
        assert_eq!(read_status(&path), Err(StatusError::Serialization));

        let mut fractional = serde_json::to_value(&status).unwrap();
        fractional["throughput_blocks_per_sec"] = serde_json::json!(1.5);
        fs::write(&path, serde_json::to_vec(&fractional).unwrap()).unwrap();
        assert_eq!(read_status(&path), Err(StatusError::Serialization));

        let mut named = serde_json::to_value(&status).unwrap();
        named["throughput_records_per_sec"] = serde_json::json!("fast");
        fs::write(&path, serde_json::to_vec(&named).unwrap()).unwrap();
        assert_eq!(read_status(&path), Err(StatusError::Serialization));

        let mut reconstruction = AuxiliarySourceStatus::starting("node-fills");
        reconstruction.record_recovered("node-file-v1:epoch", 47, 3, 1_000, None);
        let mut reconstructed = serde_json::to_value(
            CaptureStatus::new(
                KnownTime::from_unix_micros(1_000).unwrap(),
                "build-v1",
                ChainId::new("mainnet").unwrap(),
                CaptureHealth::Yellow,
            )
            .with_auxiliary_sources(vec![reconstruction]),
        )
        .unwrap();
        reconstructed["auxiliary_sources"][0]["restart_reconstruction"] =
            serde_json::json!("live-qualified");
        fs::write(&path, serde_json::to_vec(&reconstructed).unwrap()).unwrap();
        assert_eq!(read_status(&path), Err(StatusError::Serialization));

        let mut incomplete_without_durable = AuxiliarySourceStatus::starting("node-fills");
        incomplete_without_durable.restart_reconstruction = RestartReconstruction::Incomplete;
        assert_eq!(
            incomplete_without_durable.validate(),
            Err(StatusError::InvalidField)
        );
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[expect(dead_code)]
    struct FrozenCaptureStatusV4 {
        schema_version: String,
        snapshot_at_micros: i64,
        build_id: String,
        chain_id: String,
        health: CaptureHealth,
        ready: bool,
        active_committed_source: super::CommittedSourceClass,
        primary_source_health: super::CaptureSourceHealth,
        #[serde(default)]
        independent_source_health: Option<super::CaptureSourceHealth>,
        #[serde(default)]
        failover_height: Option<u64>,
        #[serde(default)]
        failover_reason: Option<crate::FailoverReason>,
        #[serde(default)]
        durable_height: Option<u64>,
        pending_blocks: u64,
        #[serde(default)]
        capture_backlog_records: u64,
        #[serde(default)]
        oldest_pending_capture_height: Option<u64>,
        #[serde(default)]
        disk_free_basis_points: Option<u16>,
        #[serde(default)]
        archive_manifest_id: Option<String>,
        #[serde(default)]
        last_error_reason: Option<String>,
        #[serde(default)]
        throughput_records_per_sec: u32,
        #[serde(default)]
        throughput_blocks_per_sec: u32,
        #[serde(default)]
        auxiliary_sources: Vec<AuxiliarySourceStatus>,
    }

    #[derive(Debug, serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    #[expect(dead_code)]
    struct FrozenCaptureStatusV5 {
        schema_version: String,
        snapshot_at_micros: i64,
        build_id: String,
        chain_id: String,
        health: CaptureHealth,
        ready: bool,
        active_committed_source: super::CommittedSourceClass,
        primary_source_health: super::CaptureSourceHealth,
        #[serde(default)]
        independent_source_health: Option<super::CaptureSourceHealth>,
        #[serde(default)]
        failover_height: Option<u64>,
        #[serde(default)]
        failover_reason: Option<crate::FailoverReason>,
        #[serde(default)]
        durable_height: Option<u64>,
        pending_blocks: u64,
        #[serde(default)]
        capture_backlog_records: u64,
        #[serde(default)]
        oldest_pending_capture_height: Option<u64>,
        #[serde(default)]
        disk_free_basis_points: Option<u16>,
        #[serde(default)]
        archive_manifest_id: Option<String>,
        #[serde(default)]
        last_error_reason: Option<String>,
        #[serde(default)]
        throughput_records_per_sec: Option<u32>,
        #[serde(default)]
        throughput_blocks_per_sec: Option<u32>,
        #[serde(default)]
        auxiliary_sources: Vec<AuxiliarySourceStatus>,
        maintenance: CaptureMaintenanceStatus,
    }

    fn capture_fixture(name: &str) -> Vec<u8> {
        fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/capture")
                .join(name),
        )
        .unwrap_or_else(|error| panic!("read fixture {name}: {error}"))
    }

    fn write_fixture(name: &str) -> (TempDir, PathBuf) {
        let root = TempDir::new().unwrap();
        let path = root.path().join("status.json");
        fs::write(&path, capture_fixture(name)).unwrap();
        (root, path)
    }

    #[test]
    fn v4_inactive_fixture_is_accepted_without_maintenance() {
        let (_root, path) = write_fixture("status-v4.json");
        let status = read_status(&path).expect("v4 fixture");
        status.validate().unwrap();
        assert!(status.maintenance().is_none());
        let value = serde_json::from_slice::<serde_json::Value>(&capture_fixture("status-v4.json"))
            .unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v4");
        assert!(value.get("maintenance").is_none());
        let decoded: FrozenCaptureStatusV4 = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.schema_version, "hl.capture.status.v4");
        assert_eq!(decoded.build_id, "synthetic-serve-status-fixture");
        assert_eq!(decoded.throughput_records_per_sec, 0);
        assert_eq!(decoded.throughput_blocks_per_sec, 0);
        assert!(status.throughput_records_per_sec().is_none());
        assert!(status.throughput_blocks_per_sec().is_none());
    }

    #[test]
    fn v5_fixture_requires_maintenance_and_is_rejected_by_frozen_v4_readers() {
        let (_root, path) = write_fixture("status-v5.json");
        let status = read_status(&path).expect("v5 fixture");
        status.validate().unwrap();
        let maintenance = status.maintenance().expect("v5 maintenance");
        assert!(maintenance.enabled());
        assert!(!maintenance.retention_authorized());
        assert_eq!(maintenance.health(), CaptureHealth::Green);
        let value = serde_json::from_slice::<serde_json::Value>(&capture_fixture("status-v5.json"))
            .unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(
            value["auxiliary_sources"][0]["restart_reconstruction"],
            "complete"
        );
        assert!(serde_json::from_value::<FrozenCaptureStatusV4>(value.clone()).is_err());
        let decoded: FrozenCaptureStatusV5 = serde_json::from_value(value).unwrap();
        assert_eq!(decoded.schema_version, "hl.capture.status.v5");
        assert!(!decoded.maintenance.retention_authorized());
    }

    #[test]
    fn v4_fixture_that_smuggles_maintenance_is_rejected() {
        let bytes = capture_fixture("status-v4-smuggled-maintenance.json");
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v4");
        assert!(serde_json::from_value::<FrozenCaptureStatusV4>(value).is_err());
        let (_root, path) = write_fixture("status-v4-smuggled-maintenance.json");
        assert_eq!(read_status(&path), Err(StatusError::InvalidSchema));
    }

    #[test]
    fn v5_without_maintenance_and_unknown_schema_are_rejected() {
        let mut v5 =
            serde_json::from_slice::<serde_json::Value>(&capture_fixture("status-v4.json"))
                .unwrap();
        v5["schema_version"] = serde_json::json!("hl.capture.status.v5");
        let root = TempDir::new().unwrap();
        let path = root.path().join("status.json");
        fs::write(&path, serde_json::to_vec(&v5).unwrap()).unwrap();
        assert_eq!(read_status(&path), Err(StatusError::InvalidSchema));

        v5["schema_version"] = serde_json::json!("hl.capture.status.v6");
        fs::write(&path, serde_json::to_vec(&v5).unwrap()).unwrap();
        assert_eq!(read_status(&path), Err(StatusError::InvalidSchema));
    }

    #[test]
    fn snapshot_bytes_are_returned_as_read_after_validation() {
        let (_root, path) = write_fixture("status-v5.json");
        let bytes = super::read_status_snapshot_bytes(&path).expect("v5 bytes");
        assert_eq!(bytes, capture_fixture("status-v5.json"));
    }

    #[test]
    fn writer_defaults_to_v5_with_inactive_maintenance_and_omitted_rates() {
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        );
        status.validate().unwrap();
        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(value["maintenance"]["enabled"], false);
        assert_eq!(value["maintenance"]["kill_switch"], false);
        assert_eq!(value["maintenance"]["health"], "green");
        assert_eq!(value["maintenance"]["retention_authorized"], false);
        assert_eq!(value["maintenance"]["pending_pack_manifest_count"], 0);
        assert_eq!(value["maintenance"]["packed_range_count"], 0);
        assert!(value["maintenance"].get("reason_code").is_none());
        assert!(value["maintenance"].get("last_scrub_at_micros").is_none());
        assert!(value.get("throughput_records_per_sec").is_none());
        assert!(value.get("throughput_blocks_per_sec").is_none());
        assert!(value.get("qualification").is_none());
        let decoded: FrozenCaptureStatusV5 = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(decoded.schema_version, "hl.capture.status.v5");
        assert!(decoded.throughput_records_per_sec.is_none());
        assert!(decoded.throughput_blocks_per_sec.is_none());

        let published = status.with_maintenance(CaptureMaintenanceStatus::idle(true, false));
        published.validate().unwrap();
        let value = serde_json::to_value(&published).unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(value["maintenance"]["enabled"], true);
        assert_eq!(value["maintenance"]["retention_authorized"], false);

        let cleared = published.with_maintenance(CaptureMaintenanceStatus::idle(false, false));
        cleared.validate().unwrap();
        let value = serde_json::to_value(&cleared).unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(value["maintenance"]["enabled"], false);
        assert_eq!(value["maintenance"]["retention_authorized"], false);

        let sampled = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Yellow,
        )
        .with_throughput(0, 0);
        let value = serde_json::to_value(&sampled).unwrap();
        assert_eq!(value["throughput_records_per_sec"], 0);
        assert_eq!(value["throughput_blocks_per_sec"], 0);
    }

    #[test]
    fn v5_only_reader_rejects_ready_v4_as_live_ready() {
        let value = serde_json::from_slice::<serde_json::Value>(&capture_fixture("status-v4.json"))
            .unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v4");
        assert_eq!(value["ready"], true);
        assert_eq!(value["health"], "green");
        assert!(value.get("maintenance").is_none());
        assert!(serde_json::from_value::<FrozenCaptureStatusV5>(value).is_err());
    }

    #[test]
    fn ready_v4_fixture_is_not_live_ready() {
        let (_root, path) = write_fixture("status-v4.json");
        let status = read_status(&path).expect("v4 fixture");
        assert!(status.ready());
        assert_eq!(status.health(), CaptureHealth::Green);
        assert!(status.maintenance().is_none());
        assert!(!status.has_fail_closed_maintenance());
        assert!(!status.live_ready());
    }

    #[test]
    fn v5_idle_maintenance_is_not_live_ready_until_ready() {
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Green,
        )
        .with_source_state(
            CommittedSourceClass::LocallyVerifiedCommitted,
            CaptureSourceHealth::Healthy,
            None,
            None,
            None,
        );
        status.validate().unwrap();
        assert!(status.has_fail_closed_maintenance());
        assert!(!status.ready());
        assert!(!status.live_ready());
    }

    #[test]
    fn v5_idle_maintenance_can_be_live_ready() {
        let status = CaptureStatus::new(
            KnownTime::from_unix_micros(1_000).unwrap(),
            "build-v1",
            ChainId::new("mainnet").unwrap(),
            CaptureHealth::Green,
        )
        .with_readiness(true)
        .with_source_state(
            CommittedSourceClass::LocallyVerifiedCommitted,
            CaptureSourceHealth::Healthy,
            None,
            None,
            None,
        );
        status.validate().unwrap();
        let maintenance = status.maintenance().expect("idle maintenance");
        assert!(!maintenance.enabled());
        assert!(!maintenance.retention_authorized());
        assert!(status.has_fail_closed_maintenance());
        assert!(status.live_ready());
    }

    #[test]
    fn into_terminal_promotes_a_v4_snapshot_to_v5() {
        let (_root, path) = write_fixture("status-v4.json");
        let status = read_status(&path).expect("v4 fixture");
        assert!(status.maintenance().is_none());
        let terminal = status.into_terminal(
            KnownTime::from_unix_micros(2).unwrap(),
            CaptureHealth::Yellow,
            Some("capture_runtime.recovering".to_owned()),
        );
        terminal.validate().unwrap();
        let value = serde_json::to_value(&terminal).unwrap();
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(value["maintenance"]["enabled"], false);
        assert_eq!(value["maintenance"]["retention_authorized"], false);
        assert_eq!(value["ready"], false);
        assert!(value.get("throughput_records_per_sec").is_none());
    }
}
