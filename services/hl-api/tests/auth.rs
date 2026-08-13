use hl_api::{ApiConfig, ConfigError};
use tempfile::tempdir;

const QUERY_BUDGETS: &str = concat!(
    "schema_version = \"hl.api.query_budgets.v1\"\n",
    "max_rows = 1024\n",
    "timeout_ms = 2000\n",
    "max_concurrency = 8\n",
);

fn write_query_budgets(directory: &std::path::Path) {
    std::fs::write(directory.join("api-query-budgets.toml"), QUERY_BUDGETS)
        .expect("write query budgets");
}

#[test]
fn credential_mode_fail_closes_without_a_credential_file() {
    let directory = tempdir().expect("temporary directory");
    write_query_budgets(directory.path());
    let error = ApiConfig::from_toml(
        "[listen]\nbind = \"127.0.0.1:8788\"\n\n[auth]\nmode = \"credential\"\n\n[query_budgets]\nfile = \"api-query-budgets.toml\"\n",
        directory.path(),
    )
    .expect_err("missing credentials must fail closed");
    assert_eq!(error, ConfigError::MissingCredentials);
    assert_eq!(error.reason_code(), "api_config.missing_credentials");
}

#[test]
fn credential_mode_fail_closes_when_the_credential_file_is_missing() {
    let directory = tempdir().expect("temporary directory");
    write_query_budgets(directory.path());
    let error = ApiConfig::from_toml(
        "[listen]\nbind = \"127.0.0.1:8788\"\n\n[auth]\nmode = \"credential\"\ncredential_file = \"missing.token\"\n\n[query_budgets]\nfile = \"api-query-budgets.toml\"\n",
        directory.path(),
    )
    .expect_err("absent credential file must fail closed");
    assert_eq!(error, ConfigError::MissingCredentials);
}

#[test]
fn loopback_dev_rejects_non_loopback_binds_and_credential_files() {
    let directory = tempdir().expect("temporary directory");
    write_query_budgets(directory.path());
    let error = ApiConfig::from_toml(
        "[listen]\nbind = \"0.0.0.0:8788\"\n\n[auth]\nmode = \"loopback-dev\"\n\n[query_budgets]\nfile = \"api-query-budgets.toml\"\n",
        directory.path(),
    )
    .expect_err("non-loopback loopback-dev must fail closed");
    assert_eq!(error, ConfigError::LoopbackRequired);

    std::fs::write(directory.path().join("token"), "secret").expect("write token");
    let error = ApiConfig::from_toml(
        "[listen]\nbind = \"127.0.0.1:8788\"\n\n[auth]\nmode = \"loopback-dev\"\ncredential_file = \"token\"\n\n[query_budgets]\nfile = \"api-query-budgets.toml\"\n",
        directory.path(),
    )
    .expect_err("loopback-dev must not accept a credential file");
    assert_eq!(error, ConfigError::CredentialFileNotAllowed);
}

#[test]
fn example_loopback_config_loads() {
    let config = ApiConfig::from_path(std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../config/api.example.toml"
    )))
    .expect("example config");
    assert!(config.bind().ip().is_loopback());
    assert!(config.canonical_health_path().is_some());
    assert!(config.capture_status_path().is_some());
    assert_eq!(config.query_budgets().max_rows(), 1024);
    assert_eq!(config.query_budgets().max_concurrency(), 8);
}

#[test]
fn loopback_config_fail_closes_without_query_budgets() {
    let directory = tempdir().expect("temporary directory");
    let error = ApiConfig::from_toml(
        "[listen]\nbind = \"127.0.0.1:8788\"\n\n[auth]\nmode = \"loopback-dev\"\n",
        directory.path(),
    )
    .expect_err("missing query budgets must fail closed");
    assert_eq!(error, ConfigError::MissingQueryBudgets);
    assert_eq!(error.reason_code(), "api_config.missing_query_budgets");
}
