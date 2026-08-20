use std::fs;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::process::Command;
use std::thread;
use std::time::Duration;

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
    assert!(stderr.contains("serve-status"));
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
            .with_source_state(
                hl_capture::CommittedSourceClass::LocallyVerifiedCommitted,
                hl_capture::CaptureSourceHealth::Healthy,
                None,
                None,
                None,
            )
            .with_durable_height(Some(BlockHeight::new(500)))
            .with_capture_capacity(12, Some(BlockHeight::new(501)), Some(2_500)),
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
    assert_eq!(value["schema_version"], "hl.capture.status.v5");
    assert_eq!(value["maintenance"]["enabled"], false);
    assert_eq!(value["maintenance"]["retention_authorized"], false);
    assert!(value.get("throughput_records_per_sec").is_none());
    assert_eq!(value["durable_height"], 500);
    assert_eq!(value["capture_backlog_records"], 12);
    assert_eq!(value["oldest_pending_capture_height"], 501);
    assert_eq!(value["disk_free_basis_points"], 2_500);
    assert_eq!(value["ready"], true);
    assert!(
        !String::from_utf8(output.stdout)
            .expect("UTF-8 status")
            .contains("alpha-desk-postgres-url")
    );
}

#[test]
fn status_json_still_reads_v4_without_inventing_maintenance_or_rates() {
    let directory = tempdir().expect("temporary directory");
    let status_path = directory.path().join("capture-status.json");
    fs::write(&status_path, capture_fixture("status-v4.json")).expect("write v4 fixture");
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
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("status JSON");
    assert_eq!(value["schema_version"], "hl.capture.status.v4");
    assert!(value.get("maintenance").is_none());
    assert!(value.get("throughput_records_per_sec").is_none());
    assert!(value.get("throughput_blocks_per_sec").is_none());
    assert_eq!(value["ready"], true);
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
fn serve_status_without_listen_is_a_usage_error() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("capture.toml");
    let config = include_str!("../../../config/capture.example.toml")
        .replace("status_listen = \"127.0.0.1:8741\"\n", "");
    fs::write(&config_path, config).expect("write config");

    let output = binary()
        .args(["serve-status", "--config"])
        .arg(&config_path)
        .output()
        .expect("run serve-status");

    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("UTF-8 stderr")
            .contains("usage: hl-capture")
    );
}

#[test]
fn serve_status_rejects_a_non_loopback_listen_address() {
    let directory = tempdir().expect("temporary directory");
    let config_path = directory.path().join("capture.toml");
    fs::write(
        &config_path,
        include_bytes!("../../../config/capture.example.toml"),
    )
    .expect("write config");

    let output = binary()
        .args(["serve-status", "--config"])
        .arg(&config_path)
        .args(["--listen", "8.8.8.8:8741"])
        .output()
        .expect("run serve-status");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("\"reason_code\":\"capture_operator.unsafe_bind\""));
}

struct KillOnDrop(std::process::Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn reserve_loopback() -> SocketAddr {
    TcpListener::bind("127.0.0.1:0")
        .expect("reserve loopback")
        .local_addr()
        .expect("loopback addr")
}

fn try_http_get(addr: SocketAddr, path: &str) -> io::Result<(u16, String)> {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(100))?;
    stream.set_read_timeout(Some(Duration::from_millis(200)))?;
    stream.set_write_timeout(Some(Duration::from_millis(200)))?;
    stream.write_all(
        format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n").as_bytes(),
    )?;
    stream.flush()?;
    let mut body = Vec::new();
    stream.read_to_end(&mut body)?;
    let body = String::from_utf8(body)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    let status = body
        .split_whitespace()
        .nth(1)
        .and_then(|value| value.parse().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing HTTP status"))?;
    Ok((status, body))
}

fn wait_for_http(addr: SocketAddr, path: &str) -> (u16, String) {
    let mut last_error = None;
    for _ in 0..100 {
        match try_http_get(addr, path) {
            Ok(response) => return response,
            Err(error) => last_error = Some(error),
        }
        thread::sleep(Duration::from_millis(20));
    }
    panic!("serve-status did not accept HTTP at {addr}: {last_error:?}");
}

#[test]
fn serve_status_cli_serves_written_v5_json_and_fails_closed_on_a_missing_file() {
    let directory = tempdir().expect("temporary directory");
    let status_path = directory.path().join("capture-status.json");
    let config_path = directory.path().join("capture.toml");
    let status_path_text = status_path.to_string_lossy();
    let config = include_str!("../../../config/capture.example.toml").replace(
        "status_path = \"state/capture-status.json\"",
        &format!("status_path = \"{status_path_text}\""),
    );
    fs::write(&config_path, config).expect("write config");
    let addr = reserve_loopback();
    let _child = KillOnDrop(
        binary()
            .args(["serve-status", "--config"])
            .arg(&config_path)
            .args(["--listen", &addr.to_string()])
            .spawn()
            .expect("spawn serve-status"),
    );

    let (missing_status, missing_body) = wait_for_http(addr, "/status");
    assert_eq!(missing_status, 503);
    assert!(missing_body.contains("capture_status."));

    StatusWriter::new(status_path)
        .expect("status writer")
        .write(
            &CaptureStatus::new(
                KnownTime::from_unix_micros(500).expect("time"),
                "build-500",
                ChainId::new("mainnet").expect("chain"),
                CaptureHealth::Green,
            )
            .with_readiness(true)
            .with_source_state(
                hl_capture::CommittedSourceClass::LocallyVerifiedCommitted,
                hl_capture::CaptureSourceHealth::Healthy,
                None,
                None,
                None,
            )
            .with_durable_height(Some(BlockHeight::new(500))),
        )
        .expect("write status");

    let (status, body) = wait_for_http(addr, "/status");
    assert_eq!(status, 200);
    let json_start = body.find("\r\n\r\n").expect("header terminator") + 4;
    let value: serde_json::Value = serde_json::from_str(&body[json_start..]).expect("status JSON");
    assert_eq!(value["schema_version"], "hl.capture.status.v5");
    assert_eq!(value["maintenance"]["enabled"], false);
    assert_eq!(value["maintenance"]["retention_authorized"], false);
    assert!(value.get("throughput_records_per_sec").is_none());
    assert_eq!(value["durable_height"], 500);
}

fn capture_fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/capture")
            .join(name),
    )
    .unwrap_or_else(|error| panic!("read fixture {name}: {error}"))
}

#[test]
fn serve_status_cli_serves_v5_fixture_json_as_read() {
    let directory = tempdir().expect("temporary directory");
    let status_path = directory.path().join("capture-status.json");
    let fixture = capture_fixture("status-v5.json");
    fs::write(&status_path, &fixture).expect("write v5 fixture");
    let config_path = directory.path().join("capture.toml");
    let status_path_text = status_path.to_string_lossy();
    let config = include_str!("../../../config/capture.example.toml").replace(
        "status_path = \"state/capture-status.json\"",
        &format!("status_path = \"{status_path_text}\""),
    );
    fs::write(&config_path, config).expect("write config");
    let addr = reserve_loopback();
    let _child = KillOnDrop(
        binary()
            .args(["serve-status", "--config"])
            .arg(&config_path)
            .args(["--listen", &addr.to_string()])
            .spawn()
            .expect("spawn serve-status"),
    );

    let (status, body) = wait_for_http(addr, "/status");
    assert_eq!(status, 200);
    let json_start = body.find("\r\n\r\n").expect("header terminator") + 4;
    assert_eq!(&body.as_bytes()[json_start..], fixture.as_slice());
    let value: serde_json::Value = serde_json::from_str(&body[json_start..]).expect("status JSON");
    assert_eq!(value["schema_version"], "hl.capture.status.v5");
    assert_eq!(value["maintenance"]["enabled"], true);
    assert_eq!(
        value["auxiliary_sources"][0]["restart_reconstruction"],
        "complete"
    );
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
