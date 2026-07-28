use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;

use crate::approvals::GateStatus;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteRequirement {
    pub implementation_commit: String,
    pub app_source: String,
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteProof {
    schema_version: u32,
    implementation_commit: String,
    app_source: String,
    checks: Vec<RemoteCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RemoteCheck {
    name: String,
    conclusion: String,
}

pub fn verify_remote_proof(
    proof_path: &Path,
    requirement: &RemoteRequirement,
) -> RemoteProofOutcome {
    let source = match fs::read_to_string(proof_path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return blocked(vec![RemoteProofReasonCode::RemoteProofMissing]);
        }
        Err(_) => return blocked(vec![RemoteProofReasonCode::RemoteProofMalformed]),
    };
    let proof: RemoteProof = match serde_json::from_str::<RemoteProof>(&source) {
        Ok(proof) if proof.schema_version == 1 => proof,
        _ => return blocked(vec![RemoteProofReasonCode::RemoteProofMalformed]),
    };

    let mut reasons = Vec::new();
    if proof.implementation_commit != requirement.implementation_commit {
        reasons.push(RemoteProofReasonCode::ImplementationCommitMismatch);
    }
    if proof.app_source != requirement.app_source {
        reasons.push(RemoteProofReasonCode::AppSourceMismatch);
    }
    let checks = proof
        .checks
        .into_iter()
        .map(|check| (check.name, check.conclusion))
        .collect::<BTreeMap<_, _>>();
    for required in &requirement.required_checks {
        match checks.get(required) {
            None => {
                if !reasons.contains(&RemoteProofReasonCode::RequiredCheckMissing) {
                    reasons.push(RemoteProofReasonCode::RequiredCheckMissing);
                }
            }
            Some(conclusion) if conclusion != "success" => {
                if !reasons.contains(&RemoteProofReasonCode::RequiredCheckNotSuccessful) {
                    reasons.push(RemoteProofReasonCode::RequiredCheckNotSuccessful);
                }
            }
            Some(_) => {}
        }
    }
    if reasons.is_empty() {
        RemoteProofOutcome {
            status: GateStatus::Pass,
            reasons,
        }
    } else {
        blocked(reasons)
    }
}

fn blocked(reasons: Vec<RemoteProofReasonCode>) -> RemoteProofOutcome {
    RemoteProofOutcome {
        status: GateStatus::Blocked,
        reasons,
    }
}
