use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::{
    artifacts::ArtifactManifest,
    canonical::{CanonicalError, canonicalize},
    config::BuilderConfig,
    identity::{parse_rustc_host, version_output_matches},
};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GateResult {
    Pass,
    Fail,
    Blocked,
    NotRun,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuilderEnvironment {
    pub os_version: String,
    pub target_triple: String,
    pub toolchains: BTreeMap<String, ExecutableEvidence>,
    pub toolchain_fingerprint: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableEvidence {
    pub id: String,
    pub sha256: String,
    pub version_output: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuilderIdentity {
    pub builder_id: String,
    pub signer_role: String,
    pub signer_fingerprint: String,
    pub resolved_paths: BTreeMap<String, PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BuilderReport {
    pub schema_version: u32,
    pub stage_id: String,
    pub implementation_commit: String,
    pub design_tag_object: String,
    pub design_commit: String,
    pub config_sha256: String,
    pub schema_sha256: String,
    pub builder_identity: BuilderIdentity,
    pub environment: BuilderEnvironment,
    pub resolved_programs: BTreeMap<String, ExecutableEvidence>,
    pub check_evidence_hashes: BTreeMap<String, String>,
    pub artifacts: ArtifactManifest,
    pub check_results: BTreeMap<String, GateResult>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ComparisonProjection {
    pub schema_version: u32,
    pub stage_id: String,
    pub implementation_commit: String,
    pub design_tag_object: String,
    pub design_commit: String,
    pub config_sha256: String,
    pub schema_sha256: String,
    pub environment: BuilderEnvironment,
    pub resolved_programs: BTreeMap<String, ExecutableEvidence>,
    pub check_evidence_hashes: BTreeMap<String, String>,
    pub artifacts: ArtifactManifest,
    pub check_results: BTreeMap<String, GateResult>,
}

impl BuilderReport {
    pub fn comparison_projection(&self) -> Result<Vec<u8>, CanonicalError> {
        canonicalize(&ComparisonProjection {
            schema_version: self.schema_version,
            stage_id: self.stage_id.clone(),
            implementation_commit: self.implementation_commit.clone(),
            design_tag_object: self.design_tag_object.clone(),
            design_commit: self.design_commit.clone(),
            config_sha256: self.config_sha256.clone(),
            schema_sha256: self.schema_sha256.clone(),
            environment: self.environment.clone(),
            resolved_programs: self.resolved_programs.clone(),
            check_evidence_hashes: self.check_evidence_hashes.clone(),
            artifacts: self.artifacts.clone(),
            check_results: self.check_results.clone(),
        })
    }

    pub fn full_hash(&self) -> Result<String, CanonicalError> {
        canonicalize(self).map(|bytes| sha256(&bytes))
    }

    pub fn projection_hash(&self) -> Result<String, CanonicalError> {
        self.comparison_projection().map(|bytes| sha256(&bytes))
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ComparisonResult {
    NotRun,
    Identical,
    Compatible,
    Different,
}

#[must_use]
pub const fn comparison_satisfies_reproducibility(comparison: ComparisonResult) -> bool {
    matches!(comparison, ComparisonResult::Identical)
}

#[must_use]
pub fn builder_ids_are_independent(first: &str, second: &str) -> bool {
    first != second && valid_builder_id(first) && valid_builder_id(second)
}

#[must_use]
pub fn valid_builder_id(builder_id: &str) -> bool {
    if !(3..=128).contains(&builder_id.len())
        || !builder_id.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'@')
        })
    {
        return false;
    }
    let normalized = builder_id.to_ascii_lowercase();
    !["unknown", "placeholder", "unidentified"]
        .iter()
        .any(|marker| normalized.contains(marker))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BuilderEvidenceValidation {
    Valid,
    IdentityInvalid,
    VersionMismatch,
}

#[must_use]
pub fn validate_builder_evidence(
    config: &BuilderConfig,
    report: &BuilderReport,
) -> BuilderEvidenceValidation {
    if !valid_builder_id(&report.builder_identity.builder_id)
        || report.environment.target_triple == "unavailable"
        || report.environment.os_version == "unavailable"
        || canonicalize(&report.environment.toolchains)
            .map(|bytes| sha256(&bytes))
            .ok()
            .as_deref()
            != Some(report.environment.toolchain_fingerprint.as_str())
    {
        return BuilderEvidenceValidation::IdentityInvalid;
    }
    for tool in &config.tools {
        let Some(identity) = report.environment.toolchains.get(&tool.id) else {
            return BuilderEvidenceValidation::IdentityInvalid;
        };
        if identity.id != tool.id {
            return BuilderEvidenceValidation::IdentityInvalid;
        }
        if !version_output_matches(
            &identity.version_output,
            tool.expected_output_contains.as_deref(),
        ) {
            return BuilderEvidenceValidation::VersionMismatch;
        }
    }
    let Some(target) = report.environment.toolchains.get(&config.target_tool) else {
        return BuilderEvidenceValidation::IdentityInvalid;
    };
    if parse_rustc_host(&target.version_output).ok().as_deref()
        != Some(report.environment.target_triple.as_str())
    {
        return BuilderEvidenceValidation::IdentityInvalid;
    }
    BuilderEvidenceValidation::Valid
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AggregateManifest {
    pub schema_version: u32,
    pub builder_report_hashes: Vec<String>,
    pub projection_hashes: Vec<String>,
    pub target_manifests: Vec<TargetManifest>,
    pub comparison: ComparisonResult,
    pub remote_proof_sha256: Option<String>,
    pub remote_result: GateResult,
    pub trust_registry_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TargetManifest {
    pub builder_report_sha256: String,
    pub artifacts: ArtifactManifest,
}

#[derive(Serialize)]
struct ComparisonManifest<'a> {
    schema_version: u32,
    projection_hashes: &'a [String],
    target_manifests: &'a [TargetManifest],
    comparison: ComparisonResult,
}

impl AggregateManifest {
    #[must_use]
    pub fn bind_external_inputs(
        self,
        remote_proof_sha256: Option<String>,
        trust_registry_sha256: String,
    ) -> Self {
        let remote_result = if remote_proof_sha256.is_some() {
            GateResult::Pass
        } else {
            GateResult::Blocked
        };
        self.bind_external_inputs_with_status(
            remote_proof_sha256,
            remote_result,
            trust_registry_sha256,
        )
    }

    #[must_use]
    pub fn bind_external_inputs_with_status(
        mut self,
        remote_proof_sha256: Option<String>,
        remote_result: GateResult,
        trust_registry_sha256: String,
    ) -> Self {
        self.remote_proof_sha256 = remote_proof_sha256;
        self.remote_result = remote_result;
        self.trust_registry_sha256 = trust_registry_sha256;
        self
    }

    pub fn full_hash(&self) -> Result<String, CanonicalError> {
        canonicalize(self).map(|bytes| sha256(&bytes))
    }

    pub fn comparison_manifest_sha256(&self) -> Result<String, CanonicalError> {
        canonicalize(&ComparisonManifest {
            schema_version: self.schema_version,
            projection_hashes: &self.projection_hashes,
            target_manifests: &self.target_manifests,
            comparison: self.comparison,
        })
        .map(|bytes| sha256(&bytes))
    }
}

pub fn single_builder_aggregate(
    report: &BuilderReport,
) -> Result<AggregateManifest, CanonicalError> {
    let full_hash = report.full_hash()?;
    Ok(AggregateManifest {
        schema_version: 1,
        builder_report_hashes: vec![full_hash.clone()],
        projection_hashes: vec![report.projection_hash()?],
        target_manifests: vec![TargetManifest {
            builder_report_sha256: full_hash,
            artifacts: report.artifacts.clone(),
        }],
        comparison: ComparisonResult::NotRun,
        remote_proof_sha256: None,
        remote_result: GateResult::NotRun,
        trust_registry_sha256: String::new(),
    })
}

pub fn aggregate_reports(
    first: &BuilderReport,
    second: &BuilderReport,
) -> Result<AggregateManifest, CanonicalError> {
    let first_projection = first.projection_hash()?;
    let second_projection = second.projection_hash()?;
    let comparison = if first_projection == second_projection {
        ComparisonResult::Identical
    } else if reports_are_cross_target_compatible(first, second) {
        ComparisonResult::Compatible
    } else {
        ComparisonResult::Different
    };
    let first_full = first.full_hash()?;
    let second_full = second.full_hash()?;
    Ok(AggregateManifest {
        schema_version: 1,
        builder_report_hashes: vec![first_full.clone(), second_full.clone()],
        projection_hashes: vec![first_projection.clone(), second_projection.clone()],
        target_manifests: vec![
            TargetManifest {
                builder_report_sha256: first_full,
                artifacts: first.artifacts.clone(),
            },
            TargetManifest {
                builder_report_sha256: second_full,
                artifacts: second.artifacts.clone(),
            },
        ],
        comparison,
        remote_proof_sha256: None,
        remote_result: GateResult::NotRun,
        trust_registry_sha256: String::new(),
    })
}

fn reports_are_cross_target_compatible(first: &BuilderReport, second: &BuilderReport) -> bool {
    if first.schema_version != second.schema_version
        || first.stage_id != second.stage_id
        || first.implementation_commit != second.implementation_commit
        || first.design_tag_object != second.design_tag_object
        || first.design_commit != second.design_commit
        || first.config_sha256 != second.config_sha256
        || first.schema_sha256 != second.schema_sha256
        || first.check_results != second.check_results
        || first.artifacts.schema_version != second.artifacts.schema_version
        || first.artifacts.artifacts.len() != second.artifacts.artifacts.len()
    {
        return false;
    }
    let mut first_artifacts = first.artifacts.artifacts.iter().collect::<Vec<_>>();
    let mut second_artifacts = second.artifacts.artifacts.iter().collect::<Vec<_>>();
    first_artifacts.sort_by_key(|artifact| &artifact.logical_name);
    second_artifacts.sort_by_key(|artifact| &artifact.logical_name);
    first_artifacts
        .into_iter()
        .zip(second_artifacts)
        .all(|(left, right)| {
            left.logical_name == right.logical_name
                && left.relative_path == right.relative_path
                && left.kind == right.kind
                && left.producer == right.producer
                && left.profile == right.profile
                && if left.target_triple == "platform-independent"
                    || right.target_triple == "platform-independent"
                {
                    left.target_triple == right.target_triple
                        && left.size_bytes == right.size_bytes
                        && left.sha256 == right.sha256
                } else {
                    left.target_triple != right.target_triple
                        || (left.size_bytes == right.size_bytes && left.sha256 == right.sha256)
                }
        })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommittedInputHashes {
    pub config_sha256: String,
    pub schema_sha256: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputHashErrorCode {
    UnsafePath,
    NotCommitted,
    Modified,
    GitUnavailable,
    ReadFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum InputHashError {
    #[error("input path is not a safe repository-relative path: {0}")]
    UnsafePath(PathBuf),
    #[error("gate input is not committed: {0}")]
    NotCommitted(PathBuf),
    #[error("gate input differs from its committed bytes: {0}")]
    Modified(PathBuf),
    #[error("git could not read a committed gate input: {0}")]
    GitUnavailable(String),
    #[error("gate input could not be read: {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl InputHashError {
    #[must_use]
    pub const fn code(&self) -> InputHashErrorCode {
        match self {
            Self::UnsafePath(_) => InputHashErrorCode::UnsafePath,
            Self::NotCommitted(_) => InputHashErrorCode::NotCommitted,
            Self::Modified(_) => InputHashErrorCode::Modified,
            Self::GitUnavailable(_) => InputHashErrorCode::GitUnavailable,
            Self::ReadFailed { .. } => InputHashErrorCode::ReadFailed,
        }
    }
}

pub fn hash_committed_inputs(
    repository: &Path,
    config_path: &Path,
    schema_path: &Path,
) -> Result<CommittedInputHashes, InputHashError> {
    Ok(CommittedInputHashes {
        config_sha256: hash_committed_file(repository, config_path)?,
        schema_sha256: hash_committed_file(repository, schema_path)?,
    })
}

fn hash_committed_file(repository: &Path, path: &Path) -> Result<String, InputHashError> {
    let path_text = safe_path(path)?;
    let committed = git_output(repository, ["show", &format!("HEAD:{path_text}")])
        .map_err(|_| InputHashError::NotCommitted(path.to_path_buf()))?;
    let working = fs::read(repository.join(path)).map_err(|source| InputHashError::ReadFailed {
        path: path.to_path_buf(),
        source,
    })?;
    if working != committed {
        return Err(InputHashError::Modified(path.to_path_buf()));
    }
    Ok(sha256(&working))
}

pub fn hash_committed_file_sha256(
    repository: &Path,
    path: &Path,
) -> Result<String, InputHashError> {
    hash_committed_file(repository, path)
}

pub fn read_committed_file_bytes(
    repository: &Path,
    path: &Path,
) -> Result<Vec<u8>, InputHashError> {
    let path_text = safe_path(path)?;
    git_output(repository, ["show", &format!("HEAD:{path_text}")])
        .map_err(|_| InputHashError::NotCommitted(path.to_path_buf()))
}

fn safe_path(path: &Path) -> Result<&str, InputHashError> {
    if path.is_absolute()
        || path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        return Err(InputHashError::UnsafePath(path.to_path_buf()));
    }
    path.to_str()
        .ok_or_else(|| InputHashError::UnsafePath(path.to_path_buf()))
}

fn git_output<const N: usize>(
    repository: &Path,
    args: [&str; N],
) -> Result<Vec<u8>, InputHashError> {
    let output = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| InputHashError::GitUnavailable(error.to_string()))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(InputHashError::GitUnavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}
