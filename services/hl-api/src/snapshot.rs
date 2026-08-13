use std::fs;
use std::path::Path;

use api_contracts::{WireHealthAssessment, WireHealthState};
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

pub const HEALTH_SCHEMA_VERSION: &str = "hl.health.v1";
pub const CAPTURE_STATUS_SCHEMA_V4: &str = "hl.capture.status.v4";
pub const CAPTURE_STATUS_SCHEMA_V5: &str = "hl.capture.status.v5";
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024;

const MAINTENANCE_FIELDS: &[&str] = &[
    "enabled",
    "kill_switch",
    "health",
    "reason_code",
    "pending_pack_manifest_count",
    "packed_range_count",
    "logical_manifest_count",
    "physical_data_object_count",
    "last_scrub_at_micros",
    "last_pack_index_at_micros",
    "last_pack_data_at_micros",
    "last_retention_at_micros",
    "retention_authorized",
];

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CaptureStatusSchema {
    V4,
    V5,
}

impl CaptureStatusSchema {
    fn parse(value: &str) -> Result<Self, SnapshotError> {
        match value {
            CAPTURE_STATUS_SCHEMA_V4 => Ok(Self::V4),
            CAPTURE_STATUS_SCHEMA_V5 => Ok(Self::V5),
            _ => Err(SnapshotError::Invalid),
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
    parse_capture_status_bytes(&bytes)
}

fn parse_capture_status_bytes(bytes: &[u8]) -> Result<Value, SnapshotError> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| SnapshotError::Invalid)?;
    reject_lossy_numbers(&value)?;
    let object = value.as_object().ok_or(SnapshotError::Invalid)?;
    require_string(object, "schema_version", None)?;
    let schema = object
        .get("schema_version")
        .and_then(Value::as_str)
        .ok_or(SnapshotError::Invalid)
        .and_then(CaptureStatusSchema::parse)?;
    require_non_negative_int(object, "snapshot_at_micros")?;
    require_string(object, "build_id", None)?;
    require_string(object, "chain_id", None)?;
    require_enum(object, "health", &["green", "yellow", "red"])?;
    require_bool(object, "ready")?;
    require_string(object, "active_committed_source", None)?;
    require_string(object, "primary_source_health", None)?;
    require_non_negative_int(object, "pending_blocks")?;
    match schema {
        CaptureStatusSchema::V4 => {
            if object.contains_key("maintenance") {
                return Err(SnapshotError::Invalid);
            }
        }
        CaptureStatusSchema::V5 => require_maintenance(object)?,
    }
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

fn require_maintenance(object: &Map<String, Value>) -> Result<(), SnapshotError> {
    let Some(Value::Object(maintenance)) = object.get("maintenance") else {
        return Err(SnapshotError::Invalid);
    };
    if maintenance
        .keys()
        .any(|key| !MAINTENANCE_FIELDS.contains(&key.as_str()))
    {
        return Err(SnapshotError::Invalid);
    }
    require_bool(maintenance, "enabled")?;
    require_bool(maintenance, "kill_switch")?;
    require_enum(maintenance, "health", &["green", "yellow", "red"])?;
    require_non_negative_int(maintenance, "pending_pack_manifest_count")?;
    require_non_negative_int(maintenance, "packed_range_count")?;
    require_non_negative_int(maintenance, "logical_manifest_count")?;
    require_non_negative_int(maintenance, "physical_data_object_count")?;
    require_bool(maintenance, "retention_authorized")?;
    for field in [
        "last_scrub_at_micros",
        "last_pack_index_at_micros",
        "last_pack_data_at_micros",
        "last_retention_at_micros",
    ] {
        if maintenance.contains_key(field) {
            require_non_negative_int(maintenance, field)?;
        }
    }
    let health = maintenance
        .get("health")
        .and_then(Value::as_str)
        .ok_or(SnapshotError::Invalid)?;
    let reason = match maintenance.get("reason_code") {
        None => None,
        Some(Value::String(value)) if !value.is_empty() => Some(value.as_str()),
        _ => return Err(SnapshotError::Invalid),
    };
    if (health == "green") != reason.is_none() {
        return Err(SnapshotError::Invalid);
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::{
        CAPTURE_STATUS_SCHEMA_V4, CAPTURE_STATUS_SCHEMA_V5, MAINTENANCE_FIELDS, SnapshotError,
        parse_capture_status_bytes,
    };
    use crate::openapi::openapi_yaml;
    use std::path::Path;

    #[test]
    fn openapi_document_describes_v4_v5_maintenance_and_503() {
        let document = openapi_yaml();
        assert!(document.contains(CAPTURE_STATUS_SCHEMA_V4));
        assert!(document.contains(CAPTURE_STATUS_SCHEMA_V5));
        assert!(document.contains("503"));
        assert!(document.contains(SnapshotError::Missing.reason_code()));
        assert!(document.contains(SnapshotError::Invalid.reason_code()));
        for field in MAINTENANCE_FIELDS {
            assert!(
                document.contains(field),
                "OpenAPI missing maintenance field {field}"
            );
        }
        assert!(!document.contains("live-qualified"));
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/api")
                .join(name),
        )
        .unwrap_or_else(|error| panic!("read fixture {name}: {error}"))
    }

    #[test]
    fn v4_inactive_fixture_is_accepted_without_maintenance() {
        let value = parse_capture_status_bytes(&fixture("capture-status.json")).expect("v4");
        assert_eq!(value["schema_version"], "hl.capture.status.v4");
        assert!(value.get("maintenance").is_none());
        assert!(value.get("fills").is_none());
        assert!(value.get("qualification").is_none());
    }

    #[test]
    fn v5_fixture_returns_maintenance_without_inventing_fills() {
        let value = parse_capture_status_bytes(&fixture("capture-status-v5.json")).expect("v5");
        assert_eq!(value["schema_version"], "hl.capture.status.v5");
        assert_eq!(value["maintenance"]["enabled"], true);
        assert_eq!(value["maintenance"]["retention_authorized"], false);
        assert_eq!(
            value["auxiliary_sources"][0]["restart_reconstruction"],
            "complete"
        );
        assert!(value.get("fills").is_none());
        assert!(value.get("qualification").is_none());
    }

    #[test]
    fn v4_fixture_that_smuggles_maintenance_is_rejected() {
        let error =
            parse_capture_status_bytes(&fixture("capture-status-v4-smuggled-maintenance.json"))
                .expect_err("v4 must reject smuggled maintenance");
        assert_eq!(error, SnapshotError::Invalid);
    }

    #[test]
    fn v5_without_maintenance_and_unknown_schema_are_rejected() {
        let mut v5 = serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
            .expect("v4 json");
        v5["schema_version"] = serde_json::json!("hl.capture.status.v5");
        let bytes = serde_json::to_vec(&v5).expect("encode");
        assert_eq!(
            parse_capture_status_bytes(&bytes).expect_err("v5 requires maintenance"),
            SnapshotError::Invalid
        );

        v5["schema_version"] = serde_json::json!("hl.capture.status.v6");
        let bytes = serde_json::to_vec(&v5).expect("encode");
        assert_eq!(
            parse_capture_status_bytes(&bytes).expect_err("unknown schema"),
            SnapshotError::Invalid
        );
    }
}
