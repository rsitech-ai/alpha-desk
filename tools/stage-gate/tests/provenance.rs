#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use sha2::Digest as _;
use stage_gate::{
    approvals::{TrustPolicy, TrustedReviewer},
    artifacts::{ArtifactManifest, ArtifactRecord},
    canonical::canonicalize,
    config::{BuilderConfig, BuilderToolConfig},
    provenance::{
        SignedEvidence, SignedEvidenceErrorCode, verify_signed_builder_report,
        verify_signed_remote_proof,
    },
    remote::RemoteRequirement,
    reports::{BuilderEnvironment, BuilderIdentity, BuilderReport, ExecutableEvidence, GateResult},
};
use tempfile::TempDir;

const PLATFORM_FINGERPRINT: &str = "0123456789abcdef0123456789abcdef01234567";
const INDEPENDENT_FINGERPRINT: &str = "89abcdef0123456789abcdef0123456789abcdef";
const BUILDER_B_FINGERPRINT: &str = "fedcba9876543210fedcba9876543210fedcba98";
const GITHUB_CI_FINGERPRINT: &str = "76543210fedcba9876543210fedcba9876543210";
const REQUIRED_CHECKS: [&str; 6] = [
    "Rust quality",
    "Rust tests",
    "Swift 6.3",
    "Static Compose policy",
    "Trusted integration smoke",
    "Reproducible service binaries",
];

#[test]
fn signed_builder_report_binds_exact_canonical_bytes_role_fingerprint_and_identity() {
    let fixture = SignedFixture::new();
    let local = builder_report("builder-a", "local", "");
    let builder_b = builder_report(
        &format!("builder-b:{BUILDER_B_FINGERPRINT}"),
        "builder-b",
        BUILDER_B_FINGERPRINT,
    );
    let report_path = fixture.temp.path().join("builder-b.json");
    let signature_path = fixture.temp.path().join("builder-b.json.asc");
    let report_bytes = canonicalize(&builder_b).unwrap();
    fs::write(&report_path, &report_bytes).unwrap();
    fs::write(&signature_path, b"detached-signature").unwrap();

    let verified = verify_signed_builder_report(
        &SignedEvidence {
            role: "builder-b".to_owned(),
            payload_path: report_path,
            signature_path,
        },
        &local,
        &builder_config(),
        &fixture.policy,
        fixture.verifier.clone(),
        1024 * 1024,
    )
    .unwrap();

    assert_eq!(verified.canonical_bytes, report_bytes);
    assert_eq!(
        verified.sha256,
        hex::encode(sha2::Sha256::digest(&verified.canonical_bytes))
    );
    assert_eq!(verified.signer_role, "builder-b");
    assert_eq!(verified.signer_fingerprint, BUILDER_B_FINGERPRINT);
    assert_eq!(verified.value, builder_b);
}

#[test]
fn signed_builder_authentication_defers_version_and_projection_comparison_to_gate_policy() {
    let fixture = SignedFixture::new();
    let local = builder_report("builder-a", "local", "");
    for (scenario, mut builder_b) in [
        (
            "version-mismatch",
            builder_report(
                &format!("builder-b:{BUILDER_B_FINGERPRINT}"),
                "builder-b",
                BUILDER_B_FINGERPRINT,
            ),
        ),
        (
            "projection-mismatch",
            builder_report(
                &format!("builder-b:{BUILDER_B_FINGERPRINT}"),
                "builder-b",
                BUILDER_B_FINGERPRINT,
            ),
        ),
    ] {
        if scenario == "version-mismatch" {
            builder_b
                .environment
                .toolchains
                .get_mut("swift")
                .unwrap()
                .version_output = "Swift version 6.2".to_owned();
            builder_b.environment.toolchain_fingerprint = hex::encode(sha2::Sha256::digest(
                canonicalize(&builder_b.environment.toolchains).unwrap(),
            ));
        } else {
            builder_b
                .check_evidence_hashes
                .insert("quality".to_owned(), "f".repeat(64));
        }
        let report_path = fixture.temp.path().join(format!("{scenario}.json"));
        let signature_path = fixture.temp.path().join(format!("{scenario}.json.asc"));
        fs::write(&report_path, canonicalize(&builder_b).unwrap()).unwrap();
        fs::write(&signature_path, b"detached-signature").unwrap();

        let verified = verify_signed_builder_report(
            &SignedEvidence {
                role: "builder-b".to_owned(),
                payload_path: report_path,
                signature_path,
            },
            &local,
            &builder_config(),
            &fixture.policy,
            fixture.verifier.clone(),
            1024 * 1024,
        )
        .unwrap_or_else(|error| panic!("{scenario} must authenticate before gate policy: {error}"));

        assert_eq!(verified.value, builder_b);
    }
}

#[test]
fn copied_builder_a_or_free_form_builder_b_identity_fails_closed() {
    let fixture = SignedFixture::new();
    let local = builder_report("builder-a", "local", "");
    for (scenario, report) in [
        ("copied-builder-a", local.clone()),
        (
            "free-form-builder-b",
            builder_report("builder-b-free-form", "builder-b", BUILDER_B_FINGERPRINT),
        ),
        (
            "wrong-role",
            builder_report(
                &format!("builder-b:{BUILDER_B_FINGERPRINT}"),
                "github-ci",
                BUILDER_B_FINGERPRINT,
            ),
        ),
        (
            "wrong-fingerprint",
            builder_report(
                &format!("builder-b:{GITHUB_CI_FINGERPRINT}"),
                "builder-b",
                GITHUB_CI_FINGERPRINT,
            ),
        ),
    ] {
        let report_path = fixture.temp.path().join(format!("{scenario}.json"));
        let signature_path = fixture.temp.path().join(format!("{scenario}.json.asc"));
        fs::write(&report_path, canonicalize(&report).unwrap()).unwrap();
        fs::write(&signature_path, b"detached-signature").unwrap();

        let error = verify_signed_builder_report(
            &SignedEvidence {
                role: "builder-b".to_owned(),
                payload_path: report_path,
                signature_path,
            },
            &local,
            &builder_config(),
            &fixture.policy,
            fixture.verifier.clone(),
            1024 * 1024,
        )
        .expect_err("unbound Builder B identity must fail closed");

        assert_eq!(
            error.code(),
            SignedEvidenceErrorCode::IdentityMismatch,
            "scenario {scenario}: {error:?}"
        );
    }
}

#[test]
fn signed_remote_proof_snapshots_exact_bytes_and_binds_full_run_identity_once() {
    let fixture = SignedFixture::new();
    let proof_path = fixture.temp.path().join("github-proof.json");
    let signature_path = fixture.temp.path().join("github-proof.json.asc");
    let proof_bytes = remote_proof_bytes();
    fs::write(&proof_path, &proof_bytes).unwrap();
    fs::write(&signature_path, b"detached-signature").unwrap();

    let verified = verify_signed_remote_proof(
        &SignedEvidence {
            role: "github-ci".to_owned(),
            payload_path: proof_path,
            signature_path,
        },
        &remote_requirement(),
        &fixture.policy,
        fixture.verifier.clone(),
        1024 * 1024,
    )
    .unwrap();

    assert_eq!(verified.canonical_bytes, proof_bytes);
    assert_eq!(
        verified.sha256,
        hex::encode(sha2::Sha256::digest(&verified.canonical_bytes))
    );
    assert_eq!(verified.signer_role, "github-ci");
    assert_eq!(verified.signer_fingerprint, GITHUB_CI_FINGERPRINT);
    assert_eq!(verified.value.run_id, 8_800_000_001_u64);
    assert_eq!(verified.value.run_attempt, 1);
    assert_eq!(verified.value.signing_check_run_id, 9_900_000_009_u64);
}

#[test]
fn signed_remote_and_builder_payloads_must_be_canonical_and_use_distinct_signers() {
    let fixture = SignedFixture::new();
    let mut policy = fixture.policy.clone();
    policy
        .reviewers
        .iter_mut()
        .find(|reviewer| reviewer.role == "github-ci")
        .unwrap()
        .fingerprint = BUILDER_B_FINGERPRINT.to_owned();
    let proof_path = fixture.temp.path().join("noncanonical-proof.json");
    let signature_path = fixture.temp.path().join("noncanonical-proof.json.asc");
    let value: serde_json::Value = serde_json::from_slice(&remote_proof_bytes()).unwrap();
    fs::write(&proof_path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    fs::write(&signature_path, b"detached-signature").unwrap();

    let error = verify_signed_remote_proof(
        &SignedEvidence {
            role: "github-ci".to_owned(),
            payload_path: proof_path,
            signature_path,
        },
        &remote_requirement(),
        &policy,
        fixture.verifier.clone(),
        1024 * 1024,
    )
    .expect_err("noncanonical bytes and reused signer identity must fail closed");

    assert!(matches!(
        error.code(),
        SignedEvidenceErrorCode::NonCanonical | SignedEvidenceErrorCode::SignerSeparationFailed
    ));
}

fn builder_config() -> BuilderConfig {
    BuilderConfig {
        target_tool: "rustc".to_owned(),
        tools: vec![
            BuilderToolConfig {
                id: "rustc".to_owned(),
                program: "rustc".to_owned(),
                args: vec!["-vV".to_owned()],
                expected_output_contains: Some("rustc 1.97.1".to_owned()),
            },
            BuilderToolConfig {
                id: "cargo".to_owned(),
                program: "cargo".to_owned(),
                args: vec!["--version".to_owned()],
                expected_output_contains: Some("cargo 1.97.1".to_owned()),
            },
            BuilderToolConfig {
                id: "swift".to_owned(),
                program: "swift".to_owned(),
                args: vec!["--version".to_owned()],
                expected_output_contains: Some("Swift version 6.3".to_owned()),
            },
        ],
    }
}

fn builder_report(builder_id: &str, signer_role: &str, signer_fingerprint: &str) -> BuilderReport {
    let tools = BTreeMap::from([
        (
            "rustc".to_owned(),
            executable(
                "rustc",
                "7",
                "rustc 1.97.1 (fixture)\nhost: aarch64-apple-darwin",
            ),
        ),
        (
            "cargo".to_owned(),
            executable("cargo", "8", "cargo 1.97.1 (fixture)"),
        ),
        (
            "swift".to_owned(),
            executable("swift", "9", "Swift version 6.3"),
        ),
    ]);
    let toolchain_fingerprint = hex::encode(sha2::Sha256::digest(canonicalize(&tools).unwrap()));
    BuilderReport {
        schema_version: 1,
        stage_id: "stage-0".to_owned(),
        implementation_commit: "95c4cd709bee9d11e2f7fc591d2861427a36cc3a".to_owned(),
        design_tag_object: "32e520a68e6596027fa0dc9673ddb70706474fef".to_owned(),
        design_commit: "412c380054d16f22549c46a59a5fe0617bc60138".to_owned(),
        config_sha256: "1".repeat(64),
        schema_sha256: "2".repeat(64),
        check_evidence_normalization: "stage-gate-semantic-v1".to_owned(),
        builder_identity: BuilderIdentity {
            builder_id: builder_id.to_owned(),
            signer_role: signer_role.to_owned(),
            signer_fingerprint: signer_fingerprint.to_owned(),
            resolved_paths: BTreeMap::from([
                ("rustc".to_owned(), PathBuf::from("/builder/bin/rustc")),
                ("cargo".to_owned(), PathBuf::from("/builder/bin/cargo")),
                ("swift".to_owned(), PathBuf::from("/builder/bin/swift")),
            ]),
        },
        environment: BuilderEnvironment {
            os_version: "darwin-27.0.0".to_owned(),
            target_triple: "aarch64-apple-darwin".to_owned(),
            toolchains: tools.clone(),
            toolchain_fingerprint,
        },
        resolved_programs: tools,
        check_evidence_hashes: BTreeMap::from([("quality".to_owned(), "a".repeat(64))]),
        artifacts: ArtifactManifest {
            schema_version: 1,
            artifacts: vec![ArtifactRecord {
                logical_name: "alpha-desk".to_owned(),
                relative_path: "dist/alpha-desk".to_owned(),
                kind: "executable".to_owned(),
                size_bytes: 6,
                sha256: "3".repeat(64),
                producer: "cargo-build".to_owned(),
                target_triple: "aarch64-apple-darwin".to_owned(),
                profile: "release".to_owned(),
            }],
        },
        check_results: BTreeMap::from([("quality".to_owned(), GateResult::Pass)]),
    }
}

fn executable(id: &str, hash_digit: &str, version_output: &str) -> ExecutableEvidence {
    ExecutableEvidence {
        id: id.to_owned(),
        sha256: hash_digit.repeat(64),
        version_output: version_output.to_owned(),
    }
}

fn remote_requirement() -> RemoteRequirement {
    RemoteRequirement {
        implementation_commit: "95c4cd709bee9d11e2f7fc591d2861427a36cc3a".to_owned(),
        repository: "s1korrrr/alpha-desk".to_owned(),
        repository_id: 1_311_268_858,
        repository_owner_id: 24_563_931,
        workflow: ".github/workflows/stage-0-evidence.yml".to_owned(),
        workflow_ref: "s1korrrr/alpha-desk/.github/workflows/stage-0-evidence.yml@refs/heads/main"
            .to_owned(),
        workflow_sha: "95c4cd709bee9d11e2f7fc591d2861427a36cc3a".to_owned(),
        trigger_workflow_id: 321_251_517,
        trigger_workflow_name: "CI".to_owned(),
        trigger_workflow_path: ".github/workflows/ci.yml".to_owned(),
        trigger_workflow_sha: "95c4cd709bee9d11e2f7fc591d2861427a36cc3a".to_owned(),
        event_name: "push".to_owned(),
        git_ref: "refs/heads/main".to_owned(),
        signing_check_name: "Stage 0 evidence signing".to_owned(),
        required_checks: REQUIRED_CHECKS.iter().map(ToString::to_string).collect(),
    }
}

fn remote_proof_bytes() -> Vec<u8> {
    let checks = REQUIRED_CHECKS
        .iter()
        .enumerate()
        .map(|(index, name)| {
            serde_json::json!({
                "check_run_id": 9_900_000_001_u64 + index as u64,
                "conclusion": "success",
                "head_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
                "name": name,
                "run_attempt": 1,
                "run_id": 8_800_000_001_u64,
            })
        })
        .collect::<Vec<_>>();
    canonicalize(&serde_json::json!({
        "checks": checks,
        "event_name": "push",
        "git_ref": "refs/heads/main",
        "head_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
        "job_workflow_file_path": ".github/workflows/stage-0-evidence.yml",
        "job_workflow_ref":
            "s1korrrr/alpha-desk/.github/workflows/stage-0-evidence.yml@refs/heads/main",
        "job_workflow_repository": "s1korrrr/alpha-desk",
        "job_workflow_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
        "repository": "s1korrrr/alpha-desk",
        "repository_id": 1_311_268_858_u64,
        "repository_owner_id": 24_563_931_u64,
        "run_attempt": 1,
        "run_id": 8_800_000_001_u64,
        "schema_version": 1,
        "signing_check": {
            "check_run_id": 9_900_000_009_u64,
            "head_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
            "name": "Stage 0 evidence signing",
            "run_attempt": 1,
            "run_id": 8_900_000_001_u64,
            "status": "in_progress",
        },
        "signing_check_run_id": 9_900_000_009_u64,
        "trigger_workflow_id": 321_251_517_u64,
        "trigger_workflow_name": "CI",
        "trigger_workflow_path": ".github/workflows/ci.yml",
        "trigger_workflow_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
        "workflow": ".github/workflows/stage-0-evidence.yml",
        "workflow_ref":
            "s1korrrr/alpha-desk/.github/workflows/stage-0-evidence.yml@refs/heads/main",
        "workflow_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
    }))
    .unwrap()
}

struct SignedFixture {
    temp: TempDir,
    verifier: PathBuf,
    policy: TrustPolicy,
}

impl SignedFixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let verifier = temp.path().join("fake-gpgv");
        let keyring = temp.path().join("trusted-signers.gpg");
        fs::write(&keyring, b"committed keyring fixture").unwrap();
        fs::write(
            &verifier,
            format!(
                concat!(
                    "#!/bin/sh\n",
                    "set -eu\n",
                    "signature=\"$5\"\n",
                    "payload=\"$6\"\n",
                    "case \"$payload\" in\n",
                    "  {}|{}) exit 41 ;;\n",
                    "esac\n",
                    "case \"$signature\" in\n",
                    "  {}|{}) exit 42 ;;\n",
                    "esac\n",
                    "if grep -q '\"builder_identity\"' \"$payload\"; then\n",
                    "  fingerprint={}\n",
                    "else\n",
                    "  fingerprint={}\n",
                    "fi\n",
                    "printf '[GNUPG:] VALIDSIG %s 0 0 0 0 0 0 0 0 0\\n' \"$fingerprint\"\n",
                ),
                shell_path(&temp.path().join("builder-b.json")),
                shell_path(&temp.path().join("github-proof.json")),
                shell_path(&temp.path().join("builder-b.json.asc")),
                shell_path(&temp.path().join("github-proof.json.asc")),
                BUILDER_B_FINGERPRINT,
                GITHUB_CI_FINGERPRINT,
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&verifier).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&verifier, permissions).unwrap();
        let policy = TrustPolicy {
            schema_version: 1,
            keyring_path: keyring,
            reviewers: vec![
                trusted("platform-data", PLATFORM_FINGERPRINT),
                trusted("independent", INDEPENDENT_FINGERPRINT),
                trusted("builder-b", BUILDER_B_FINGERPRINT),
                trusted("github-ci", GITHUB_CI_FINGERPRINT),
            ],
        };
        Self {
            temp,
            verifier,
            policy,
        }
    }
}

fn trusted(role: &str, fingerprint: &str) -> TrustedReviewer {
    TrustedReviewer {
        role: role.to_owned(),
        fingerprint: fingerprint.to_owned(),
    }
}

fn shell_path(path: &Path) -> String {
    format!("{:?}", path.to_string_lossy())
}
