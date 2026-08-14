use std::io::{ErrorKind, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use hl_api::{
    AUXILIARY_SOURCE_HEALTH, AUXILIARY_SOURCE_QUALIFICATION, ApiConfig, AppState, AuthMode,
    CAPTURE_SOURCE_HEALTH, CAPTURE_STATUS_SCHEMA_IDS, COMMITTED_SOURCE_CLASSES,
    CORE_DEADLETTER_REASON_CODES, HEALTH_JSON_FIELDS, LAST_HEARTBEAT_THROUGHPUT_FIELDS,
    LEDGER_UNSUPPORTED_EVENT_REASON_CODES, READYZ_200_DESCRIPTION, READYZ_503_DESCRIPTION,
    READYZ_GET_DESCRIPTION, RESTART_RECONSTRUCTION, ROUTER_PATHS,
    SNAPSHOT_UNAVAILABLE_REASON_CODES, auxiliary_source_cursor_epoch_is_optional_string,
    auxiliary_source_durable_offset_is_optional_u64, auxiliary_source_health_openapi_enum,
    auxiliary_source_id_is_required_string, auxiliary_source_local_sequence_is_optional_u64,
    auxiliary_source_partial_line_is_required_bool, auxiliary_source_qualification_openapi_enum,
    auxiliary_source_spool_records_is_required_u64,
    auxiliary_source_unarchived_records_is_required_u64, capture_source_health_openapi_enum,
    committed_source_class_openapi_enum, core_deadletter_reason_openapi_enum,
    health_503_response_ref, health_503_schema_ref, health_reason_code_is_unrestricted_string,
    independent_source_health_openapi_enum, is_core_deadletter_reason,
    is_ledger_unsupported_event_reason, ledger_unsupported_event_reason_openapi_enum, openapi_yaml,
    readyz_200_description, readyz_200_schema_ref, readyz_503_description, readyz_503_schema_ref,
    readyz_get_description, restart_reconstruction_openapi_enum, spawn_local,
    unavailable_response_schema_ref,
};
use http::Request;
use serde_json::Value;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const DEFAULT_QUERY_BUDGETS: &str = concat!(
    "schema_version = \"hl.api.query_budgets.v1\"\n",
    "max_rows = 1024\n",
    "timeout_ms = 2000\n",
    "max_concurrency = 8\n",
);

fn write_query_budgets(directory: &Path, body: &str) {
    std::fs::write(directory.join("api-query-budgets.toml"), body).expect("write query budgets");
}

fn write_config(
    directory: &Path,
    bind: &str,
    mode: &str,
    credential_file: Option<&Path>,
    canonical_health: Option<&Path>,
    capture_status: Option<&Path>,
) -> std::path::PathBuf {
    let config_path = directory.join("api.toml");
    let mut body = format!("[listen]\nbind = \"{bind}\"\n\n[auth]\nmode = \"{mode}\"\n");
    if let Some(path) = credential_file {
        body.push_str(&format!("credential_file = \"{}\"\n", path.display()));
    }
    body.push_str("\n[snapshots]\n");
    if let Some(path) = canonical_health {
        body.push_str(&format!("canonical_health = \"{}\"\n", path.display()));
    }
    if let Some(path) = capture_status {
        body.push_str(&format!("capture_status = \"{}\"\n", path.display()));
    }
    write_query_budgets(directory, DEFAULT_QUERY_BUDGETS);
    body.push_str("\n[query_budgets]\nfile = \"api-query-budgets.toml\"\n");
    std::fs::write(&config_path, body).expect("write config");
    config_path
}

fn state_from(
    directory: &Path,
    mode: &str,
    credential_file: Option<&Path>,
    canonical_health: Option<&Path>,
    capture_status: Option<&Path>,
) -> AppState {
    let config_path = write_config(
        directory,
        "127.0.0.1:0",
        mode,
        credential_file,
        canonical_health,
        capture_status,
    );
    AppState::from_config(ApiConfig::from_path(&config_path).expect("config"))
}

async fn call(state: &AppState, path: &str, headers: &[(&str, &str)]) -> (u16, Value) {
    let mut builder = Request::builder().method("GET").uri(path);
    for (name, value) in headers {
        builder = builder.header(*name, *value);
    }
    let request = builder.body(Bytes::new()).expect("request");
    let (status, body) = state.handle(request).await;
    let value = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body)
            .unwrap_or(Value::String(String::from_utf8_lossy(&body).into_owned()))
    };
    (status.as_u16(), value)
}

#[tokio::test]
async fn healthz_returns_proto_health_without_inventing_canonical_data() {
    let directory = tempdir().expect("temporary directory");
    let state = state_from(directory.path(), "loopback-dev", None, None, None);

    let (status, body) = call(&state, "/healthz", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.health.v1");
    assert_eq!(body["scope"], "api:process");
    assert_eq!(body["state"], "HEALTH_STATE_GREEN");
    assert_eq!(body["reason_code"], "healthy");

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_ne!(
        body["schema_version"], "hl.health.v1",
        "/v1/health 503 must stay hl.api.error.v1, not HealthAssessment"
    );
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_missing");
    assert!(body.get("state").is_none());
    let document = openapi_yaml();
    assert_eq!(
        health_503_response_ref(document),
        Some("#/components/responses/Unavailable"),
        "/v1/health 503 must $ref Unavailable while the handler returns hl.api.error.v1"
    );
    assert!(
        health_503_schema_ref(document).is_none(),
        "/v1/health 503 must not inline HealthAssessment while the handler returns hl.api.error.v1"
    );
    assert_ne!(
        health_503_schema_ref(document),
        Some("#/components/schemas/HealthAssessment"),
        "switching /v1/health 503 to HealthAssessment must fail while the handler returns hl.api.error.v1"
    );
    assert_eq!(
        unavailable_response_schema_ref(document),
        Some("#/components/schemas/ApiError"),
        "shared Unavailable must stay ApiError for /v1/health 503"
    );
}

#[tokio::test]
async fn readyz_fail_closes_when_no_snapshots_are_configured() {
    let directory = tempdir().expect("temporary directory");
    let state = state_from(directory.path(), "loopback-dev", None, None, None);
    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["state"], "HEALTH_STATE_RED");
    assert_eq!(body["reason_code"], "no_required_dependencies");
}

#[tokio::test]
async fn versioned_reads_serve_validated_fixtures_and_reject_invalid_snapshots() {
    let directory = tempdir().expect("temporary directory");
    let health_path = directory.path().join("canonical-health.json");
    let capture_path = directory.path().join("capture-status.json");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/api/canonical-health.json"),
        &health_path,
    )
    .expect("copy health fixture");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/api/capture-status.json"),
        &capture_path,
    )
    .expect("copy capture fixture");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        Some(&health_path),
        Some(&capture_path),
    );

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.health.v1");
    assert_eq!(body["scope"], "canonical");
    assert_eq!(body["state"], "HEALTH_STATE_GREEN");

    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.capture.status.v4");
    assert_eq!(body["health"], "green");
    assert_eq!(body["pending_blocks"], 0);
    assert!(body.get("maintenance").is_none());
    assert!(body.get("fills").is_none());
    assert!(body.get("qualification").is_none());

    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["state"], "HEALTH_STATE_GREEN");

    let mut invalid = std::fs::OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(&capture_path)
        .expect("truncate capture snapshot");
    invalid
        .write_all(
            br#"{"schema_version":"hl.capture.status.v4","health":"green","ready":true,"px":1.5}"#,
        )
        .expect("write invalid snapshot");
    drop(invalid);

    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

fn copy_api_fixture(directory: &Path, name: &str) -> std::path::PathBuf {
    let destination = directory.join(name);
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/api")
            .join(name),
        &destination,
    )
    .unwrap_or_else(|error| panic!("copy fixture {name}: {error}"));
    destination
}

#[tokio::test]
async fn capture_status_serves_v5_maintenance_and_rejects_v4_smuggled_maintenance() {
    let directory = tempdir().expect("temporary directory");
    let v5_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let state = state_from(directory.path(), "loopback-dev", None, None, Some(&v5_path));

    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.capture.status.v5");
    assert_eq!(body["maintenance"]["enabled"], true);
    assert_eq!(body["maintenance"]["retention_authorized"], false);
    assert_eq!(
        body["auxiliary_sources"][0]["restart_reconstruction"],
        "complete"
    );
    assert_eq!(body["auxiliary_sources"][0]["health"], "starting");
    assert_eq!(body["auxiliary_sources"][0]["qualification"], "unqualified");
    assert!(body.get("fills").is_none());

    let smuggled = copy_api_fixture(
        directory.path(),
        "capture-status-v4-smuggled-maintenance.json",
    );
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&smuggled),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn unknown_active_committed_source_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v4 json");

    for source in COMMITTED_SOURCE_CLASSES {
        value["active_committed_source"] = serde_json::json!(source);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known source"),
        )
        .expect("write known source");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 200, "{source} must remain a typed capture status");
        assert_eq!(body["active_committed_source"], *source);
        assert_eq!(body["schema_version"], "hl.capture.status.v4");
    }

    value["active_committed_source"] = serde_json::json!("primary");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode unknown source"),
    )
    .expect("write unknown source");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn unknown_primary_source_health_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v4 json");

    for health in CAPTURE_SOURCE_HEALTH {
        value["primary_source_health"] = serde_json::json!(health);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known health"),
        )
        .expect("write known health");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 200, "{health} must remain a typed capture status");
        assert_eq!(body["primary_source_health"], *health);
        assert_eq!(body["schema_version"], "hl.capture.status.v4");
    }

    value["primary_source_health"] = serde_json::json!("degraded");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode unknown health"),
    )
    .expect("write unknown health");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn unknown_independent_source_health_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v4 json");

    for health in CAPTURE_SOURCE_HEALTH {
        value["independent_source_health"] = serde_json::json!(health);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known independent health"),
        )
        .expect("write known independent health");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(
            status, 200,
            "independent {health} must remain a typed capture status"
        );
        assert_eq!(body["independent_source_health"], *health);
    }

    value["independent_source_health"] = serde_json::json!("latched");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode unknown independent health"),
    )
    .expect("write unknown independent health");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn unknown_auxiliary_source_health_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    for health in AUXILIARY_SOURCE_HEALTH {
        value["auxiliary_sources"][0]["health"] = serde_json::json!(health);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known auxiliary health"),
        )
        .expect("write known auxiliary health");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 200, "{health} must remain a typed capture status");
        assert_eq!(body["auxiliary_sources"][0]["health"], *health);
        assert_eq!(body["schema_version"], "hl.capture.status.v5");
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("health");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted auxiliary health"),
    )
    .expect("write omitted auxiliary health");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 200,
        "omitted auxiliary source health must stay valid"
    );
    assert!(body["auxiliary_sources"][0].get("health").is_none());

    value["auxiliary_sources"][0]["health"] = serde_json::json!("range-unavailable");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode unknown auxiliary health"),
    )
    .expect("write unknown auxiliary health");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn unknown_restart_reconstruction_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    for reconstruction in RESTART_RECONSTRUCTION {
        value["auxiliary_sources"][0]["restart_reconstruction"] = serde_json::json!(reconstruction);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known reconstruction"),
        )
        .expect("write known reconstruction");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(
            status, 200,
            "{reconstruction} must remain a typed capture status"
        );
        assert_eq!(
            body["auxiliary_sources"][0]["restart_reconstruction"],
            *reconstruction
        );
        assert_eq!(body["schema_version"], "hl.capture.status.v5");
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("restart_reconstruction");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted reconstruction"),
    )
    .expect("write omitted reconstruction");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 200,
        "omitted restart_reconstruction must stay valid"
    );
    assert!(
        body["auxiliary_sources"][0]
            .get("restart_reconstruction")
            .is_none()
    );

    value["auxiliary_sources"][0]["restart_reconstruction"] = serde_json::json!("NotRequired");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode unknown reconstruction"),
    )
    .expect("write unknown reconstruction");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn unknown_auxiliary_source_qualification_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    for qualification in AUXILIARY_SOURCE_QUALIFICATION {
        value["auxiliary_sources"][0]["qualification"] = serde_json::json!(qualification);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known qualification"),
        )
        .expect("write known qualification");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(
            status, 200,
            "{qualification} must remain a typed capture status"
        );
        assert_eq!(
            body["auxiliary_sources"][0]["qualification"],
            *qualification
        );
        assert_eq!(body["schema_version"], "hl.capture.status.v5");
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("qualification");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted qualification"),
    )
    .expect("write omitted qualification");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 200,
        "omitted auxiliary source qualification must stay valid"
    );
    assert!(body["auxiliary_sources"][0].get("qualification").is_none());

    value["auxiliary_sources"][0]["qualification"] = serde_json::json!("Unqualified");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode unknown qualification"),
    )
    .expect("write unknown qualification");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn present_non_array_auxiliary_sources_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 200,
        "array of known object auxiliary sources must stay 200"
    );
    assert!(body["auxiliary_sources"].is_array());
    assert!(body["auxiliary_sources"][0].is_object());
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    value
        .as_object_mut()
        .expect("status object")
        .remove("auxiliary_sources");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted auxiliary_sources"),
    )
    .expect("write omitted auxiliary_sources");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "omitted auxiliary_sources must stay valid");
    assert!(body.get("auxiliary_sources").is_none());
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    for field in [
        serde_json::json!("not-an-array"),
        serde_json::json!({"not": "an-array"}),
        serde_json::json!(null),
        serde_json::json!(true),
    ] {
        value["auxiliary_sources"] = field.clone();
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-array auxiliary_sources"),
        )
        .expect("write non-array auxiliary_sources");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{field} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }
}

#[tokio::test]
async fn non_object_auxiliary_source_item_is_snapshot_invalid() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");
    let known = value["auxiliary_sources"][0].clone();

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "known object auxiliary source must stay 200");
    assert!(body["auxiliary_sources"][0].is_object());
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    for item in [
        serde_json::json!("not-an-object"),
        serde_json::json!(1),
        serde_json::json!(null),
    ] {
        value["auxiliary_sources"] = serde_json::json!([known.clone(), item]);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-object auxiliary item"),
        )
        .expect("write non-object auxiliary item");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{item} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }
}

#[tokio::test]
async fn nested_auxiliary_source_id_is_required_string() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "known string source_id must stay 200");
    assert_eq!(
        body["auxiliary_sources"][0]["source_id"],
        "node-misc-events"
    );
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    for source_id in [
        serde_json::json!(1),
        serde_json::json!(true),
        serde_json::json!(null),
        serde_json::json!({"not": "a-string"}),
        serde_json::json!(["not-a-string"]),
        serde_json::json!(""),
    ] {
        value["auxiliary_sources"][0]["source_id"] = source_id.clone();
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-string source_id"),
        )
        .expect("write non-string source_id");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{source_id} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("source_id");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted source_id"),
    )
    .expect("write omitted source_id");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 503, "omitted nested source_id must not fail open");
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");

    value["auxiliary_sources"] = serde_json::json!([{}]);
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode empty auxiliary item"),
    )
    .expect("write empty auxiliary item");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 503,
        "empty auxiliary source object must not fail open"
    );
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn nested_auxiliary_spool_records_is_required_u64() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "known u64 spool_records must stay 200");
    assert_eq!(body["auxiliary_sources"][0]["spool_records"], 0);
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    for spool_records in [3_u64, u64::MAX] {
        value["auxiliary_sources"][0]["spool_records"] = serde_json::json!(spool_records);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known u64 spool_records"),
        )
        .expect("write known u64 spool_records");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 200, "{spool_records} must stay 200");
        assert_eq!(body["auxiliary_sources"][0]["spool_records"], spool_records);
    }

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
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-u64 spool_records"),
        )
        .expect("write non-u64 spool_records");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{spool_records} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("spool_records");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted spool_records"),
    )
    .expect("write omitted spool_records");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 503,
        "omitted nested spool_records must not fail open"
    );
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn nested_auxiliary_unarchived_records_is_required_u64() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "known u64 unarchived_records must stay 200");
    assert_eq!(body["auxiliary_sources"][0]["unarchived_records"], 0);
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    for unarchived_records in [3_u64, u64::MAX] {
        value["auxiliary_sources"][0]["unarchived_records"] = serde_json::json!(unarchived_records);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known u64 unarchived_records"),
        )
        .expect("write known u64 unarchived_records");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 200, "{unarchived_records} must stay 200");
        assert_eq!(
            body["auxiliary_sources"][0]["unarchived_records"],
            unarchived_records
        );
    }

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
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-u64 unarchived_records"),
        )
        .expect("write non-u64 unarchived_records");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{unarchived_records} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("unarchived_records");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted unarchived_records"),
    )
    .expect("write omitted unarchived_records");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 503,
        "omitted nested unarchived_records must not fail open"
    );
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn nested_auxiliary_partial_line_is_required_bool() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "known bool partial_line must stay 200");
    assert_eq!(body["auxiliary_sources"][0]["partial_line"], false);
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    value["auxiliary_sources"][0]["partial_line"] = serde_json::json!(true);
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode known true partial_line"),
    )
    .expect("write known true partial_line");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "true must stay 200");
    assert_eq!(body["auxiliary_sources"][0]["partial_line"], true);

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
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-bool partial_line"),
        )
        .expect("write non-bool partial_line");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{partial_line} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("partial_line");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted partial_line"),
    )
    .expect("write omitted partial_line");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 503,
        "omitted nested partial_line must not fail open"
    );
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
}

#[tokio::test]
async fn nested_auxiliary_cursor_epoch_is_optional_string() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "omitted nested cursor_epoch must stay 200");
    assert!(body["auxiliary_sources"][0].get("cursor_epoch").is_none());
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    value["auxiliary_sources"][0]["cursor_epoch"] = serde_json::json!("node-file-v1:epoch");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode known string cursor_epoch"),
    )
    .expect("write known string cursor_epoch");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "known string cursor_epoch must stay 200");
    assert_eq!(
        body["auxiliary_sources"][0]["cursor_epoch"],
        "node-file-v1:epoch"
    );

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("cursor_epoch");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted cursor_epoch"),
    )
    .expect("write omitted cursor_epoch");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 200,
        "omitted nested cursor_epoch after removal must stay 200"
    );
    assert!(body["auxiliary_sources"][0].get("cursor_epoch").is_none());

    for cursor_epoch in [
        serde_json::json!(1),
        serde_json::json!(true),
        serde_json::json!(null),
        serde_json::json!({"not": "a-string"}),
        serde_json::json!(["not-a-string"]),
        serde_json::json!(""),
    ] {
        value["auxiliary_sources"][0]["cursor_epoch"] = cursor_epoch.clone();
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-string cursor_epoch"),
        )
        .expect("write non-string cursor_epoch");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{cursor_epoch} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }
}

#[tokio::test]
async fn nested_auxiliary_durable_offset_is_optional_u64() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "omitted nested durable_offset must stay 200");
    assert!(body["auxiliary_sources"][0].get("durable_offset").is_none());
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    for durable_offset in [0_u64, 47, u64::MAX] {
        value["auxiliary_sources"][0]["durable_offset"] = serde_json::json!(durable_offset);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known u64 durable_offset"),
        )
        .expect("write known u64 durable_offset");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 200, "{durable_offset} must stay 200");
        assert_eq!(
            body["auxiliary_sources"][0]["durable_offset"],
            durable_offset
        );
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("durable_offset");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted durable_offset"),
    )
    .expect("write omitted durable_offset");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 200,
        "omitted nested durable_offset after removal must stay 200"
    );
    assert!(body["auxiliary_sources"][0].get("durable_offset").is_none());

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
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-u64 durable_offset"),
        )
        .expect("write non-u64 durable_offset");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{durable_offset} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }
}

#[tokio::test]
async fn nested_auxiliary_local_sequence_is_optional_u64() {
    let directory = tempdir().expect("temporary directory");
    let capture_path = copy_api_fixture(directory.path(), "capture-status-v5.json");
    let mut value: Value =
        serde_json::from_slice(&std::fs::read(&capture_path).expect("read fixture"))
            .expect("v5 json");

    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(status, 200, "omitted nested local_sequence must stay 200");
    assert!(body["auxiliary_sources"][0].get("local_sequence").is_none());
    assert_eq!(body["schema_version"], "hl.capture.status.v5");

    for local_sequence in [0_u64, 47, u64::MAX] {
        value["auxiliary_sources"][0]["local_sequence"] = serde_json::json!(local_sequence);
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode known u64 local_sequence"),
        )
        .expect("write known u64 local_sequence");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 200, "{local_sequence} must stay 200");
        assert_eq!(
            body["auxiliary_sources"][0]["local_sequence"],
            local_sequence
        );
    }

    value["auxiliary_sources"][0]
        .as_object_mut()
        .expect("auxiliary source object")
        .remove("local_sequence");
    std::fs::write(
        &capture_path,
        serde_json::to_vec(&value).expect("encode omitted local_sequence"),
    )
    .expect("write omitted local_sequence");
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        None,
        Some(&capture_path),
    );
    let (status, body) = call(&state, "/v1/capture/status", &[]).await;
    assert_eq!(
        status, 200,
        "omitted nested local_sequence after removal must stay 200"
    );
    assert!(body["auxiliary_sources"][0].get("local_sequence").is_none());

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
        std::fs::write(
            &capture_path,
            serde_json::to_vec(&value).expect("encode non-u64 local_sequence"),
        )
        .expect("write non-u64 local_sequence");
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            None,
            Some(&capture_path),
        );
        let (status, body) = call(&state, "/v1/capture/status", &[]).await;
        assert_eq!(status, 503, "{local_sequence} must not fail open");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "data_unavailable");
        assert_eq!(body["reason_code"], "snapshot_invalid");
    }
}

fn write_health_snapshot(directory: &Path, name: &str, state: &str, reason_code: &str) -> PathBuf {
    let path = directory.join(name);
    std::fs::write(
        &path,
        format!(
            r#"{{"schema_version":"hl.health.v1","scope":"canonical","state":"{state}","reason_code":"{reason_code}","observed_at_micros":1,"suppresses":[]}}"#
        ),
    )
    .expect("write health snapshot");
    path
}

#[tokio::test]
async fn canonical_health_types_core_deadletter_reasons_and_does_not_become_ready() {
    let directory = tempdir().expect("temporary directory");
    for reason_code in CORE_DEADLETTER_REASON_CODES {
        let health_path = write_health_snapshot(
            directory.path(),
            &format!("{reason_code}.json"),
            "HEALTH_STATE_RED",
            reason_code,
        );
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            Some(&health_path),
            None,
        );
        let (status, body) = call(&state, "/v1/health", &[]).await;
        assert_eq!(status, 200, "{reason_code}");
        assert_eq!(body["schema_version"], "hl.health.v1");
        assert_eq!(body["state"], "HEALTH_STATE_RED");
        assert_eq!(body["reason_code"], *reason_code);
        assert!(is_core_deadletter_reason(reason_code));

        let (status, body) = call(&state, "/readyz", &[]).await;
        assert_eq!(status, 503, "{reason_code} must not become ready");
        assert_eq!(body["state"], "HEALTH_STATE_RED");
        let aggregate = body["reason_code"].as_str().expect("aggregate reason");
        assert!(
            aggregate.contains(reason_code),
            "readyz must surface typed {reason_code}, got {aggregate}"
        );
    }
}

#[tokio::test]
async fn canonical_health_types_ledger_unsupported_event_and_does_not_become_ready() {
    let directory = tempdir().expect("temporary directory");
    for reason_code in LEDGER_UNSUPPORTED_EVENT_REASON_CODES {
        let health_path = write_health_snapshot(
            directory.path(),
            &format!("{reason_code}.json"),
            "HEALTH_STATE_RED",
            reason_code,
        );
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            Some(&health_path),
            None,
        );
        let (status, body) = call(&state, "/v1/health", &[]).await;
        assert_eq!(status, 200, "{reason_code}");
        assert_eq!(body["schema_version"], "hl.health.v1");
        assert_eq!(body["state"], "HEALTH_STATE_RED");
        assert_eq!(body["reason_code"], *reason_code);
        assert!(is_ledger_unsupported_event_reason(reason_code));

        let (status, body) = call(&state, "/readyz", &[]).await;
        assert_eq!(status, 503, "{reason_code} must not become ready");
        assert_eq!(body["state"], "HEALTH_STATE_RED");
        let aggregate = body["reason_code"].as_str().expect("aggregate reason");
        assert!(
            aggregate.contains(reason_code),
            "readyz must surface typed {reason_code}, got {aggregate}"
        );
    }
}

#[tokio::test]
async fn green_or_amber_ledger_unsupported_event_health_fails_closed() {
    let directory = tempdir().expect("temporary directory");
    for reason_code in LEDGER_UNSUPPORTED_EVENT_REASON_CODES {
        let green = write_health_snapshot(
            directory.path(),
            &format!("green-{reason_code}.json"),
            "HEALTH_STATE_GREEN",
            reason_code,
        );
        let state = state_from(directory.path(), "loopback-dev", None, Some(&green), None);
        let (status, body) = call(&state, "/v1/health", &[]).await;
        assert_eq!(status, 503, "{reason_code}");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["reason_code"], "snapshot_invalid");
        let (status, body) = call(&state, "/readyz", &[]).await;
        assert_eq!(status, 503, "{reason_code} must not become ready");
        assert_eq!(body["state"], "HEALTH_STATE_RED");

        let amber = write_health_snapshot(
            directory.path(),
            &format!("amber-{reason_code}.json"),
            "HEALTH_STATE_AMBER",
            reason_code,
        );
        let state = state_from(directory.path(), "loopback-dev", None, Some(&amber), None);
        let (status, body) = call(&state, "/v1/health", &[]).await;
        assert_eq!(status, 503, "{reason_code}");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["reason_code"], "snapshot_invalid");
        let (status, body) = call(&state, "/readyz", &[]).await;
        assert_eq!(status, 503, "{reason_code} must not become ready");
        assert_eq!(body["state"], "HEALTH_STATE_RED");
    }
}

#[tokio::test]
async fn unknown_ledger_sibling_red_stays_typed_fail_closed_and_does_not_become_ready() {
    let directory = tempdir().expect("temporary directory");
    const UNKNOWN_RED: &str = "ledger.invented";
    assert!(
        !LEDGER_UNSUPPORTED_EVENT_REASON_CODES.contains(&UNKNOWN_RED),
        "HTTP unknown-RED coverage must use a sibling outside the frozen enum"
    );
    assert!(!is_ledger_unsupported_event_reason(UNKNOWN_RED));
    let health_path = write_health_snapshot(
        directory.path(),
        "unknown-ledger-red.json",
        "HEALTH_STATE_RED",
        UNKNOWN_RED,
    );
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        Some(&health_path),
        None,
    );

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.health.v1");
    assert_eq!(body["state"], "HEALTH_STATE_RED");
    assert_eq!(body["reason_code"], UNKNOWN_RED);
    assert_ne!(
        body["reason_code"], "snapshot_invalid",
        "production serves unknown RED as typed fail-closed, not snapshot_invalid"
    );
    assert!(!is_ledger_unsupported_event_reason(UNKNOWN_RED));

    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503, "unknown RED must not become ready");
    assert_eq!(body["state"], "HEALTH_STATE_RED");
    let aggregate = body["reason_code"].as_str().expect("aggregate reason");
    assert!(
        aggregate.contains(UNKNOWN_RED),
        "readyz must surface typed {UNKNOWN_RED}, got {aggregate}"
    );
    assert!(
        !aggregate.contains("snapshot_invalid"),
        "unknown RED must not be rewritten as snapshot_invalid, got {aggregate}"
    );
}

#[tokio::test]
async fn unknown_red_reason_stays_typed_fail_closed_and_does_not_become_ready() {
    let directory = tempdir().expect("temporary directory");
    const UNKNOWN_RED: &str = "core.deadletter_invented";
    assert!(
        !CORE_DEADLETTER_REASON_CODES.contains(&UNKNOWN_RED),
        "HTTP unknown-RED coverage must use a sibling outside the frozen enum"
    );
    assert!(!is_core_deadletter_reason(UNKNOWN_RED));
    let health_path = write_health_snapshot(
        directory.path(),
        "unknown-red.json",
        "HEALTH_STATE_RED",
        UNKNOWN_RED,
    );
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        Some(&health_path),
        None,
    );

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.health.v1");
    assert_eq!(body["state"], "HEALTH_STATE_RED");
    assert_eq!(body["reason_code"], UNKNOWN_RED);
    assert_ne!(
        body["reason_code"], "snapshot_invalid",
        "production serves unknown RED as typed fail-closed, not snapshot_invalid"
    );
    assert_ne!(body["code"], "data_unavailable");

    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503, "unknown RED must not become ready");
    assert_eq!(body["state"], "HEALTH_STATE_RED");
    let aggregate = body["reason_code"].as_str().expect("aggregate reason");
    assert!(
        aggregate.contains(UNKNOWN_RED),
        "readyz must surface typed {UNKNOWN_RED}, got {aggregate}"
    );
    assert!(
        !aggregate.contains("snapshot_invalid"),
        "unknown RED must not be rewritten as snapshot_invalid, got {aggregate}"
    );
}

#[tokio::test]
async fn green_deadletter_or_unknown_health_fails_closed() {
    let directory = tempdir().expect("temporary directory");
    let health_path = write_health_snapshot(
        directory.path(),
        "green-deadletter.json",
        "HEALTH_STATE_GREEN",
        "core.deadletter_unsafe_path",
    );
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        Some(&health_path),
        None,
    );
    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["reason_code"], "snapshot_invalid");
    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["state"], "HEALTH_STATE_RED");

    let unknown = write_health_snapshot(
        directory.path(),
        "green-unknown.json",
        "HEALTH_STATE_GREEN",
        "not.a.documented.reason",
    );
    let state = state_from(directory.path(), "loopback-dev", None, Some(&unknown), None);
    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["reason_code"], "snapshot_invalid");
    let (status, _) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503);
}

#[tokio::test]
async fn amber_core_deadletter_health_is_snapshot_invalid_and_not_ready() {
    let directory = tempdir().expect("temporary directory");
    for reason_code in CORE_DEADLETTER_REASON_CODES {
        let health_path = write_health_snapshot(
            directory.path(),
            &format!("amber-{reason_code}.json"),
            "HEALTH_STATE_AMBER",
            reason_code,
        );
        let state = state_from(
            directory.path(),
            "loopback-dev",
            None,
            Some(&health_path),
            None,
        );
        let (status, body) = call(&state, "/v1/health", &[]).await;
        assert_eq!(status, 503, "{reason_code}");
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["reason_code"], "snapshot_invalid");
        let (status, body) = call(&state, "/readyz", &[]).await;
        assert_eq!(status, 503, "{reason_code} must not become ready");
        assert_eq!(body["state"], "HEALTH_STATE_RED");
    }
}

#[tokio::test]
async fn amber_invented_ledger_reason_stays_typed_and_does_not_become_ready() {
    let directory = tempdir().expect("temporary directory");
    const INVENTED: &str = "ledger.invented";
    assert!(
        !LEDGER_UNSUPPORTED_EVENT_REASON_CODES.contains(&INVENTED),
        "invented ledger.* must stay outside the frozen consume-poison enum"
    );
    assert!(!is_ledger_unsupported_event_reason(INVENTED));
    let health_path = write_health_snapshot(
        directory.path(),
        "amber-ledger-invented.json",
        "HEALTH_STATE_AMBER",
        INVENTED,
    );
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        Some(&health_path),
        None,
    );

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.health.v1");
    assert_eq!(body["state"], "HEALTH_STATE_AMBER");
    assert_eq!(body["reason_code"], INVENTED);
    assert_ne!(
        body["reason_code"], "snapshot_invalid",
        "AMBER invented ledger.* is not a family prefix; it must stay typed"
    );

    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503, "AMBER invented ledger.* must not become ready");
    assert_eq!(body["schema_version"], "hl.health.v1");
    let aggregate = body["reason_code"].as_str().expect("aggregate reason");
    assert!(
        aggregate.contains(INVENTED),
        "readyz must surface typed {INVENTED}, got {aggregate}"
    );
    assert!(
        !aggregate.contains("snapshot_invalid"),
        "AMBER invented ledger.* must not be rewritten as snapshot_invalid, got {aggregate}"
    );
}

#[tokio::test]
async fn amber_lag_health_is_typed_and_does_not_become_ready() {
    let directory = tempdir().expect("temporary directory");
    const LAG: &str = "lag";
    assert!(
        !CORE_DEADLETTER_REASON_CODES.contains(&LAG),
        "HTTP AMBER lag coverage must stay outside the frozen dead-letter enum"
    );
    assert!(!is_core_deadletter_reason(LAG));
    let health_path = write_health_snapshot(
        directory.path(),
        "amber-lag.json",
        "HEALTH_STATE_AMBER",
        LAG,
    );
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        Some(&health_path),
        None,
    );

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["schema_version"], "hl.health.v1");
    assert_eq!(body["state"], "HEALTH_STATE_AMBER");
    assert_eq!(body["reason_code"], LAG);
    assert_ne!(
        body["reason_code"], "snapshot_invalid",
        "AMBER lag must stay typed, not snapshot_invalid"
    );
    assert_ne!(body["code"], "data_unavailable");

    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503, "AMBER lag must not become ready");
    assert_ne!(status, 200, "AMBER lag must not be /readyz 200");
    assert_eq!(
        body["schema_version"], "hl.health.v1",
        "AMBER lag /readyz 503 must stay typed health, not ApiError"
    );
    assert_ne!(
        body["schema_version"], "hl.api.error.v1",
        "AMBER lag /readyz 503 must not be hl.api.error.v1"
    );
    assert!(
        body.get("code").is_none(),
        "health body must not carry ApiError code, got {body}"
    );
    assert_ne!(
        body["state"], "HEALTH_STATE_GREEN",
        "lag must not be treated as GREEN-ready"
    );
    let aggregate = body["reason_code"].as_str().expect("aggregate reason");
    assert!(
        aggregate.contains(LAG),
        "readyz must surface typed {LAG}, got {aggregate}"
    );
    assert!(
        !aggregate.contains("snapshot_invalid"),
        "AMBER lag must not be rewritten as snapshot_invalid, got {aggregate}"
    );
}

#[tokio::test]
async fn unknown_amber_deadletter_sibling_is_snapshot_invalid_and_not_ready() {
    let directory = tempdir().expect("temporary directory");
    const UNKNOWN_AMBER: &str = "core.deadletter_invented";
    assert!(
        !CORE_DEADLETTER_REASON_CODES.contains(&UNKNOWN_AMBER),
        "HTTP unknown-AMBER coverage must use a sibling outside the frozen enum"
    );
    assert!(!is_core_deadletter_reason(UNKNOWN_AMBER));
    let health_path = write_health_snapshot(
        directory.path(),
        "unknown-amber.json",
        "HEALTH_STATE_AMBER",
        UNKNOWN_AMBER,
    );
    let state = state_from(
        directory.path(),
        "loopback-dev",
        None,
        Some(&health_path),
        None,
    );

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 503);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_invalid");
    assert_ne!(
        body["state"], "HEALTH_STATE_AMBER",
        "unknown AMBER sibling must not be serve-time accepted as typed AMBER"
    );

    let (status, body) = call(&state, "/readyz", &[]).await;
    assert_eq!(status, 503, "unknown AMBER sibling must not become ready");
    assert_eq!(body["state"], "HEALTH_STATE_RED");
}

#[tokio::test]
async fn stream_paths_and_websocket_upgrades_are_typed_501() {
    let directory = tempdir().expect("temporary directory");
    let state = state_from(directory.path(), "loopback-dev", None, None, None);

    for path in ["/v1/stream", "/v1/stream/canonical-events"] {
        let (status, body) = call(&state, path, &[]).await;
        assert_eq!(status, 501);
        assert_eq!(body["schema_version"], "hl.api.error.v1");
        assert_eq!(body["code"], "not_implemented");
        assert_eq!(body["reason_code"], "stream.websocket_unspecified");
    }

    let (status, body) = call(
        &state,
        "/v1/stream",
        &[("Upgrade", "websocket"), ("Connection", "Upgrade")],
    )
    .await;
    assert_eq!(status, 501);
    assert_eq!(body["reason_code"], "stream.websocket_unspecified");
}

#[tokio::test]
async fn credential_mode_rejects_missing_bearer_and_accepts_a_matching_token() {
    let directory = tempdir().expect("temporary directory");
    let token_path = directory.path().join("token");
    std::fs::write(&token_path, "local-dev-token\n").expect("write token");
    let health_path = directory.path().join("canonical-health.json");
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/api/canonical-health.json"),
        &health_path,
    )
    .expect("copy health fixture");
    let state = state_from(
        directory.path(),
        "credential",
        Some(&token_path),
        Some(&health_path),
        None,
    );
    assert_eq!(
        ApiConfig::from_path(&write_config(
            directory.path(),
            "127.0.0.1:0",
            "credential",
            Some(&token_path),
            Some(&health_path),
            None,
        ))
        .expect("config")
        .auth_mode(),
        AuthMode::Credential
    );

    let (status, body) = call(&state, "/healthz", &[]).await;
    assert_eq!(status, 200);
    assert_eq!(body["scope"], "api:process");

    let (status, body) = call(&state, "/v1/health", &[]).await;
    assert_eq!(status, 401);
    assert_eq!(body["reason_code"], "auth.missing_bearer");

    let (status, body) = call(
        &state,
        "/v1/health",
        &[("Authorization", "Bearer local-dev-token")],
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(body["scope"], "canonical");
}

#[test]
fn openapi_document_covers_router_paths_and_health_fields() {
    let document = openapi_yaml();
    assert_eq!(
        document,
        include_str!("../../../schemas/openapi/v1/openapi.yaml")
    );
    for path in ROUTER_PATHS {
        assert!(document.contains(path), "missing path {path}");
    }
    for field in HEALTH_JSON_FIELDS {
        assert!(document.contains(field), "missing health field {field}");
    }
    assert!(document.contains("HEALTH_STATE_GREEN"));
    for schema_id in CAPTURE_STATUS_SCHEMA_IDS {
        assert!(
            document.contains(schema_id),
            "missing capture status schema {schema_id}"
        );
    }
    assert!(document.contains("503"));
    for reason_code in SNAPSHOT_UNAVAILABLE_REASON_CODES {
        assert!(
            document.contains(reason_code),
            "missing snapshot reason {reason_code}"
        );
    }
    assert!(document.contains("501"));
    assert!(document.contains("query_budget_exceeded"));
    assert!(document.contains("429"));
    assert!(document.contains("max_rows"));
    assert!(document.contains("last-heartbeat"));
    for field in LAST_HEARTBEAT_THROUGHPUT_FIELDS {
        assert!(
            document.contains(field),
            "missing last-heartbeat field {field}"
        );
    }
    assert!(
        document.contains("not live-qualified"),
        "OpenAPI must name last-heartbeat throughput as not live-qualified"
    );
    assert_eq!(
        document.matches("live-qualified").count(),
        document.matches("not live-qualified").count(),
        "OpenAPI must not claim live-qualified sources"
    );
    assert!(document.contains("not invent fills"));
    assert!(document.contains("not a fills feed"));
    assert!(document.contains("CoreDeadLetterReasonCode"));
    let enum_values = core_deadletter_reason_openapi_enum(document)
        .expect("OpenAPI must define CoreDeadLetterReasonCode.enum");
    assert_eq!(
        enum_values, CORE_DEADLETTER_REASON_CODES,
        "YAML enum must match the frozen const; prose mentions do not count"
    );
    assert!(health_reason_code_is_unrestricted_string(document));
    assert!(
        document.contains("no inline enum"),
        "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
    );
    for reason_code in CORE_DEADLETTER_REASON_CODES {
        assert!(
            is_core_deadletter_reason(reason_code),
            "helper must accept {reason_code}"
        );
    }
    assert!(document.contains("LedgerUnsupportedEventReasonCode"));
    let ledger_enum = ledger_unsupported_event_reason_openapi_enum(document)
        .expect("OpenAPI must define LedgerUnsupportedEventReasonCode.enum");
    assert_eq!(
        ledger_enum, LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
        "YAML enum must match the frozen const; prose mentions do not count"
    );
    for reason_code in LEDGER_UNSUPPORTED_EVENT_REASON_CODES {
        assert!(
            is_ledger_unsupported_event_reason(reason_code),
            "helper must accept {reason_code}"
        );
        assert!(
            document.contains(reason_code),
            "OpenAPI must list {reason_code}"
        );
    }
    let committed_enum = committed_source_class_openapi_enum(document)
        .expect("OpenAPI must define CaptureStatusBase.active_committed_source.enum");
    assert_eq!(
        committed_enum, COMMITTED_SOURCE_CLASSES,
        "YAML enum must match the frozen const; prose mentions do not count"
    );
    for source in COMMITTED_SOURCE_CLASSES {
        assert!(document.contains(source), "OpenAPI must list {source}");
    }
    let source_health_enum = capture_source_health_openapi_enum(document)
        .expect("OpenAPI must define CaptureStatusBase.primary_source_health.enum");
    assert_eq!(
        source_health_enum, CAPTURE_SOURCE_HEALTH,
        "YAML enum must match the frozen const; prose mentions do not count"
    );
    let independent_health_enum = independent_source_health_openapi_enum(document)
        .expect("OpenAPI must define CaptureStatusBase.independent_source_health.enum");
    assert_eq!(
        independent_health_enum, CAPTURE_SOURCE_HEALTH,
        "optional independent_source_health must freeze the same closed set"
    );
    for health in CAPTURE_SOURCE_HEALTH {
        assert!(document.contains(health), "OpenAPI must list {health}");
    }
    let reconstruction_enum = restart_reconstruction_openapi_enum(document).expect(
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.restart_reconstruction.enum",
    );
    assert_eq!(
        reconstruction_enum, RESTART_RECONSTRUCTION,
        "YAML enum must match the frozen const; prose mentions do not count"
    );
    for reconstruction in RESTART_RECONSTRUCTION {
        assert!(
            document.contains(reconstruction),
            "OpenAPI must list {reconstruction}"
        );
    }
    let auxiliary_health_enum = auxiliary_source_health_openapi_enum(document)
        .expect("OpenAPI must define CaptureStatusBase.auxiliary_sources.items.health.enum");
    assert_eq!(
        auxiliary_health_enum, AUXILIARY_SOURCE_HEALTH,
        "YAML enum must match the frozen const; prose mentions do not count"
    );
    assert_ne!(
        auxiliary_health_enum.as_slice(),
        CAPTURE_SOURCE_HEALTH,
        "auxiliary health must not reuse the committed source health set"
    );
    for health in AUXILIARY_SOURCE_HEALTH {
        assert!(document.contains(health), "OpenAPI must list {health}");
    }
    let qualification_enum = auxiliary_source_qualification_openapi_enum(document)
        .expect("OpenAPI must define CaptureStatusBase.auxiliary_sources.items.qualification.enum");
    assert_eq!(
        qualification_enum, AUXILIARY_SOURCE_QUALIFICATION,
        "YAML enum must match the frozen const; prose mentions do not count"
    );
    for qualification in AUXILIARY_SOURCE_QUALIFICATION {
        assert!(
            document.contains(qualification),
            "OpenAPI must list {qualification}"
        );
    }
    assert!(
        auxiliary_source_id_is_required_string(document),
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.source_id as a required string"
    );
    assert!(
        auxiliary_source_spool_records_is_required_u64(document),
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.spool_records as a required u64 integer"
    );
    assert!(
        auxiliary_source_unarchived_records_is_required_u64(document),
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.unarchived_records as a required u64 integer"
    );
    assert!(
        auxiliary_source_partial_line_is_required_bool(document),
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.partial_line as a required boolean"
    );
    assert!(
        auxiliary_source_cursor_epoch_is_optional_string(document),
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.cursor_epoch as an optional string"
    );
    assert!(
        auxiliary_source_durable_offset_is_optional_u64(document),
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.durable_offset as an optional u64 integer"
    );
    assert!(
        auxiliary_source_local_sequence_is_optional_u64(document),
        "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.local_sequence as an optional u64 integer"
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
    assert_eq!(
        readyz_503_schema_ref(document),
        Some("#/components/schemas/HealthAssessment"),
        "/readyz 503 must $ref HealthAssessment, not ApiError"
    );
    assert_eq!(
        health_503_response_ref(document),
        Some("#/components/responses/Unavailable"),
        "/v1/health 503 must $ref named Unavailable, not HealthAssessment"
    );
    assert!(
        health_503_schema_ref(document).is_none(),
        "/v1/health 503 must stay a named Unavailable $ref, not an inline schema"
    );
    assert_ne!(
        health_503_schema_ref(document),
        Some("#/components/schemas/HealthAssessment"),
        "/v1/health 503 must not switch to HealthAssessment while the handler returns hl.api.error.v1"
    );
    assert_eq!(
        unavailable_response_schema_ref(document),
        Some("#/components/schemas/ApiError"),
        "shared Unavailable must stay ApiError for /v1/health 503"
    );
    assert_eq!(
        readyz_200_schema_ref(document),
        Some("#/components/schemas/HealthAssessment"),
        "/readyz 200 must $ref HealthAssessment, not ApiError"
    );
    assert_eq!(
        readyz_200_description(document).as_deref(),
        Some(READYZ_200_DESCRIPTION),
        "/readyz 200 path description must stay GREEN-only by exact equality"
    );
    assert_eq!(
        readyz_503_description(document).as_deref(),
        Some(READYZ_503_DESCRIPTION),
        "/readyz 503 path description must stay health-not-ApiError by exact equality"
    );
    assert_eq!(
        readyz_get_description(document).as_deref(),
        Some(READYZ_GET_DESCRIPTION),
        "/readyz GET operation description must stay health-not-ApiError by exact equality"
    );
}

#[tokio::test]
async fn served_openapi_matches_capture_status_v4_v5_and_503_contract() {
    let directory = tempdir().expect("temporary directory");
    let state = state_from(directory.path(), "loopback-dev", None, None, None);
    let (status, body) = call(&state, "/v1/openapi.yaml", &[]).await;
    assert_eq!(status, 200);
    let document = body.as_str().expect("OpenAPI YAML");
    assert_eq!(document, openapi_yaml());
    for schema_id in CAPTURE_STATUS_SCHEMA_IDS {
        assert!(
            document.contains(schema_id),
            "served OpenAPI missing {schema_id}"
        );
    }
    assert!(document.contains("503"));
    for reason_code in SNAPSHOT_UNAVAILABLE_REASON_CODES {
        assert!(
            document.contains(reason_code),
            "served OpenAPI missing {reason_code}"
        );
    }
    assert!(document.contains("last-heartbeat"));
    for field in LAST_HEARTBEAT_THROUGHPUT_FIELDS {
        assert!(
            document.contains(field),
            "served OpenAPI missing last-heartbeat field {field}"
        );
    }
    assert!(document.contains("not live-qualified"));
    assert!(document.contains("501"));
    assert!(document.contains("CoreDeadLetterReasonCode"));
    let enum_values = core_deadletter_reason_openapi_enum(document)
        .expect("served OpenAPI must define CoreDeadLetterReasonCode.enum");
    assert_eq!(
        enum_values, CORE_DEADLETTER_REASON_CODES,
        "served YAML enum must match the frozen const; prose mentions do not count"
    );
    assert!(health_reason_code_is_unrestricted_string(document));
    assert!(
        document.contains("no inline enum"),
        "served OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
    );
    assert!(document.contains("LedgerUnsupportedEventReasonCode"));
    let ledger_enum = ledger_unsupported_event_reason_openapi_enum(document)
        .expect("served OpenAPI must define LedgerUnsupportedEventReasonCode.enum");
    assert_eq!(
        ledger_enum, LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
        "served YAML enum must match the frozen const; prose mentions do not count"
    );
    let committed_enum = committed_source_class_openapi_enum(document)
        .expect("served OpenAPI must define CaptureStatusBase.active_committed_source.enum");
    assert_eq!(
        committed_enum, COMMITTED_SOURCE_CLASSES,
        "served YAML enum must match the frozen const; prose mentions do not count"
    );
    let source_health_enum = capture_source_health_openapi_enum(document)
        .expect("served OpenAPI must define CaptureStatusBase.primary_source_health.enum");
    assert_eq!(
        source_health_enum, CAPTURE_SOURCE_HEALTH,
        "served YAML enum must match the frozen const; prose mentions do not count"
    );
    let independent_health_enum = independent_source_health_openapi_enum(document)
        .expect("served OpenAPI must define CaptureStatusBase.independent_source_health.enum");
    assert_eq!(
        independent_health_enum, CAPTURE_SOURCE_HEALTH,
        "served optional independent_source_health must freeze the same closed set"
    );
    let reconstruction_enum = restart_reconstruction_openapi_enum(document).expect(
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.restart_reconstruction.enum",
    );
    assert_eq!(
        reconstruction_enum, RESTART_RECONSTRUCTION,
        "served YAML enum must match the frozen const; prose mentions do not count"
    );
    let auxiliary_health_enum = auxiliary_source_health_openapi_enum(document)
        .expect("served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.health.enum");
    assert_eq!(
        auxiliary_health_enum, AUXILIARY_SOURCE_HEALTH,
        "served YAML enum must match the frozen const; prose mentions do not count"
    );
    assert_ne!(
        auxiliary_health_enum.as_slice(),
        CAPTURE_SOURCE_HEALTH,
        "served auxiliary health must not reuse the committed source health set"
    );
    let qualification_enum = auxiliary_source_qualification_openapi_enum(document).expect(
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.qualification.enum",
    );
    assert_eq!(
        qualification_enum, AUXILIARY_SOURCE_QUALIFICATION,
        "served YAML enum must match the frozen const; prose mentions do not count"
    );
    assert!(
        auxiliary_source_id_is_required_string(document),
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.source_id as a required string"
    );
    assert!(
        auxiliary_source_spool_records_is_required_u64(document),
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.spool_records as a required u64 integer"
    );
    assert!(
        auxiliary_source_unarchived_records_is_required_u64(document),
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.unarchived_records as a required u64 integer"
    );
    assert!(
        auxiliary_source_partial_line_is_required_bool(document),
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.partial_line as a required boolean"
    );
    assert!(
        auxiliary_source_cursor_epoch_is_optional_string(document),
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.cursor_epoch as an optional string"
    );
    assert!(
        auxiliary_source_durable_offset_is_optional_u64(document),
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.durable_offset as an optional u64 integer"
    );
    assert!(
        auxiliary_source_local_sequence_is_optional_u64(document),
        "served OpenAPI must define CaptureStatusBase.auxiliary_sources.items.local_sequence as an optional u64 integer"
    );
    assert!(
        document.contains("core.deadletter_* family-prefix"),
        "served OpenAPI must name AMBER-family core.deadletter_* as the 503 prefix"
    );
    assert!(
        document.contains("Unknown HEALTH_STATE_RED codes stay 200 typed"),
        "served OpenAPI must name unknown RED as 200 typed fail-closed"
    );
    assert_eq!(
        readyz_503_schema_ref(document),
        Some("#/components/schemas/HealthAssessment"),
        "served /readyz 503 must $ref HealthAssessment, not ApiError"
    );
    assert_eq!(
        health_503_response_ref(document),
        Some("#/components/responses/Unavailable"),
        "served /v1/health 503 must $ref named Unavailable, not HealthAssessment"
    );
    assert!(
        health_503_schema_ref(document).is_none(),
        "served /v1/health 503 must stay a named Unavailable $ref, not an inline schema"
    );
    assert_ne!(
        health_503_schema_ref(document),
        Some("#/components/schemas/HealthAssessment"),
        "served /v1/health 503 must not switch to HealthAssessment while the handler returns hl.api.error.v1"
    );
    assert_eq!(
        unavailable_response_schema_ref(document),
        Some("#/components/schemas/ApiError"),
        "served Unavailable must stay ApiError for /v1/health 503"
    );
    assert_eq!(
        readyz_200_schema_ref(document),
        Some("#/components/schemas/HealthAssessment"),
        "served /readyz 200 must $ref HealthAssessment, not ApiError"
    );
    assert_eq!(
        readyz_200_description(document).as_deref(),
        Some(READYZ_200_DESCRIPTION),
        "served /readyz 200 path description must stay GREEN-only by exact equality"
    );
    assert_eq!(
        readyz_503_description(document).as_deref(),
        Some(READYZ_503_DESCRIPTION),
        "served /readyz 503 path description must stay health-not-ApiError by exact equality"
    );
    assert_eq!(
        readyz_get_description(document).as_deref(),
        Some(READYZ_GET_DESCRIPTION),
        "served /readyz GET operation description must stay health-not-ApiError by exact equality"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tcp_listener_serves_healthz() {
    let directory = tempdir().expect("temporary directory");
    let config_path = write_config(
        directory.path(),
        "127.0.0.1:0",
        "loopback-dev",
        None,
        None,
        None,
    );
    let handle = spawn_local(ApiConfig::from_path(&config_path).expect("config"))
        .await
        .expect("bind");
    let addr = handle.addr();
    let (status, body) = tcp_get(addr, "/healthz").await;
    assert_eq!(status, 200);
    let value: Value = serde_json::from_slice(&body).expect("health JSON");
    assert_eq!(value["schema_version"], "hl.health.v1");
    assert_eq!(value["scope"], "api:process");
}

async fn tcp_get(addr: SocketAddr, path: &str) -> (u16, Vec<u8>) {
    let mut last_error = None;
    for _ in 0..50 {
        match try_tcp_get(addr, path).await {
            Ok(result) => return result,
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                last_error = Some(error);
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(error) => panic!("request failed: {error}"),
        }
    }
    panic!("hl-api did not accept connections: {last_error:?}");
}

async fn try_tcp_get(addr: SocketAddr, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
    let mut stream = TcpStream::connect(addr).await?;
    let request = format!(
        "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(request.as_bytes()).await?;
    let mut buffer = Vec::new();
    stream.read_to_end(&mut buffer).await?;
    let text = std::str::from_utf8(&buffer)
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "response is not UTF-8"))?;
    let (header, body) = text.split_once("\r\n\r\n").ok_or_else(|| {
        std::io::Error::new(
            ErrorKind::InvalidData,
            format!("missing header terminator in {}", text.len()),
        )
    })?;
    let status = header
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| std::io::Error::new(ErrorKind::InvalidData, "missing status"))?
        .parse()
        .map_err(|_| std::io::Error::new(ErrorKind::InvalidData, "invalid status"))?;
    Ok((status, body.as_bytes().to_vec()))
}
