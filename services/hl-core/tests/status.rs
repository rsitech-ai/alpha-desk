use std::time::Duration;

use hl_core::{CoreStatusHandle, StatusError, accept_status, serve_status};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_util::sync::CancellationToken;

async fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, String) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match TcpStream::connect(addr).await {
                Ok(mut stream) => {
                    stream
                        .write_all(
                            format!(
                                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
                            )
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
                    return (status, body);
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
    })
    .await
    .expect("http response")
}

fn json_from_http(body: &str) -> serde_json::Value {
    let json_start = body.find("\r\n\r\n").expect("header terminator") + 4;
    serde_json::from_str(&body[json_start..]).expect("JSON body")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn loopback_status_serves_ready_health_status_and_metrics() {
    let status = CoreStatusHandle::starting(Some(200));
    status.mark_ready();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_status(
        listener,
        status.clone(),
        cancellation.child_token(),
    ));

    let (health_code, health_body) = http_get(addr, "/healthz").await;
    assert_eq!(health_code, 200);
    let health = json_from_http(&health_body);
    assert_eq!(health["schema_version"], "hl.core.health.v1");
    assert_eq!(health["ok"], true);
    assert_eq!(health["ready"], true);
    assert_eq!(health["live_qualified"], false);
    assert_eq!(health["stage_2_qualified"], false);

    let (status_code, status_body) = http_get(addr, "/status").await;
    assert_eq!(status_code, 200);
    let value = json_from_http(&status_body);
    assert_eq!(value["schema_version"], "hl.core.status.v1");
    assert_eq!(value["ready"], true);
    assert_eq!(value["last_applied_watermark"], 200);
    assert_eq!(value["live_qualified"], false);
    assert_eq!(value["stage_2_qualified"], false);
    assert!(value.get("fail_closed_reason").is_none());

    let (metrics_code, metrics_body) = http_get(addr, "/metrics").await;
    assert_eq!(metrics_code, 200);
    assert!(metrics_body.contains("hl_core_ready 1"));
    assert!(metrics_body.contains("hl_core_live_qualified 0"));
    assert!(metrics_body.contains("hl_core_stage_2_qualified 0"));
    assert!(metrics_body.contains("hl_core_last_applied_watermark 200"));

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn not_ready_fail_closed_status_uses_503_and_keeps_qualification_false() {
    let status = CoreStatusHandle::starting(None);
    status.fail_closed("ledger.unsupported_event");
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("local addr");
    let cancellation = CancellationToken::new();
    let server = tokio::spawn(accept_status(listener, status, cancellation.child_token()));

    let (health_code, health_body) = http_get(addr, "/healthz").await;
    assert_eq!(health_code, 503);
    let health = json_from_http(&health_body);
    assert_eq!(health["ok"], false);
    assert_eq!(health["ready"], false);
    assert_eq!(health["reason_code"], "ledger.unsupported_event");
    assert_eq!(health["live_qualified"], false);
    assert_eq!(health["stage_2_qualified"], false);

    let (status_code, status_body) = http_get(addr, "/status").await;
    assert_eq!(status_code, 200);
    let value = json_from_http(&status_body);
    assert_eq!(value["ready"], false);
    assert_eq!(value["fail_closed_reason"], "ledger.unsupported_event");
    assert_eq!(value["live_qualified"], false);
    assert_eq!(value["stage_2_qualified"], false);

    let (metrics_code, metrics_body) = http_get(addr, "/metrics").await;
    assert_eq!(metrics_code, 200);
    assert!(metrics_body.contains("hl_core_ready 0"));
    assert!(metrics_body.contains("hl_core_live_qualified 0"));
    assert!(metrics_body.contains("hl_core_stage_2_qualified 0"));
    assert!(!metrics_body.contains("hl_core_last_applied_watermark"));

    cancellation.cancel();
    server.await.expect("join").expect("serve stops");
}

#[tokio::test]
async fn serve_status_rejects_non_loopback_bind() {
    let error = serve_status(
        CoreStatusHandle::starting(None),
        "8.8.8.8:8742".parse().expect("addr"),
        CancellationToken::new(),
    )
    .await
    .expect_err("non-loopback");
    assert_eq!(error, StatusError::UnsafeBind);
    assert_eq!(error.reason_code(), "core_status.unsafe_bind");
}

#[tokio::test]
async fn serve_status_rejects_unspecified_bind() {
    let error = serve_status(
        CoreStatusHandle::starting(None),
        "0.0.0.0:8742".parse().expect("addr"),
        CancellationToken::new(),
    )
    .await;
    assert_eq!(error, Err(StatusError::UnsafeBind));
}
