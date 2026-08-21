use std::{fs, os::unix::fs::PermissionsExt, process::Command};

fn block_json(confirmation: &str) -> String {
    format!(
        r#"{{
  "schema": "hl.core.local-replay-block.v1",
  "source_qualification": "synthetic_unassessed",
  "stage_1_qualified": false,
  "stage_2_qualified": false,
  "chain_id": "mainnet",
  "block_height": 200,
  "block_time_micros": 200,
  "confirmation_class": "{confirmation}",
  "source_block_hashes": {{"local-replay": "{hash}"}}
}}
"#,
        hash = hex::encode([1_u8; 32])
    )
}

fn write_block(confirmation: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().expect("temp block");
    fs::set_permissions(file.path(), fs::Permissions::from_mode(0o600)).expect("private file");
    fs::write(file.path(), block_json(confirmation)).expect("write block");
    file
}

#[test]
fn inspect_block_denies_corrections_with_a_stable_reason_and_does_not_apply() {
    let corrected = write_block("corrected");
    let output = Command::new(env!("CARGO_BIN_EXE_hl-core"))
        .args(["inspect-block", corrected.path().to_str().expect("utf8")])
        .output()
        .expect("hl-core inspect-block");
    assert!(!output.status.success());
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "ERROR ledger.correction_unimplemented\n"
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn inspect_block_classifies_committed_blocks_without_applying() {
    let committed = write_block("committed-primary");
    let output = Command::new(env!("CARGO_BIN_EXE_hl-core"))
        .args(["inspect-block", committed.path().to_str().expect("utf8")])
        .output()
        .expect("hl-core inspect-block");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "INSPECT admitted=true applied=false confirmation=committed-primary\n"
    );
}

#[test]
fn inspect_block_usage_is_stable() {
    let output = Command::new(env!("CARGO_BIN_EXE_hl-core"))
        .arg("unknown")
        .output()
        .expect("hl-core usage");
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("usage: hl-core inspect-block"));
    assert!(String::from_utf8_lossy(&output.stderr).contains("hl-core run --config"));
}

#[test]
fn run_config_validates_the_example_without_opening_nats() {
    let path = format!(
        "{}/../../config/core.example.toml",
        env!("CARGO_MANIFEST_DIR")
    );
    let output = Command::new(env!("CARGO_BIN_EXE_hl-core"))
        .args(["run", "--config", &path])
        .output()
        .expect("hl-core run");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
