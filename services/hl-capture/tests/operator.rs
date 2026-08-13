use std::fs;
use std::path::PathBuf;
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

async fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, String) {
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
    serde_json::from_value(value).expect("valid v4 snapshot")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn operator_status_serves_v4_json_health_and_sse() {
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
    assert_eq!(health_status, 200);
    assert!(health_body.contains("\"schema_version\":\"hl.capture.health.v1\""));
    assert!(health_body.contains("\"health\":\"yellow\""));
    assert!(health_body.contains("\"ready\":false"));

    let (status_code, status_body) = http_get(addr, "/status").await;
    assert_eq!(status_code, 200);
    let value = json_from_http(&status_body);
    assert_eq!(value["schema_version"], "hl.capture.status.v4");
    assert_eq!(value["durable_height"], 12);
    assert_eq!(value["capture_backlog_records"], 1);
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
    assert!(sse_text.contains("hl.capture.status.v4"));

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
