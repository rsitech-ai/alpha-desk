#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use stage_gate::approvals::{ApprovalDecision, ApprovalStatement, canonical_statement_bytes};
use tempfile::TempDir;

#[test]
fn fixture_run_writes_only_ignored_output_and_stays_blocked_without_external_evidence() {
    let fixture = CliFixture::new();
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");
    let tracked_before = git_output(
        fixture.repository.path(),
        ["status", "--porcelain=v1", "--untracked-files=all"],
    );

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .env("STAGE_GATE_BUILDER_ID", "fixture-builder-a")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    assert!(output_path.is_file());
    let builder_output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.builder.json");
    assert!(builder_output_path.is_file());
    let builder_bytes = fs::read(&builder_output_path).unwrap();
    let builder: stage_gate::reports::BuilderReport =
        serde_json::from_slice(&builder_bytes).unwrap();
    assert_eq!(
        builder_bytes,
        stage_gate::canonical::canonicalize(&builder).unwrap()
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(report["overall_result"], "BLOCKED");
    assert_eq!(report["stage_outcome"], "HOLD");
    assert_eq!(
        report["reason_codes"],
        serde_json::json!([
            "second_builder_unavailable",
            "platform_data_approval_missing",
            "independent_review_missing",
            "openpgp_tooling_unavailable",
            "required_github_checks_unavailable"
        ])
    );
    assert_eq!(report["aggregate_manifest"]["comparison"], "NOT_RUN");
    assert_eq!(
        report["aggregate_manifest"]["builder_report_hashes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        report["aggregate_manifest"]["trust_registry_sha256"]
            .as_str()
            .unwrap()
            .len(),
        64
    );
    assert_eq!(
        report["aggregate_evidence_sha256"].as_str().unwrap().len(),
        64
    );
    assert_eq!(
        report["comparison_manifest_sha256"].as_str().unwrap().len(),
        64
    );
    assert_eq!(
        fs::read_to_string(
            fixture
                .repository
                .path()
                .join("target/stage-gates/check-order.txt")
        )
        .unwrap(),
        "verify\nquality\n"
    );
    assert_eq!(
        git_output(
            fixture.repository.path(),
            ["status", "--porcelain=v1", "--untracked-files=all"]
        ),
        tracked_before
    );
    assert!(git_check_ignored(fixture.repository.path(), &output_path));
    assert!(git_check_ignored(
        fixture.repository.path(),
        &builder_output_path
    ));
    assert!(
        !fixture
            .repository
            .path()
            .join("docs/stage-gates/stage-0-foundations.md")
            .exists()
    );
    assert!(
        git_output(
            fixture.repository.path(),
            ["tag", "--list", "stage-0-foundations"]
        )
        .is_empty()
    );
}

#[test]
fn missing_explicit_local_builder_id_is_a_stable_blocking_reason() {
    let fixture = CliFixture::new();
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert!(
        report["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("builder_identity_unavailable"))
    );
    assert_eq!(report["stage_outcome"], "HOLD");
}

#[test]
fn builder_version_mismatch_is_a_stable_blocking_reason() {
    let fixture = CliFixture::new();
    let config_path = fixture
        .repository
        .path()
        .join("config/stage-gates/stage-0.toml");
    let source = fs::read_to_string(&config_path)
        .unwrap()
        .replace("fixture-tool 1.0.0", "fixture-tool 2.0.0");
    fs::write(&config_path, source).unwrap();
    git(fixture.repository.path(), ["add", "."]);
    git(
        fixture.repository.path(),
        ["commit", "-q", "-m", "require unavailable tool version"],
    );
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .env("STAGE_GATE_BUILDER_ID", "fixture-builder-a")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert!(
        report["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("builder_version_mismatch"))
    );
    assert_eq!(report["stage_outcome"], "HOLD");
}

#[test]
fn committed_invalid_utf8_trust_registry_is_a_stable_blocking_reason() {
    let fixture = CliFixture::new();
    let policy_path = fixture
        .repository
        .path()
        .join("config/stage-gates/trust-policy.toml");
    fs::write(&policy_path, [0xff, 0xfe, 0xfd]).unwrap();
    git(fixture.repository.path(), ["add", "."]);
    git(
        fixture.repository.path(),
        ["commit", "-q", "-m", "commit unreadable trust registry"],
    );
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .env("STAGE_GATE_BUILDER_ID", "fixture-builder-a")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert!(
        report["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("trust_registry_unconfigured"))
    );
    assert_eq!(report["stage_outcome"], "HOLD");
}

#[test]
fn resolved_approval_verifier_path_and_hash_are_bound_into_builder_report() {
    let fixture = CliFixture::new();
    let config_path = fixture
        .repository
        .path()
        .join("config/stage-gates/stage-0.toml");
    let verifier = std::env::current_exe().unwrap();
    let source = fs::read_to_string(&config_path)
        .unwrap()
        .replace("\"/definitely/missing/gpgv\"", &toml_string(&verifier));
    fs::write(&config_path, source).unwrap();
    git(fixture.repository.path(), ["add", "."]);
    git(
        fixture.repository.path(),
        ["commit", "-q", "-m", "pin test approval verifier"],
    );
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .env("STAGE_GATE_BUILDER_ID", "fixture-builder-a")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    let identity = &report["builder_report"]["resolved_programs"]["approval:gpgv"];
    assert_eq!(
        identity["resolved_path"],
        verifier.canonicalize().unwrap().to_string_lossy().as_ref()
    );
    assert_eq!(identity["sha256"].as_str().unwrap().len(), 64);
}

#[test]
fn approval_verifier_mutating_a_tracked_file_fails_final_snapshot_check() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let verifier = repository.join("fake-gpgv");
    let script = format!(
        concat!(
            "#!/bin/bash\n",
            "set -euo pipefail\n",
            "printf '%s\\n' mutated >> \"{}\"\n",
            "statement=\"${{@:$#}}\"\n",
            "case \"${{statement}}\" in\n",
            "  *platform-data*) fingerprint=0123456789abcdef0123456789abcdef01234567 ;;\n",
            "  *) fingerprint=89abcdef0123456789abcdef0123456789abcdef ;;\n",
            "esac\n",
            "printf '[GNUPG:] VALIDSIG %s\\n' \"${{fingerprint}}\"\n",
        ),
        repository.join("design.md").display()
    );
    fs::write(&verifier, script).unwrap();
    let mut permissions = fs::metadata(&verifier).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&verifier, permissions).unwrap();
    let config_path = repository.join("config/stage-gates/stage-0.toml");
    let source = fs::read_to_string(&config_path)
        .unwrap()
        .replace("\"/definitely/missing/gpgv\"", "\"./fake-gpgv\"");
    fs::write(&config_path, source).unwrap();
    git(repository, ["add", "."]);
    git(repository, ["commit", "-q", "-m", "add fake verifier"]);

    let output_path = repository.join("target/stage-gates/stage-0.json");
    let preliminary = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(repository)
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .env("STAGE_GATE_BUILDER_ID", "fixture-builder-a")
        .output()
        .unwrap();
    assert_eq!(preliminary.status.code(), Some(2), "{preliminary:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    let external = repository.join("target/stage-gates/external");
    let inputs = repository.join("target/stage-gates/inputs");
    fs::create_dir_all(&external).unwrap();
    fs::create_dir_all(&inputs).unwrap();
    fs::write(inputs.join("reviewers.gpg"), b"fake keyring").unwrap();
    for (role, fingerprint) in [
        ("platform-data", "0123456789abcdef0123456789abcdef01234567"),
        ("independent", "89abcdef0123456789abcdef0123456789abcdef"),
    ] {
        let statement = ApprovalStatement {
            schema_version: 1,
            stage_id: "stage-0".to_owned(),
            role: role.to_owned(),
            decision: ApprovalDecision::Approve,
            implementation_commit: report["builder_report"]["implementation_commit"]
                .as_str()
                .unwrap()
                .to_owned(),
            design_tag_object: report["builder_report"]["design_tag_object"]
                .as_str()
                .unwrap()
                .to_owned(),
            design_commit: report["builder_report"]["design_commit"]
                .as_str()
                .unwrap()
                .to_owned(),
            aggregate_evidence_sha256: report["aggregate_evidence_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
            comparison_manifest_sha256: report["comparison_manifest_sha256"]
                .as_str()
                .unwrap()
                .to_owned(),
            known_limitations: Vec::new(),
            known_limitations_sha256:
                "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".to_owned(),
            signer_fingerprint: fingerprint.to_owned(),
            signed_at_utc: "2026-07-28T00:00:00Z".to_owned(),
        };
        fs::write(
            external.join(format!("{role}.json")),
            canonical_statement_bytes(&statement).unwrap(),
        )
        .unwrap();
        fs::write(external.join(format!("{role}.json.asc")), b"fake signature").unwrap();
    }
    fs::remove_file(&output_path).unwrap();

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(repository)
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .env("STAGE_GATE_BUILDER_ID", "fixture-builder-a")
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!output_path.exists());
    assert!(
        fs::read_to_string(repository.join("design.md"))
            .unwrap()
            .contains("mutated")
    );
}

#[test]
fn output_path_outside_configured_ignored_root_is_rejected() {
    let fixture = CliFixture::new();
    let escaped = fixture.repository.path().join("stage-0.json");

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&escaped)
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert!(!escaped.exists());
}

#[test]
fn checked_in_stage_zero_config_lists_the_complete_check_and_artifact_contract() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(manifest_dir.join("../../config/stage-gates/stage-0.toml")).unwrap();
    let config = stage_gate::config::GateConfig::parse(&source).unwrap();
    let check_ids = config
        .checks
        .iter()
        .map(|check| check.id.as_str())
        .collect::<Vec<_>>();
    let artifact_names = config
        .artifacts
        .iter()
        .map(|artifact| artifact.id.as_str())
        .collect::<Vec<_>>();
    let builder_tools = config
        .builder
        .tools
        .iter()
        .map(|tool| tool.id.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        check_ids,
        vec![
            "verify",
            "quality",
            "generated",
            "fixtures",
            "compose",
            "ansible",
            "release-binaries",
            "reproducible",
        ]
    );
    for required in [
        "cargo-lock",
        "cargo-config",
        "schema-descriptor",
        "fixture-manifest",
        "rust-toolchain",
        "swift-package",
        "hl-api",
        "hl-capture",
        "hl-core",
        "hl-analytics",
        "hl-research",
        "architecture-check",
        "build-info",
        "fixture-inspect",
        "schema-check",
        "schema-generate",
        "stage-gate",
    ] {
        assert!(
            artifact_names.contains(&required),
            "missing artifact {required}"
        );
    }
    assert_eq!(
        builder_tools,
        vec!["rustc", "cargo", "swift", "just", "docker", "os"]
    );
    assert!(
        config
            .program_roots
            .iter()
            .any(|root| root == "$CARGO_HOME/bin")
    );
    let compose = config
        .checks
        .iter()
        .find(|check| check.id == "compose")
        .unwrap();
    assert_eq!(compose.program, "./tools/ci/stage-0-compose-smoke.sh");
    for check_id in ["release-binaries", "reproducible"] {
        let check = config
            .checks
            .iter()
            .find(|check| check.id == check_id)
            .unwrap();
        assert_eq!(
            check.env.get("SOURCE_DATE_EPOCH").map(String::as_str),
            Some("1784894400"),
            "{check_id} must build the exact reproducible artifact epoch"
        );
    }
    for (tool_id, expected) in [
        ("rustc", "rustc 1.97.1"),
        ("cargo", "cargo 1.97.1"),
        ("swift", "Swift version 6.3"),
    ] {
        let tool = config
            .builder
            .tools
            .iter()
            .find(|tool| tool.id == tool_id)
            .unwrap();
        assert_eq!(tool.expected_output_contains.as_deref(), Some(expected));
    }
    assert_eq!(
        config.builder_report_output_path,
        "target/stage-gates/stage-0.builder.json"
    );
}

#[test]
fn compose_smoke_has_bounded_health_and_non_destructive_cleanup_contract() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source =
        fs::read_to_string(manifest_dir.join("../../tools/ci/stage-0-compose-smoke.sh")).unwrap();

    assert!(source.contains("ps --all --quiet"));
    assert!(!source.contains("ps -q"));
    assert!(source.contains("up -d --wait --wait-timeout 120"));
    assert!(source.contains("wait-for-dev-stack.sh"));
    assert!(source.contains("down --timeout 60 --remove-orphans"));
    assert!(!source.contains("down --volumes"));
}

#[test]
fn stage_gate_cli_check_helper() {
    let Ok(label) = std::env::var("STAGE_GATE_FIXTURE_CHECK") else {
        return;
    };
    let order = PathBuf::from(std::env::var("STAGE_GATE_FIXTURE_ORDER").unwrap());
    let mut options = fs::OpenOptions::new();
    options.create(true).append(true);
    use std::io::Write as _;
    writeln!(options.open(order).unwrap(), "{label}").unwrap();
}

#[test]
fn stage_gate_cli_identity_helper() {
    println!("fixture-tool 1.0.0");
    println!("host: fixture-target");
}

fn stage_gate_binary() -> &'static str {
    env!("CARGO_BIN_EXE_stage-gate")
}

struct CliFixture {
    repository: TempDir,
}

impl CliFixture {
    fn new() -> Self {
        let repository = TempDir::new().unwrap();
        git(repository.path(), ["init", "-q"]);
        git(
            repository.path(),
            ["config", "user.name", "Stage Gate Test"],
        );
        git(
            repository.path(),
            ["config", "user.email", "stage-gate@example.invalid"],
        );
        fs::write(repository.path().join(".gitignore"), "target/\n").unwrap();
        fs::write(repository.path().join("design.md"), "approved design\n").unwrap();
        git(repository.path(), ["add", "."]);
        git(repository.path(), ["commit", "-q", "-m", "design"]);
        let design_commit = git_output(repository.path(), ["rev-parse", "HEAD"]);
        git(
            repository.path(),
            [
                "tag",
                "-a",
                "design-approved-v1.0.0",
                "-m",
                "approved design",
            ],
        );
        let tag_object = git_output(
            repository.path(),
            ["rev-parse", "design-approved-v1.0.0^{tag}"],
        );

        fs::create_dir_all(repository.path().join("config/stage-gates")).unwrap();
        fs::write(
            repository
                .path()
                .join("config/stage-gates/gate.schema.json"),
            b"{\"type\":\"object\"}\n",
        )
        .unwrap();
        fs::write(
            repository
                .path()
                .join("config/stage-gates/trust-policy.toml"),
            concat!(
                "schema_version = 1\n",
                "keyring_path = \"target/stage-gates/inputs/reviewers.gpg\"\n",
                "[[reviewers]]\n",
                "role = \"platform-data\"\n",
                "fingerprint = \"0123456789abcdef0123456789abcdef01234567\"\n",
                "[[reviewers]]\n",
                "role = \"independent\"\n",
                "fingerprint = \"89abcdef0123456789abcdef0123456789abcdef\"\n",
            ),
        )
        .unwrap();
        fs::write(
            repository.path().join("artifact.bin"),
            b"fixture artifact\n",
        )
        .unwrap();
        let check_program = std::env::current_exe().unwrap();
        let order_path = repository.path().join("target/stage-gates/check-order.txt");
        let config = fixture_config(&design_commit, &tag_object, &check_program, &order_path);
        fs::write(
            repository.path().join("config/stage-gates/stage-0.toml"),
            config,
        )
        .unwrap();
        git(repository.path(), ["add", "."]);
        git(repository.path(), ["commit", "-q", "-m", "gate inputs"]);
        Self { repository }
    }
}

fn fixture_config(
    design_commit: &str,
    tag_object: &str,
    check_program: &Path,
    order_path: &Path,
) -> String {
    let program = toml_string(check_program);
    let program_root = toml_string(check_program.parent().unwrap());
    let order = toml_string(order_path);
    format!(
        r#"
schema_version = 1
stage_id = "stage-0"
schema_path = "config/stage-gates/gate.schema.json"
output_root = "target/stage-gates"
builder_report_output_path = "target/stage-gates/stage-0.builder.json"
whole_gate_timeout_seconds = 30
max_output_bytes = 16384
allowed_programs = [{program}, "/definitely/missing/gpgv"]
program_roots = [{program_root}]

[design]
tag = "design-approved-v1.0.0"
object = "{tag_object}"
commit = "{design_commit}"

[comparison]
second_builder_report_path = "target/stage-gates/external/builder-b.json"

[builder]
target_tool = "fixture-tool"

[[builder.tools]]
id = "fixture-tool"
program = {program}
args = ["--exact", "stage_gate_cli_identity_helper", "--nocapture"]
expected_output_contains = "fixture-tool 1.0.0"

[approvals]
policy_path = "config/stage-gates/trust-policy.toml"
required_roles = ["platform-data", "independent"]
gpgv_program = "/definitely/missing/gpgv"
known_limitations = []

[[approvals.evidence]]
role = "platform-data"
statement_path = "target/stage-gates/external/platform-data.json"
signature_path = "target/stage-gates/external/platform-data.json.asc"

[[approvals.evidence]]
role = "independent"
statement_path = "target/stage-gates/external/independent.json"
signature_path = "target/stage-gates/external/independent.json.asc"

[remote]
proof_path = "target/stage-gates/external/github.json"
app_source = "rsitech-ai/alpha-desk"
required_checks = ["rust-linux", "swift-macos"]

[[artifacts]]
id = "fixture"
path = "artifact.bin"
kind = "fixture"
producer = "fixture"
target_triple = "platform-independent"
profile = "test"

[[checks]]
id = "verify"
program = {program}
args = ["--exact", "stage_gate_cli_check_helper", "--nocapture"]
cwd = "."
timeout_seconds = 5

[checks.env]
STAGE_GATE_FIXTURE_CHECK = "verify"
STAGE_GATE_FIXTURE_ORDER = {order}

[[checks]]
id = "quality"
program = {program}
args = ["--exact", "stage_gate_cli_check_helper", "--nocapture"]
cwd = "."
timeout_seconds = 5

[checks.env]
STAGE_GATE_FIXTURE_CHECK = "quality"
STAGE_GATE_FIXTURE_ORDER = {order}
"#
    )
}

fn toml_string(path: &Path) -> String {
    format!("{:?}", path.to_string_lossy())
}

fn git<const N: usize>(repository: &Path, args: [&str; N]) {
    let status = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(status.success());
}

fn git_output<const N: usize>(repository: &Path, args: [&str; N]) -> String {
    let output = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

fn git_check_ignored(repository: &Path, path: &Path) -> bool {
    Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(["check-ignore", "-q"])
        .arg(path)
        .status()
        .unwrap()
        .success()
}
