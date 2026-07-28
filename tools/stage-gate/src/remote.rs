use std::{collections::BTreeSet, fs::File, io::Read as _, path::Path};

use rustix::fs::{FileType, Mode, OFlags, fstat, open};
use serde::{Deserialize, Serialize};

use crate::{approvals::GateStatus, canonical::canonicalize_json_str};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteRequirement {
    pub implementation_commit: String,
    pub repository: String,
    pub repository_id: u64,
    pub repository_owner_id: u64,
    pub workflow: String,
    pub workflow_ref: String,
    pub workflow_sha: String,
    pub event_name: String,
    pub git_ref: String,
    pub signing_check_name: String,
    pub required_checks: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RemoteProofReasonCode {
    RemoteProofMissing,
    RemoteProofMalformed,
    ImplementationCommitMismatch,
    AppSourceMismatch,
    RequiredCheckMissing,
    RequiredCheckNotSuccessful,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteProofOutcome {
    pub status: GateStatus,
    pub reasons: Vec<RemoteProofReasonCode>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteProof {
    pub schema_version: u32,
    pub repository: String,
    pub repository_id: u64,
    pub repository_owner_id: u64,
    pub workflow: String,
    pub workflow_ref: String,
    pub workflow_sha: String,
    pub event_name: String,
    pub git_ref: String,
    pub head_sha: String,
    pub run_id: u64,
    pub run_attempt: u64,
    pub signing_check_run_id: u64,
    pub signing_check: SigningCheck,
    pub checks: Vec<RemoteCheck>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SigningCheck {
    pub check_run_id: u64,
    pub name: String,
    pub status: String,
    pub head_sha: String,
    pub run_id: u64,
    pub run_attempt: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteCheck {
    pub check_run_id: u64,
    pub name: String,
    pub conclusion: String,
    pub head_sha: String,
    pub run_id: u64,
    pub run_attempt: u64,
}

pub fn verify_remote_proof(
    proof_path: &Path,
    requirement: &RemoteRequirement,
) -> RemoteProofOutcome {
    let bytes = match read_regular_nofollow(proof_path, 4 * 1024 * 1024) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return blocked(vec![RemoteProofReasonCode::RemoteProofMissing]);
        }
        Err(_) => return blocked(vec![RemoteProofReasonCode::RemoteProofMalformed]),
    };
    match parse_and_validate(&bytes, requirement) {
        Ok(_) => RemoteProofOutcome {
            status: GateStatus::Pass,
            reasons: Vec::new(),
        },
        Err(reasons) => blocked(reasons),
    }
}

pub(crate) fn parse_and_validate(
    bytes: &[u8],
    requirement: &RemoteRequirement,
) -> Result<RemoteProof, Vec<RemoteProofReasonCode>> {
    let source = std::str::from_utf8(bytes)
        .map_err(|_| vec![RemoteProofReasonCode::RemoteProofMalformed])?;
    let canonical = canonicalize_json_str(source)
        .map_err(|_| vec![RemoteProofReasonCode::RemoteProofMalformed])?;
    if canonical != bytes {
        return Err(vec![RemoteProofReasonCode::RemoteProofMalformed]);
    }
    let proof: RemoteProof = serde_json::from_slice(bytes)
        .map_err(|_| vec![RemoteProofReasonCode::RemoteProofMalformed])?;
    if proof.schema_version != 1
        || proof.run_id == 0
        || proof.run_attempt == 0
        || proof.signing_check_run_id == 0
    {
        return Err(vec![RemoteProofReasonCode::RemoteProofMalformed]);
    }

    let mut reasons = Vec::new();
    if proof.head_sha != requirement.implementation_commit {
        reasons.push(RemoteProofReasonCode::ImplementationCommitMismatch);
    }
    if proof.repository != requirement.repository
        || proof.repository_id != requirement.repository_id
        || proof.repository_owner_id != requirement.repository_owner_id
        || proof.workflow != requirement.workflow
        || proof.workflow_ref != requirement.workflow_ref
        || proof.workflow_sha != requirement.workflow_sha
        || proof.event_name != requirement.event_name
        || proof.git_ref != requirement.git_ref
    {
        reasons.push(RemoteProofReasonCode::AppSourceMismatch);
    }

    let expected_names = requirement
        .required_checks
        .iter()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let actual_names = proof
        .checks
        .iter()
        .map(|check| check.name.as_str())
        .collect::<BTreeSet<_>>();
    let ids = proof
        .checks
        .iter()
        .map(|check| check.check_run_id)
        .collect::<BTreeSet<_>>();
    if actual_names != expected_names
        || actual_names.len() != proof.checks.len()
        || ids.len() != proof.checks.len()
    {
        reasons.push(RemoteProofReasonCode::RequiredCheckMissing);
    }
    if proof.checks.iter().any(|check| {
        check.conclusion != "success"
            || check.head_sha != proof.head_sha
            || check.run_id != proof.run_id
            || check.run_attempt != proof.run_attempt
            || check.check_run_id == proof.signing_check_run_id
    }) {
        reasons.push(RemoteProofReasonCode::RequiredCheckNotSuccessful);
    }
    if proof.signing_check.check_run_id != proof.signing_check_run_id
        || proof.signing_check.name != requirement.signing_check_name
        || proof.signing_check.status != "in_progress"
        || proof.signing_check.head_sha != proof.head_sha
        || proof.signing_check.run_id == 0
        || proof.signing_check.run_id == proof.run_id
        || proof.signing_check.run_attempt == 0
    {
        reasons.push(RemoteProofReasonCode::RequiredCheckNotSuccessful);
    }
    reasons.sort();
    reasons.dedup();
    if reasons.is_empty() {
        Ok(proof)
    } else {
        Err(reasons)
    }
}

fn read_regular_nofollow(path: &Path, max_bytes: usize) -> std::io::Result<Vec<u8>> {
    let fd = open(
        path,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(std::io::Error::from)?;
    let stat = fstat(&fd).map_err(std::io::Error::from)?;
    if !FileType::from_raw_mode(stat.st_mode).is_file() {
        return Err(std::io::Error::other("evidence is not a regular file"));
    }
    let mut file = File::from(fd);
    let mut bytes = Vec::new();
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)?;
    if bytes.len() > max_bytes {
        return Err(std::io::Error::other("evidence exceeds configured limit"));
    }
    Ok(bytes)
}

fn blocked(reasons: Vec<RemoteProofReasonCode>) -> RemoteProofOutcome {
    RemoteProofOutcome {
        status: GateStatus::Blocked,
        reasons,
    }
}
