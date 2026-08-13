use std::path::Path;
use std::time::Duration;

use bytes::Bytes;
use hl_api::{ApiConfig, AppState, BudgetError, ConfigError};
use http::Request;
use serde_json::Value;
use tempfile::tempdir;

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

fn call(app: &AppState, path: &str) -> (u16, Value) {
    let request = Request::builder()
        .method("GET")
        .uri(path)
        .body(Bytes::new())
        .expect("request");
    let (status, body) = app.handle(request);
    let value = serde_json::from_slice(&body).expect("JSON error body");
    (status.as_u16(), value)
}

#[test]
fn oversized_limit_is_typed_400_query_budget_exceeded() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 2, 2000, 8);

    let (status, body) = call(&app, "/v1/health?limit=3");
    assert_eq!(status, 400);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "query_budget_exceeded");
    assert_eq!(body["reason_code"], "query.max_rows");

    let (status, body) = call(&app, "/v1/stream?limit=999999");
    assert_eq!(status, 400);
    assert_eq!(body["code"], "query_budget_exceeded");
    assert_eq!(body["reason_code"], "query.max_rows");
}

#[test]
fn limit_within_budget_reaches_snapshot_fail_closed_not_budget() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 2, 2000, 8);
    let (status, body) = call(&app, "/v1/health?limit=2");
    assert_eq!(status, 503);
    assert_eq!(body["code"], "data_unavailable");
    assert_eq!(body["reason_code"], "snapshot_missing");

    let (status, body) = call(&app, "/v1/stream?limit=1");
    assert_eq!(status, 501);
    assert_eq!(body["reason_code"], "stream.websocket_unspecified");
}

#[test]
fn offset_pagination_is_typed_400() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 8, 2000, 8);
    let (status, body) = call(&app, "/v1/health?offset=1");
    assert_eq!(status, 400);
    assert_eq!(body["code"], "invalid_query");
    assert_eq!(body["reason_code"], "query.offset_forbidden");
}

#[test]
fn exhausted_concurrency_is_typed_429() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 8, 2000, 1);
    let _permit = app
        .query_budgets()
        .try_acquire()
        .expect("hold the only slot");
    let (status, body) = call(&app, "/v1/health");
    assert_eq!(status, 429);
    assert_eq!(body["schema_version"], "hl.api.error.v1");
    assert_eq!(body["code"], "query_budget_exceeded");
    assert_eq!(body["reason_code"], "query.concurrency");
}

#[test]
fn healthz_is_not_subject_to_query_budgets() {
    let directory = tempdir().expect("temporary directory");
    let app = state(directory.path(), 1, 2000, 1);
    let _permit = app
        .query_budgets()
        .try_acquire()
        .expect("hold the only slot");
    let (status, body) = call(&app, "/healthz");
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
        .execute(None, async {
            tokio::time::sleep(Duration::from_millis(200)).await;
            1_u8
        })
        .await
        .expect_err("timeout must fail closed");
    assert_eq!(error, BudgetError::Timeout);
    assert_eq!(error.status().as_u16(), 429);
    assert_eq!(error.code(), "query_budget_exceeded");
    assert_eq!(error.reason_code(), "query.timeout");
}
