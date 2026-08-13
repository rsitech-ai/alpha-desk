#![cfg(unix)]

use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    os::unix::process::CommandExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use sha2::Digest as _;
use stage_gate::{
    approvals::{
        ApprovalDecision, ApprovalStatement, TrustPolicy, TrustedReviewer,
        canonical_statement_bytes,
    },
    config::GateConfig,
    provenance::{SignedEvidence, verify_signed_builder_report},
};
use tempfile::TempDir;

const BUILDER_B_FINGERPRINT: &str = "fedcba9876543210fedcba9876543210fedcba98";

fn trusted_reviewer(role: &str, fingerprint: &str) -> TrustedReviewer {
    TrustedReviewer {
        role: role.to_owned(),
        fingerprint: fingerprint.to_owned(),
    }
}

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
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
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
fn missing_explicit_local_builder_id_fails_before_reusing_stale_evidence() {
    let fixture = CliFixture::new();
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");
    let builder_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.builder.json");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    fs::write(&output_path, br#"{"overall_result":"PASS"}"#).unwrap();
    fs::write(&builder_path, br#"{"overall_result":"PASS"}"#).unwrap();

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!output_path.exists());
    assert!(!builder_path.exists());
}

#[test]
fn explicit_builder_b_role_and_fingerprint_emit_a_signable_bound_identity() {
    let fixture = CliFixture::new();
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");
    let local_output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();
    assert_eq!(local_output.status.code(), Some(2), "{local_output:?}");
    let local: stage_gate::reports::BuilderReport = serde_json::from_slice(
        &fs::read(
            fixture
                .repository
                .path()
                .join("target/stage-gates/stage-0.builder.json"),
        )
        .unwrap(),
    )
    .unwrap();

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&output_path)
        .args([
            "--builder-role",
            "builder-b",
            "--builder-fingerprint",
            BUILDER_B_FINGERPRINT,
        ])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let builder: stage_gate::reports::BuilderReport = serde_json::from_slice(
        &fs::read(
            fixture
                .repository
                .path()
                .join("target/stage-gates/stage-0.builder.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(
        builder.builder_identity.builder_id,
        format!("builder-b:{BUILDER_B_FINGERPRINT}")
    );
    assert_eq!(builder.builder_identity.signer_role, "builder-b");
    assert_eq!(
        builder.builder_identity.signer_fingerprint,
        BUILDER_B_FINGERPRINT
    );

    let signed = TempDir::new().unwrap();
    let payload_path = signed.path().join("builder-b.json");
    let signature_path = signed.path().join("builder-b.json.asc");
    let keyring_path = signed.path().join("trusted.gpg");
    let verifier_path = signed.path().join("fake-gpgv");
    let payload = stage_gate::canonical::canonicalize(&builder).unwrap();
    fs::write(&payload_path, &payload).unwrap();
    fs::write(&signature_path, hex::encode(sha2::Sha256::digest(&payload))).unwrap();
    fs::write(&keyring_path, b"fixture keyring").unwrap();
    fs::write(
        &verifier_path,
        format!(
            concat!(
                "#!/bin/sh\n",
                "set -eu\n",
                "expected=\"$(/usr/bin/shasum -a 256 \"$6\" | /usr/bin/awk '{{print $1}}')\"\n",
                "actual=\"$(/bin/cat \"$5\")\"\n",
                "[ \"$actual\" = \"$expected\" ]\n",
                "printf '[GNUPG:] VALIDSIG {}\\n'\n",
            ),
            BUILDER_B_FINGERPRINT
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&verifier_path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&verifier_path, permissions).unwrap();
    let policy = TrustPolicy {
        schema_version: 1,
        keyring_path,
        reviewers: vec![
            trusted_reviewer("platform-data", "0123456789abcdef0123456789abcdef01234567"),
            trusted_reviewer("independent", "89abcdef0123456789abcdef0123456789abcdef"),
            trusted_reviewer("builder-b", BUILDER_B_FINGERPRINT),
            trusted_reviewer("github-ci", "76543210fedcba9876543210fedcba9876543210"),
        ],
    };
    let config = GateConfig::parse(
        &fs::read_to_string(
            fixture
                .repository
                .path()
                .join("config/stage-gates/stage-0.toml"),
        )
        .unwrap(),
    )
    .unwrap();

    let verified = verify_signed_builder_report(
        &SignedEvidence {
            role: "builder-b".to_owned(),
            payload_path,
            signature_path,
        },
        &local,
        &config.builder,
        &policy,
        verifier_path,
        1024 * 1024,
    )
    .unwrap();
    assert_eq!(verified.canonical_bytes, payload);
    assert_eq!(verified.value, builder);
}

#[test]
fn builder_b_role_and_full_lowercase_fingerprint_are_an_exact_argument_pair() {
    let fixture = CliFixture::new();
    let output_path = fixture
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");
    for (scenario, extra) in [
        ("role-only", vec!["--builder-role", "builder-b"]),
        (
            "wrong-role",
            vec![
                "--builder-role",
                "builder-c",
                "--builder-fingerprint",
                BUILDER_B_FINGERPRINT,
            ],
        ),
        (
            "uppercase-fingerprint",
            vec![
                "--builder-role",
                "builder-b",
                "--builder-fingerprint",
                "FEDCBA9876543210FEDCBA9876543210FEDCBA98",
            ],
        ),
    ] {
        let output = Command::new(stage_gate_binary())
            .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
            .arg(fixture.repository.path())
            .arg("--output")
            .arg(&output_path)
            .args(extra)
            .env_clear()
            .output()
            .unwrap();

        assert_eq!(
            output.status.code(),
            Some(1),
            "{scenario} must fail before producing evidence: {output:?}"
        );
    }
}

#[test]
fn invalid_stage_zero_invocations_invalidate_only_fixed_outputs_and_preserve_inputs() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let output_path = repository.join("target/stage-gates/stage-0.json");
    let builder_path = repository.join("target/stage-gates/stage-0.builder.json");
    let historical_path = repository.join("target/stage-gates/stage-0-builder-report.json");
    let input_path = repository.join("target/stage-gates/inputs/builder-b.json");
    fs::create_dir_all(input_path.parent().unwrap()).unwrap();
    fs::write(&input_path, br#"{"external":"must survive"}"#).unwrap();

    let scenarios = [
        ("role-only", vec!["--builder-role", "builder-b"]),
        (
            "wrong-role",
            vec![
                "--builder-role",
                "builder-c",
                "--builder-fingerprint",
                BUILDER_B_FINGERPRINT,
            ],
        ),
        (
            "uppercase-fingerprint",
            vec![
                "--builder-role",
                "builder-b",
                "--builder-fingerprint",
                "FEDCBA9876543210FEDCBA9876543210FEDCBA98",
            ],
        ),
        ("missing-output-value", vec!["--output"]),
        (
            "malformed-output",
            vec!["--output", "target/stage-gates/nested/stage-0.json"],
        ),
    ];

    for (scenario, extra) in scenarios {
        for stale in [&output_path, &builder_path, &historical_path] {
            fs::write(
                stale,
                br#"{"overall_result":"PASS","stage_outcome":"ACCEPTED"}"#,
            )
            .unwrap();
        }
        let output = Command::new(stage_gate_binary())
            .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
            .arg(repository)
            .args(extra)
            .env_clear()
            .output()
            .unwrap();

        assert_eq!(
            output.status.code(),
            Some(1),
            "{scenario} must fail closed: {output:?}"
        );
        for stale in [&output_path, &builder_path, &historical_path] {
            assert!(
                !stale.exists(),
                "{scenario} left stale output {}",
                stale.display()
            );
        }
        assert_eq!(
            fs::read(&input_path).unwrap(),
            br#"{"external":"must survive"}"#,
            "{scenario} must preserve external inputs"
        );
    }
}

#[test]
fn malformed_flag_values_never_retarget_early_stage_zero_cleanup() {
    let selected = CliFixture::new();
    let unrelated = CliFixture::new();
    let selected_output = selected
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");
    let unrelated_output = unrelated
        .repository
        .path()
        .join("target/stage-gates/stage-0.json");
    fs::create_dir_all(selected_output.parent().unwrap()).unwrap();
    fs::create_dir_all(unrelated_output.parent().unwrap()).unwrap();
    fs::write(&selected_output, br#"{"overall_result":"PASS"}"#).unwrap();
    fs::write(&unrelated_output, br#"{"overall_result":"PASS"}"#).unwrap();

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(selected.repository.path())
        .args(["--builder-id", "--repository"])
        .arg(unrelated.repository.path())
        .args(["--output"])
        .arg(&selected_output)
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(
        !selected_output.exists(),
        "the repository consumed by the CLI parser must have stale output invalidated"
    );
    assert_eq!(
        fs::read(&unrelated_output).unwrap(),
        br#"{"overall_result":"PASS"}"#,
        "a flag-looking builder value must not retarget cleanup to another repository"
    );
}

#[test]
fn mixed_present_invalid_and_missing_approvals_fail_at_the_gate_boundary() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let external = repository.join("target/stage-gates/external");
    fs::create_dir_all(&external).unwrap();
    fs::write(
        external.join("platform-data.json"),
        b"{\"not\":\"a canonical approval statement\"}",
    )
    .unwrap();
    fs::write(
        external.join("platform-data.json.asc"),
        b"present invalid signature input",
    )
    .unwrap();
    let output_path = repository.join("target/stage-gates/stage-0.json");

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(repository)
        .arg("--output")
        .arg(&output_path)
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    let reasons = report["reason_codes"].as_array().unwrap();
    assert_eq!(report["overall_result"], "FAIL");
    assert_eq!(report["stage_outcome"], "HOLD");
    assert!(reasons.contains(&serde_json::json!("approval_evidence_invalid")));
    assert!(reasons.contains(&serde_json::json!("independent_review_missing")));
}

#[test]
fn explicit_validated_local_builder_id_is_published_without_an_environment_fallback() {
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
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(
        report["builder_report"]["builder_identity"]["builder_id"],
        "fixture-builder-a"
    );
    assert!(
        !report["reason_codes"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("builder_identity_unavailable"))
    );
}

#[test]
fn stage_zero_just_recipe_forwards_the_required_local_builder_id() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = Command::new("just")
        .args(["--dry-run", "stage-0-gate", "fixture-builder-a"])
        .current_dir(repository)
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap())
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    let rendered = String::from_utf8(output.stderr).unwrap();
    assert!(
        rendered.contains(
            "run config/stage-gates/stage-0.toml --output \
             target/stage-gates/stage-0.json --builder-id 'fixture-builder-a'"
        ),
        "unexpected recipe expansion: {rendered}"
    );
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
        .args(["--builder-id", "fixture-builder-a"])
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
        .args(["--builder-id", "fixture-builder-a"])
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
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(2), "{output:?}");
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    let identity = &report["builder_report"]["resolved_programs"]["approval:gpgv"];
    assert_eq!(
        report["builder_report"]["builder_identity"]["resolved_paths"]["approval:gpgv"],
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
            "#!/bin/sh\n",
            "set -eu\n",
            "printf '%s\\n' mutated >> \"{}\"\n",
            "statement=\"$1\"\n",
            "for arg in \"$@\"; do\n",
            "  statement=\"$arg\"\n",
            "done\n",
            "fingerprint=89abcdef0123456789abcdef0123456789abcdef\n",
            "if /usr/bin/grep -F '\"role\":\"platform-data\"' \"$statement\" >/dev/null 2>&1; then\n",
            "  fingerprint=0123456789abcdef0123456789abcdef01234567\n",
            "fi\n",
            "printf '[GNUPG:] VALIDSIG %s\\n' \"$fingerprint\"\n",
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
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
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
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let failure: serde_json::Value =
        serde_json::from_slice(&fs::read(&output_path).unwrap()).unwrap();
    assert_eq!(failure["stage_outcome"], "HOLD", "{failure:#}");
    assert_eq!(failure["overall_result"], "FAIL", "{failure:#}");
    assert_eq!(failure["failure_phase"], "repository", "{failure:#}");
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
    fs::write(&escaped, b"unrelated file must survive\n").unwrap();

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&escaped)
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(
        fs::read(&escaped).unwrap(),
        b"unrelated file must survive\n",
        "cleanup must never be derived from the rejected output path"
    );
}

#[test]
fn custom_output_name_inside_gate_root_is_rejected() {
    let fixture = CliFixture::new();
    let custom = fixture
        .repository
        .path()
        .join("target/stage-gates/custom.json");
    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(fixture.repository.path())
        .arg("--output")
        .arg(&custom)
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    assert!(!custom.exists());
}

#[test]
fn post_preflight_check_failure_replaces_stale_pass_with_canonical_hold_report() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let config_path = repository.join("config/stage-gates/stage-0.toml");
    let mut source = fs::read_to_string(&config_path).unwrap().replace(
        "STAGE_GATE_FIXTURE_CHECK = \"quality\"",
        concat!(
            "STAGE_GATE_FIXTURE_CHECK = \"quality\"\n",
            "STAGE_GATE_FIXTURE_FAIL = \"1\""
        ),
    );
    let check_program = std::env::current_exe().unwrap();
    let order_path = repository.join("target/stage-gates/check-order.txt");
    source.push_str(&format!(
        r#"

[[checks]]
id = "after-quality"
program = {}
args = ["--exact", "stage_gate_cli_check_helper", "--nocapture"]
cwd = "."
timeout_seconds = 5

[checks.env]
STAGE_GATE_FIXTURE_CHECK = "after-quality"
STAGE_GATE_FIXTURE_ORDER = {}
"#,
        toml_string(&check_program),
        toml_string(&order_path),
    ));
    fs::write(&config_path, source).unwrap();
    git(repository, ["add", "."]);
    git(
        repository,
        ["commit", "-q", "-m", "make quality check fail"],
    );
    let expected_commit = git_output(repository, ["rev-parse", "HEAD"]);
    let output_path = repository.join("target/stage-gates/stage-0.json");
    let builder_path = repository.join("target/stage-gates/stage-0.builder.json");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    fs::write(
        &output_path,
        br#"{"overall_result":"PASS","stage_outcome":"ACCEPTED"}"#,
    )
    .unwrap();
    fs::write(&builder_path, br#"{"stale":"builder-pass"}"#).unwrap();

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(repository)
        .arg("--output")
        .arg(&output_path)
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let bytes = fs::read(&output_path).expect("failure evidence must replace stale PASS");
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bytes, stage_gate::canonical::canonicalize(&report).unwrap());
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["stage_id"], "stage-0");
    assert_eq!(report["stage_outcome"], "HOLD");
    assert_eq!(report["overall_result"], "FAIL");
    assert_eq!(report["error_code"], "check_failed");
    assert_eq!(report["implementation_commit"], expected_commit);
    assert_eq!(report["check_results"]["verify"], "PASS");
    assert_eq!(report["check_results"]["quality"], "FAIL");
    assert_eq!(report["check_results"]["after-quality"], "NOT_RUN");
    assert!(
        !builder_path.exists(),
        "a failed invocation must remove stale builder evidence"
    );
}

#[test]
fn malformed_config_replaces_stale_pass_with_bootstrap_failure_report() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let output_path = repository.join("target/stage-gates/stage-0.json");
    let builder_path = repository.join("target/stage-gates/stage-0.builder.json");
    let historical_builder_path = repository.join("target/stage-gates/stage-0-builder-report.json");
    let input_path = repository.join("target/stage-gates/inputs/builder-b.json");
    fs::create_dir_all(output_path.parent().unwrap()).unwrap();
    fs::create_dir_all(input_path.parent().unwrap()).unwrap();
    fs::write(&output_path, br#"{"overall_result":"PASS"}"#).unwrap();
    fs::write(&builder_path, br#"{"overall_result":"PASS"}"#).unwrap();
    fs::write(&historical_builder_path, br#"{"overall_result":"PASS"}"#).unwrap();
    fs::write(&input_path, br#"{"external":"must survive"}"#).unwrap();
    fs::write(
        repository.join("config/stage-gates/stage-0.toml"),
        b"this is not valid = [toml\n",
    )
    .unwrap();

    let output = Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(repository)
        .arg("--output")
        .arg(&output_path)
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let bytes = fs::read(&output_path).unwrap();
    let report: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(bytes, stage_gate::canonical::canonicalize(&report).unwrap());
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["stage_outcome"], "HOLD");
    assert_eq!(report["overall_result"], "FAIL");
    assert_eq!(report["error_code"], "config_failed");
    assert!(
        !builder_path.exists(),
        "malformed config must invalidate the stable builder report"
    );
    assert!(
        !historical_builder_path.exists(),
        "malformed config must invalidate the previously advertised builder report"
    );
    assert_eq!(
        fs::read(&input_path).unwrap(),
        br#"{"external":"must survive"}"#,
        "bootstrap cleanup must never delete external inputs"
    );
}

#[test]
fn output_root_parent_symlink_swap_continues_only_in_retained_directory() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let marker = repository.join(".git/stage-gate-ready");
    let release = repository.join(".git/stage-gate-release");
    configure_quality_wait(repository, &marker, &release);
    let output_path = repository.join("target/stage-gates/stage-0.json");
    let output_root = repository.join("target/stage-gates");
    let retained_root = repository.join("target/stage-gates-before-swap");
    let outside = TempDir::new().unwrap();

    let child = spawn_gate(repository, &output_path);
    wait_for_path(&marker);
    fs::rename(&output_root, &retained_root).unwrap();
    symlink(outside.path(), &output_root).unwrap();
    fs::write(&release, b"continue\n").unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(
        fs::read_dir(outside.path()).unwrap().count(),
        0,
        "the replaced parent must never receive gate output"
    );
    assert!(
        retained_root.join("stage-0.json").is_file(),
        "the retained output directory must receive the report"
    );
    assert!(!output.status.success(), "{output:?}");
}

#[test]
fn predictable_temporary_symlink_attack_fails_closed_without_overwriting_target() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let marker = repository.join(".git/stage-gate-ready");
    let release = repository.join(".git/stage-gate-release");
    configure_quality_wait(repository, &marker, &release);
    let output_path = repository.join("target/stage-gates/stage-0.json");
    let output_root = repository.join("target/stage-gates");
    let victim = repository.join(".git/must-not-be-overwritten");
    fs::write(&victim, b"original\n").unwrap();

    let child = spawn_gate(repository, &output_path);
    wait_for_path(&marker);
    let predictable = output_root.join(format!(".stage-gate-{}.tmp", child.id()));
    symlink(&victim, predictable).unwrap();
    fs::write(&release, b"continue\n").unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(fs::read(&victim).unwrap(), b"original\n");
    assert!(!output.status.success(), "{output:?}");
}

#[test]
fn precreated_predictable_temporary_file_is_never_reused_or_replaced() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let marker = repository.join(".git/stage-gate-ready");
    let release = repository.join(".git/stage-gate-release");
    configure_quality_wait(repository, &marker, &release);
    let output_path = repository.join("target/stage-gates/stage-0.json");
    let output_root = repository.join("target/stage-gates");

    let child = spawn_gate(repository, &output_path);
    wait_for_path(&marker);
    let predictable = output_root.join(format!(".stage-gate-{}.tmp", child.id()));
    fs::write(&predictable, b"attacker-owned\n").unwrap();
    fs::write(&release, b"continue\n").unwrap();
    let _output = child.wait_with_output().unwrap();

    assert_eq!(
        fs::read(&predictable).unwrap(),
        b"attacker-owned\n",
        "exclusive randomized temporary creation must not reuse a predictable file"
    );
}

#[test]
fn final_output_symlink_race_is_removed_without_following_it() {
    let fixture = CliFixture::new();
    let repository = fixture.repository.path();
    let marker = repository.join(".git/stage-gate-ready");
    let release = repository.join(".git/stage-gate-release");
    configure_quality_wait(repository, &marker, &release);
    let output_path = repository.join("target/stage-gates/stage-0.json");
    let victim = repository.join(".git/final-output-victim");
    fs::write(&victim, b"original\n").unwrap();

    let child = spawn_gate(repository, &output_path);
    wait_for_path(&marker);
    fs::remove_file(&output_path).unwrap();
    symlink(&victim, &output_path).unwrap();
    fs::write(&release, b"continue\n").unwrap();
    let output = child.wait_with_output().unwrap();

    assert_eq!(fs::read(&victim).unwrap(), b"original\n");
    assert!(
        fs::symlink_metadata(&output_path).unwrap().is_file(),
        "the raced symlink must be replaced as a directory entry"
    );
    assert_eq!(output.status.code(), Some(2), "{output:?}");
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
    let os = config
        .builder
        .tools
        .iter()
        .find(|tool| tool.id == "os")
        .unwrap();
    assert_eq!(os.args, ["-s", "-r", "-m"]);
    assert!(
        !os.args.iter().any(|argument| argument == "-a"),
        "OS evidence must exclude hostname-bearing uname -a output"
    );
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
    assert!(source.contains("\"${compose[@]}\" up -d"));
    assert!(!source.contains("up -d --wait"));
    assert!(source.contains("wait-for-dev-stack.sh"));
    assert!(source.contains("down --timeout 60 --volumes --remove-orphans"));
    assert!(source.contains("stage-0.override.yaml"));
    assert!(source.contains("--project-name"));
    assert!(!source.contains("alpha-desk-dev_"));
}

#[test]
fn compose_partial_start_failure_cleans_only_unique_gate_owned_resources() {
    let fixture = ComposeSmokeFixture::new();
    let output = fixture.run("partial");

    assert!(!output.status.success(), "{output:?}");
    let calls = fixture.calls();
    let compose_calls = calls
        .lines()
        .filter(|line| line.contains(" compose ") || line.starts_with("compose "))
        .collect::<Vec<_>>();
    assert!(!compose_calls.is_empty());
    assert!(compose_calls.iter().all(|line| {
        line.contains("--project-name alpha-desk-stage0-")
            && line.contains("compose.yaml")
            && line.contains("stage-0.override.yaml")
    }));
    assert!(
        compose_calls
            .iter()
            .any(|line| line.contains("down --timeout 60 --volumes --remove-orphans"))
    );
    assert!(calls.contains("volume inspect alpha-desk-stage0-"));
    assert!(calls.contains("network inspect alpha-desk-stage0-"));
    assert!(!calls.contains("alpha-desk-dev"));
}

#[test]
fn compose_timeout_cleans_only_the_timed_out_gate_project() {
    let fixture = ComposeSmokeFixture::new();

    let output = fixture.run("timeout");

    assert_eq!(output.status.code(), Some(124), "{output:?}");
    let calls = fixture.calls();
    let cleanup = calls
        .lines()
        .find(|line| line.contains(" down "))
        .expect("timeout must trigger cleanup");
    assert!(cleanup.contains("--project-name alpha-desk-stage0-"));
    assert!(cleanup.contains("down --timeout 60 --volumes --remove-orphans"));
    assert!(!cleanup.contains("alpha-desk-dev"));
}

#[test]
fn compose_signal_interruption_cleans_only_the_interrupted_gate_project() {
    let fixture = ComposeSmokeFixture::new();
    let mut child = fixture.spawn("signal");
    wait_for_path(&fixture.signal_marker);
    let status = Command::new("/bin/kill")
        .args(["-TERM", &format!("-{}", child.id())])
        .status()
        .unwrap();
    assert!(status.success());

    let status = child.wait().unwrap();

    assert!(!status.success());
    let calls = fixture.calls();
    let cleanup = calls
        .lines()
        .find(|line| line.contains(" down "))
        .expect("signal must trigger cleanup");
    assert!(cleanup.contains("--project-name alpha-desk-stage0-"));
    assert!(cleanup.contains("down --timeout 60 --volumes --remove-orphans"));
    assert!(!cleanup.contains("alpha-desk-dev"));
}

#[test]
fn two_compose_runs_use_distinct_projects_and_never_cross_delete() {
    let fixture = ComposeSmokeFixture::new();

    assert!(fixture.run("success").status.success());
    assert!(fixture.run("success").status.success());

    let calls = fixture.calls();
    let cleanup_projects = calls
        .lines()
        .filter(|line| line.contains(" down "))
        .map(compose_project_from_call)
        .collect::<Vec<_>>();
    assert_eq!(cleanup_projects.len(), 2, "{calls}");
    assert_ne!(cleanup_projects[0], cleanup_projects[1], "{calls}");
    assert!(
        cleanup_projects
            .iter()
            .all(|project| project.starts_with("alpha-desk-stage0-"))
    );
    assert!(!calls.contains("alpha-desk-dev"));
}

#[test]
fn compose_rejects_merged_config_that_retains_occupied_base_ports() {
    let fixture = ComposeSmokeFixture::new();

    let output = fixture.run("merged-conflict");

    assert!(!output.status.success());
    let calls = fixture.calls();
    assert!(calls.contains("config --format json"), "{calls}");
    assert!(
        !calls.lines().any(|line| line.contains(" up ")),
        "merged config must fail before startup: {calls}"
    );
}

#[test]
fn compose_rejects_external_resources_even_when_their_names_match() {
    let fixture = ComposeSmokeFixture::new();

    let output = fixture.run("external-resource");

    assert!(!output.status.success(), "{output:?}");
    assert!(
        !fixture.calls().lines().any(|line| line.contains(" up ")),
        "external resources must be rejected before startup"
    );
}

#[test]
fn compose_success_parameterizes_wait_consumer_with_exact_gate_ports() {
    let fixture = ComposeSmokeFixture::new();

    let output = fixture.run("success");

    assert!(output.status.success(), "{output:?}");
    let calls = fixture.calls();
    let project = calls
        .lines()
        .find(|line| line.contains(" config --format json"))
        .map(compose_project_from_call)
        .unwrap();
    assert_eq!(project, project.to_ascii_lowercase());
    let wait = fs::read_to_string(&fixture.wait_log).unwrap();
    let fields = wait.trim().split('|').collect::<Vec<_>>();
    assert_eq!(fields.len(), 8, "{wait}");
    assert!(fields[0].starts_with("alpha-desk-stage0-"));
    assert!(fields[1].contains("compose.yaml"));
    assert!(fields[1].contains("stage-0.override.yaml"));
    assert_eq!(
        &fields[2..],
        &["18222", "18123", "15432", "19000", "13134", "18428"]
    );
}

#[test]
fn stage_zero_compose_override_uses_dedicated_ports_and_gate_owned_resources() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let override_source =
        fs::read_to_string(manifest_dir.join("../../infra/docker-compose/stage-0.override.yaml"))
            .expect("the Stage 0 override must be committed");
    assert!(
        override_source.matches("ports: !override").count() >= 6,
        "every service port list must fully replace the base list"
    );
    for mapping in [
        "14222:4222",
        "18222:8222",
        "18123:8123",
        "15432:5432",
        "19000:9000",
        "14317:4317",
        "14318:4318",
        "13134:13133",
        "18428:8428",
    ] {
        assert!(
            override_source.contains(mapping),
            "missing dedicated Stage 0 mapping {mapping}"
        );
    }
    for occupied in ["127.0.0.1:5432:5432", "127.0.0.1:9000:9000"] {
        assert!(
            !override_source.contains(occupied),
            "Stage 0 must not bind occupied host mapping {occupied}"
        );
    }
    for owned_name in [
        "${STAGE_GATE_COMPOSE_PROJECT}_nats-data",
        "${STAGE_GATE_COMPOSE_PROJECT}_clickhouse-data",
        "${STAGE_GATE_COMPOSE_PROJECT}_postgres-data",
        "${STAGE_GATE_COMPOSE_PROJECT}_minio-data",
        "${STAGE_GATE_COMPOSE_PROJECT}_victoriametrics-data",
        "${STAGE_GATE_COMPOSE_PROJECT}_network",
    ] {
        assert!(
            override_source.contains(owned_name),
            "Stage 0 resource must use exact owned name {owned_name}"
        );
    }
    assert!(!override_source.contains("alpha-desk-dev_"));

    let wait_source =
        fs::read_to_string(manifest_dir.join("../../tools/ci/wait-for-dev-stack.sh")).unwrap();
    for variable in [
        "DEV_STACK_COMPOSE_PROJECT",
        "DEV_STACK_COMPOSE_FILES",
        "DEV_STACK_NATS_MONITOR_PORT",
        "DEV_STACK_CLICKHOUSE_PORT",
        "DEV_STACK_POSTGRES_PORT",
        "DEV_STACK_MINIO_PORT",
        "DEV_STACK_OTEL_HEALTH_PORT",
        "DEV_STACK_VICTORIAMETRICS_PORT",
    ] {
        assert!(
            wait_source.contains(variable),
            "wait consumer must support {variable}"
        );
    }
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
    if std::env::var("STAGE_GATE_FIXTURE_FAIL").as_deref() == Ok("1") {
        std::process::exit(29);
    }
    if let Ok(marker) = std::env::var("STAGE_GATE_FIXTURE_WAIT_MARKER") {
        fs::write(marker, b"ready\n").unwrap();
        let release = PathBuf::from(std::env::var("STAGE_GATE_FIXTURE_RELEASE").unwrap());
        let deadline = Instant::now() + Duration::from_secs(5);
        while !release.exists() {
            assert!(
                Instant::now() < deadline,
                "test release marker was not written"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    std::process::exit(0);
}

#[test]
fn stage_gate_cli_identity_helper() {
    println!("fixture-tool 1.0.0");
    println!("host: fixture-target");
    std::process::exit(0);
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
second_builder_signature_path = "target/stage-gates/external/builder-b.json.asc"
signer_role = "builder-b"

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
signature_path = "target/stage-gates/external/github.json.asc"
signer_role = "github-ci"
repository = "s1korrrr/alpha-desk"
repository_id = 1311268858
repository_owner_id = 24563931
workflow = ".github/workflows/stage-0-evidence.yml"
workflow_ref = "s1korrrr/alpha-desk/.github/workflows/stage-0-evidence.yml@refs/heads/main"
trigger_workflow_id = 321251517
trigger_workflow_name = "CI"
trigger_workflow_path = ".github/workflows/ci.yml"
event_name = "push"
git_ref = "refs/heads/main"
signing_check_name = "Stage 0 evidence signing"
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

fn configure_quality_wait(repository: &Path, marker: &Path, release: &Path) {
    let config_path = repository.join("config/stage-gates/stage-0.toml");
    let source = fs::read_to_string(&config_path).unwrap().replace(
        "STAGE_GATE_FIXTURE_CHECK = \"quality\"",
        &format!(
            concat!(
                "STAGE_GATE_FIXTURE_CHECK = \"quality\"\n",
                "STAGE_GATE_FIXTURE_WAIT_MARKER = {}\n",
                "STAGE_GATE_FIXTURE_RELEASE = {}"
            ),
            toml_string(marker),
            toml_string(release)
        ),
    );
    fs::write(&config_path, source).unwrap();
    git(repository, ["add", "."]);
    git(
        repository,
        ["commit", "-q", "-m", "add deterministic output race hook"],
    );
}

fn spawn_gate(repository: &Path, output_path: &Path) -> std::process::Child {
    Command::new(stage_gate_binary())
        .args(["run", "config/stage-gates/stage-0.toml", "--repository"])
        .arg(repository)
        .arg("--output")
        .arg(output_path)
        .args(["--builder-id", "fixture-builder-a"])
        .env_clear()
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_for_path(path: &Path) {
    // Reaching this hook includes gate startup, identity probing, and the
    // preceding check. Under a parallel workspace test run those steps can
    // legitimately take longer than the individual check's five-second
    // timeout. Keep the harness wait bounded by the gate's declared
    // whole-run budget instead of conflating scheduler delay with a failed
    // race-safety assertion.
    let deadline = Instant::now() + Duration::from_secs(30);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn write_executable(path: &Path, source: &str) {
    fs::write(path, source).unwrap();
    let mut permissions = fs::metadata(path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

struct ComposeSmokeFixture {
    _temp: TempDir,
    script: PathBuf,
    fake_bin: PathBuf,
    docker_log: PathBuf,
    wait_log: PathBuf,
    signal_marker: PathBuf,
}

impl ComposeSmokeFixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let root = temp.path();
        let tools = root.join("tools/ci");
        let infra = root.join("infra/docker-compose");
        let fake_bin = root.join("fake-bin");
        fs::create_dir_all(&tools).unwrap();
        fs::create_dir_all(&infra).unwrap();
        fs::create_dir_all(&fake_bin).unwrap();
        let script = tools.join("stage-0-compose-smoke.sh");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tools/ci/stage-0-compose-smoke.sh"),
            &script,
        )
        .unwrap();
        write_executable(
            &infra.join("test-contract.sh"),
            "#!/bin/sh\nset -eu\nexit 0\n",
        );
        write_executable(
            &tools.join("wait-for-dev-stack.sh"),
            concat!(
                "#!/bin/sh\n",
                "set -eu\n",
                "printf '%s\\n' ",
                "\"$DEV_STACK_COMPOSE_PROJECT|$DEV_STACK_COMPOSE_FILES|",
                "$DEV_STACK_NATS_MONITOR_PORT|$DEV_STACK_CLICKHOUSE_PORT|",
                "$DEV_STACK_POSTGRES_PORT|$DEV_STACK_MINIO_PORT|",
                "$DEV_STACK_OTEL_HEALTH_PORT|$DEV_STACK_VICTORIAMETRICS_PORT\" ",
                ">> \"$STAGE_GATE_WAIT_LOG\"\n",
            ),
        );
        fs::write(infra.join("compose.yaml"), "services: {}\n").unwrap();
        fs::write(
            infra.join("stage-0.override.yaml"),
            "services: {}\nvolumes: {}\nnetworks: {}\n",
        )
        .unwrap();
        let docker_log = root.join("docker.log");
        let wait_log = root.join("wait.log");
        let signal_marker = root.join("signal-ready");
        write_executable(
            &fake_bin.join("docker"),
            concat!(
                "#!/bin/sh\n",
                "set -eu\n",
                "printf '%s\\n' \"$*\" >> \"$STAGE_GATE_DOCKER_LOG\"\n",
                "project=''\n",
                "previous=''\n",
                "for argument in \"$@\"; do\n",
                "  if [ \"$previous\" = '--project-name' ]; then project=\"$argument\"; fi\n",
                "  previous=\"$argument\"\n",
                "done\n",
                "case \" $* \" in\n",
                "  *' volume inspect '*|*' network inspect '*) exit 1 ;;\n",
                "  *' config --format json '*)\n",
                "    if [ \"$STAGE_GATE_FAKE_SCENARIO\" = 'merged-conflict' ]; then\n",
                "      postgres_port=5432\n",
                "      minio_port=9000\n",
                "    else\n",
                "      postgres_port=15432\n",
                "      minio_port=19000\n",
                "    fi\n",
                "    if [ \"$STAGE_GATE_FAKE_SCENARIO\" = 'external-resource' ]; then\n",
                "      external=',\"external\":true'\n",
                "    else\n",
                "      external=''\n",
                "    fi\n",
                "    printf '%s\\n' ",
                "'{\"name\":\"'\"$project\"'\",\"services\":{",
                "\"nats\":{\"ports\":[{\"host_ip\":\"127.0.0.1\",\"published\":\"14222\",\"target\":4222},{\"host_ip\":\"127.0.0.1\",\"published\":\"18222\",\"target\":8222}]},",
                "\"clickhouse\":{\"ports\":[{\"host_ip\":\"127.0.0.1\",\"published\":\"18123\",\"target\":8123}]},",
                "\"postgres\":{\"ports\":[{\"host_ip\":\"127.0.0.1\",\"published\":\"'\"$postgres_port\"'\",\"target\":5432}]},",
                "\"minio\":{\"ports\":[{\"host_ip\":\"127.0.0.1\",\"published\":\"'\"$minio_port\"'\",\"target\":9000}]},",
                "\"otel-collector\":{\"ports\":[{\"host_ip\":\"127.0.0.1\",\"published\":\"14317\",\"target\":4317},{\"host_ip\":\"127.0.0.1\",\"published\":\"14318\",\"target\":4318},{\"host_ip\":\"127.0.0.1\",\"published\":\"13134\",\"target\":13133}]},",
                "\"victoriametrics\":{\"ports\":[{\"host_ip\":\"127.0.0.1\",\"published\":\"18428\",\"target\":8428}]}},",
                "\"volumes\":{\"nats-data\":{\"name\":\"'\"$project\"'_nats-data\",\"driver\":\"local\"'\"$external\"'},",
                "\"clickhouse-data\":{\"name\":\"'\"$project\"'_clickhouse-data\",\"driver\":\"local\"},",
                "\"postgres-data\":{\"name\":\"'\"$project\"'_postgres-data\",\"driver\":\"local\"},",
                "\"minio-data\":{\"name\":\"'\"$project\"'_minio-data\",\"driver\":\"local\"},",
                "\"victoriametrics-data\":{\"name\":\"'\"$project\"'_victoriametrics-data\",\"driver\":\"local\"}},",
                "\"networks\":{\"default\":{\"name\":\"'\"$project\"'_network\",\"driver\":\"bridge\",",
                "\"ipam\":{\"config\":[{\"subnet\":\"172.31.0.0/16\"}]}'\"$external\"'}}}'\n",
                "    ;;\n",
                "  *' up '*)\n",
                "    case \"$STAGE_GATE_FAKE_SCENARIO\" in\n",
                "      partial) exit 17 ;;\n",
                "      timeout) exit 124 ;;\n",
                "      signal)\n",
                "        : > \"$STAGE_GATE_SIGNAL_MARKER\"\n",
                "        sleep 30\n",
                "        ;;\n",
                "      success) exit 0 ;;\n",
                "      *) exit 92 ;;\n",
                "    esac\n",
                "    ;;\n",
                "  *) exit 0 ;;\n",
                "esac\n",
            ),
        );
        Self {
            _temp: temp,
            script,
            fake_bin,
            docker_log,
            wait_log,
            signal_marker,
        }
    }

    fn command(&self, scenario: &str) -> Command {
        let inherited_path = std::env::var("PATH").unwrap();
        let mut command = Command::new(&self.script);
        command
            .env_clear()
            .env(
                "PATH",
                format!("{}:{inherited_path}", self.fake_bin.display()),
            )
            .env("STAGE_GATE_DOCKER_LOG", &self.docker_log)
            .env("STAGE_GATE_WAIT_LOG", &self.wait_log)
            .env("STAGE_GATE_SIGNAL_MARKER", &self.signal_marker)
            .env("STAGE_GATE_FAKE_SCENARIO", scenario);
        command
    }

    fn run(&self, scenario: &str) -> std::process::Output {
        self.command(scenario).output().unwrap()
    }

    fn spawn(&self, scenario: &str) -> std::process::Child {
        let mut command = self.command(scenario);
        command
            .process_group(0)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap()
    }

    fn calls(&self) -> String {
        fs::read_to_string(&self.docker_log).unwrap()
    }
}

fn compose_project_from_call(call: &str) -> &str {
    let fields = call.split_ascii_whitespace().collect::<Vec<_>>();
    let index = fields
        .iter()
        .position(|field| *field == "--project-name")
        .expect("Compose call must include --project-name");
    fields[index + 1]
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
