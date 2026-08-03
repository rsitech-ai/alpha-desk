use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use domain_types::{BlockHeight, ChainId, KnownTime};
use serde::{Deserialize, Serialize};

const STATUS_SCHEMA_VERSION: &str = "hl.capture.status.v4";
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
        self.durable_offset = Some(durable_offset);
        self.local_sequence = Some(local_sequence);
        self.spool_records = local_sequence;
        self.unarchived_records = 0;
        self.last_durable_wall_micros = Some(last_durable_wall_micros);
        self.quarantine_reason = quarantine_reason.map(ToOwned::to_owned);
        self.last_error_reason = None;
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
        {
            return Err(StatusError::InvalidField);
        }
        Ok(())
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    auxiliary_sources: Vec<AuxiliarySourceStatus>,
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
            schema_version: STATUS_SCHEMA_VERSION.to_owned(),
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
            auxiliary_sources: Vec::new(),
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
    pub fn with_auxiliary_sources(mut self, sources: Vec<AuxiliarySourceStatus>) -> Self {
        self.auxiliary_sources = sources;
        self
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
        self
    }

    pub(crate) fn belongs_to(&self, build_id: &str, chain_id: &ChainId) -> bool {
        self.build_id == build_id && self.chain_id == chain_id.as_str()
    }

    fn validate(&self) -> Result<(), StatusError> {
        if self.schema_version != STATUS_SCHEMA_VERSION {
            return Err(StatusError::InvalidSchema);
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
    Ok(status)
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

    use domain_types::{ChainId, KnownTime};
    use tempfile::TempDir;

    use super::{
        AuxiliaryQualificationState, AuxiliarySourceHealth, AuxiliarySourceStatus, CaptureHealth,
        CaptureStatus, StatusError, read_status,
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
        assert_eq!(value["schema_version"], "hl.capture.status.v4");
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
}
