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
fn cost_and_role_changes_are_visible_in_matrix_and_diff() {
    let root = temp_root();
    let original = sample_manifest();
    let parsed = parse_manifest(&original).expect("sample");
    validate_manifest(&parsed).expect("sample valid");
    let generated = render_coverage_matrix(&parsed);
    assert!(
        generated.contains("| source_role | request_cost | state_target |"),
        "matrix must render role/cost/target columns"
    );
    write_workspace(&root, &original, Some(&generated));

    let matching = bin()
        .args(["render-docs", "--check", "--root"])
        .arg(&root)
        .output()
        .expect("render-docs --check matching");
    assert_eq!(matching.status.code(), Some(0), "{}", stderr(&matching));

    let role_shifted = original.replace(
        "source_role = \"reconciliation\"",
        "source_role = \"enrichment\"",
    );
    write_workspace(&root, &role_shifted, Some(&generated));
    let role_check = bin()
        .args(["render-docs", "--check", "--root"])
        .arg(&root)
        .output()
        .expect("render-docs --check role");
    assert_eq!(role_check.status.code(), Some(1));
    assert_eq!(
        stderr(&role_check),
        "coverage-matrix: generated output differs from docs/hyperliquid/coverage-matrix.md\n"
    );

    let cost_shifted = original.replace(
        "request_cost = \"base:2\"",
        "request_cost = \"base:2 variable:window\"",
    );
    write_workspace(&root, &cost_shifted, Some(&generated));
    let cost_check = bin()
        .args(["render-docs", "--check", "--root"])
        .arg(&root)
        .output()
        .expect("render-docs --check cost");
    assert_eq!(cost_check.status.code(), Some(1));
    assert_eq!(
        stderr(&cost_check),
        "coverage-matrix: generated output differs from docs/hyperliquid/coverage-matrix.md\n"
    );

    let left_report = coverage_report(&parsed);
    assert_eq!(left_report.rows[0].source_role, "reconciliation");
    assert_eq!(left_report.rows[0].request_cost, "base:2");
    assert_eq!(left_report.rows[0].state_target, "reference_snapshot");

    let role_manifest = parse_manifest(&role_shifted).expect("role");
    let cost_manifest = parse_manifest(&cost_shifted).expect("cost");
    let target_manifest = parse_manifest(&original.replace(
        "state_target = \"reference_snapshot\"",
        "state_target = \"evm_fact\"",
    ))
    .expect("target");
    let role_diff = diff_reports(&left_report, &coverage_report(&role_manifest));
    assert_eq!(role_diff.changed, vec!["official.info.all_mids".to_owned()]);
    let cost_diff = diff_reports(&left_report, &coverage_report(&cost_manifest));
    assert_eq!(cost_diff.changed, vec!["official.info.all_mids".to_owned()]);
    let target_diff = diff_reports(&left_report, &coverage_report(&target_manifest));
    assert_eq!(
        target_diff.changed,
        vec!["official.info.all_mids".to_owned()]
    );

    let left_path = root.join("left.json");
    let right_path = root.join("right.json");
    fs::write(
        &left_path,
        encode_coverage_report(&left_report).expect("encode"),
    )
    .expect("left");
    fs::write(
        &right_path,
        encode_coverage_report(&coverage_report(&role_manifest)).expect("encode"),
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
    assert_eq!(report.schema_version, 2);
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
    let error = parse_coverage_report(r#"{"schema_version":2,"rows":[],"unexpected":true}"#)
        .expect_err("unknown fields must fail");
    assert!(error.contains("invalid coverage report"), "{error}");
}

#[test]
fn coverage_report_rejects_schema_version_mismatch() {
    let v1_missing_fields = r#"{"schema_version":1,"rows":[{"id":"official.info.all_mids","source":"official","transport":"rest_info","identifier":"allMids","domain":"market_data","status":"planned","owner":"hl-capture"}]}"#;
    let error = parse_coverage_report(v1_missing_fields).expect_err("v1 must fail closed");
    assert!(
        error.contains("coverage report schema_version must be 2, got 1"),
        "{error}"
    );
    assert!(
        !error.contains("missing field"),
        "version mismatch must not look like a serde field error: {error}"
    );

    let root = temp_root();
    write_workspace(&root, &sample_manifest(), None);
    let left_path = root.join("left.json");
    let right_path = root.join("right.json");
    fs::write(&left_path, format!("{v1_missing_fields}\n")).expect("v1 left");
    let current = coverage_report(&parse_manifest(&sample_manifest()).expect("sample"));
    fs::write(
        &right_path,
        encode_coverage_report(&current).expect("encode"),
    )
    .expect("v2 right");
    let mismatched = bin()
        .args([
            "diff",
            "--left",
            left_path.to_str().unwrap(),
            "--right",
            right_path.to_str().unwrap(),
        ])
        .output()
        .expect("diff version mismatch");
    assert_eq!(mismatched.status.code(), Some(1));
    assert_eq!(
        stderr(&mismatched),
        "coverage report schema_version must be 2, got 1\n"
    );
    fs::remove_dir_all(&root).expect("cleanup");
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
