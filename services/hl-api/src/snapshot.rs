use std::fs;
use std::path::Path;

use api_contracts::{WireHealthAssessment, WireHealthState};
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

pub const HEALTH_SCHEMA_VERSION: &str = "hl.health.v1";
pub const CAPTURE_STATUS_SCHEMA_VERSION: &str = "hl.capture.status.v4";
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SnapshotError {
    #[error("snapshot_missing")]
    Missing,
    #[error("snapshot_invalid")]
    Invalid,
}

impl SnapshotError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Missing => "snapshot_missing",
            Self::Invalid => "snapshot_invalid",
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct HealthDocument {
    schema_version: String,
    scope: String,
    state: String,
    reason_code: String,
    observed_at_micros: i64,
    suppresses: Vec<String>,
}

pub fn load_canonical_health(path: Option<&Path>) -> Result<WireHealthAssessment, SnapshotError> {
    let Some(path) = path else {
        return Err(SnapshotError::Missing);
    };
    let bytes = read_bounded(path)?;
    let document: HealthDocument =
        serde_json::from_slice(&bytes).map_err(|_| SnapshotError::Invalid)?;
    if document.schema_version != HEALTH_SCHEMA_VERSION {
        return Err(SnapshotError::Invalid);
    }
    let state = WireHealthState::parse(&document.state).map_err(|_| SnapshotError::Invalid)?;
    let assessment = WireHealthAssessment::try_new(
        document.scope,
        state,
        document.reason_code,
        document.observed_at_micros,
        document.suppresses,
    )
    .map_err(|_| SnapshotError::Invalid)?;
    WireHealthAssessment::decode(&assessment.encode_to_vec()).map_err(|_| SnapshotError::Invalid)
}

pub fn load_capture_status(path: Option<&Path>) -> Result<Value, SnapshotError> {
    let Some(path) = path else {
        return Err(SnapshotError::Missing);
    };
    let bytes = read_bounded(path)?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| SnapshotError::Invalid)?;
    reject_lossy_numbers(&value)?;
    let object = value.as_object().ok_or(SnapshotError::Invalid)?;
    require_string(
        object,
        "schema_version",
        Some(CAPTURE_STATUS_SCHEMA_VERSION),
    )?;
    require_non_negative_int(object, "snapshot_at_micros")?;
    require_string(object, "build_id", None)?;
    require_string(object, "chain_id", None)?;
    require_enum(object, "health", &["green", "yellow", "red"])?;
    require_bool(object, "ready")?;
    require_string(object, "active_committed_source", None)?;
    require_string(object, "primary_source_health", None)?;
    require_non_negative_int(object, "pending_blocks")?;
    Ok(value)
}

fn read_bounded(path: &Path) -> Result<Vec<u8>, SnapshotError> {
    let metadata = fs::metadata(path).map_err(|_| SnapshotError::Missing)?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_SNAPSHOT_BYTES {
        return Err(SnapshotError::Invalid);
    }
    fs::read(path).map_err(|_| SnapshotError::Missing)
}

fn reject_lossy_numbers(value: &Value) -> Result<(), SnapshotError> {
    match value {
        Value::Number(number) if number.as_i64().is_none() && number.as_u64().is_none() => {
            Err(SnapshotError::Invalid)
        }
        Value::Array(items) => items.iter().try_for_each(reject_lossy_numbers),
        Value::Object(fields) => fields.values().try_for_each(reject_lossy_numbers),
        _ => Ok(()),
    }
}

fn require_string(
    object: &Map<String, Value>,
    field: &str,
    expected: Option<&str>,
) -> Result<(), SnapshotError> {
    let Value::String(value) = object.get(field).ok_or(SnapshotError::Invalid)? else {
        return Err(SnapshotError::Invalid);
    };
    if value.is_empty() || expected.is_some_and(|expected| value != expected) {
        return Err(SnapshotError::Invalid);
    }
    Ok(())
}

fn require_enum(
    object: &Map<String, Value>,
    field: &str,
    allowed: &[&str],
) -> Result<(), SnapshotError> {
    let Value::String(value) = object.get(field).ok_or(SnapshotError::Invalid)? else {
        return Err(SnapshotError::Invalid);
    };
    if allowed.contains(&value.as_str()) {
        Ok(())
    } else {
        Err(SnapshotError::Invalid)
    }
}

fn require_bool(object: &Map<String, Value>, field: &str) -> Result<(), SnapshotError> {
    match object.get(field) {
        Some(Value::Bool(_)) => Ok(()),
        _ => Err(SnapshotError::Invalid),
    }
}

fn require_non_negative_int(object: &Map<String, Value>, field: &str) -> Result<(), SnapshotError> {
    match object.get(field) {
        Some(Value::Number(number))
            if number.as_u64().is_some() || number.as_i64().is_some_and(|value| value >= 0) =>
        {
            Ok(())
        }
        _ => Err(SnapshotError::Invalid),
    }
}
