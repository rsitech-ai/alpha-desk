use std::fs::{self, File};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use domain_types::{BlockHeight, ChainId, KnownTime};
use serde::{Deserialize, Serialize};

const STATUS_SCHEMA_VERSION: &str = "hl.capture.status.v2";
const MAX_STATUS_BYTES: usize = 16 * 1024;
const MAX_STATUS_TEXT_BYTES: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureHealth {
    Green,
    Yellow,
    Red,
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
            durable_height: None,
            pending_blocks: 0,
            capture_backlog_records: 0,
            oldest_pending_capture_height: None,
            disk_free_basis_points: None,
            archive_manifest_id: None,
            last_error_reason: None,
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
        if self
            .disk_free_basis_points
            .is_some_and(|basis_points| basis_points > 10_000)
            || (self.capture_backlog_records == 0 && self.oldest_pending_capture_height.is_some())
            || (self.capture_backlog_records > 0 && self.oldest_pending_capture_height.is_none())
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

fn validate_reason_code(value: &str) -> Result<(), StatusError> {
    validate_status_text(value)?;
    if value.bytes().all(|byte| {
        byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_')
    }) {
        Ok(())
    } else {
        Err(StatusError::InvalidField)
    }
}
