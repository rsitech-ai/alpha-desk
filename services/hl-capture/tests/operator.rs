use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use domain_types::{BlockHeight, ChainId, KnownTime};
use hl_capture::{
    CaptureHealth, CaptureSourceHealth, CaptureStatus, CommittedSourceClass, OperatorError,
    StatusWriter, accept_operator_status, serve_operator_status,
};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

async fn http_get(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut stream = TcpStream::connect(addr).await.expect("connect");
    stream
        .write_all(
            format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .await
        .expect("write request");
    stream.flush().await.expect("flush request");
    let mut body = Vec::new();
    stream.read_to_end(&mut body).await.expect("read response");
    let body = String::from_utf8(body).expect("UTF-8 response");
    let status = body
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .expect("HTTP status");
    (status, body)
}

fn json_from_http(body: &str) -> serde_json::Value {
    let json_start = body.find("\r\n\r\n").expect("header terminator") + 4;
    serde_json::from_str(&body[json_start..]).expect("JSON body")
}

fn http_body_bytes(body: &str) -> &[u8] {
    let json_start = body.find("\r\n\r\n").expect("header terminator") + 4;
    &body.as_bytes()[json_start..]
}

fn capture_fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/capture")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("read fixture {name}: {error}"))
}

async fn serve_fixture(
    name: &str,
) -> (
    tempfile::TempDir,
    SocketAddr,
    CancellationToken,
    tokio::task::JoinHandle<Result<(), OperatorError>>,
) {
    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("capture-status.json");
    fs::write(&status_path, capture_fixture(name)).expect("write fixture");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path,
        cancellation.child_token(),
    ));
    (directory, addr, cancellation, server)
}

fn sample_status() -> CaptureStatus {
    let status = CaptureStatus::new(
        KnownTime::from_unix_micros(1_000).expect("time"),
        "build-operator",
        ChainId::new("mainnet").expect("chain"),
        CaptureHealth::Yellow,
    )
    .with_readiness(false)
    .with_source_state(
        CommittedSourceClass::LocallyVerifiedCommitted,
        CaptureSourceHealth::Starting,
        None,
        None,
        None,
    )
    .with_durable_height(Some(BlockHeight::new(12)))
    .with_capture_capacity(1, Some(BlockHeight::new(12)), Some(4_200))
    .with_throughput(3, 1)
    .with_last_error_reason(Some("capture_runtime.recovering".to_owned()));
    let mut value = serde_json::to_value(&status).expect("serialize sample");
    value["auxiliary_sources"] = serde_json::json!([{
        "source_id": "node-fills",
        "health": "healthy",
        "qualification": "unqualified",
        "cursor_epoch": "node-file-v1:epoch-a",
        "tail_cursor_epoch": "node-file-v1:epoch-a",
        "durable_offset": 47,
        "local_sequence": 3,
        "spool_records": 3,
        "unarchived_records": 0,
        "partial_line": false,
        "last_durable_wall_micros": 1_000,
        "restart_reconstruction": "incomplete"
    }]);
    serde_json::from_value(value).expect("valid v5 snapshot")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_status_serves_written_v5_json_health_and_sse() {
    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("capture-status.json");
    StatusWriter::new(status_path.clone())
        .expect("status writer")
        .write(&sample_status())
        .expect("write status");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path,
        cancellation.child_token(),
    ));

    let (health_status, health_body) = http_get(addr, "/healthz").await;
    assert_eq!(health_status, 503);
    let health = json_from_http(&health_body);
    assert_eq!(health["schema_version"], "hl.capture.health.v1");
    assert_eq!(health["ok"], false);
    assert_eq!(health["reason_code"], "capture_health.not_ready");
    assert_eq!(health["ready"], false);

    let (status_code, status_body) = http_get(addr, "/status").await;
    assert_eq!(status_code, 200);
    let value = json_from_http(&status_body);
    assert_eq!(value["schema_version"], "hl.capture.status.v5");
    assert_eq!(value["maintenance"]["enabled"], false);
    assert_eq!(value["maintenance"]["retention_authorized"], false);
    assert_eq!(value["durable_height"], 12);
    assert_eq!(value["capture_backlog_records"], 1);
    assert_eq!(value["throughput_records_per_sec"], 3);
    assert_eq!(value["throughput_blocks_per_sec"], 1);
    assert_eq!(
        value["auxiliary_sources"][0]["restart_reconstruction"],
        "incomplete"
    );

    let mut events = TcpStream::connect(addr).await.expect("sse connect");
    events
        .write_all(b"GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .expect("sse request");
    events.flush().await.expect("sse flush");
    let mut sse_body = Vec::new();
    let mut buffer = [0_u8; 1_024];
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let read = events.read(&mut buffer).await.expect("sse read");
            assert_ne!(read, 0, "sse closed before the first status event");
            sse_body.extend_from_slice(&buffer[..read]);
            if std::str::from_utf8(&sse_body)
                .unwrap_or("")
                .contains("\"restart_reconstruction\":\"incomplete\"")
            {
                break;
            }
        }
    })
    .await
    .expect("first SSE status event");
    let sse_text = std::str::from_utf8(&sse_body).expect("UTF-8 SSE");
    assert!(sse_text.contains("event: status"));
    assert!(sse_text.contains("hl.capture.status.v5"));

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_status_sse_emits_when_the_status_file_changes() {
    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("capture-status.json");
    let writer = StatusWriter::new(status_path.clone()).expect("status writer");
    writer.write(&sample_status()).expect("write status");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path,
        cancellation.child_token(),
    ));

    let mut events = TcpStream::connect(addr).await.expect("sse connect");
    events
        .write_all(b"GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .expect("sse request");
    events.flush().await.expect("sse flush");

    let mut sse_body = Vec::new();
    let mut buffer = [0_u8; 1_024];
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let read = events.read(&mut buffer).await.expect("sse read");
            assert_ne!(read, 0, "sse closed before the first status event");
            sse_body.extend_from_slice(&buffer[..read]);
            if std::str::from_utf8(&sse_body)
                .unwrap_or("")
                .contains("\"durable_height\":12")
            {
                break;
            }
        }
    })
    .await
    .expect("first SSE status event");

    writer
        .write(
            &CaptureStatus::new(
                KnownTime::from_unix_micros(2_000).expect("time"),
                "build-operator",
                ChainId::new("mainnet").expect("chain"),
                CaptureHealth::Yellow,
            )
            .with_durable_height(Some(BlockHeight::new(13)))
            .with_capture_capacity(2, Some(BlockHeight::new(13)), Some(4_200)),
        )
        .expect("rewrite status");

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if std::str::from_utf8(&sse_body)
                .unwrap_or("")
                .contains("\"durable_height\":13")
            {
                break;
            }
            let read = events.read(&mut buffer).await.expect("sse read");
            assert_ne!(read, 0, "sse closed before the updated status event");
            sse_body.extend_from_slice(&buffer[..read]);
        }
    })
    .await
    .expect("updated SSE status event");

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_status_reports_disconnected_when_the_snapshot_is_missing() {
    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("missing-status.json");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path,
        cancellation.child_token(),
    ));

    let (status, body) = http_get(addr, "/status").await;
    assert_eq!(status, 503);
    assert!(body.contains("capture_status."));
    let (health_status, health_body) = http_get(addr, "/healthz").await;
    assert_eq!(health_status, 503);
    assert!(health_body.contains("\"ok\":false"));

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_status_fails_closed_on_invalid_status_json() {
    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("capture-status.json");
    fs::write(&status_path, "{not-json").expect("write invalid status");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path,
        cancellation.child_token(),
    ));

    let (status, body) = http_get(addr, "/status").await;
    assert_eq!(status, 503);
    let value = json_from_http(&body);
    assert_eq!(value["schema_version"], "hl.capture.error.v1");
    assert_eq!(value["reason_code"], "capture_status.serialization");

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_status_serves_v4_and_v5_fixtures_as_read() {
    let (_v4_dir, v4_addr, v4_cancel, v4_server) = serve_fixture("status-v4.json").await;
    let (v4_status, v4_body) = http_get(v4_addr, "/status").await;
    assert_eq!(v4_status, 200);
    assert_eq!(http_body_bytes(&v4_body), capture_fixture("status-v4.json"));
    let v4_value = json_from_http(&v4_body);
    assert_eq!(v4_value["schema_version"], "hl.capture.status.v4");
    assert!(v4_value.get("maintenance").is_none());
    let (v4_health_status, v4_health_body) = http_get(v4_addr, "/healthz").await;
    assert_eq!(v4_health_status, 503);
    let v4_health = json_from_http(&v4_health_body);
    assert_eq!(v4_health["ok"], false);
    assert_eq!(v4_health["reason_code"], "capture_health.not_ready");
    assert_eq!(v4_health["ready"], false);

    let mut events = TcpStream::connect(v4_addr).await.expect("sse connect");
    events
        .write_all(b"GET /events HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n")
        .await
        .expect("sse request");
    events.flush().await.expect("sse flush");
    let mut sse_body = Vec::new();
    let mut buffer = [0_u8; 1_024];
    let mut expected = b"event: status\ndata: ".to_vec();
    expected.extend_from_slice(&capture_fixture("status-v4.json"));
    expected.extend_from_slice(b"\n\n");
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let read = events.read(&mut buffer).await.expect("sse read");
            assert_ne!(read, 0, "sse closed before the first status event");
            sse_body.extend_from_slice(&buffer[..read]);
            if sse_body
                .windows(expected.len())
                .any(|window| window == expected.as_slice())
            {
                break;
            }
        }
    })
    .await
    .expect("SSE /events leftover v4 bytes as-read");
    v4_cancel.cancel();
    v4_server.await.expect("join").expect("serve stops");

    let (_v5_dir, v5_addr, v5_cancel, v5_server) = serve_fixture("status-v5.json").await;
    let (v5_status, v5_body) = http_get(v5_addr, "/status").await;
    assert_eq!(v5_status, 200);
    assert_eq!(http_body_bytes(&v5_body), capture_fixture("status-v5.json"));
    let v5_value = json_from_http(&v5_body);
    assert_eq!(v5_value["schema_version"], "hl.capture.status.v5");
    assert_eq!(v5_value["maintenance"]["enabled"], true);
    assert_eq!(v5_value["maintenance"]["retention_authorized"], false);
    assert_eq!(
        v5_value["auxiliary_sources"][0]["restart_reconstruction"],
        "complete"
    );
    let (health_status, health_body) = http_get(v5_addr, "/healthz").await;
    assert_eq!(health_status, 200);
    assert!(health_body.contains("\"health\":\"green\""));
    assert!(health_body.contains("\"ready\":true"));
    v5_cancel.cancel();
    v5_server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_healthz_rejects_valid_v5_when_not_ready() {
    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("capture-status.json");
    let status = CaptureStatus::new(
        KnownTime::from_unix_micros(1_000).expect("time"),
        "build-operator",
        ChainId::new("mainnet").expect("chain"),
        CaptureHealth::Green,
    )
    .with_readiness(false)
    .with_source_state(
        CommittedSourceClass::LocallyVerifiedCommitted,
        CaptureSourceHealth::Healthy,
        None,
        None,
        None,
    );
    StatusWriter::new(status_path.clone())
        .expect("status writer")
        .write(&status)
        .expect("write status");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path,
        cancellation.child_token(),
    ));

    let (status_code, status_body) = http_get(addr, "/status").await;
    assert_eq!(status_code, 200);
    let value = json_from_http(&status_body);
    assert_eq!(value["schema_version"], "hl.capture.status.v5");
    assert_eq!(value["maintenance"]["enabled"], false);
    assert_eq!(value["ready"], false);

    let (health_status, health_body) = http_get(addr, "/healthz").await;
    assert_eq!(health_status, 503);
    let health = json_from_http(&health_body);
    assert_eq!(health["ok"], false);
    assert_eq!(health["reason_code"], "capture_health.not_ready");
    assert_eq!(health["ready"], false);

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_healthz_ready_accepts_v5_idle_maintenance() {
    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("capture-status.json");
    let status = CaptureStatus::new(
        KnownTime::from_unix_micros(1_000).expect("time"),
        "build-operator",
        ChainId::new("mainnet").expect("chain"),
        CaptureHealth::Green,
    )
    .with_readiness(true)
    .with_source_state(
        CommittedSourceClass::LocallyVerifiedCommitted,
        CaptureSourceHealth::Healthy,
        None,
        None,
        None,
    );
    StatusWriter::new(status_path.clone())
        .expect("status writer")
        .write(&status)
        .expect("write status");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path,
        cancellation.child_token(),
    ));

    let (status_code, status_body) = http_get(addr, "/status").await;
    assert_eq!(status_code, 200);
    let value = json_from_http(&status_body);
    assert_eq!(value["schema_version"], "hl.capture.status.v5");
    assert_eq!(value["maintenance"]["enabled"], false);
    assert_eq!(value["maintenance"]["retention_authorized"], false);
    assert_eq!(value["ready"], true);

    let (health_status, health_body) = http_get(addr, "/healthz").await;
    assert_eq!(health_status, 200);
    let health = json_from_http(&health_body);
    assert_eq!(health["ok"], true);
    assert_eq!(health["health"], "green");
    assert_eq!(health["ready"], true);

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_status_rejects_v4_smuggled_maintenance_v5_without_and_unknown_schema() {
    let (_smuggled_dir, smuggled_addr, smuggled_cancel, smuggled_server) =
        serve_fixture("status-v4-smuggled-maintenance.json").await;
    let (status, body) = http_get(smuggled_addr, "/status").await;
    assert_eq!(status, 503);
    let value = json_from_http(&body);
    assert_eq!(value["reason_code"], "capture_status.invalid_schema");
    smuggled_cancel.cancel();
    smuggled_server.await.expect("join").expect("serve stops");

    let directory = tempdir().expect("temp directory");
    let status_path = directory.path().join("capture-status.json");
    let mut v5 = serde_json::from_slice::<serde_json::Value>(&capture_fixture("status-v4.json"))
        .expect("v4 json");
    v5["schema_version"] = serde_json::json!("hl.capture.status.v5");
    fs::write(&status_path, serde_json::to_vec(&v5).expect("encode")).expect("write v5 without");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_operator_status(
        listener,
        status_path.clone(),
        cancellation.child_token(),
    ));
    let (status, body) = http_get(addr, "/status").await;
    assert_eq!(status, 503);
    assert_eq!(
        json_from_http(&body)["reason_code"],
        "capture_status.invalid_schema"
    );

    v5["schema_version"] = serde_json::json!("hl.capture.status.v6");
    fs::write(&status_path, serde_json::to_vec(&v5).expect("encode")).expect("write unknown");
    let (status, body) = http_get(addr, "/status").await;
    assert_eq!(status, 503);
    assert_eq!(
        json_from_http(&body)["reason_code"],
        "capture_status.invalid_schema"
    );

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test]
async fn serve_operator_status_rejects_non_loopback_bind() {
    let error = serve_operator_status(
        PathBuf::from("state/capture-status.json"),
        "8.8.8.8:8741".parse().expect("addr"),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(error, Err(OperatorError::UnsafeBind));
}

#[tokio::test]
async fn info_budget_route_serves_snapshot_file() {
    let (directory, addr, cancellation, server) = serve_fixture("status-v5.json").await;
    let (status, _) = http_get(addr, "/info-budget").await;
    assert_eq!(status, 404);

    let mut budget =
        hl_capture::RequestBudget::official("official-info", 75, 0, 1).expect("budget");
    hl_capture::write_info_budget_snapshot(
        &directory.path().join("capture-status.json"),
        &budget.snapshot(0),
    )
    .expect("write budget");
    let (status, body) = http_get(addr, "/info-budget").await;
    assert_eq!(status, 200);
    let json = json_from_http(&body);
    assert_eq!(json["schema_version"], "hl.capture.info-budget.v1");
    assert_eq!(json["egress_id"], "official-info");
    assert_eq!(json["ceiling_weight_per_minute"], 1200);

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test]
async fn ws_plan_route_serves_snapshot_file() {
    let (directory, addr, cancellation, server) = serve_fixture("status-v5.json").await;
    let (status, _) = http_get(addr, "/ws-plan").await;
    assert_eq!(status, 404);

    let plan = hl_capture::plan_subscriptions(
        hl_capture::PlannerConfig::official(),
        hl_capture::PlannerInput::new(vec![hl_capture::SubscriptionDemand::new("allMids")]),
    );
    let body = hl_capture::encode_ws_plan_status(&plan, &[]).expect("encode");
    hl_capture::write_ws_plan_snapshot(&directory.path().join("capture-status.json"), &body)
        .expect("write plan");
    let (status, response) = http_get(addr, "/ws-plan").await;
    assert_eq!(status, 200);
    let json = json_from_http(&response);
    assert_eq!(json["schema_version"], "hl.capture.ws-plan.v1");
    assert_eq!(json["max_connections"], 10);
    assert_eq!(json["reserved_connections"], 1);
    let (status, _) = http_get(addr, "/status").await;
    assert_eq!(status, 200);

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}
