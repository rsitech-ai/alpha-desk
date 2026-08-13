/// Checked-in OpenAPI document generated from the health proto JSON fields,
/// capture-status v4/v5 required keys, fail-closed query budgets, and the HTTP
/// router. This is not a production authentication, availability, or SLO
/// contract.
pub fn openapi_yaml() -> &'static str {
    include_str!("../../../schemas/openapi/v1/openapi.yaml")
}

pub const HEALTH_JSON_FIELDS: &[&str] = &[
    "schema_version",
    "scope",
    "state",
    "reason_code",
    "observed_at_micros",
    "suppresses",
];

pub const ROUTER_PATHS: &[&str] = &[
    "/healthz",
    "/readyz",
    "/v1/health",
    "/v1/capture/status",
    "/v1/stream",
    "/v1/stream/canonical-events",
    "/v1/openapi.yaml",
];
