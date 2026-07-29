use std::fs;
use std::process::Command;

use domain_types::{BlockHeight, ChainId, KnownTime};
use hl_capture::{CaptureHealth, CaptureStatus, StatusWriter};
use tempfile::tempdir;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hl-capture"))
}

#[test]
fn missing_command_is_a_usage_error_without_a_panic() {
    let output = binary().output().expect("run hl-capture");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("usage: hl-capture"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn check_config_emits_stable_machine_readable_success() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("capture.toml");
    fs::write(
        &config_path,
        include_bytes!("../../../config/capture.example.toml"),
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
    assert_eq!(value["schema_version"], "hl.capture.check.v1");
    assert_eq!(value["valid"], true);
}

#[test]
fn invalid_config_reports_only_a_stable_reason_code() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("capture.toml");
    fs::write(
        &config_path,
        "parser_version = \"secret-inline-value\"\nunknown = true\n",
    )
    .expect("write invalid config");

    let output = binary()
        .args(["check-config", "--config"])
        .arg(&config_path)
        .output()
        .expect("run check-config");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("\"reason_code\":\"capture_config.invalid_toml\""));
    assert!(!stderr.contains("secret-inline-value"));
    assert!(!stderr.contains(config_path.to_string_lossy().as_ref()));
}

#[test]
fn status_outputs_the_validated_atomic_snapshot_without_config_secrets() {
    let directory = tempdir().expect("temporary directory");
    let status_path = directory.path().join("capture-status.json");
    StatusWriter::new(status_path.clone())
        .expect("status writer")
        .write(
            &CaptureStatus::new(
                KnownTime::from_unix_micros(500).expect("time"),
                "build-500",
                ChainId::new("mainnet").expect("chain"),
                CaptureHealth::Green,
            )
            .with_readiness(true)
            .with_durable_height(Some(BlockHeight::new(500))),
        )
        .expect("write status");
    let config_path = directory.path().join("capture.toml");
    let status_path_text = status_path.to_string_lossy();
    let config = include_str!("../../../config/capture.example.toml").replace(
        "status_path = \"state/capture-status.json\"",
        &format!("status_path = \"{status_path_text}\""),
    );
    fs::write(&config_path, config).expect("write config");

    let output = binary()
        .args(["status", "--config"])
        .arg(&config_path)
        .arg("--json")
        .output()
        .expect("run status");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status JSON");
    assert_eq!(value["schema_version"], "hl.capture.status.v1");
    assert_eq!(value["durable_height"], 500);
    assert_eq!(value["ready"], true);
    assert!(
        !String::from_utf8(output.stdout)
            .expect("UTF-8 status")
            .contains("alpha-desk-postgres-url")
    );
}

#[test]
fn production_run_reaches_the_protected_infrastructure_boundary_without_leaking_secrets() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("capture.toml");
    fs::write(
        &config_path,
        include_bytes!("../../../config/capture.example.toml"),
    )
    .expect("write config");

    let output = binary()
        .args(["run", "--config"])
        .arg(&config_path)
        .output()
        .expect("run production command");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("\"reason_code\":\"capture_connect.secret\""));
    assert!(!stderr.contains("postgresql://"));
}

#[test]
fn fixture_replay_requires_an_explicit_bounded_block_count() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("capture.toml");
    fs::write(
        &config_path,
        include_bytes!("../../../config/capture.example.toml"),
    )
    .expect("write config");

    let output = binary()
        .args(["fixture-replay", "--config"])
        .arg(&config_path)
        .output()
        .expect("run fixture command");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("UTF-8 stderr")
            .contains("usage: hl-capture")
    );
}
