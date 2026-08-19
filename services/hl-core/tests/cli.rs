use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

use tempfile::tempdir;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hl-core"))
}

#[test]
fn missing_command_is_a_usage_error_without_a_panic() {
    let output = binary().output().expect("run hl-core");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("usage: hl-core"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn check_config_emits_stable_machine_readable_success() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("core.toml");
    fs::write(
        &config_path,
        include_bytes!("../../../config/core.example.toml"),
    )
    .expect("write config");

    let output = binary()
        .args(["check-config", "--config"])
        .arg(&config_path)
        .output()
        .expect("run check-config");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("machine-readable check result");
    assert_eq!(value["schema_version"], "hl.core.check.v1");
    assert_eq!(value["valid"], true);
}

#[test]
fn check_config_missing_store_reports_only_a_stable_reason_code() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("core.toml");
    fs::write(&config_path, missing_store_toml()).expect("write config");

    let output = binary()
        .args(["check-config", "--config"])
        .arg(&config_path)
        .output()
        .expect("run check-config");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("\"reason_code\":\"core_config.missing_store\""));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn check_config_missing_nats_reports_only_a_stable_reason_code() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("core.toml");
    fs::write(&config_path, missing_nats_toml()).expect("write config");

    let output = binary()
        .args(["check-config", "--config"])
        .arg(&config_path)
        .output()
        .expect("run check-config");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("\"reason_code\":\"core_config.missing_nats\""));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn check_config_rejects_non_loopback_status_listen() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("core.toml");
    fs::write(
        &config_path,
        include_str!("../../../config/core.example.toml")
            .replace("listen = \"127.0.0.1:8742\"", "listen = \"8.8.8.8:8742\""),
    )
    .expect("write config");

    let output = binary()
        .args(["check-config", "--config"])
        .arg(&config_path)
        .output()
        .expect("run check-config");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("\"reason_code\":\"core_config.invalid_status_listen\""));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn run_missing_store_fails_closed_before_nats() {
    let directory = tempdir().expect("temporary directory");
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700))
        .expect("private parent");
    let config_path = directory.path().join("core.toml");
    let missing_store = directory.path().join("missing-parent").join("state");
    fs::write(&config_path, valid_toml(&missing_store)).expect("write config");

    let output = binary()
        .args(["run", "--config"])
        .arg(&config_path)
        .output()
        .expect("run hl-core");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("\"reason_code\":\"core_runtime.store\""));
    assert!(!stderr.contains("alpha-desk-nats-core-password"));
    assert!(!stderr.contains("panicked"));
}

fn missing_store_toml() -> String {
    r#"
chain_id = "mainnet"
first_height = 1
shutdown_grace_millis = 15000
idle_poll_millis = 250

[nats]
server_url = "nats://127.0.0.1:4222"
stream = "HL_CANONICAL"
username = "core"
password_path = "/run/secrets/alpha-desk-nats-core-password"
connect_timeout_millis = 5000
acknowledgement_timeout_millis = 5000
max_ack_inflight = 64
durable_name = "hl-core-file-replay"
fetch_batch = 64
"#
    .to_owned()
}

fn missing_nats_toml() -> String {
    r#"
chain_id = "mainnet"
first_height = 1
shutdown_grace_millis = 15000
idle_poll_millis = 250

[store]
path = "state/core-file-store"
"#
    .to_owned()
}

fn valid_toml(store_path: &std::path::Path) -> String {
    format!(
        r#"
chain_id = "mainnet"
first_height = 1
shutdown_grace_millis = 15000
idle_poll_millis = 250

[store]
path = "{path}"

[nats]
server_url = "nats://127.0.0.1:4222"
stream = "HL_CANONICAL"
username = "core"
password_path = "/run/secrets/alpha-desk-nats-core-password"
connect_timeout_millis = 5000
acknowledgement_timeout_millis = 5000
max_ack_inflight = 64
durable_name = "hl-core-file-replay"
fetch_batch = 64
"#,
        path = store_path.display()
    )
}
