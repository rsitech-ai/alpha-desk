/// Checked-in OpenAPI document generated from the health proto JSON fields,
/// capture-status v4 (inactive) / v5 (maintenance) required keys, optional
/// last-heartbeat throughput integers, fail-closed query budgets, frozen
/// core dead-letter reason codes, and the HTTP router. This is not a
/// production authentication, availability, or SLO contract, it does not
/// invent fills or mark sources live or qualified, and it is not a live core.
pub fn openapi_yaml() -> &'static str {
    include_str!("../../../schemas/openapi/v1/openapi.yaml")
}

pub use crate::snapshot::{CORE_DEADLETTER_REASON_CODES, is_core_deadletter_reason};

pub const HEALTH_JSON_FIELDS: &[&str] = &[
    "schema_version",
    "scope",
    "state",
    "reason_code",
    "observed_at_micros",
    "suppresses",
];

pub const CAPTURE_STATUS_SCHEMA_IDS: &[&str] = &[
    crate::snapshot::CAPTURE_STATUS_SCHEMA_V4,
    crate::snapshot::CAPTURE_STATUS_SCHEMA_V5,
];

pub const SNAPSHOT_UNAVAILABLE_REASON_CODES: &[&str] = &[
    crate::snapshot::SnapshotError::Missing.reason_code(),
    crate::snapshot::SnapshotError::Invalid.reason_code(),
];

pub const LAST_HEARTBEAT_THROUGHPUT_FIELDS: &[&str] =
    &["throughput_records_per_sec", "throughput_blocks_per_sec"];

pub const ROUTER_PATHS: &[&str] = &[
    "/healthz",
    "/readyz",
    "/v1/health",
    "/v1/capture/status",
    "/v1/stream",
    "/v1/stream/canonical-events",
    "/v1/openapi.yaml",
];
