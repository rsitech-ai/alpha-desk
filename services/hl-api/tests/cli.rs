use std::process::Command;

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hl-api"))
}

#[test]
fn missing_command_is_a_usage_error_without_a_panic() {
    let output = binary().output().expect("run hl-api");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(stderr.contains("usage: hl-api"));
    assert!(!stderr.contains("panicked"));
}

#[test]
fn check_config_accepts_the_example_loopback_config() {
    let output = binary()
        .args([
            "check-config",
            "--config",
            concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/api.example.toml"),
        ])
        .output()
        .expect("run check-config");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("machine-readable check result");
    assert_eq!(value["schema_version"], "hl.api.check.v1");
    assert_eq!(value["valid"], true);
}

#[test]
fn print_openapi_matches_the_checked_in_document() {
    let output = binary()
        .args(["print-openapi"])
        .output()
        .expect("run print-openapi");
    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let expected = include_str!("../../../schemas/openapi/v1/openapi.yaml");
    assert_eq!(
        String::from_utf8(output.stdout).expect("UTF-8 OpenAPI"),
        expected
    );
}

#[test]
fn credential_mode_without_secrets_reports_a_stable_reason_code() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let config_path = directory.path().join("api.toml");
    std::fs::write(
        &config_path,
        "[listen]\nbind = \"127.0.0.1:8788\"\n\n[auth]\nmode = \"credential\"\n",
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
    assert!(stderr.contains("\"reason_code\":\"api_config.missing_credentials\""));
    assert!(!stderr.contains("panicked"));
}
