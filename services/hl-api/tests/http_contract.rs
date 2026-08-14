use std::io::{ErrorKind, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use hl_api::{
    ApiConfig, AppState, AuthMode, CAPTURE_STATUS_SCHEMA_IDS, CORE_DEADLETTER_REASON_CODES,
    HEALTH_JSON_FIELDS, LAST_HEARTBEAT_THROUGHPUT_FIELDS, ROUTER_PATHS,
    SNAPSHOT_UNAVAILABLE_REASON_CODES, core_deadletter_reason_openapi_enum,
    health_reason_code_is_unrestricted_string, is_core_deadletter_reason, openapi_yaml,
    spawn_local,
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
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_missing");
    assert!(body.get("state").is_none());
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
    for reason_code in CORE_DEADLETTER_REASON_CODES {
        assert!(
            is_core_deadletter_reason(reason_code),
            "helper must accept {reason_code}"
        );
    }
    assert!(document.contains("Unknown codes fail closed"));
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
