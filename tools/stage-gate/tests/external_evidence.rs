use std::{
    fs,
    os::unix::fs::{PermissionsExt as _, symlink},
    path::PathBuf,
    process::Command,
};

use sha2::Digest as _;
use stage_gate::{
    approvals::{
        ApprovalBinding, ApprovalDecision, ApprovalEvidence, ApprovalReasonCode,
        ApprovalRequirements, ApprovalStatement, GateStatus, TrustPolicy, TrustedReviewer,
        canonical_statement_bytes, verify_approvals,
    },
    remote::{RemoteProofReasonCode, RemoteRequirement, verify_remote_proof},
};
use tempfile::TempDir;

const PLATFORM_FINGERPRINT: &str = "0123456789abcdef0123456789abcdef01234567";
const INDEPENDENT_FINGERPRINT: &str = "89abcdef0123456789abcdef0123456789abcdef";
const REQUIRED_CHECKS: [&str; 6] = [
    "Rust quality",
    "Rust tests",
    "Swift 6.3",
    "Static Compose policy",
    "Trusted integration smoke",
    "Reproducible service binaries",
];

#[test]
fn workflow_jq_command_substitution_emits_exact_canonical_fixture_bytes() {
    let temp = TempDir::new().unwrap();
    let input = temp.path().join("proof.json");
    let value: serde_json::Value = serde_json::from_slice(&authenticated_remote_proof()).unwrap();
    fs::write(&input, serde_json::to_vec_pretty(&value).unwrap()).unwrap();

    let output = Command::new("/bin/bash")
        .args([
            "-c",
            "proof=\"$(jq -cS . \"$1\")\"; printf '%s' \"$proof\"",
            "workflow-proof",
        ])
        .arg(&input)
        .output()
        .unwrap();

    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        output.stdout,
        stage_gate::canonical::canonicalize(&value).unwrap()
    );
}

#[test]
fn approval_statement_bytes_are_canonical_and_exact() {
    let statement = statement();
    let bytes = canonical_statement_bytes(&statement).unwrap();

    assert_eq!(
        String::from_utf8(bytes).unwrap(),
        concat!(
            "{\"aggregate_evidence_sha256\":\"",
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "\",\"comparison_manifest_sha256\":\"",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "\",\"decision\":\"APPROVE\",\"design_commit\":",
            "\"412c380054d16f22549c46a59a5fe0617bc60138\",",
            "\"design_tag_object\":\"32e520a68e6596027fa0dc9673ddb70706474fef\",",
            "\"implementation_commit\":\"95c4cd709bee9d11e2f7fc591d2861427a36cc3a\",",
            "\"known_limitations\":[],\"known_limitations_sha256\":",
            "\"4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945\",",
            "\"role\":\"platform-data\",\"schema_version\":1,",
            "\"signed_at_utc\":\"2026-07-28T00:00:00Z\",",
            "\"signer_fingerprint\":\"0123456789abcdef0123456789abcdef01234567\",",
            "\"stage_id\":\"stage-0\"}"
        )
    );
}

#[test]
fn approval_statement_is_role_specific_and_binds_aggregate_evidence() {
    let platform = statement();
    let mut independent = statement();
    independent.role = "independent".to_owned();
    independent.signer_fingerprint = INDEPENDENT_FINGERPRINT.to_owned();

    assert_ne!(
        canonical_statement_bytes(&platform).unwrap(),
        canonical_statement_bytes(&independent).unwrap()
    );
    platform.validate().unwrap();
    independent.validate().unwrap();
}

#[test]
fn approval_statement_must_match_the_expected_known_limitations() {
    let fixture = ApprovalFixture::new();
    let mut different = statement();
    different.known_limitations = vec!["signer-selected limitation".to_owned()];
    different.known_limitations_sha256 = hex::encode(sha2::Sha256::digest(
        stage_gate::canonical::canonicalize(&different.known_limitations).unwrap(),
    ));
    fs::write(
        &fixture.evidence[0].statement_path,
        canonical_statement_bytes(&different).unwrap(),
    )
    .unwrap();

    let outcome = verify_approvals(
        &fixture.binding,
        &fixture.policy,
        &fixture.evidence,
        PathBuf::from("/definitely/missing/gpgv"),
    );

    assert_eq!(
        outcome.reasons,
        vec![ApprovalReasonCode::ApprovalBindingMismatch]
    );
}

#[test]
fn approval_timestamp_requires_uppercase_t_and_terminal_z() {
    let mut lower_t = statement();
    lower_t.signed_at_utc = "2026-07-28t00:00:00Z".to_owned();

    assert_eq!(
        lower_t.validate(),
        Err(ApprovalReasonCode::ApprovalStatementInvalid)
    );
}

#[test]
fn trust_policy_requires_full_pinned_fingerprints_and_distinct_reviewers() {
    let requirements = ApprovalRequirements::stage_zero();
    let abbreviated = TrustPolicy {
        schema_version: 1,
        keyring_path: PathBuf::from("config/stage-gates/reviewers.gpg"),
        reviewers: vec![
            reviewer("platform-data", "01234567"),
            reviewer("independent", INDEPENDENT_FINGERPRINT),
        ],
    };
    let same_reviewer = TrustPolicy {
        schema_version: 1,
        keyring_path: PathBuf::from("config/stage-gates/reviewers.gpg"),
        reviewers: vec![
            reviewer("platform-data", PLATFORM_FINGERPRINT),
            reviewer("independent", PLATFORM_FINGERPRINT),
        ],
    };

    assert_eq!(
        abbreviated.validate(&requirements).unwrap_err(),
        ApprovalReasonCode::UntrustedReviewer
    );
    assert_eq!(
        same_reviewer.validate(&requirements).unwrap_err(),
        ApprovalReasonCode::ReviewerRoleSeparationFailed
    );
}

#[test]
fn missing_openpgp_tooling_is_blocked() {
    let fixture = ApprovalFixture::new();
    let outcome = verify_approvals(
        &fixture.binding,
        &fixture.policy,
        &fixture.evidence,
        PathBuf::from("/definitely/missing/gpgv"),
    );

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![ApprovalReasonCode::OpenPgpToolingUnavailable]
    );
}

#[test]
fn invalid_detached_signature_is_blocked() {
    let fixture = ApprovalFixture::new();
    let outcome = verify_approvals(
        &fixture.binding,
        &fixture.policy,
        &fixture.evidence,
        PathBuf::from("/usr/bin/false"),
    );

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![ApprovalReasonCode::InvalidDetachedSignature]
    );
}

#[test]
fn verifier_requires_one_clean_validsig_for_the_exact_expected_fingerprint() {
    for (scenario, body) in [
        (
            "nonzero",
            concat!(
                "printf '[GNUPG:] VALIDSIG %s 0 0 0 0 0 0 0 0 0\\n' \"$fingerprint\"\n",
                "exit 9\n"
            ),
        ),
        ("zero-without-validsig", "exit 0\n"),
        (
            "wrong-fingerprint",
            concat!(
                "printf '[GNUPG:] VALIDSIG ",
                "ffffffffffffffffffffffffffffffffffffffff 0 0 0 0 0 0 0 0 0\\n'\n",
            ),
        ),
        (
            "multiple-validsig",
            concat!(
                "printf '[GNUPG:] VALIDSIG %s 0 0 0 0 0 0 0 0 0\\n' \"$fingerprint\"\n",
                "printf '[GNUPG:] VALIDSIG ",
                "ffffffffffffffffffffffffffffffffffffffff 0 0 0 0 0 0 0 0 0\\n'\n",
            ),
        ),
        (
            "bad-signature-status",
            concat!(
                "printf '[GNUPG:] BADSIG %s rejected\\n' \"$fingerprint\"\n",
                "printf '[GNUPG:] VALIDSIG %s 0 0 0 0 0 0 0 0 0\\n' \"$fingerprint\"\n",
            ),
        ),
        (
            "revoked-key-status",
            concat!(
                "printf '[GNUPG:] REVKEYSIG %s revoked\\n' \"$fingerprint\"\n",
                "printf '[GNUPG:] VALIDSIG %s 0 0 0 0 0 0 0 0 0\\n' \"$fingerprint\"\n",
            ),
        ),
        (
            "error-status",
            concat!(
                "printf '[GNUPG:] ERRSIG %s 1 2 3 4 5 6\\n' \"$fingerprint\"\n",
                "printf '[GNUPG:] VALIDSIG %s 0 0 0 0 0 0 0 0 0\\n' \"$fingerprint\"\n",
            ),
        ),
    ] {
        let fixture = ApprovalFixture::new();
        let verifier = fixture._temp.path().join(format!("gpgv-{scenario}"));
        fs::write(
            &verifier,
            format!(
                concat!(
                    "#!/bin/sh\n",
                    "set -eu\n",
                    "statement=\"$6\"\n",
                    "case \"$(sed -n 's/.*\\\"role\\\":\\\"\\([^\\\"]*\\)\\\".*/\\1/p' \"$statement\")\" in\n",
                    "  platform-data) fingerprint={} ;;\n",
                    "  independent) fingerprint={} ;;\n",
                    "  *) exit 43 ;;\n",
                    "esac\n",
                    "{}",
                ),
                PLATFORM_FINGERPRINT, INDEPENDENT_FINGERPRINT, body
            ),
        )
        .unwrap();
        let mut permissions = fs::metadata(&verifier).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(&verifier, permissions).unwrap();

        let outcome = verify_approvals(
            &fixture.binding,
            &fixture.policy,
            &fixture.evidence,
            verifier,
        );

        assert_eq!(
            outcome.status,
            GateStatus::Blocked,
            "scenario {scenario} must fail closed: {outcome:?}"
        );
        assert_eq!(
            outcome.reasons,
            vec![ApprovalReasonCode::InvalidDetachedSignature],
            "scenario {scenario}"
        );
    }
}

#[test]
fn approval_for_different_statement_bytes_is_blocked_before_gpg() {
    let fixture = ApprovalFixture::new();
    fs::write(&fixture.evidence[0].statement_path, b"different bytes").unwrap();

    let outcome = verify_approvals(
        &fixture.binding,
        &fixture.policy,
        &fixture.evidence,
        PathBuf::from("/definitely/missing/gpgv"),
    );

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![ApprovalReasonCode::ApprovalStatementMismatch]
    );
}

#[test]
fn approval_verifier_uses_private_snapshots_instead_of_caller_paths() {
    let fixture = ApprovalFixture::new();
    let verifier = fixture._temp.path().join("snapshot-checking-gpgv");
    let original_paths = fixture
        .evidence
        .iter()
        .flat_map(|item| [&item.signature_path, &item.statement_path])
        .map(|path| format!("{:?}", path.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    fs::write(
        &verifier,
        format!(
            concat!(
                "#!/bin/sh\n",
                "set -eu\n",
                "signature=\"$5\"\n",
                "statement=\"$6\"\n",
                "for original in {}; do\n",
                "  [ \"$signature\" != \"$original\" ] || exit 41\n",
                "  [ \"$statement\" != \"$original\" ] || exit 42\n",
                "done\n",
                "case \"$(sed -n 's/.*\\\"role\\\":\\\"\\([^\\\"]*\\)\\\".*/\\1/p' \"$statement\")\" in\n",
                "  platform-data) fingerprint={} ;;\n",
                "  independent) fingerprint={} ;;\n",
                "  *) exit 43 ;;\n",
                "esac\n",
                "printf '[GNUPG:] VALIDSIG %s\\n' \"$fingerprint\"\n",
            ),
            original_paths,
            PLATFORM_FINGERPRINT,
            INDEPENDENT_FINGERPRINT,
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&verifier).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&verifier, permissions).unwrap();

    let outcome = verify_approvals(
        &fixture.binding,
        &fixture.policy,
        &fixture.evidence,
        verifier,
    );

    assert_eq!(outcome.status, GateStatus::Pass, "{outcome:?}");
    assert!(outcome.reasons.is_empty());
}

#[test]
fn absent_remote_proof_is_blocked() {
    let temp = TempDir::new().unwrap();
    let outcome = verify_remote_proof(&temp.path().join("missing.json"), &remote_requirement());

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![RemoteProofReasonCode::RemoteProofMissing]
    );
}

#[test]
fn remote_proof_must_be_a_regular_non_symlink_file() {
    let temp = TempDir::new().unwrap();
    let target = temp.path().join("proof-target.json");
    let proof = temp.path().join("proof.json");
    fs::write(&target, valid_remote_proof()).unwrap();
    symlink(&target, &proof).unwrap();

    let outcome = verify_remote_proof(&proof, &remote_requirement());

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![RemoteProofReasonCode::RemoteProofMalformed]
    );
}

#[test]
fn remote_proof_must_be_the_exact_canonical_json_bytes() {
    let temp = TempDir::new().unwrap();
    let proof = temp.path().join("proof.json");
    fs::write(
        &proof,
        serde_json::to_vec_pretty(&legacy_remote_proof_value()).unwrap(),
    )
    .unwrap();

    let outcome = verify_remote_proof(&proof, &remote_requirement());

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![RemoteProofReasonCode::RemoteProofMalformed]
    );
}

#[test]
fn malformed_remote_proof_is_blocked() {
    let temp = TempDir::new().unwrap();
    let proof = temp.path().join("proof.json");
    fs::write(&proof, b"{not-json").unwrap();

    let outcome = verify_remote_proof(&proof, &remote_requirement());

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![RemoteProofReasonCode::RemoteProofMalformed]
    );
}

#[test]
fn remote_proof_must_match_commit_source_and_exact_check_names() {
    let temp = TempDir::new().unwrap();
    let proof = temp.path().join("proof.json");
    fs::write(
        &proof,
        br#"{
          "schema_version": 1,
          "implementation_commit": "0000000000000000000000000000000000000000",
          "app_source": "wrong/repository",
          "checks": [
            {"name": "rust-linux", "conclusion": "success"},
            {"name": "renamed-macos", "conclusion": "success"}
          ]
        }"#,
    )
    .unwrap();

    let outcome = verify_remote_proof(&proof, &remote_requirement());

    assert_eq!(outcome.status, GateStatus::Blocked);
    assert_eq!(
        outcome.reasons,
        vec![RemoteProofReasonCode::RemoteProofMalformed]
    );
}

#[test]
fn canonical_remote_proof_with_full_immutable_run_identity_is_accepted() {
    let temp = TempDir::new().unwrap();
    let proof = temp.path().join("proof.json");
    fs::write(&proof, authenticated_remote_proof()).unwrap();

    let outcome = verify_remote_proof(&proof, &remote_requirement());

    assert_eq!(outcome.status, GateStatus::Pass, "{outcome:?}");
    assert!(outcome.reasons.is_empty());
}

#[test]
fn remote_proof_rejects_duplicate_extra_and_cross_run_check_identity() {
    let baseline: serde_json::Value =
        serde_json::from_slice(&authenticated_remote_proof()).unwrap();
    let mut invalid = Vec::new();

    let mut value = baseline.clone();
    value["repository_id"] = serde_json::json!(999_999_u64);
    invalid.push(("repository-id", value));
    let mut value = baseline.clone();
    value["repository_owner_id"] = serde_json::json!(999_998_u64);
    invalid.push(("repository-owner-id", value));
    let mut value = baseline.clone();
    value["workflow_sha"] = serde_json::json!("0".repeat(40));
    invalid.push(("workflow-sha", value));
    let mut value = baseline.clone();
    value["workflow_ref"] =
        serde_json::json!("rsitech-ai/alpha-desk/.github/workflows/other.yml@refs/heads/main");
    invalid.push(("workflow-ref", value));
    let mut value = baseline.clone();
    value["event_name"] = serde_json::json!("pull_request_target");
    invalid.push(("event", value));
    let mut value = baseline.clone();
    value["git_ref"] = serde_json::json!("refs/heads/release");
    invalid.push(("git-ref", value));
    let mut value = baseline.clone();
    value["head_sha"] = serde_json::json!("0".repeat(40));
    invalid.push(("head-sha", value));
    let mut value = baseline.clone();
    value["checks"][0]["run_id"] = serde_json::json!(8_800_000_002_u64);
    invalid.push(("cross-run", value));
    let mut value = baseline.clone();
    value["checks"][0]["run_attempt"] = serde_json::json!(2_u64);
    invalid.push(("cross-attempt", value));
    let mut value = baseline.clone();
    let duplicate_id = value["checks"][0]["check_run_id"].clone();
    value["checks"][1]["check_run_id"] = duplicate_id;
    invalid.push(("duplicate-check-id", value));
    let mut value = baseline.clone();
    let duplicate_name = value["checks"][0]["name"].clone();
    value["checks"][1]["name"] = duplicate_name;
    invalid.push(("duplicate-check-name", value));
    let mut value = baseline.clone();
    let required_check_id = value["checks"][0]["check_run_id"].clone();
    value["signing_check_run_id"] = required_check_id;
    invalid.push(("signer-is-required-check", value));
    let mut value = baseline.clone();
    value["signing_check"]["check_run_id"] = serde_json::json!(9_900_000_099_u64);
    invalid.push(("signing-check-id-mismatch", value));
    let mut value = baseline.clone();
    value["signing_check"]["run_id"] = serde_json::json!(8_800_000_001_u64);
    invalid.push(("signing-check-reuses-ci-run", value));
    let mut value = baseline.clone();
    value["signing_check"]["run_attempt"] = serde_json::json!(0_u64);
    invalid.push(("signing-check-zero-attempt", value));
    let mut value = baseline.clone();
    value["signing_check"]["head_sha"] = serde_json::json!("0".repeat(40));
    invalid.push(("signing-check-cross-head", value));
    let mut value = baseline.clone();
    value["signing_check"]["status"] = serde_json::json!("completed");
    invalid.push(("signing-check-not-in-progress", value));
    let mut value = baseline;
    value["checks"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({
            "check_run_id": 9_900_000_099_u64,
            "conclusion": "success",
            "head_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
            "name": "unexpected",
            "run_attempt": 1,
            "run_id": 8_800_000_001_u64
        }));
    invalid.push(("extra-check", value));

    for (scenario, value) in invalid {
        let temp = TempDir::new().unwrap();
        let proof = temp.path().join("proof.json");
        fs::write(&proof, stage_gate::canonical::canonicalize(&value).unwrap()).unwrap();

        let outcome = verify_remote_proof(&proof, &remote_requirement());

        assert_eq!(
            outcome.status,
            GateStatus::Blocked,
            "scenario {scenario} must fail closed: {outcome:?}"
        );
    }
}

fn statement() -> ApprovalStatement {
    ApprovalStatement {
        schema_version: 1,
        stage_id: "stage-0".to_owned(),
        role: "platform-data".to_owned(),
        decision: ApprovalDecision::Approve,
        implementation_commit: "95c4cd709bee9d11e2f7fc591d2861427a36cc3a".to_owned(),
        design_tag_object: "32e520a68e6596027fa0dc9673ddb70706474fef".to_owned(),
        design_commit: "412c380054d16f22549c46a59a5fe0617bc60138".to_owned(),
        aggregate_evidence_sha256: "a".repeat(64),
        comparison_manifest_sha256: "b".repeat(64),
        known_limitations: Vec::new(),
        known_limitations_sha256:
            "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".to_owned(),
        signer_fingerprint: PLATFORM_FINGERPRINT.to_owned(),
        signed_at_utc: "2026-07-28T00:00:00Z".to_owned(),
    }
}

fn reviewer(role: &str, fingerprint: &str) -> TrustedReviewer {
    TrustedReviewer {
        role: role.to_owned(),
        fingerprint: fingerprint.to_owned(),
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
        event_name: "push".to_owned(),
        git_ref: "refs/heads/main".to_owned(),
        signing_check_name: "Stage 0 evidence signing".to_owned(),
        required_checks: REQUIRED_CHECKS.iter().map(ToString::to_string).collect(),
    }
}

fn legacy_remote_proof_value() -> serde_json::Value {
    serde_json::json!({
        "schema_version": 1,
        "implementation_commit": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
        "app_source": "s1korrrr/alpha-desk",
        "checks": REQUIRED_CHECKS
            .iter()
            .map(|name| serde_json::json!({"name": name, "conclusion": "success"}))
            .collect::<Vec<_>>(),
    })
}

fn valid_remote_proof() -> Vec<u8> {
    stage_gate::canonical::canonicalize(&legacy_remote_proof_value()).unwrap()
}

fn authenticated_remote_proof() -> Vec<u8> {
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
    stage_gate::canonical::canonicalize(&serde_json::json!({
        "checks": checks,
        "event_name": "push",
        "git_ref": "refs/heads/main",
        "head_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
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
        "workflow": ".github/workflows/stage-0-evidence.yml",
        "workflow_ref":
            "s1korrrr/alpha-desk/.github/workflows/stage-0-evidence.yml@refs/heads/main",
        "workflow_sha": "95c4cd709bee9d11e2f7fc591d2861427a36cc3a",
    }))
    .unwrap()
}

struct ApprovalFixture {
    _temp: TempDir,
    binding: ApprovalBinding,
    policy: TrustPolicy,
    evidence: Vec<ApprovalEvidence>,
}

impl ApprovalFixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let binding = binding();
        let mut evidence = Vec::new();
        for (role, fingerprint) in [
            ("platform-data", PLATFORM_FINGERPRINT),
            ("independent", INDEPENDENT_FINGERPRINT),
        ] {
            let statement_path = temp.path().join(format!("{role}.statement.json"));
            let signature_path = temp.path().join(format!("{role}.statement.json.asc"));
            let mut role_statement = statement();
            role_statement.role = role.to_owned();
            role_statement.signer_fingerprint = fingerprint.to_owned();
            let statement_bytes = canonical_statement_bytes(&role_statement).unwrap();
            fs::write(&statement_path, &statement_bytes).unwrap();
            fs::write(&signature_path, b"invalid test signature").unwrap();
            evidence.push(ApprovalEvidence {
                role: role.to_owned(),
                claimed_fingerprint: fingerprint.to_owned(),
                statement_path,
                signature_path,
            });
        }
        Self {
            _temp: temp,
            binding,
            policy: TrustPolicy {
                schema_version: 1,
                keyring_path: PathBuf::from("config/stage-gates/reviewers.gpg"),
                reviewers: vec![
                    reviewer("platform-data", PLATFORM_FINGERPRINT),
                    reviewer("independent", INDEPENDENT_FINGERPRINT),
                ],
            },
            evidence,
        }
    }
}

fn binding() -> ApprovalBinding {
    ApprovalBinding {
        stage_id: "stage-0".to_owned(),
        implementation_commit: "95c4cd709bee9d11e2f7fc591d2861427a36cc3a".to_owned(),
        design_tag_object: "32e520a68e6596027fa0dc9673ddb70706474fef".to_owned(),
        design_commit: "412c380054d16f22549c46a59a5fe0617bc60138".to_owned(),
        aggregate_evidence_sha256: "a".repeat(64),
        comparison_manifest_sha256: "b".repeat(64),
        known_limitations: Vec::new(),
        known_limitations_sha256:
            "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945".to_owned(),
    }
}
