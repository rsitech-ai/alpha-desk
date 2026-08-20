use std::io::ErrorKind;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use bytes::Bytes;
use hl_api::{ApiConfig, AppState, BudgetError, ConfigError, spawn_state};
use http::Request;
use serde_json::Value;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

fn write_budgets(directory: &Path, max_rows: u32, timeout_ms: u32, max_concurrency: u32) {
    std::fs::write(
        directory.join("api-query-budgets.toml"),
        format!(
            "schema_version = \"hl.api.query_budgets.v1\"\nmax_rows = {max_rows}\ntimeout_ms = {timeout_ms}\nmax_concurrency = {max_concurrency}\n"
        ),
    )
    .expect("write query budgets");
}

fn write_config(
    directory: &Path,
    max_rows: u32,
    timeout_ms: u32,
    max_concurrency: u32,
) -> std::path::PathBuf {
    write_budgets(directory, max_rows, timeout_ms, max_concurrency);
    let config_path = directory.join("api.toml");
    std::fs::write(
        &config_path,
        "[listen]\nbind = \"127.0.0.1:0\"\n\n[auth]\nmode = \"loopback-dev\"\n\n[query_budgets]\nfile = \"api-query-budgets.toml\"\n",
    )
    .expect("write config");
    config_path
}

fn state(directory: &Path, max_rows: u32, timeout_ms: u32, max_concurrency: u32) -> AppState {
    AppState::from_config(
        ApiConfig::from_path(&write_config(
            directory,
            max_rows,
            timeout_ms,
            max_concurrency,
        ))
        .expect("config"),
    )
}

async fn call(app: &AppState, path: &str) -> (u16, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Bytes::new())
        .expect("request");
    let (status, body) = app.handle(request).await;
    let value = serde_json::from_slice(&body).expect("JSON error body");
    (status.as_u16(), value)
}

struct BlockingSnapshot {
    release: Arc<(Mutex<bool>, Condvar)>,
}

impl BlockingSnapshot {
    fn new() -> Self {
        Self {
            release: Arc::new((Mutex::new(false), Condvar::new())),
        }
    }

    fn hook(&self) -> impl Fn() + Send + Sync + 'static {
        let release = Arc::clone(&self.release);
        move || {
            let (lock, cvar) = &*release;
            let mut done = lock.lock().expect("snapshot double lock");
            while !*done {
                done = cvar.wait(done).expect("snapshot double wait");
            }
        }
    }

    fn release(&self) {
        let (lock, cvar) = &*self.release;
        let mut done = lock.lock().expect("snapshot double lock");
        *done = true;
        cvar.notify_all();
    }
}

impl Drop for BlockingSnapshot {
    fn drop(&mut self) {
        self.release();
    }
}

#[tokio::test]
async fn oversized_limit_is_typed_400_query_budget_exceeded() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 2, 2000, 8);

    let (status, body) = call(&app, "/v1/health?limit=3").await;
    assert_eq!(status, 400);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "query_budget_exceeded");
    assert_eq!(body["reason_code"], "query.max_rows");

    let (status, body) = call(&app, "/v1/stream?limit=999999").await;
    assert_eq!(status, 400);
    assert_eq!(body["code"], "query_budget_exceeded");
    assert_eq!(body["reason_code"], "query.max_rows");
}

#[tokio::test]
async fn limit_within_budget_reaches_snapshot_fail_closed_not_budget() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 2, 2000, 8);
    let (status, body) = call(&app, "/v1/health?limit=2").await;
    assert_eq!(status, 503);
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_missing");

    let (status, body) = call(&app, "/v1/stream?limit=1").await;
    assert_eq!(status, 501);
    assert_eq!(body["reason_code"], "stream.websocket_unspecified");
}

#[tokio::test]
async fn offset_pagination_is_typed_400() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 8, 2000, 8);
    let (status, body) = call(&app, "/v1/health?offset=1").await;
    assert_eq!(status, 400);
    assert_eq!(body["code"], "invalid_query");
    assert_eq!(body["reason_code"], "query.offset_forbidden");
}

#[tokio::test]
async fn exhausted_concurrency_is_typed_429() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 8, 2000, 1);
    let _permit = app
        .query_budgets()
        .try_acquire()
        .expect("hold the only slot");
    let (status, body) = call(&app, "/v1/health").await;
    assert_eq!(status, 429);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "query_budget_exceeded");
    assert_eq!(body["reason_code"], "query.concurrency");
}

#[tokio::test]
async fn healthz_is_not_subject_to_query_budgets() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 1, 2000, 1);
    let _permit = app
        .query_budgets()
        .try_acquire()
        .expect("hold the only slot");
    let (status, body) = call(&app, "/healthz").await;
    assert_eq!(status, 200);
    assert_eq!(body["scope"], "api:process");
}

#[test]
fn missing_or_zero_budget_file_fail_closes_at_startup() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("api.toml");
    std::fs::write(
        &config_path,
        "[listen]\nbind = \"127.0.0.1:0\"\n\n[auth]\nmode = \"loopback-dev\"\n\n[query_budgets]\nfile = \"api-query-budgets.toml\"\n",
    )
    .expect("write config");
    let error = ApiConfig::from_path(&config_path).expect_err("missing file");
    assert_eq!(error, ConfigError::MissingQueryBudgets);

    write_budgets(directory.path(), 0, 2000, 8);
    let error = ApiConfig::from_path(&config_path).expect_err("zero max_rows");
    assert_eq!(error, ConfigError::InvalidQueryBudgets);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn timeout_budget_fail_closes_with_429() {
    let directory = tempdir().expect("temporary directory");
    let config = ApiConfig::from_path(&write_config(directory.path(), 8, 20, 8)).expect("config");
    let error = config
        .query_budgets()
        .execute(None, || {
            std::thread::sleep(Duration::from_millis(200));
            1_u8
        })
        .await
        .expect_err("timeout must fail closed");
    assert_eq!(error, BudgetError::Timeout);
    assert_eq!(error.status().as_u16(), 429);
    assert_eq!(error.code(), "query_budget_exceeded");
    assert_eq!(error.reason_code(), "query.timeout");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn handle_timeout_abandons_blocking_snapshot_reads() {
    let directory = tempdir().expect("temporary directory");
    let blocking = BlockingSnapshot::new();
    let app = state(directory.path(), 8, 20, 8).with_snapshot_read_hook(blocking.hook());
    let (status, body) = tokio::time::timeout(Duration::from_secs(2), call(&app, "/v1/health"))
        .await
        .expect("handle must return when snapshot I/O blocks past timeout_ms");
    assert_eq!(status, 429);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "query_budget_exceeded");
    assert_eq!(body["reason_code"], "query.timeout");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn http_timeout_abandons_blocking_snapshot_io() {
    let directory = tempdir().expect("temporary directory");
    let blocking = BlockingSnapshot::new();
    let handle =
        spawn_state(state(directory.path(), 8, 20, 8).with_snapshot_read_hook(blocking.hook()))
            .await
            .expect("bind");
    let (status, body) =
        tokio::time::timeout(Duration::from_secs(2), tcp_get(handle.addr(), "/v1/health"))
            .await
            .expect("HTTP must return when snapshot I/O blocks past timeout_ms");
    assert_eq!(status, 429);
    let value: Value = serde_json::from_slice(&body).expect("JSON error body");
    assert_eq!(value["schema_version"], "hl.api.error.v1");
    assert_eq!(value["code"], "query_budget_exceeded");
    assert_eq!(value["reason_code"], "query.timeout");
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
