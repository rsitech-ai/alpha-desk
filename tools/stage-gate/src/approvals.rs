use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::canonical::{CanonicalError, canonicalize, canonicalize_json_str};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GateStatus {
    Pass,
    Fail,
    Blocked,
    NotRun,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ApprovalStatement {
    pub schema_version: u32,
    pub stage_id: String,
    pub role: String,
    pub decision: ApprovalDecision,
    pub implementation_commit: String,
    pub design_tag_object: String,
    pub design_commit: String,
    pub aggregate_evidence_sha256: String,
    pub comparison_manifest_sha256: String,
    pub known_limitations: Vec<String>,
    pub known_limitations_sha256: String,
    pub signer_fingerprint: String,
    pub signed_at_utc: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ApprovalDecision {
    Approve,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalBinding {
    pub stage_id: String,
    pub implementation_commit: String,
    pub design_tag_object: String,
    pub design_commit: String,
    pub aggregate_evidence_sha256: String,
    pub comparison_manifest_sha256: String,
    pub known_limitations: Vec<String>,
    pub known_limitations_sha256: String,
}

pub fn canonical_statement_bytes(statement: &ApprovalStatement) -> Result<Vec<u8>, CanonicalError> {
    canonicalize(statement)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedReviewer {
    pub role: String,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct TrustPolicy {
    pub schema_version: u32,
    pub keyring_path: PathBuf,
    pub reviewers: Vec<TrustedReviewer>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalRequirements {
    pub required_roles: Vec<String>,
}

impl ApprovalRequirements {
    #[must_use]
    pub fn stage_zero() -> Self {
        Self {
            required_roles: vec!["platform-data".to_owned(), "independent".to_owned()],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalEvidence {
    pub role: String,
    pub claimed_fingerprint: String,
    pub statement_path: PathBuf,
    pub signature_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum ApprovalReasonCode {
    ApprovalPolicyInvalid,
    RequiredApprovalMissing,
    UntrustedReviewer,
    ReviewerRoleSeparationFailed,
    ApprovalStatementMismatch,
    ApprovalStatementInvalid,
    ApprovalBindingMismatch,
    OpenPgpToolingUnavailable,
    InvalidDetachedSignature,
}

impl ApprovalStatement {
    pub fn validate(&self) -> Result<(), ApprovalReasonCode> {
        if self.schema_version != 1
            || !matches!(self.decision, ApprovalDecision::Approve)
            || !is_full_fingerprint(&self.signer_fingerprint)
            || !is_lower_hex(&self.implementation_commit, 40)
            || !is_lower_hex(&self.design_tag_object, 40)
            || !is_lower_hex(&self.design_commit, 40)
            || !is_lower_hex(&self.aggregate_evidence_sha256, 64)
            || !is_lower_hex(&self.comparison_manifest_sha256, 64)
            || !is_lower_hex(&self.known_limitations_sha256, 64)
            || self.role.is_empty()
            || self.stage_id.is_empty()
        {
            return Err(ApprovalReasonCode::ApprovalStatementInvalid);
        }
        let timestamp = chrono::DateTime::parse_from_rfc3339(&self.signed_at_utc)
            .map_err(|_| ApprovalReasonCode::ApprovalStatementInvalid)?;
        if timestamp.offset().local_minus_utc() != 0
            || self.signed_at_utc.as_bytes().get(10) != Some(&b'T')
            || !self.signed_at_utc.ends_with('Z')
        {
            return Err(ApprovalReasonCode::ApprovalStatementInvalid);
        }
        let limitations = canonicalize(&self.known_limitations)
            .map_err(|_| ApprovalReasonCode::ApprovalStatementInvalid)?;
        if hex::encode(Sha256::digest(limitations)) != self.known_limitations_sha256 {
            return Err(ApprovalReasonCode::ApprovalStatementInvalid);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApprovalOutcome {
    pub status: GateStatus,
    pub reasons: Vec<ApprovalReasonCode>,
}

impl TrustPolicy {
    pub fn validate(&self, requirements: &ApprovalRequirements) -> Result<(), ApprovalReasonCode> {
        if self.schema_version != 1 {
            return Err(ApprovalReasonCode::ApprovalPolicyInvalid);
        }
        let mut by_role = BTreeMap::new();
        for reviewer in &self.reviewers {
            if !is_full_fingerprint(&reviewer.fingerprint) {
                return Err(ApprovalReasonCode::UntrustedReviewer);
            }
            if by_role
                .insert(&reviewer.role, &reviewer.fingerprint)
                .is_some()
            {
                return Err(ApprovalReasonCode::ApprovalPolicyInvalid);
            }
        }
        let mut fingerprints = BTreeSet::new();
        for role in &requirements.required_roles {
            let fingerprint = by_role
                .get(role)
                .ok_or(ApprovalReasonCode::RequiredApprovalMissing)?;
            if !fingerprints.insert(*fingerprint) {
                return Err(ApprovalReasonCode::ReviewerRoleSeparationFailed);
            }
        }
        Ok(())
    }
}

pub fn verify_approvals(
    binding: &ApprovalBinding,
    policy: &TrustPolicy,
    evidence: &[ApprovalEvidence],
    gpgv_program: PathBuf,
) -> ApprovalOutcome {
    let requirements = ApprovalRequirements::stage_zero();
    if let Err(reason) = policy.validate(&requirements) {
        return blocked(reason);
    }
    let policy_by_role = policy
        .reviewers
        .iter()
        .map(|reviewer| (reviewer.role.as_str(), reviewer.fingerprint.as_str()))
        .collect::<BTreeMap<_, _>>();

    for role in &requirements.required_roles {
        let Some(item) = evidence.iter().find(|item| &item.role == role) else {
            return blocked(ApprovalReasonCode::RequiredApprovalMissing);
        };
        let Some(expected_fingerprint) = policy_by_role.get(role.as_str()) else {
            return blocked(ApprovalReasonCode::RequiredApprovalMissing);
        };
        if item.claimed_fingerprint != *expected_fingerprint {
            return blocked(ApprovalReasonCode::UntrustedReviewer);
        }
        let statement_bytes = match fs::read(&item.statement_path) {
            Ok(bytes) => bytes,
            Err(_) => return blocked(ApprovalReasonCode::ApprovalStatementMismatch),
        };
        let canonical = match std::str::from_utf8(&statement_bytes)
            .ok()
            .and_then(|source| canonicalize_json_str(source).ok())
        {
            Some(canonical) if canonical == statement_bytes => canonical,
            _ => return blocked(ApprovalReasonCode::ApprovalStatementMismatch),
        };
        let statement: ApprovalStatement = match serde_json::from_slice(&canonical) {
            Ok(statement) => statement,
            Err(_) => return blocked(ApprovalReasonCode::ApprovalStatementInvalid),
        };
        if let Err(reason) = statement.validate() {
            return blocked(reason);
        }
        if statement.stage_id != binding.stage_id
            || statement.implementation_commit != binding.implementation_commit
            || statement.design_tag_object != binding.design_tag_object
            || statement.design_commit != binding.design_commit
            || statement.aggregate_evidence_sha256 != binding.aggregate_evidence_sha256
            || statement.comparison_manifest_sha256 != binding.comparison_manifest_sha256
            || statement.known_limitations != binding.known_limitations
            || statement.known_limitations_sha256 != binding.known_limitations_sha256
            || statement.role != item.role
            || statement.signer_fingerprint != *expected_fingerprint
        {
            return blocked(ApprovalReasonCode::ApprovalBindingMismatch);
        }
        if !item.signature_path.is_file() {
            return blocked(ApprovalReasonCode::InvalidDetachedSignature);
        }

        let output = Command::new(&gpgv_program)
            .args(["--status-fd", "1", "--keyring"])
            .arg(&policy.keyring_path)
            .arg(&item.signature_path)
            .arg(&item.statement_path)
            .env_clear()
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output();
        let output = match output {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return blocked(ApprovalReasonCode::OpenPgpToolingUnavailable);
            }
            Err(_) => return blocked(ApprovalReasonCode::InvalidDetachedSignature),
        };
        if !output.status.success()
            || !validsig_fingerprints(&output.stdout)
                .iter()
                .any(|fingerprint| fingerprint == expected_fingerprint)
        {
            return blocked(ApprovalReasonCode::InvalidDetachedSignature);
        }
    }
    ApprovalOutcome {
        status: GateStatus::Pass,
        reasons: Vec::new(),
    }
}

fn blocked(reason: ApprovalReasonCode) -> ApprovalOutcome {
    ApprovalOutcome {
        status: GateStatus::Blocked,
        reasons: vec![reason],
    }
}

fn is_full_fingerprint(value: &str) -> bool {
    is_lower_hex(value, 40)
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validsig_fingerprints(status: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(status)
        .lines()
        .filter_map(|line| line.strip_prefix("[GNUPG:] VALIDSIG "))
        .filter_map(|fields| fields.split_ascii_whitespace().next())
        .filter(|fingerprint| {
            fingerprint.len() == 40 && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .map(str::to_ascii_lowercase)
        .collect()
}
