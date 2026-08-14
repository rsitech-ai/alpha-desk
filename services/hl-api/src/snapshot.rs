use std::fs;
use std::path::Path;

use api_contracts::{WireHealthAssessment, WireHealthState};
use serde::Deserialize;
use serde_json::{Map, Value};
use thiserror::Error;

pub const HEALTH_SCHEMA_VERSION: &str = "hl.health.v1";
pub const CAPTURE_STATUS_SCHEMA_V4: &str = "hl.capture.status.v4";
pub const CAPTURE_STATUS_SCHEMA_V5: &str = "hl.capture.status.v5";
const READY_REASON_CODE: &str = "healthy";
const MAX_SNAPSHOT_BYTES: u64 = 16 * 1024;

/// Frozen hl-core file dead-letter fail-closed reason codes. Documented on
/// OpenAPI so clients type them instead of treating them as a generic 500.
/// Unknown RED codes still fail closed as typed RED and must not become ready.
/// AMBER plus any `core.deadletter_*` sibling (frozen or invented) is
/// `snapshot_invalid`. This crate does not vendor hl-core and this is not a
/// live core or Stage 2 PASS.
pub const CORE_DEADLETTER_REASON_CODES: &[&str] = &[
    "core.deadletter_unsafe_path",
    "core.deadletter_io",
    "core.deadletter_invalid_record",
    "core.deadletter_serialization",
    "core.deadletter_corrupt",
];

const CORE_DEADLETTER_REASON_PREFIX: &str = "core.deadletter_";

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
    parse_canonical_health_bytes(&bytes)
}

fn parse_canonical_health_bytes(bytes: &[u8]) -> Result<WireHealthAssessment, SnapshotError> {
    let document: HealthDocument =
        serde_json::from_slice(bytes).map_err(|_| SnapshotError::Invalid)?;
    if document.schema_version != HEALTH_SCHEMA_VERSION {
        return Err(SnapshotError::Invalid);
    }
    let state = WireHealthState::parse(&document.state).map_err(|_| SnapshotError::Invalid)?;
    reject_inconsistent_ready_reason(state, &document.reason_code)?;
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

fn reject_inconsistent_ready_reason(
    state: WireHealthState,
    reason_code: &str,
) -> Result<(), SnapshotError> {
    match state {
        WireHealthState::Green => {
            if reason_code == READY_REASON_CODE {
                Ok(())
            } else {
                Err(SnapshotError::Invalid)
            }
        }
        WireHealthState::Amber => {
            // Dead-letter family codes are RED fail-closed reasons. AMBER plus
            // a frozen enum value or an invented sibling is inconsistent, not
            // typed AMBER. Other AMBER reasons stay typed; this is not a
            // catalog of valid AMBER codes.
            if is_core_deadletter_family(reason_code) {
                Err(SnapshotError::Invalid)
            } else {
                Ok(())
            }
        }
        WireHealthState::Red => Ok(()),
    }
}

#[must_use]
pub fn is_core_deadletter_reason(reason_code: &str) -> bool {
    CORE_DEADLETTER_REASON_CODES.contains(&reason_code)
}

#[must_use]
fn is_core_deadletter_family(reason_code: &str) -> bool {
    reason_code.starts_with(CORE_DEADLETTER_REASON_PREFIX)
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
        CAPTURE_STATUS_SCHEMA_V4, CAPTURE_STATUS_SCHEMA_V5, CORE_DEADLETTER_REASON_CODES,
        MAINTENANCE_FIELDS, SnapshotError, is_core_deadletter_family, is_core_deadletter_reason,
        parse_canonical_health_bytes, parse_capture_status_bytes,
    };
    use crate::openapi::{
        LAST_HEARTBEAT_THROUGHPUT_FIELDS, core_deadletter_reason_openapi_enum,
        health_reason_code_is_unrestricted_string, openapi_yaml,
    };
    use api_contracts::WireHealthState;
    use std::path::Path;

    fn assert_openapi_does_not_claim_live_qualified(document: &str) {
        assert!(
            document.contains("not live-qualified"),
            "OpenAPI must name last-heartbeat throughput as not live-qualified"
        );
        assert_eq!(
            document.matches("live-qualified").count(),
            document.matches("not live-qualified").count(),
            "OpenAPI must not claim live-qualified sources"
        );
    }

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
        assert_openapi_does_not_claim_live_qualified(document);
    }

    #[test]
    fn openapi_document_describes_last_heartbeat_throughput() {
        let document = openapi_yaml();
        assert!(document.contains("last-heartbeat"));
        for field in LAST_HEARTBEAT_THROUGHPUT_FIELDS {
            assert!(
                document.contains(field),
                "OpenAPI missing last-heartbeat field {field}"
            );
        }
        assert!(document.contains("501"));
        assert_openapi_does_not_claim_live_qualified(document);
        assert!(document.contains("not invent fills"));
        assert!(document.contains("not a fills feed"));
    }

    #[test]
    fn openapi_document_lists_core_deadletter_fail_closed_reasons() {
        let document = openapi_yaml();
        assert!(document.contains("CoreDeadLetterReasonCode"));
        let enum_values = core_deadletter_reason_openapi_enum(document)
            .expect("OpenAPI must define components.schemas.CoreDeadLetterReasonCode.enum");
        assert_eq!(
            enum_values, CORE_DEADLETTER_REASON_CODES,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        for reason_code in CORE_DEADLETTER_REASON_CODES {
            assert!(
                is_core_deadletter_reason(reason_code),
                "helper must accept documented dead-letter reason {reason_code}"
            );
        }
        assert!(
            !is_core_deadletter_reason("healthy"),
            "healthy must not be classified as a dead-letter fail-closed reason"
        );
        assert!(
            !is_core_deadletter_reason("core.deadletter_unknown"),
            "undocumented sibling codes must not be treated as ready"
        );
        for reason_code in CORE_DEADLETTER_REASON_CODES {
            assert!(
                is_core_deadletter_family(reason_code),
                "frozen dead-letter {reason_code} is in the core.deadletter_* family"
            );
        }
        assert!(
            is_core_deadletter_family("core.deadletter_invented"),
            "invented siblings stay in the family even when outside the frozen enum"
        );
        assert!(
            !is_core_deadletter_family("lag"),
            "non-deadletter AMBER reasons must not be classified as the family"
        );
        assert!(document.contains("Unknown codes fail closed"));
        assert!(
            document.contains("core.deadletter_* family-prefix"),
            "OpenAPI must name AMBER-family core.deadletter_* as the 503 prefix"
        );
        assert!(
            document.contains("Unknown HEALTH_STATE_RED codes stay 200 typed"),
            "OpenAPI must name unknown RED as 200 typed fail-closed"
        );
        assert_openapi_does_not_claim_live_qualified(document);
    }

    fn health_bytes(state: &str, reason_code: &str) -> Vec<u8> {
        format!(
            r#"{{"schema_version":"hl.health.v1","scope":"canonical","state":"{state}","reason_code":"{reason_code}","observed_at_micros":1,"suppresses":[]}}"#
        )
        .into_bytes()
    }

    #[test]
    fn green_healthy_canonical_health_remains_ready() {
        let assessment =
            parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_GREEN", "healthy"))
                .expect("healthy green");
        assert_eq!(assessment.state, WireHealthState::Green);
        assert_eq!(assessment.reason_code, "healthy");
    }

    #[test]
    fn red_core_deadletter_health_is_accepted_as_typed_fail_closed() {
        for reason_code in CORE_DEADLETTER_REASON_CODES {
            let assessment =
                parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_RED", reason_code))
                    .unwrap_or_else(|error| panic!("{reason_code} should parse: {error}"));
            assert_eq!(assessment.state, WireHealthState::Red);
            assert_eq!(assessment.reason_code, *reason_code);
        }
    }

    #[test]
    fn green_or_amber_core_deadletter_health_is_rejected() {
        for reason_code in CORE_DEADLETTER_REASON_CODES {
            assert_eq!(
                parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_GREEN", reason_code))
                    .expect_err("green dead-letter must not become ready"),
                SnapshotError::Invalid
            );
            assert_eq!(
                parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_AMBER", reason_code))
                    .expect_err("amber dead-letter must fail closed"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn unknown_green_reason_fails_closed_and_does_not_become_ready() {
        assert_eq!(
            parse_canonical_health_bytes(&health_bytes(
                "HEALTH_STATE_GREEN",
                "core.deadletter_invented"
            ))
            .expect_err("unknown green reason must fail closed"),
            SnapshotError::Invalid
        );
        let assessment = parse_canonical_health_bytes(&health_bytes(
            "HEALTH_STATE_RED",
            "core.deadletter_invented",
        ))
        .expect("unknown red remains typed fail-closed, not ready");
        assert_eq!(assessment.state, WireHealthState::Red);
        assert_eq!(assessment.reason_code, "core.deadletter_invented");
        assert!(!is_core_deadletter_reason(&assessment.reason_code));
    }

    #[test]
    fn unknown_amber_deadletter_sibling_is_snapshot_invalid() {
        assert_eq!(
            parse_canonical_health_bytes(&health_bytes(
                "HEALTH_STATE_AMBER",
                "core.deadletter_invented"
            ))
            .expect_err("unknown amber dead-letter sibling must not be typed AMBER"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn amber_non_deadletter_reason_remains_typed() {
        let assessment = parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_AMBER", "lag"))
            .expect("AMBER without a dead-letter family reason stays typed");
        assert_eq!(assessment.state, WireHealthState::Amber);
        assert_eq!(assessment.reason_code, "lag");
        assert!(!is_core_deadletter_family(&assessment.reason_code));
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
        assert!(value.get("throughput_records_per_sec").is_none());
        assert!(value.get("throughput_blocks_per_sec").is_none());
    }

    #[test]
    fn last_heartbeat_throughput_fields_pass_through_as_read() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        value["throughput_records_per_sec"] = serde_json::json!(3);
        value["throughput_blocks_per_sec"] = serde_json::json!(1);
        let bytes = serde_json::to_vec(&value).expect("encode");
        let parsed = parse_capture_status_bytes(&bytes).expect("as-read extras");
        assert_eq!(parsed["throughput_records_per_sec"], 3);
        assert_eq!(parsed["throughput_blocks_per_sec"], 1);
        assert!(parsed.get("fills").is_none());
        assert!(parsed.get("qualification").is_none());
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
