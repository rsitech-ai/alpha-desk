use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

use hyperliquid_capabilities::{
    coverage_report, diff_reports, encode_coverage_report, parse_coverage_report, parse_manifest,
    render_coverage_matrix, validate_manifest,
};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hyperliquid-capabilities"))
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).expect("diagnostics must be UTF-8")
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).expect("stdout must be UTF-8")
}

fn temp_root() -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "alpha-desk-hyperliquid-capabilities-{}-{sequence}",
        std::process::id()
    ));
    fs::create_dir_all(path.join("config/hyperliquid")).expect("temp config");
    fs::create_dir_all(path.join("docs/hyperliquid")).expect("temp docs");
    path
}

fn write_workspace(root: &Path, manifest: &str, matrix: Option<&str>) {
    fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = []\n").expect("cargo toml");
    fs::write(root.join("config/hyperliquid/capabilities.toml"), manifest).expect("manifest");
    if let Some(matrix) = matrix {
        fs::write(root.join("docs/hyperliquid/coverage-matrix.md"), matrix).expect("matrix");
    }
}

fn sample_manifest() -> String {
    r#"
schema_version = 1

[[capability]]
id = "official.info.all_mids"
source = "official"
network = ["mainnet", "testnet"]
transport = "rest_info"
identifier = "allMids"
domain = "market_data"
source_role = "reconciliation"
request_cost = "base:2"
pagination = "none"
parser = "planned"
fixture_set = "none"
retention = "raw_indefinite"
freshness_target_ms = 1000
owner = "hl-capture"
state_target = "reference_snapshot"
status = "planned"
limitations = "REST /info adapter is not on this tree"
"#
    .to_owned()
}

#[test]
fn usage_exit_code_is_stable() {
    let output = bin().output().expect("binary must start");
    assert_eq!(output.status.code(), Some(2));
    assert!(stdout(&output).is_empty());
    assert!(stderr(&output).starts_with("usage: hyperliquid-capabilities"));
}

#[test]
fn generated_matrix_differs_from_committed_matrix() {
    let root = temp_root();
    let manifest = sample_manifest();
    let parsed = parse_manifest(&manifest).expect("sample");
    validate_manifest(&parsed).expect("sample valid");
    let generated = render_coverage_matrix(&parsed);
    write_workspace(&root, &manifest, Some("stale matrix\n"));

    let check = bin()
        .args(["render-docs", "--check", "--root"])
        .arg(&root)
        .output()
        .expect("render-docs --check");
    assert_eq!(check.status.code(), Some(1));
    assert_eq!(
        stderr(&check),
        "coverage-matrix: generated output differs from docs/hyperliquid/coverage-matrix.md\n"
    );

    write_workspace(&root, &manifest, Some(&generated));
    let ok = bin()
        .args(["render-docs", "--check", "--root"])
        .arg(&root)
        .output()
        .expect("render-docs --check matching");
    assert_eq!(ok.status.code(), Some(0), "{}", stderr(&ok));
    assert_eq!(stdout(&ok), "coverage-matrix:ok\n");

    fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn coverage_and_diff_round_trip() {
    let root = temp_root();
    let manifest = sample_manifest();
    write_workspace(&root, &manifest, None);

    let coverage = bin()
        .args(["coverage", "--root"])
        .arg(&root)
        .output()
        .expect("coverage");
    assert_eq!(coverage.status.code(), Some(0), "{}", stderr(&coverage));
    let report = parse_coverage_report(&stdout(&coverage)).expect("coverage json");
    assert_eq!(report.rows.len(), 1);

    let left_path = root.join("left.json");
    let right_path = root.join("right.json");
    fs::write(&left_path, encode_coverage_report(&report).expect("encode")).expect("left");
    let mut shifted = report.clone();
    shifted.rows[0].status = "implemented".to_owned();
    fs::write(
        &right_path,
        encode_coverage_report(&shifted).expect("encode"),
    )
    .expect("right");

    let changed = bin()
        .args([
            "diff",
            "--left",
            left_path.to_str().unwrap(),
            "--right",
            right_path.to_str().unwrap(),
        ])
        .output()
        .expect("diff");
    assert_eq!(changed.status.code(), Some(1));
    assert_eq!(stdout(&changed), "changed: official.info.all_mids\n");

    let identical = bin()
        .args([
            "diff",
            "--left",
            left_path.to_str().unwrap(),
            "--right",
            left_path.to_str().unwrap(),
        ])
        .output()
        .expect("diff identical");
    assert_eq!(identical.status.code(), Some(0), "{}", stderr(&identical));
    assert_eq!(stdout(&identical), "coverage-diff:identical\n");

    fs::remove_dir_all(&root).expect("cleanup");
}

#[test]
fn coverage_report_rejects_unknown_fields() {
    let error = parse_coverage_report(r#"{"schema_version":1,"rows":[],"unexpected":true}"#)
        .expect_err("unknown fields must fail");
    assert!(error.contains("invalid coverage report"), "{error}");
}

#[test]
fn library_diff_detects_added_and_removed_rows() {
    let manifest = parse_manifest(&sample_manifest()).expect("sample");
    let left = coverage_report(&manifest);
    let mut right = left.clone();
    right.rows.clear();
    let diff = diff_reports(&left, &right);
    assert_eq!(diff.removed, vec!["official.info.all_mids".to_owned()]);
    assert!(diff.added.is_empty());
}

#[test]
fn committed_workspace_validate_and_matrix_check_pass() {
    let root = workspace_root();
    let output = bin()
        .args(["validate", "--root"])
        .arg(&root)
        .output()
        .expect("validate");
    assert_eq!(output.status.code(), Some(0), "{}", stderr(&output));

    let check = bin()
        .args(["render-docs", "--check", "--root"])
        .arg(&root)
        .output()
        .expect("render-docs --check");
    assert_eq!(check.status.code(), Some(0), "{}", stderr(&check));
}
