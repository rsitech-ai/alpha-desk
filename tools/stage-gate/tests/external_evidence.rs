use std::{fs, path::PathBuf};

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
        vec![
            RemoteProofReasonCode::ImplementationCommitMismatch,
            RemoteProofReasonCode::AppSourceMismatch,
            RemoteProofReasonCode::RequiredCheckMissing,
        ]
    );
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
        app_source: "rsitech-ai/alpha-desk".to_owned(),
        required_checks: vec!["rust-linux".to_owned(), "swift-macos".to_owned()],
    }
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
