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

/// Frozen hl-core consume-poison / action-bearing reject reason. Documented
/// on OpenAPI so clients type it instead of treating it as a generic 500.
/// Unknown RED codes still fail closed as typed RED and must not become
/// ready. GREEN plus this code is `snapshot_invalid`. AMBER plus this exact
/// code is `snapshot_invalid`; invented `ledger.*` siblings are not a family
/// prefix. This crate does not vendor hl-core and this is not a live core or
/// Stage 2 PASS.
pub const LEDGER_UNSUPPORTED_EVENT_REASON_CODES: &[&str] = &["ledger.unsupported_event"];

/// Capture writer kebab-case committed source class. Unknown values are
/// `snapshot_invalid`. This crate does not vendor hl-capture and this is
/// not a live capture or Stage PASS. `HealthAssessment.reason_code` stays a
/// free string so unknown RED is not closed out.
pub const COMMITTED_SOURCE_CLASSES: &[&str] =
    &["locally-verified-committed", "independent-committed"];

/// Capture writer kebab-case committed source health. Unknown values are
/// `snapshot_invalid`. This crate does not vendor hl-capture and this is
/// not a live capture or Stage PASS. `HealthAssessment.reason_code` stays a
/// free string so unknown RED is not closed out.
pub const CAPTURE_SOURCE_HEALTH: &[&str] = &["starting", "healthy", "range-unavailable"];

/// Capture writer kebab-case auxiliary source health. Present unknown
/// values are `snapshot_invalid`. Omitted stays omitted. This crate
/// does not vendor hl-capture and this is not a live capture or Stage PASS.
/// `HealthAssessment.reason_code` stays a free string so unknown RED is not
/// closed out. This is not committed source health.
pub const AUXILIARY_SOURCE_HEALTH: &[&str] = &["starting", "healthy", "quarantined", "latched"];

/// Capture writer kebab-case auxiliary restart reconstruction. Present
/// unknown values are `snapshot_invalid`. Omitted stays omitted. This crate
/// does not vendor hl-capture and this is not a live capture or Stage PASS.
/// `HealthAssessment.reason_code` stays a free string so unknown RED is not
/// closed out.
pub const RESTART_RECONSTRUCTION: &[&str] = &["not-required", "incomplete", "complete"];

/// Capture writer kebab-case auxiliary source qualification. Present
/// unknown values are `snapshot_invalid`. Omitted stays omitted. This crate
/// does not vendor hl-capture and this is not a live capture or Stage PASS.
/// `HealthAssessment.reason_code` stays a free string so unknown RED is not
/// closed out.
pub const AUXILIARY_SOURCE_QUALIFICATION: &[&str] = &["unqualified", "qualified"];

/// Capture writer kebab-case top-level failover reason. Present unknown
/// values are `snapshot_invalid`. Omitted stays omitted. This crate
/// does not vendor hl-capture and this is not a live capture or Stage PASS.
/// `HealthAssessment.reason_code` stays a free string so unknown RED is not
/// closed out. This is not a closed enum of `HealthAssessment.reason_code`.
pub const FAILOVER_REASONS: &[&str] = &["primary-range-unavailable"];

/// Capture writer `MAX_AUXILIARY_SOURCES`. Present arrays longer than this
/// are `snapshot_invalid`. Omitted and empty arrays stay valid. Duplicate
/// present `source_id` is `snapshot_invalid`. Distinct ids stay valid when
/// strictly increasing (`previous >= source_id` is `snapshot_invalid`).
/// Present unknown nested properties are `snapshot_invalid`. Known objects
/// without extras stay valid. This crate does not vendor hl-capture and this
/// is not a live capture or Stage PASS. `HealthAssessment.reason_code` stays
/// a free string so unknown RED is not closed out.
pub const MAX_AUXILIARY_SOURCES: usize = 16;

/// Capture writer `AuxiliarySourceStatus` public keys plus this stack's
/// already-typed optional `restart_reconstruction`. Present unknown nested
/// properties are `snapshot_invalid`. Known objects without extras stay
/// valid. This is not CaptureStatusBase extra keys. Top-level `failover_height`
/// is an optional u64. Top-level `failover_reason` is an optional kebab-case
/// enum. Top-level `durable_height` is an optional u64. Top-level
/// `capture_backlog_records` is a required u64. Top-level
/// `oldest_pending_capture_height` is an optional u64. Top-level
/// `disk_free_basis_points` is an optional u16. Top-level
/// `archive_manifest_id` is an optional non-empty string. Writer
/// `validate_status_text` trim/control/512 lives only in the capture writer
/// and is not copied here. Top-level `last_error_reason` is an optional
/// non-empty string. Top-level last-heartbeat `throughput_records_per_sec`
/// and `throughput_blocks_per_sec` are optional u64 integers; this stack's
/// capture writer does not serialize them. `HealthAssessment.reason_code`
/// stays a free string so unknown RED is not closed out.
const AUXILIARY_SOURCE_FIELDS: &[&str] = &[
    "source_id",
    "health",
    "qualification",
    "cursor_epoch",
    "tail_cursor_epoch",
    "durable_offset",
    "local_sequence",
    "spool_records",
    "unarchived_records",
    "unread_bytes",
    "partial_line",
    "last_durable_wall_micros",
    "quarantine_reason",
    "last_error_reason",
    "restart_reconstruction",
];

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
            // typed AMBER. ledger.unsupported_event is likewise a RED
            // consume-poison reason; AMBER plus that exact code is
            // inconsistent. Invented ledger.* siblings are not a family
            // prefix. Other AMBER reasons stay typed; this is not a catalog
            // of valid AMBER codes.
            if is_core_deadletter_family(reason_code)
                || is_ledger_unsupported_event_reason(reason_code)
            {
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
pub fn is_ledger_unsupported_event_reason(reason_code: &str) -> bool {
    LEDGER_UNSUPPORTED_EVENT_REASON_CODES.contains(&reason_code)
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
    require_enum(object, "active_committed_source", COMMITTED_SOURCE_CLASSES)?;
    require_enum(object, "primary_source_health", CAPTURE_SOURCE_HEALTH)?;
    if object.contains_key("independent_source_health") {
        require_enum(object, "independent_source_health", CAPTURE_SOURCE_HEALTH)?;
    }
    if object.contains_key("failover_height") {
        require_u64(object, "failover_height")?;
    }
    if object.contains_key("failover_reason") {
        require_enum(object, "failover_reason", FAILOVER_REASONS)?;
    }
    if object.contains_key("durable_height") {
        require_u64(object, "durable_height")?;
    }
    if object.contains_key("last_error_reason") {
        require_string(object, "last_error_reason", None)?;
    }
    require_auxiliary_source_closed_fields(object)?;
    require_non_negative_int(object, "pending_blocks")?;
    require_u64(object, "capture_backlog_records")?;
    if object.contains_key("oldest_pending_capture_height") {
        require_u64(object, "oldest_pending_capture_height")?;
    }
    if object.contains_key("disk_free_basis_points") {
        require_u16(object, "disk_free_basis_points")?;
    }
    if object.contains_key("archive_manifest_id") {
        require_string(object, "archive_manifest_id", None)?;
    }
    if object.contains_key("throughput_records_per_sec") {
        require_u64(object, "throughput_records_per_sec")?;
    }
    if object.contains_key("throughput_blocks_per_sec") {
        require_u64(object, "throughput_blocks_per_sec")?;
    }
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

fn require_auxiliary_source_closed_fields(
    object: &Map<String, Value>,
) -> Result<(), SnapshotError> {
    let sources = match object.get("auxiliary_sources") {
        None => return Ok(()),
        Some(Value::Array(sources)) => sources,
        Some(_) => return Err(SnapshotError::Invalid),
    };
    if sources.len() > MAX_AUXILIARY_SOURCES {
        return Err(SnapshotError::Invalid);
    }
    let mut previous: Option<&str> = None;
    for source in sources {
        let Value::Object(source) = source else {
            return Err(SnapshotError::Invalid);
        };
        if source
            .keys()
            .any(|key| !AUXILIARY_SOURCE_FIELDS.contains(&key.as_str()))
        {
            return Err(SnapshotError::Invalid);
        }
        require_string(source, "source_id", None)?;
        let Some(Value::String(source_id)) = source.get("source_id") else {
            return Err(SnapshotError::Invalid);
        };
        if previous.is_some_and(|value| value >= source_id.as_str()) {
            return Err(SnapshotError::Invalid);
        }
        previous = Some(source_id.as_str());
        require_u64(source, "spool_records")?;
        require_u64(source, "unarchived_records")?;
        require_bool(source, "partial_line")?;
        if source.contains_key("cursor_epoch") {
            require_string(source, "cursor_epoch", None)?;
        }
        if source.contains_key("tail_cursor_epoch") {
            require_string(source, "tail_cursor_epoch", None)?;
        }
        if source.contains_key("durable_offset") {
            require_u64(source, "durable_offset")?;
        }
        if source.contains_key("local_sequence") {
            require_u64(source, "local_sequence")?;
        }
        if source.contains_key("unread_bytes") {
            require_u64(source, "unread_bytes")?;
        }
        if source.contains_key("last_durable_wall_micros") {
            require_i64(source, "last_durable_wall_micros")?;
        }
        if source.contains_key("quarantine_reason") {
            require_string(source, "quarantine_reason", None)?;
        }
        if source.contains_key("last_error_reason") {
            require_string(source, "last_error_reason", None)?;
        }
        if source.contains_key("health") {
            require_enum(source, "health", AUXILIARY_SOURCE_HEALTH)?;
        }
        if source.contains_key("restart_reconstruction") {
            require_enum(source, "restart_reconstruction", RESTART_RECONSTRUCTION)?;
        }
        if source.contains_key("qualification") {
            require_enum(source, "qualification", AUXILIARY_SOURCE_QUALIFICATION)?;
        }
    }
    Ok(())
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

fn require_u64(object: &Map<String, Value>, field: &str) -> Result<(), SnapshotError> {
    match object.get(field) {
        Some(Value::Number(number)) if number.as_u64().is_some() => Ok(()),
        _ => Err(SnapshotError::Invalid),
    }
}

/// Present JSON integer that fits `u16`. Capture writer emits
/// `Option<u16>` (`skip_serializing_if`) for top-level
/// `disk_free_basis_points`. Writer `validate` rejects `> 10_000`; that
/// range lives only in the capture writer and is not copied here.
fn require_u16(object: &Map<String, Value>, field: &str) -> Result<(), SnapshotError> {
    match object.get(field) {
        Some(Value::Number(number)) => match number.as_u64() {
            Some(value) => match u16::try_from(value) {
                Ok(_) => Ok(()),
                Err(_) => Err(SnapshotError::Invalid),
            },
            None => Err(SnapshotError::Invalid),
        },
        Some(Value::Null)
        | Some(Value::Bool(_))
        | Some(Value::String(_))
        | Some(Value::Array(_))
        | Some(Value::Object(_))
        | None => Err(SnapshotError::Invalid),
    }
}

/// Present JSON integer that fits `i64`. Capture writer emits
/// `Option<i64>` (`skip_serializing_if`) with the durable cluster; this
/// is not `require_u64` and does not invent extra range bounds.
fn require_i64(object: &Map<String, Value>, field: &str) -> Result<(), SnapshotError> {
    match object.get(field) {
        Some(Value::Number(number)) if number.as_i64().is_some() => Ok(()),
        _ => Err(SnapshotError::Invalid),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AUXILIARY_SOURCE_HEALTH, AUXILIARY_SOURCE_QUALIFICATION, CAPTURE_SOURCE_HEALTH,
        CAPTURE_STATUS_SCHEMA_V4, CAPTURE_STATUS_SCHEMA_V5, COMMITTED_SOURCE_CLASSES,
        CORE_DEADLETTER_REASON_CODES, FAILOVER_REASONS, LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
        MAINTENANCE_FIELDS, MAX_AUXILIARY_SOURCES, RESTART_RECONSTRUCTION, SnapshotError,
        is_core_deadletter_family, is_core_deadletter_reason, is_ledger_unsupported_event_reason,
        parse_canonical_health_bytes, parse_capture_status_bytes,
    };
    use crate::openapi::{
        LAST_HEARTBEAT_THROUGHPUT_FIELDS, auxiliary_source_cursor_epoch_is_optional_string,
        auxiliary_source_durable_offset_is_optional_u64, auxiliary_source_health_openapi_enum,
        auxiliary_source_id_is_required_string,
        auxiliary_source_items_forbid_additional_properties,
        auxiliary_source_last_durable_wall_micros_is_optional_i64,
        auxiliary_source_last_error_reason_is_optional_string,
        auxiliary_source_local_sequence_is_optional_u64,
        auxiliary_source_partial_line_is_required_bool,
        auxiliary_source_qualification_openapi_enum,
        auxiliary_source_quarantine_reason_is_optional_string,
        auxiliary_source_spool_records_is_required_u64,
        auxiliary_source_tail_cursor_epoch_is_optional_string,
        auxiliary_source_unarchived_records_is_required_u64,
        auxiliary_source_unread_bytes_is_optional_u64, auxiliary_sources_max_items_is_writer_cap,
        capture_source_health_openapi_enum, capture_status_archive_manifest_id_is_optional_string,
        capture_status_capture_backlog_records_is_required_u64,
        capture_status_disk_free_basis_points_is_optional_u16,
        capture_status_durable_height_is_optional_u64,
        capture_status_failover_height_is_optional_u64,
        capture_status_failover_reason_is_optional_enum,
        capture_status_failover_reason_openapi_enum,
        capture_status_last_error_reason_is_optional_string,
        capture_status_oldest_pending_capture_height_is_optional_u64,
        capture_status_throughput_blocks_per_sec_is_optional_u64,
        capture_status_throughput_records_per_sec_is_optional_u64,
        committed_source_class_openapi_enum, core_deadletter_reason_openapi_enum,
        health_reason_code_is_unrestricted_string, independent_source_health_openapi_enum,
        ledger_unsupported_event_reason_openapi_enum, openapi_yaml,
        restart_reconstruction_openapi_enum,
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

    #[test]
    fn openapi_document_lists_ledger_unsupported_event_fail_closed_reason() {
        let document = openapi_yaml();
        assert!(document.contains("LedgerUnsupportedEventReasonCode"));
        let enum_values = ledger_unsupported_event_reason_openapi_enum(document)
            .expect("OpenAPI must define components.schemas.LedgerUnsupportedEventReasonCode.enum");
        assert_eq!(
            enum_values, LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        for reason_code in LEDGER_UNSUPPORTED_EVENT_REASON_CODES {
            assert!(
                is_ledger_unsupported_event_reason(reason_code),
                "helper must accept documented consume-poison reason {reason_code}"
            );
        }
        assert!(
            !is_ledger_unsupported_event_reason("healthy"),
            "healthy must not be classified as consume-poison fail-closed"
        );
        assert!(
            !is_ledger_unsupported_event_reason("ledger.invented"),
            "undocumented ledger.* siblings must not be treated as the frozen code"
        );
        assert!(
            !is_ledger_unsupported_event_reason("core.deadletter_corrupt"),
            "dead-letter codes must not be classified as ledger.unsupported_event"
        );
        assert!(document.contains("Unknown codes fail closed"));
        assert!(
            document.contains("Unknown HEALTH_STATE_RED codes stay 200 typed"),
            "OpenAPI must name unknown RED as 200 typed fail-closed"
        );
        assert!(
            document.contains("consume-poison"),
            "OpenAPI must name ledger.unsupported_event as consume-poison"
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
        assert!(!is_ledger_unsupported_event_reason(&assessment.reason_code));
    }

    #[test]
    fn red_ledger_unsupported_event_health_is_accepted_as_typed_fail_closed() {
        for reason_code in LEDGER_UNSUPPORTED_EVENT_REASON_CODES {
            let assessment =
                parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_RED", reason_code))
                    .unwrap_or_else(|error| panic!("{reason_code} should parse: {error}"));
            assert_eq!(assessment.state, WireHealthState::Red);
            assert_eq!(assessment.reason_code, *reason_code);
            assert!(is_ledger_unsupported_event_reason(&assessment.reason_code));
        }
    }

    #[test]
    fn green_or_amber_ledger_unsupported_event_health_is_rejected() {
        for reason_code in LEDGER_UNSUPPORTED_EVENT_REASON_CODES {
            assert_eq!(
                parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_GREEN", reason_code))
                    .expect_err("green consume-poison must not become ready"),
                SnapshotError::Invalid
            );
            assert_eq!(
                parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_AMBER", reason_code))
                    .expect_err("amber consume-poison must fail closed"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn unknown_ledger_sibling_red_stays_typed_and_green_is_invalid() {
        assert_eq!(
            parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_GREEN", "ledger.invented"))
                .expect_err("unknown green ledger reason must fail closed"),
            SnapshotError::Invalid
        );
        let assessment =
            parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_RED", "ledger.invented"))
                .expect("unknown red ledger sibling remains typed fail-closed, not ready");
        assert_eq!(assessment.state, WireHealthState::Red);
        assert_eq!(assessment.reason_code, "ledger.invented");
        assert!(!is_ledger_unsupported_event_reason(&assessment.reason_code));
        let amber =
            parse_canonical_health_bytes(&health_bytes("HEALTH_STATE_AMBER", "ledger.invented"))
                .expect("invented ledger.* is not a family prefix; AMBER stays typed");
        assert_eq!(amber.state, WireHealthState::Amber);
        assert_eq!(amber.reason_code, "ledger.invented");
        assert!(!is_ledger_unsupported_event_reason(&amber.reason_code));
    }

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../fixtures/api")
                .join(name),
        )
        .unwrap_or_else(|error| panic!("read fixture {name}: {error}"))
    }

    fn auxiliary_sources_with_distinct_ids(
        known: &serde_json::Value,
        count: usize,
    ) -> Vec<serde_json::Value> {
        (0..count)
            .map(|index| {
                let mut item = known.clone();
                item["source_id"] = serde_json::json!(format!("aux-source-{index:02}"));
                item
            })
            .collect()
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
    fn known_top_level_throughput_records_per_sec_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("throughput_records_per_sec").is_none(),
            "v4 fixture must omit optional top-level throughput_records_per_sec"
        );
        for throughput_records_per_sec in [0_u64, 3, u64::MAX] {
            value["throughput_records_per_sec"] = serde_json::json!(throughput_records_per_sec);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes).unwrap_or_else(|error| {
                panic!("{throughput_records_per_sec} should parse: {error}")
            });
            assert_eq!(
                parsed["throughput_records_per_sec"],
                throughput_records_per_sec
            );
            assert!(
                parsed.get("throughput_blocks_per_sec").is_none(),
                "typing throughput_records_per_sec must not couple it to throughput_blocks_per_sec"
            );
            assert!(parsed.get("fills").is_none());
            assert!(parsed.get("qualification").is_none());
            assert!(
                parsed.get("archive_manifest_id").is_none(),
                "typing throughput_records_per_sec must not couple it to archive_manifest_id"
            );
            assert!(
                parsed.get("disk_free_basis_points").is_none(),
                "typing throughput_records_per_sec must not couple it to disk_free_basis_points"
            );
            assert!(
                parsed.get("durable_height").is_none(),
                "typing throughput_records_per_sec must not couple it to durable_height"
            );
        }
    }

    #[test]
    fn omitted_top_level_throughput_records_per_sec_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("throughput_records_per_sec").is_none(),
            "v4 fixture must omit optional throughput_records_per_sec"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted throughput_records_per_sec");
        let parsed =
            parse_capture_status_bytes(&bytes).expect("omitted throughput_records_per_sec");
        assert!(parsed.get("throughput_records_per_sec").is_none());

        value["throughput_records_per_sec"] = serde_json::json!(3_u64);
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("throughput_records_per_sec");
        let bytes = serde_json::to_vec(&value).expect("encode removed throughput_records_per_sec");
        let parsed =
            parse_capture_status_bytes(&bytes).expect("removed throughput_records_per_sec");
        assert!(parsed.get("throughput_records_per_sec").is_none());
    }

    #[test]
    fn present_non_u64_top_level_throughput_records_per_sec_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for throughput_records_per_sec in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["throughput_records_per_sec"] = throughput_records_per_sec.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 throughput_records_per_sec must not fail open"),
                SnapshotError::Invalid,
                "{throughput_records_per_sec} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_throughput_blocks_per_sec_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("throughput_blocks_per_sec").is_none(),
            "v4 fixture must omit optional top-level throughput_blocks_per_sec"
        );
        for throughput_blocks_per_sec in [0_u64, 1, u64::MAX] {
            value["throughput_blocks_per_sec"] = serde_json::json!(throughput_blocks_per_sec);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes).unwrap_or_else(|error| {
                panic!("{throughput_blocks_per_sec} should parse: {error}")
            });
            assert_eq!(
                parsed["throughput_blocks_per_sec"],
                throughput_blocks_per_sec
            );
            assert!(
                parsed.get("throughput_records_per_sec").is_none(),
                "typing throughput_blocks_per_sec must not couple it to throughput_records_per_sec"
            );
            assert!(parsed.get("fills").is_none());
            assert!(parsed.get("qualification").is_none());
            assert!(
                parsed.get("archive_manifest_id").is_none(),
                "typing throughput_blocks_per_sec must not couple it to archive_manifest_id"
            );
            assert!(
                parsed.get("disk_free_basis_points").is_none(),
                "typing throughput_blocks_per_sec must not couple it to disk_free_basis_points"
            );
            assert!(
                parsed.get("durable_height").is_none(),
                "typing throughput_blocks_per_sec must not couple it to durable_height"
            );
        }
    }

    #[test]
    fn omitted_top_level_throughput_blocks_per_sec_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("throughput_blocks_per_sec").is_none(),
            "v4 fixture must omit optional throughput_blocks_per_sec"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted throughput_blocks_per_sec");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted throughput_blocks_per_sec");
        assert!(parsed.get("throughput_blocks_per_sec").is_none());

        value["throughput_blocks_per_sec"] = serde_json::json!(1_u64);
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("throughput_blocks_per_sec");
        let bytes = serde_json::to_vec(&value).expect("encode removed throughput_blocks_per_sec");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed throughput_blocks_per_sec");
        assert!(parsed.get("throughput_blocks_per_sec").is_none());
    }

    #[test]
    fn present_non_u64_top_level_throughput_blocks_per_sec_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for throughput_blocks_per_sec in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["throughput_blocks_per_sec"] = throughput_blocks_per_sec.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 throughput_blocks_per_sec must not fail open"),
                SnapshotError::Invalid,
                "{throughput_blocks_per_sec} must be snapshot_invalid"
            );
        }
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
        assert_eq!(value["auxiliary_sources"][0]["health"], "starting");
        assert_eq!(
            value["auxiliary_sources"][0]["qualification"],
            "unqualified"
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

    #[test]
    fn openapi_document_lists_committed_source_class_enum() {
        let document = openapi_yaml();
        let enum_values = committed_source_class_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.active_committed_source.enum");
        assert_eq!(
            enum_values, COMMITTED_SOURCE_CLASSES,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn closed_active_committed_source_values_are_accepted() {
        for source in COMMITTED_SOURCE_CLASSES {
            let mut value =
                serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                    .expect("v4 json");
            value["active_committed_source"] = serde_json::json!(source);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{source} should parse: {error}"));
            assert_eq!(parsed["active_committed_source"], *source);
        }
    }

    #[test]
    fn unknown_or_empty_active_committed_source_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for source in ["primary", "locally_verified_committed", ""] {
            value["active_committed_source"] = serde_json::json!(source);
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("unknown committed source must not be a free string"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn openapi_document_lists_capture_source_health_enum() {
        let document = openapi_yaml();
        let enum_values = capture_source_health_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.primary_source_health.enum");
        assert_eq!(
            enum_values, CAPTURE_SOURCE_HEALTH,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        let independent_values = independent_source_health_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.independent_source_health.enum");
        assert_eq!(
            independent_values, CAPTURE_SOURCE_HEALTH,
            "optional independent_source_health must freeze the same closed set"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn closed_capture_source_health_values_are_accepted() {
        for health in CAPTURE_SOURCE_HEALTH {
            let mut value =
                serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                    .expect("v4 json");
            value["primary_source_health"] = serde_json::json!(health);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{health} should parse: {error}"));
            assert_eq!(parsed["primary_source_health"], *health);
            assert!(parsed.get("independent_source_health").is_none());

            value["independent_source_health"] = serde_json::json!(health);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("independent {health} should parse: {error}"));
            assert_eq!(parsed["independent_source_health"], *health);
        }
    }

    #[test]
    fn unknown_or_empty_primary_source_health_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for health in ["degraded", "latched", "range_unavailable", ""] {
            value["primary_source_health"] = serde_json::json!(health);
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("unknown primary source health must not be a free string"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn unknown_independent_source_health_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for health in ["degraded", "latched", "range_unavailable", ""] {
            value["independent_source_health"] = serde_json::json!(health);
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("unknown independent source health must not be a free string"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn openapi_document_lists_auxiliary_source_health_enum() {
        let document = openapi_yaml();
        let enum_values = auxiliary_source_health_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.auxiliary_sources.items.health.enum");
        assert_eq!(
            enum_values, AUXILIARY_SOURCE_HEALTH,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        assert_ne!(
            AUXILIARY_SOURCE_HEALTH, CAPTURE_SOURCE_HEALTH,
            "auxiliary health must not reuse the committed source health set"
        );
        assert!(
            !AUXILIARY_SOURCE_HEALTH.contains(&"range-unavailable"),
            "committed range-unavailable is not auxiliary source health"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_lists_restart_reconstruction_enum() {
        let document = openapi_yaml();
        let enum_values = restart_reconstruction_openapi_enum(document).expect(
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.restart_reconstruction.enum",
        );
        assert_eq!(
            enum_values, RESTART_RECONSTRUCTION,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_lists_auxiliary_source_qualification_enum() {
        let document = openapi_yaml();
        let enum_values = auxiliary_source_qualification_openapi_enum(document).expect(
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.qualification.enum",
        );
        assert_eq!(
            enum_values, AUXILIARY_SOURCE_QUALIFICATION,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_requires_auxiliary_source_id_string() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_id_is_required_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.source_id as a required string"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_requires_auxiliary_spool_records_u64() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_spool_records_is_required_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.spool_records as a required u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_requires_auxiliary_unarchived_records_u64() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_unarchived_records_is_required_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.unarchived_records as a required u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_requires_auxiliary_partial_line_bool() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_partial_line_is_required_bool(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.partial_line as a required boolean"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_cursor_epoch_optional_string() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_cursor_epoch_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.cursor_epoch as an optional string"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_tail_cursor_epoch_optional_string() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_tail_cursor_epoch_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.tail_cursor_epoch as an optional string"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_durable_offset_optional_u64() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_durable_offset_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.durable_offset as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_local_sequence_optional_u64() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_local_sequence_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.local_sequence as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_unread_bytes_optional_u64() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_unread_bytes_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.unread_bytes as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_last_durable_wall_micros_optional_i64() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_last_durable_wall_micros_is_optional_i64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.last_durable_wall_micros as an optional i64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_quarantine_reason_optional_string() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_quarantine_reason_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.quarantine_reason as an optional string"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_auxiliary_last_error_reason_optional_string() {
        let document = openapi_yaml();
        assert!(
            auxiliary_source_last_error_reason_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.last_error_reason as an optional string"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_last_error_reason_optional_string() {
        let document = openapi_yaml();
        assert!(
            capture_status_last_error_reason_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.last_error_reason as an optional string"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_failover_height_optional_u64() {
        let document = openapi_yaml();
        assert!(
            capture_status_failover_height_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.failover_height as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_lists_failover_reason_enum() {
        let document = openapi_yaml();
        let enum_values = capture_status_failover_reason_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.failover_reason.enum");
        assert_eq!(
            enum_values, FAILOVER_REASONS,
            "YAML enum must match the frozen const; prose mentions do not count"
        );
        assert!(
            capture_status_failover_reason_is_optional_enum(document),
            "OpenAPI must define CaptureStatusBase.failover_reason as an optional kebab-case enum"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_durable_height_optional_u64() {
        let document = openapi_yaml();
        assert!(
            capture_status_durable_height_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.durable_height as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_requires_top_level_capture_backlog_records_u64() {
        let document = openapi_yaml();
        assert!(
            capture_status_capture_backlog_records_is_required_u64(document),
            "OpenAPI must define CaptureStatusBase.capture_backlog_records as a required u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_oldest_pending_capture_height_optional_u64() {
        let document = openapi_yaml();
        assert!(
            capture_status_oldest_pending_capture_height_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.oldest_pending_capture_height as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_disk_free_basis_points_optional_u16() {
        let document = openapi_yaml();
        assert!(
            capture_status_disk_free_basis_points_is_optional_u16(document),
            "OpenAPI must define CaptureStatusBase.disk_free_basis_points as an optional u16 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_archive_manifest_id_optional_string() {
        let document = openapi_yaml();
        assert!(
            capture_status_archive_manifest_id_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.archive_manifest_id as an optional string"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_throughput_records_per_sec_optional_u64() {
        let document = openapi_yaml();
        assert!(
            capture_status_throughput_records_per_sec_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.throughput_records_per_sec as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_types_top_level_throughput_blocks_per_sec_optional_u64() {
        let document = openapi_yaml();
        assert!(
            capture_status_throughput_blocks_per_sec_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.throughput_blocks_per_sec as an optional u64 integer"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn openapi_document_caps_auxiliary_sources_at_writer_max() {
        let document = openapi_yaml();
        assert_eq!(MAX_AUXILIARY_SOURCES, 16);
        assert!(
            auxiliary_sources_max_items_is_writer_cap(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.maxItems as the capture writer cap"
        );
        assert!(
            auxiliary_source_items_forbid_additional_properties(document),
            "OpenAPI must set CaptureStatusBase.auxiliary_sources.items.additionalProperties false"
        );
        assert!(
            document.contains("Duplicate present source_id"),
            "OpenAPI must describe source_id uniqueness without uniqueItems"
        );
        assert!(
            document.contains("strictly increasing"),
            "OpenAPI must describe source_id sort order without uniqueItems"
        );
        assert!(
            document.contains("Present unknown nested properties"),
            "OpenAPI must describe nested extra keys as snapshot_invalid"
        );
        assert!(
            document.contains("not CaptureStatusBase additionalProperties"),
            "OpenAPI must not close CaptureStatusBase extra keys in this leftover"
        );
        assert!(
            !document.contains("Sort order stays untyped"),
            "OpenAPI must not leave source_id sort order untyped"
        );
        assert!(
            health_reason_code_is_unrestricted_string(document),
            "reason_code must stay a free string so unknown RED codes fail closed"
        );
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn closed_auxiliary_source_health_values_are_accepted() {
        for health in AUXILIARY_SOURCE_HEALTH {
            let mut value =
                serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                    .expect("v5 json");
            value["auxiliary_sources"][0]["health"] = serde_json::json!(health);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{health} should parse: {error}"));
            assert_eq!(parsed["auxiliary_sources"][0]["health"], *health);
        }
    }

    #[test]
    fn omitted_auxiliary_source_health_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("health");
        let bytes = serde_json::to_vec(&value).expect("encode");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted auxiliary health");
        assert!(parsed["auxiliary_sources"][0].get("health").is_none());
    }

    #[test]
    fn unknown_or_empty_auxiliary_source_health_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for health in ["range-unavailable", "degraded", "Starting", ""] {
            value["auxiliary_sources"][0]["health"] = serde_json::json!(health);
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("unknown auxiliary source health must not be a free string"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn closed_restart_reconstruction_values_are_accepted() {
        for reconstruction in RESTART_RECONSTRUCTION {
            let mut value =
                serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                    .expect("v5 json");
            value["auxiliary_sources"][0]["restart_reconstruction"] =
                serde_json::json!(reconstruction);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{reconstruction} should parse: {error}"));
            assert_eq!(
                parsed["auxiliary_sources"][0]["restart_reconstruction"],
                *reconstruction
            );
        }
    }

    #[test]
    fn omitted_restart_reconstruction_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("restart_reconstruction");
        let bytes = serde_json::to_vec(&value).expect("encode");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted reconstruction");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("restart_reconstruction")
                .is_none()
        );
    }

    #[test]
    fn unknown_or_empty_restart_reconstruction_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for reconstruction in ["NotRequired", "not_required", "Complete", ""] {
            value["auxiliary_sources"][0]["restart_reconstruction"] =
                serde_json::json!(reconstruction);
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("unknown restart reconstruction must not be a free string"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn closed_auxiliary_source_qualification_values_are_accepted() {
        for qualification in AUXILIARY_SOURCE_QUALIFICATION {
            let mut value =
                serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                    .expect("v5 json");
            value["auxiliary_sources"][0]["qualification"] = serde_json::json!(qualification);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{qualification} should parse: {error}"));
            assert_eq!(
                parsed["auxiliary_sources"][0]["qualification"],
                *qualification
            );
        }
    }

    #[test]
    fn omitted_auxiliary_source_qualification_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("qualification");
        let bytes = serde_json::to_vec(&value).expect("encode");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted qualification");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("qualification")
                .is_none()
        );
    }

    #[test]
    fn unknown_or_empty_auxiliary_source_qualification_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for qualification in ["Unqualified", "Qualified", "un_qualified", ""] {
            value["auxiliary_sources"][0]["qualification"] = serde_json::json!(qualification);
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("unknown auxiliary source qualification must not be a free string"),
                SnapshotError::Invalid
            );
        }
    }

    #[test]
    fn omitted_auxiliary_sources_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value
            .as_object_mut()
            .expect("status object")
            .remove("auxiliary_sources");
        let bytes = serde_json::to_vec(&value).expect("encode");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted auxiliary_sources");
        assert!(parsed.get("auxiliary_sources").is_none());
    }

    #[test]
    fn object_auxiliary_source_items_are_accepted() {
        let value = parse_capture_status_bytes(&fixture("capture-status-v5.json")).expect("v5");
        assert!(value["auxiliary_sources"].is_array());
        assert!(value["auxiliary_sources"][0].is_object());
        assert_eq!(
            value["auxiliary_sources"][0]["source_id"],
            "node-misc-events"
        );
        assert_eq!(value["auxiliary_sources"][0]["health"], "starting");
        assert_eq!(value["auxiliary_sources"][0]["spool_records"], 0);
        assert_eq!(value["auxiliary_sources"][0]["unarchived_records"], 0);
        assert_eq!(value["auxiliary_sources"][0]["partial_line"], false);
    }

    #[test]
    fn empty_auxiliary_sources_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"] = serde_json::json!([]);
        let bytes = serde_json::to_vec(&value).expect("encode empty auxiliary_sources");
        let parsed = parse_capture_status_bytes(&bytes).expect("empty auxiliary_sources");
        assert_eq!(parsed["auxiliary_sources"], serde_json::json!([]));
    }

    #[test]
    fn auxiliary_sources_at_writer_cap_are_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        let known = value["auxiliary_sources"][0].clone();
        value["auxiliary_sources"] = serde_json::Value::Array(auxiliary_sources_with_distinct_ids(
            &known,
            MAX_AUXILIARY_SOURCES,
        ));
        let bytes = serde_json::to_vec(&value).expect("encode cap auxiliary_sources");
        let parsed = parse_capture_status_bytes(&bytes).expect("writer-cap auxiliary_sources");
        assert_eq!(
            parsed["auxiliary_sources"]
                .as_array()
                .expect("auxiliary_sources array")
                .len(),
            MAX_AUXILIARY_SOURCES
        );
    }

    #[test]
    fn auxiliary_sources_above_writer_cap_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        let known = value["auxiliary_sources"][0].clone();
        value["auxiliary_sources"] = serde_json::Value::Array(auxiliary_sources_with_distinct_ids(
            &known,
            MAX_AUXILIARY_SOURCES + 1,
        ));
        let bytes = serde_json::to_vec(&value).expect("encode over-cap auxiliary_sources");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("over-cap auxiliary_sources must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn distinct_auxiliary_source_ids_are_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        let known = value["auxiliary_sources"][0].clone();
        let mut other = known.clone();
        other["source_id"] = serde_json::json!("node-fills");
        value["auxiliary_sources"] = serde_json::json!([other, known]);
        let bytes = serde_json::to_vec(&value).expect("encode sorted distinct source_id");
        let parsed = parse_capture_status_bytes(&bytes).expect("sorted distinct source_id");
        assert_eq!(parsed["auxiliary_sources"][0]["source_id"], "node-fills");
        assert_eq!(
            parsed["auxiliary_sources"][1]["source_id"],
            "node-misc-events"
        );
    }

    #[test]
    fn descending_auxiliary_source_ids_are_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        let known = value["auxiliary_sources"][0].clone();
        let mut other = known.clone();
        other["source_id"] = serde_json::json!("node-fills");
        value["auxiliary_sources"] = serde_json::json!([known, other]);
        let bytes = serde_json::to_vec(&value).expect("encode descending source_id");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("descending distinct source_id must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn duplicate_auxiliary_source_id_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        let known = value["auxiliary_sources"][0].clone();
        value["auxiliary_sources"] = serde_json::json!([known.clone(), known]);
        let bytes = serde_json::to_vec(&value).expect("encode duplicate source_id");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("duplicate present source_id must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn known_auxiliary_source_object_without_extras_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        let parsed = parse_capture_status_bytes(&fixture("capture-status-v5.json")).expect("v5");
        assert_eq!(
            parsed["auxiliary_sources"][0]["restart_reconstruction"],
            "complete"
        );

        value["auxiliary_sources"][0]["cursor_epoch"] = serde_json::json!("node-file-v1:epoch");
        value["auxiliary_sources"][0]["tail_cursor_epoch"] =
            serde_json::json!("node-file-v1:epoch");
        value["auxiliary_sources"][0]["durable_offset"] = serde_json::json!(0_u64);
        value["auxiliary_sources"][0]["local_sequence"] = serde_json::json!(0_u64);
        value["auxiliary_sources"][0]["unread_bytes"] = serde_json::json!(0_u64);
        value["auxiliary_sources"][0]["last_durable_wall_micros"] = serde_json::json!(1_i64);
        value["auxiliary_sources"][0]["quarantine_reason"] =
            serde_json::json!("source.schema_drift");
        value["auxiliary_sources"][0]["last_error_reason"] = serde_json::json!("source.timeout");
        let bytes = serde_json::to_vec(&value).expect("encode known nested keys");
        let parsed = parse_capture_status_bytes(&bytes).expect("known nested keys");
        assert_eq!(
            parsed["auxiliary_sources"][0]["last_error_reason"],
            "source.timeout"
        );
        assert_eq!(
            parsed["auxiliary_sources"][0]["restart_reconstruction"],
            "complete"
        );
    }

    #[test]
    fn present_unknown_auxiliary_source_property_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for extra in ["fills", "invented", "adapter"] {
            value["auxiliary_sources"][0][extra] = serde_json::json!(true);
            let bytes = serde_json::to_vec(&value).expect("encode extra nested key");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present unknown nested property must not fail open"),
                SnapshotError::Invalid,
                "{extra} must be snapshot_invalid"
            );
            value["auxiliary_sources"][0]
                .as_object_mut()
                .expect("auxiliary source object")
                .remove(extra);
        }
    }

    #[test]
    fn known_auxiliary_spool_records_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for spool_records in [0_u64, 3, u64::MAX] {
            value["auxiliary_sources"][0]["spool_records"] = serde_json::json!(spool_records);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{spool_records} should parse: {error}"));
            assert_eq!(
                parsed["auxiliary_sources"][0]["spool_records"],
                spool_records
            );
        }
    }

    #[test]
    fn present_non_u64_auxiliary_spool_records_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for spool_records in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["auxiliary_sources"][0]["spool_records"] = spool_records.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 spool_records must not fail open"),
                SnapshotError::Invalid,
                "{spool_records} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn omitted_auxiliary_spool_records_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("spool_records");
        let bytes = serde_json::to_vec(&value).expect("encode omitted spool_records");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("omitted nested spool_records must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn known_auxiliary_unarchived_records_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for unarchived_records in [0_u64, 3, u64::MAX] {
            value["auxiliary_sources"][0]["unarchived_records"] =
                serde_json::json!(unarchived_records);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{unarchived_records} should parse: {error}"));
            assert_eq!(
                parsed["auxiliary_sources"][0]["unarchived_records"],
                unarchived_records
            );
        }
    }

    #[test]
    fn present_non_u64_auxiliary_unarchived_records_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for unarchived_records in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["auxiliary_sources"][0]["unarchived_records"] = unarchived_records.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 unarchived_records must not fail open"),
                SnapshotError::Invalid,
                "{unarchived_records} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn omitted_auxiliary_unarchived_records_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("unarchived_records");
        let bytes = serde_json::to_vec(&value).expect("encode omitted unarchived_records");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("omitted nested unarchived_records must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn known_auxiliary_partial_line_bool_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for partial_line in [false, true] {
            value["auxiliary_sources"][0]["partial_line"] = serde_json::json!(partial_line);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{partial_line} should parse: {error}"));
            assert_eq!(parsed["auxiliary_sources"][0]["partial_line"], partial_line);
        }
    }

    #[test]
    fn present_non_bool_auxiliary_partial_line_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for partial_line in [
            serde_json::json!("true"),
            serde_json::json!("false"),
            serde_json::json!(0),
            serde_json::json!(1),
            serde_json::json!(null),
            serde_json::json!({"not": "a-bool"}),
            serde_json::json!(["not-a-bool"]),
        ] {
            value["auxiliary_sources"][0]["partial_line"] = partial_line.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-bool partial_line must not fail open"),
                SnapshotError::Invalid,
                "{partial_line} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn omitted_auxiliary_partial_line_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("partial_line");
        let bytes = serde_json::to_vec(&value).expect("encode omitted partial_line");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("omitted nested partial_line must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn known_auxiliary_cursor_epoch_string_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]["cursor_epoch"] = serde_json::json!("node-file-v1:epoch");
        let bytes = serde_json::to_vec(&value).expect("encode known cursor_epoch");
        let parsed = parse_capture_status_bytes(&bytes).expect("known string cursor_epoch");
        assert_eq!(
            parsed["auxiliary_sources"][0]["cursor_epoch"],
            "node-file-v1:epoch"
        );
    }

    #[test]
    fn omitted_auxiliary_cursor_epoch_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0].get("cursor_epoch").is_none(),
            "v5 fixture must omit optional cursor_epoch"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted cursor_epoch");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted cursor_epoch");
        assert!(parsed["auxiliary_sources"][0].get("cursor_epoch").is_none());

        value["auxiliary_sources"][0]["cursor_epoch"] = serde_json::json!("node-file-v1:epoch");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("cursor_epoch");
        let bytes = serde_json::to_vec(&value).expect("encode removed cursor_epoch");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed cursor_epoch");
        assert!(parsed["auxiliary_sources"][0].get("cursor_epoch").is_none());
    }

    #[test]
    fn present_non_string_auxiliary_cursor_epoch_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for cursor_epoch in [
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
            serde_json::json!(""),
        ] {
            value["auxiliary_sources"][0]["cursor_epoch"] = cursor_epoch.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-string or empty cursor_epoch must not fail open"),
                SnapshotError::Invalid,
                "{cursor_epoch} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_auxiliary_tail_cursor_epoch_string_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]["tail_cursor_epoch"] =
            serde_json::json!("node-file-v1:epoch");
        let bytes = serde_json::to_vec(&value).expect("encode known tail_cursor_epoch");
        let parsed = parse_capture_status_bytes(&bytes).expect("known string tail_cursor_epoch");
        assert_eq!(
            parsed["auxiliary_sources"][0]["tail_cursor_epoch"],
            "node-file-v1:epoch"
        );
    }

    #[test]
    fn omitted_auxiliary_tail_cursor_epoch_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0]
                .get("tail_cursor_epoch")
                .is_none(),
            "v5 fixture must omit optional tail_cursor_epoch"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted tail_cursor_epoch");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted tail_cursor_epoch");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("tail_cursor_epoch")
                .is_none()
        );

        value["auxiliary_sources"][0]["tail_cursor_epoch"] =
            serde_json::json!("node-file-v1:epoch");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("tail_cursor_epoch");
        let bytes = serde_json::to_vec(&value).expect("encode removed tail_cursor_epoch");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed tail_cursor_epoch");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("tail_cursor_epoch")
                .is_none()
        );
    }

    #[test]
    fn present_non_string_auxiliary_tail_cursor_epoch_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for tail_cursor_epoch in [
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
            serde_json::json!(""),
        ] {
            value["auxiliary_sources"][0]["tail_cursor_epoch"] = tail_cursor_epoch.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-string or empty tail_cursor_epoch must not fail open"),
                SnapshotError::Invalid,
                "{tail_cursor_epoch} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_auxiliary_durable_offset_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for durable_offset in [0_u64, 47, u64::MAX] {
            value["auxiliary_sources"][0]["durable_offset"] = serde_json::json!(durable_offset);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{durable_offset} should parse: {error}"));
            assert_eq!(
                parsed["auxiliary_sources"][0]["durable_offset"],
                durable_offset
            );
        }
    }

    #[test]
    fn omitted_auxiliary_durable_offset_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0]
                .get("durable_offset")
                .is_none(),
            "v5 fixture must omit optional durable_offset"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted durable_offset");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted durable_offset");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("durable_offset")
                .is_none()
        );

        value["auxiliary_sources"][0]["durable_offset"] = serde_json::json!(47_u64);
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("durable_offset");
        let bytes = serde_json::to_vec(&value).expect("encode removed durable_offset");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed durable_offset");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("durable_offset")
                .is_none()
        );
    }

    #[test]
    fn present_non_u64_auxiliary_durable_offset_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for durable_offset in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["auxiliary_sources"][0]["durable_offset"] = durable_offset.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 durable_offset must not fail open"),
                SnapshotError::Invalid,
                "{durable_offset} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_auxiliary_local_sequence_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for local_sequence in [0_u64, 47, u64::MAX] {
            value["auxiliary_sources"][0]["local_sequence"] = serde_json::json!(local_sequence);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{local_sequence} should parse: {error}"));
            assert_eq!(
                parsed["auxiliary_sources"][0]["local_sequence"],
                local_sequence
            );
        }
    }

    #[test]
    fn omitted_auxiliary_local_sequence_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0]
                .get("local_sequence")
                .is_none(),
            "v5 fixture must omit optional local_sequence"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted local_sequence");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted local_sequence");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("local_sequence")
                .is_none()
        );

        value["auxiliary_sources"][0]["local_sequence"] = serde_json::json!(47_u64);
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("local_sequence");
        let bytes = serde_json::to_vec(&value).expect("encode removed local_sequence");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed local_sequence");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("local_sequence")
                .is_none()
        );
    }

    #[test]
    fn present_non_u64_auxiliary_local_sequence_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for local_sequence in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["auxiliary_sources"][0]["local_sequence"] = local_sequence.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 local_sequence must not fail open"),
                SnapshotError::Invalid,
                "{local_sequence} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_auxiliary_unread_bytes_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for unread_bytes in [0_u64, 47, u64::MAX] {
            value["auxiliary_sources"][0]["unread_bytes"] = serde_json::json!(unread_bytes);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{unread_bytes} should parse: {error}"));
            assert_eq!(parsed["auxiliary_sources"][0]["unread_bytes"], unread_bytes);
        }
    }

    #[test]
    fn omitted_auxiliary_unread_bytes_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0].get("unread_bytes").is_none(),
            "v5 fixture must omit optional unread_bytes"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted unread_bytes");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted unread_bytes");
        assert!(parsed["auxiliary_sources"][0].get("unread_bytes").is_none());

        value["auxiliary_sources"][0]["unread_bytes"] = serde_json::json!(47_u64);
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("unread_bytes");
        let bytes = serde_json::to_vec(&value).expect("encode removed unread_bytes");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed unread_bytes");
        assert!(parsed["auxiliary_sources"][0].get("unread_bytes").is_none());
    }

    #[test]
    fn present_non_u64_auxiliary_unread_bytes_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for unread_bytes in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["auxiliary_sources"][0]["unread_bytes"] = unread_bytes.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 unread_bytes must not fail open"),
                SnapshotError::Invalid,
                "{unread_bytes} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_auxiliary_last_durable_wall_micros_i64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for last_durable_wall_micros in [0_i64, 1_000, i64::MAX, -1] {
            value["auxiliary_sources"][0]["last_durable_wall_micros"] =
                serde_json::json!(last_durable_wall_micros);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{last_durable_wall_micros} should parse: {error}"));
            assert_eq!(
                parsed["auxiliary_sources"][0]["last_durable_wall_micros"],
                last_durable_wall_micros
            );
        }
    }

    #[test]
    fn omitted_auxiliary_last_durable_wall_micros_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0]
                .get("last_durable_wall_micros")
                .is_none(),
            "v5 fixture must omit optional last_durable_wall_micros"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted last_durable_wall_micros");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted last_durable_wall_micros");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("last_durable_wall_micros")
                .is_none()
        );

        value["auxiliary_sources"][0]["last_durable_wall_micros"] = serde_json::json!(1_000_i64);
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("last_durable_wall_micros");
        let bytes = serde_json::to_vec(&value).expect("encode removed last_durable_wall_micros");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed last_durable_wall_micros");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("last_durable_wall_micros")
                .is_none()
        );
    }

    #[test]
    fn present_non_i64_auxiliary_last_durable_wall_micros_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for last_durable_wall_micros in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "an-i64"}),
            serde_json::json!(["not-an-i64"]),
            serde_json::json!(u64::MAX),
            serde_json::json!(1.5),
        ] {
            value["auxiliary_sources"][0]["last_durable_wall_micros"] =
                last_durable_wall_micros.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-i64 last_durable_wall_micros must not fail open"),
                SnapshotError::Invalid,
                "{last_durable_wall_micros} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_auxiliary_quarantine_reason_string_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]["quarantine_reason"] =
            serde_json::json!("source.schema_drift");
        let bytes = serde_json::to_vec(&value).expect("encode known quarantine_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("known string quarantine_reason");
        assert_eq!(
            parsed["auxiliary_sources"][0]["quarantine_reason"],
            "source.schema_drift"
        );
    }

    #[test]
    fn omitted_auxiliary_quarantine_reason_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0]
                .get("quarantine_reason")
                .is_none(),
            "v5 fixture must omit optional quarantine_reason"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted quarantine_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted quarantine_reason");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("quarantine_reason")
                .is_none()
        );

        value["auxiliary_sources"][0]["quarantine_reason"] =
            serde_json::json!("source.schema_drift");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("quarantine_reason");
        let bytes = serde_json::to_vec(&value).expect("encode removed quarantine_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed quarantine_reason");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("quarantine_reason")
                .is_none()
        );
    }

    #[test]
    fn present_non_string_auxiliary_quarantine_reason_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for quarantine_reason in [
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
            serde_json::json!(""),
        ] {
            value["auxiliary_sources"][0]["quarantine_reason"] = quarantine_reason.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-string or empty quarantine_reason must not fail open"),
                SnapshotError::Invalid,
                "{quarantine_reason} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_auxiliary_last_error_reason_string_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]["last_error_reason"] =
            serde_json::json!("source.temporary_disconnect");
        let bytes = serde_json::to_vec(&value).expect("encode known last_error_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("known string last_error_reason");
        assert_eq!(
            parsed["auxiliary_sources"][0]["last_error_reason"],
            "source.temporary_disconnect"
        );
    }

    #[test]
    fn omitted_auxiliary_last_error_reason_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        assert!(
            value["auxiliary_sources"][0]
                .get("last_error_reason")
                .is_none(),
            "v5 fixture must omit optional last_error_reason"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted last_error_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted last_error_reason");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("last_error_reason")
                .is_none()
        );

        value["auxiliary_sources"][0]["last_error_reason"] =
            serde_json::json!("source.temporary_disconnect");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("last_error_reason");
        let bytes = serde_json::to_vec(&value).expect("encode removed last_error_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed last_error_reason");
        assert!(
            parsed["auxiliary_sources"][0]
                .get("last_error_reason")
                .is_none()
        );
    }

    #[test]
    fn present_non_string_auxiliary_last_error_reason_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for last_error_reason in [
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
            serde_json::json!(""),
        ] {
            value["auxiliary_sources"][0]["last_error_reason"] = last_error_reason.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-string or empty last_error_reason must not fail open"),
                SnapshotError::Invalid,
                "{last_error_reason} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_last_error_reason_string_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("last_error_reason").is_none(),
            "v4 fixture must omit optional top-level last_error_reason"
        );
        value["last_error_reason"] = serde_json::json!("capture_bus.unavailable");
        let bytes = serde_json::to_vec(&value).expect("encode known last_error_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("known string last_error_reason");
        assert_eq!(parsed["last_error_reason"], "capture_bus.unavailable");
        assert!(parsed.get("auxiliary_sources").is_none());
    }

    #[test]
    fn omitted_top_level_last_error_reason_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("last_error_reason").is_none(),
            "v4 fixture must omit optional last_error_reason"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted last_error_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted last_error_reason");
        assert!(parsed.get("last_error_reason").is_none());

        value["last_error_reason"] = serde_json::json!("capture_bus.unavailable");
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("last_error_reason");
        let bytes = serde_json::to_vec(&value).expect("encode removed last_error_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed last_error_reason");
        assert!(parsed.get("last_error_reason").is_none());
    }

    #[test]
    fn present_non_string_top_level_last_error_reason_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for last_error_reason in [
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
            serde_json::json!(""),
        ] {
            value["last_error_reason"] = last_error_reason.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-string or empty last_error_reason must not fail open"),
                SnapshotError::Invalid,
                "{last_error_reason} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_failover_height_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("failover_height").is_none(),
            "v4 fixture must omit optional top-level failover_height"
        );
        for failover_height in [0_u64, 47, u64::MAX] {
            value["failover_height"] = serde_json::json!(failover_height);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{failover_height} should parse: {error}"));
            assert_eq!(parsed["failover_height"], failover_height);
            assert!(parsed.get("auxiliary_sources").is_none());
        }
    }

    #[test]
    fn omitted_top_level_failover_height_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("failover_height").is_none(),
            "v4 fixture must omit optional failover_height"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted failover_height");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted failover_height");
        assert!(parsed.get("failover_height").is_none());

        value["failover_height"] = serde_json::json!(47_u64);
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("failover_height");
        let bytes = serde_json::to_vec(&value).expect("encode removed failover_height");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed failover_height");
        assert!(parsed.get("failover_height").is_none());
    }

    #[test]
    fn present_non_u64_top_level_failover_height_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for failover_height in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["failover_height"] = failover_height.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 failover_height must not fail open"),
                SnapshotError::Invalid,
                "{failover_height} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_failover_reason_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("failover_reason").is_none(),
            "v4 fixture must omit optional top-level failover_reason"
        );
        for reason in FAILOVER_REASONS {
            value["failover_reason"] = serde_json::json!(reason);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{reason} should parse: {error}"));
            assert_eq!(parsed["failover_reason"], *reason);
            assert!(parsed.get("auxiliary_sources").is_none());
            assert!(
                parsed.get("failover_height").is_none(),
                "typing failover_reason must not couple it to failover_height"
            );
        }
    }

    #[test]
    fn omitted_top_level_failover_reason_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("failover_reason").is_none(),
            "v4 fixture must omit optional failover_reason"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted failover_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted failover_reason");
        assert!(parsed.get("failover_reason").is_none());

        value["failover_reason"] = serde_json::json!("primary-range-unavailable");
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("failover_reason");
        let bytes = serde_json::to_vec(&value).expect("encode removed failover_reason");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed failover_reason");
        assert!(parsed.get("failover_reason").is_none());
    }

    #[test]
    fn present_unknown_or_non_string_top_level_failover_reason_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for failover_reason in [
            serde_json::json!("primary_range_unavailable"),
            serde_json::json!("PrimaryRangeUnavailable"),
            serde_json::json!("range-unavailable"),
            serde_json::json!(""),
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
        ] {
            value["failover_reason"] = failover_reason.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes).expect_err(
                    "present unknown, empty, or non-string failover_reason must not fail open"
                ),
                SnapshotError::Invalid,
                "{failover_reason} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_durable_height_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("durable_height").is_none(),
            "v4 fixture must omit optional top-level durable_height"
        );
        for durable_height in [0_u64, 47, u64::MAX] {
            value["durable_height"] = serde_json::json!(durable_height);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{durable_height} should parse: {error}"));
            assert_eq!(parsed["durable_height"], durable_height);
            assert!(parsed.get("auxiliary_sources").is_none());
            assert!(
                parsed.get("failover_height").is_none(),
                "typing durable_height must not couple it to failover_height"
            );
            assert!(
                parsed.get("failover_reason").is_none(),
                "typing durable_height must not couple it to failover_reason"
            );
        }
    }

    #[test]
    fn omitted_top_level_durable_height_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("durable_height").is_none(),
            "v4 fixture must omit optional durable_height"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted durable_height");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted durable_height");
        assert!(parsed.get("durable_height").is_none());

        value["durable_height"] = serde_json::json!(47_u64);
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("durable_height");
        let bytes = serde_json::to_vec(&value).expect("encode removed durable_height");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed durable_height");
        assert!(parsed.get("durable_height").is_none());
    }

    #[test]
    fn present_non_u64_top_level_durable_height_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for durable_height in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["durable_height"] = durable_height.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 durable_height must not fail open"),
                SnapshotError::Invalid,
                "{durable_height} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_capture_backlog_records_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert_eq!(
            value["capture_backlog_records"], 0,
            "v4 fixture must include required capture_backlog_records"
        );
        for capture_backlog_records in [0_u64, 12, u64::MAX] {
            value["capture_backlog_records"] = serde_json::json!(capture_backlog_records);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{capture_backlog_records} should parse: {error}"));
            assert_eq!(parsed["capture_backlog_records"], capture_backlog_records);
            assert!(parsed.get("auxiliary_sources").is_none());
            assert!(
                parsed.get("durable_height").is_none(),
                "typing capture_backlog_records must not couple it to durable_height"
            );
            assert!(
                parsed.get("failover_height").is_none(),
                "typing capture_backlog_records must not couple it to failover_height"
            );
            assert!(
                parsed.get("failover_reason").is_none(),
                "typing capture_backlog_records must not couple it to failover_reason"
            );
            assert!(
                parsed.get("oldest_pending_capture_height").is_none(),
                "typing capture_backlog_records must not couple it to oldest_pending_capture_height"
            );
        }
    }

    #[test]
    fn omitted_top_level_capture_backlog_records_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("capture_backlog_records");
        let bytes = serde_json::to_vec(&value).expect("encode omitted capture_backlog_records");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("omitted top-level capture_backlog_records must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn present_non_u64_top_level_capture_backlog_records_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for capture_backlog_records in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["capture_backlog_records"] = capture_backlog_records.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 capture_backlog_records must not fail open"),
                SnapshotError::Invalid,
                "{capture_backlog_records} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_oldest_pending_capture_height_u64_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("oldest_pending_capture_height").is_none(),
            "v4 fixture must omit optional top-level oldest_pending_capture_height"
        );
        for oldest_pending_capture_height in [0_u64, 47, u64::MAX] {
            value["oldest_pending_capture_height"] =
                serde_json::json!(oldest_pending_capture_height);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes).unwrap_or_else(|error| {
                panic!("{oldest_pending_capture_height} should parse: {error}")
            });
            assert_eq!(
                parsed["oldest_pending_capture_height"],
                oldest_pending_capture_height
            );
            assert_eq!(
                parsed["capture_backlog_records"], 0,
                "typing oldest_pending_capture_height must not couple it to capture_backlog_records"
            );
            assert!(
                parsed.get("durable_height").is_none(),
                "typing oldest_pending_capture_height must not couple it to durable_height"
            );
            assert!(
                parsed.get("failover_height").is_none(),
                "typing oldest_pending_capture_height must not couple it to failover_height"
            );
            assert!(
                parsed.get("failover_reason").is_none(),
                "typing oldest_pending_capture_height must not couple it to failover_reason"
            );
        }
    }

    #[test]
    fn omitted_top_level_oldest_pending_capture_height_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("oldest_pending_capture_height").is_none(),
            "v4 fixture must omit optional oldest_pending_capture_height"
        );
        let bytes =
            serde_json::to_vec(&value).expect("encode omitted oldest_pending_capture_height");
        let parsed =
            parse_capture_status_bytes(&bytes).expect("omitted oldest_pending_capture_height");
        assert!(parsed.get("oldest_pending_capture_height").is_none());

        value["oldest_pending_capture_height"] = serde_json::json!(47_u64);
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("oldest_pending_capture_height");
        let bytes =
            serde_json::to_vec(&value).expect("encode removed oldest_pending_capture_height");
        let parsed =
            parse_capture_status_bytes(&bytes).expect("removed oldest_pending_capture_height");
        assert!(parsed.get("oldest_pending_capture_height").is_none());
    }

    #[test]
    fn present_non_u64_top_level_oldest_pending_capture_height_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for oldest_pending_capture_height in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u64"}),
            serde_json::json!(["not-a-u64"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
        ] {
            value["oldest_pending_capture_height"] = oldest_pending_capture_height.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u64 oldest_pending_capture_height must not fail open"),
                SnapshotError::Invalid,
                "{oldest_pending_capture_height} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_disk_free_basis_points_u16_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("disk_free_basis_points").is_none(),
            "v4 fixture must omit optional top-level disk_free_basis_points"
        );
        for disk_free_basis_points in [0_u16, 47, 10_000, 10_001, u16::MAX] {
            value["disk_free_basis_points"] = serde_json::json!(disk_free_basis_points);
            let bytes = serde_json::to_vec(&value).expect("encode");
            let parsed = parse_capture_status_bytes(&bytes)
                .unwrap_or_else(|error| panic!("{disk_free_basis_points} should parse: {error}"));
            assert_eq!(
                parsed["disk_free_basis_points"],
                u64::from(disk_free_basis_points)
            );
            assert_eq!(
                parsed["capture_backlog_records"], 0,
                "typing disk_free_basis_points must not couple it to capture_backlog_records"
            );
            assert!(
                parsed.get("oldest_pending_capture_height").is_none(),
                "typing disk_free_basis_points must not couple it to oldest_pending_capture_height"
            );
            assert!(
                parsed.get("archive_manifest_id").is_none(),
                "typing disk_free_basis_points must not fold in archive_manifest_id"
            );
            assert!(
                parsed.get("durable_height").is_none(),
                "typing disk_free_basis_points must not couple it to durable_height"
            );
            assert!(
                parsed.get("failover_height").is_none(),
                "typing disk_free_basis_points must not couple it to failover_height"
            );
            assert!(
                parsed.get("failover_reason").is_none(),
                "typing disk_free_basis_points must not couple it to failover_reason"
            );
        }
    }

    #[test]
    fn writer_disk_free_basis_points_range_is_not_copied_onto_api_parse() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        value["disk_free_basis_points"] = serde_json::json!(10_001_u16);
        let bytes = serde_json::to_vec(&value).expect("encode writer-range 10001");
        let parsed = parse_capture_status_bytes(&bytes)
            .expect("10001 must stay 200 at API parse; writer 10000 max is not copied");
        assert_eq!(parsed["disk_free_basis_points"], 10_001_u64);
    }

    #[test]
    fn omitted_top_level_disk_free_basis_points_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("disk_free_basis_points").is_none(),
            "v4 fixture must omit optional disk_free_basis_points"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted disk_free_basis_points");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted disk_free_basis_points");
        assert!(parsed.get("disk_free_basis_points").is_none());

        value["disk_free_basis_points"] = serde_json::json!(47_u16);
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("disk_free_basis_points");
        let bytes = serde_json::to_vec(&value).expect("encode removed disk_free_basis_points");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed disk_free_basis_points");
        assert!(parsed.get("disk_free_basis_points").is_none());
    }

    #[test]
    fn present_non_u16_top_level_disk_free_basis_points_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for disk_free_basis_points in [
            serde_json::json!("0"),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-u16"}),
            serde_json::json!(["not-a-u16"]),
            serde_json::json!(-1),
            serde_json::json!(1.5),
            serde_json::json!(65_536_u32),
        ] {
            value["disk_free_basis_points"] = disk_free_basis_points.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-u16 disk_free_basis_points must not fail open"),
                SnapshotError::Invalid,
                "{disk_free_basis_points} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn known_top_level_archive_manifest_id_string_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("archive_manifest_id").is_none(),
            "v4 fixture must omit optional top-level archive_manifest_id"
        );
        value["archive_manifest_id"] = serde_json::json!("manifest-42");
        let bytes = serde_json::to_vec(&value).expect("encode known archive_manifest_id");
        let parsed = parse_capture_status_bytes(&bytes).expect("known string archive_manifest_id");
        assert_eq!(parsed["archive_manifest_id"], "manifest-42");
        assert_eq!(
            parsed["capture_backlog_records"], 0,
            "typing archive_manifest_id must not couple it to capture_backlog_records"
        );
        assert!(
            parsed.get("oldest_pending_capture_height").is_none(),
            "typing archive_manifest_id must not couple it to oldest_pending_capture_height"
        );
        assert!(
            parsed.get("disk_free_basis_points").is_none(),
            "typing archive_manifest_id must not couple it to disk_free_basis_points"
        );
        assert!(
            parsed.get("durable_height").is_none(),
            "typing archive_manifest_id must not couple it to durable_height"
        );
        assert!(
            parsed.get("failover_height").is_none(),
            "typing archive_manifest_id must not couple it to failover_height"
        );
        assert!(
            parsed.get("failover_reason").is_none(),
            "typing archive_manifest_id must not couple it to failover_reason"
        );
        assert!(parsed.get("auxiliary_sources").is_none());
    }

    #[test]
    fn writer_archive_manifest_id_text_rules_are_not_copied_onto_api_parse() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for archive_manifest_id in [
            serde_json::json!(" padded "),
            serde_json::Value::String("a".repeat(513)),
            serde_json::json!("manifest\u{0001}"),
        ] {
            value["archive_manifest_id"] = archive_manifest_id.clone();
            let bytes =
                serde_json::to_vec(&value).expect("encode writer-invalid archive_manifest_id");
            let parsed = parse_capture_status_bytes(&bytes).unwrap_or_else(|error| {
                panic!("{archive_manifest_id} must stay valid at API parse; writer 512/trim/control is not copied: {error}")
            });
            assert_eq!(parsed["archive_manifest_id"], archive_manifest_id);
        }
    }

    #[test]
    fn omitted_top_level_archive_manifest_id_is_accepted() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        assert!(
            value.get("archive_manifest_id").is_none(),
            "v4 fixture must omit optional archive_manifest_id"
        );
        let bytes = serde_json::to_vec(&value).expect("encode omitted archive_manifest_id");
        let parsed = parse_capture_status_bytes(&bytes).expect("omitted archive_manifest_id");
        assert!(parsed.get("archive_manifest_id").is_none());

        value["archive_manifest_id"] = serde_json::json!("manifest-42");
        value
            .as_object_mut()
            .expect("capture status object")
            .remove("archive_manifest_id");
        let bytes = serde_json::to_vec(&value).expect("encode removed archive_manifest_id");
        let parsed = parse_capture_status_bytes(&bytes).expect("removed archive_manifest_id");
        assert!(parsed.get("archive_manifest_id").is_none());
    }

    #[test]
    fn present_non_string_top_level_archive_manifest_id_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status.json"))
                .expect("v4 json");
        for archive_manifest_id in [
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
            serde_json::json!(""),
        ] {
            value["archive_manifest_id"] = archive_manifest_id.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes).expect_err(
                    "present non-string or empty archive_manifest_id must not fail open"
                ),
                SnapshotError::Invalid,
                "{archive_manifest_id} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn present_non_string_auxiliary_source_id_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for source_id in [
            serde_json::json!(1),
            serde_json::json!(true),
            serde_json::json!(null),
            serde_json::json!({"not": "a-string"}),
            serde_json::json!(["not-a-string"]),
            serde_json::json!(""),
        ] {
            value["auxiliary_sources"][0]["source_id"] = source_id.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-string or empty source_id must not fail open"),
                SnapshotError::Invalid,
                "{source_id} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn omitted_or_empty_auxiliary_source_id_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        value["auxiliary_sources"][0]
            .as_object_mut()
            .expect("auxiliary source object")
            .remove("source_id");
        let bytes = serde_json::to_vec(&value).expect("encode omitted source_id");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("omitted nested source_id must not fail open"),
            SnapshotError::Invalid
        );

        value["auxiliary_sources"] = serde_json::json!([{}]);
        let bytes = serde_json::to_vec(&value).expect("encode empty auxiliary item");
        assert_eq!(
            parse_capture_status_bytes(&bytes)
                .expect_err("empty auxiliary source object must not fail open"),
            SnapshotError::Invalid
        );
    }

    #[test]
    fn present_non_array_auxiliary_sources_is_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        for field in [
            serde_json::json!("not-an-array"),
            serde_json::json!({"not": "an-array"}),
            serde_json::json!(null),
            serde_json::json!(true),
        ] {
            value["auxiliary_sources"] = field.clone();
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-array auxiliary_sources must not fail open"),
                SnapshotError::Invalid,
                "{field} must be snapshot_invalid"
            );
        }
    }

    #[test]
    fn non_object_auxiliary_source_items_are_snapshot_invalid() {
        let mut value =
            serde_json::from_slice::<serde_json::Value>(&fixture("capture-status-v5.json"))
                .expect("v5 json");
        let known = value["auxiliary_sources"][0].clone();
        for item in [
            serde_json::json!("not-an-object"),
            serde_json::json!(1),
            serde_json::json!(null),
        ] {
            value["auxiliary_sources"] = serde_json::json!([known.clone(), item]);
            let bytes = serde_json::to_vec(&value).expect("encode");
            assert_eq!(
                parse_capture_status_bytes(&bytes)
                    .expect_err("present non-object auxiliary source item must not fail open"),
                SnapshotError::Invalid
            );
        }
    }
}
