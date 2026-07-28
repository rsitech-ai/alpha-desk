use std::{
    collections::BTreeMap,
    env, fs,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::{
    approvals::{
        ApprovalBinding, ApprovalEvidence, ApprovalReasonCode, ApprovalRequirements, GateStatus,
        TrustPolicy, verify_approvals,
    },
    artifacts::{ArtifactRequest, collect_artifacts},
    canonical::canonicalize,
    config::GateConfig,
    identity::{
        capture_executable_identity_with_env, executable_file_identity, expand_program_roots,
        parse_rustc_host, resolve_program, version_output_matches,
    },
    process::{CommandSpec, OutputPolicy},
    remote::{RemoteRequirement, verify_remote_proof},
    reports::{
        AggregateManifest, BuilderEnvironment, BuilderEvidenceValidation, BuilderReport,
        ComparisonResult, GateResult, aggregate_reports, builder_ids_are_independent,
        comparison_satisfies_reproducibility, hash_committed_file_sha256, hash_committed_inputs,
        read_committed_file_bytes, single_builder_aggregate, valid_builder_id,
        validate_builder_evidence,
    },
    runner::{DesignExpectation, RepositorySnapshot, run_guarded_checks},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateReasonCode {
    BuilderIdentityUnavailable,
    BuilderVersionMismatch,
    SecondBuilderUnavailable,
    SecondBuilderIdentityInvalid,
    SecondBuilderVersionMismatch,
    SecondBuilderNotIdentical,
    SecondBuilderMismatch,
    TrustRegistryUnconfigured,
    PlatformDataApprovalMissing,
    IndependentReviewMissing,
    OpenpgpToolingUnavailable,
    ApprovalVerificationUnavailable,
    RequiredGithubChecksUnavailable,
}

#[must_use]
pub const fn comparison_gate_reason(comparison: ComparisonResult) -> Option<GateReasonCode> {
    match comparison {
        ComparisonResult::Identical => None,
        ComparisonResult::Compatible => Some(GateReasonCode::SecondBuilderNotIdentical),
        ComparisonResult::Different => Some(GateReasonCode::SecondBuilderMismatch),
        ComparisonResult::NotRun => Some(GateReasonCode::SecondBuilderUnavailable),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StageOutcome {
    Accepted,
    Hold,
}

#[derive(Clone, Debug, Serialize)]
pub struct GateRunReport {
    pub schema_version: u32,
    pub stage_id: String,
    pub stage_outcome: StageOutcome,
    pub overall_result: GateStatus,
    pub reason_codes: Vec<GateReasonCode>,
    pub builder_report: BuilderReport,
    pub builder_report_sha256: String,
    pub comparison_projection_sha256: String,
    pub aggregate_manifest: AggregateManifest,
    pub aggregate_evidence_sha256: String,
    pub comparison_manifest_sha256: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GateRunError {
    #[error("gate configuration could not be read: {0}")]
    ConfigRead(String),
    #[error("gate configuration is invalid: {0}")]
    ConfigInvalid(String),
    #[error("requested output must remain under the configured ignored output root")]
    UnsafeOutput,
    #[error("configured program could not be resolved: {0}")]
    ProgramUnavailable(String),
    #[error("repository precondition failed: {0}")]
    Repository(String),
    #[error("gate check failed: {0}")]
    Check(String),
    #[error("artifact collection failed: {0}")]
    Artifact(String),
    #[error("committed gate input verification failed: {0}")]
    Input(String),
    #[error("canonical report generation failed: {0}")]
    Report(String),
    #[error("report could not be written: {0}")]
    Output(String),
}

pub fn run_gate(
    repository: &Path,
    config_path: &Path,
    requested_output: &Path,
) -> Result<GateRunReport, GateRunError> {
    let repository = repository
        .canonicalize()
        .map_err(|error| GateRunError::Repository(error.to_string()))?;
    let relative_config = repository_relative(&repository, config_path)?;
    let config_source = fs::read_to_string(repository.join(&relative_config))
        .map_err(|error| GateRunError::ConfigRead(error.to_string()))?;
    let config = GateConfig::parse(&config_source)
        .map_err(|error| GateRunError::ConfigInvalid(error.to_string()))?;
    let output = prepare_output_path(&repository, &config, requested_output)?;
    let builder_output = prepare_output_path(
        &repository,
        &config,
        Path::new(&config.builder_report_output_path),
    )?;
    if output == builder_output {
        return Err(GateRunError::UnsafeOutput);
    }
    let input_hashes = hash_committed_inputs(
        &repository,
        &relative_config,
        Path::new(&config.schema_path),
    )
    .map_err(|error| GateRunError::Input(error.to_string()))?;
    let trust_registry_sha256 =
        hash_committed_file_sha256(&repository, Path::new(&config.approvals.policy_path))
            .map_err(|error| GateRunError::Input(error.to_string()))?;

    let snapshot = RepositorySnapshot::capture(
        &repository,
        &DesignExpectation {
            tag: config.design.tag.clone(),
            tag_object: config.design.object.clone(),
            commit: config.design.commit.clone(),
        },
    )
    .map_err(|error| GateRunError::Repository(error.to_string()))?;
    let roots = expand_program_roots(&config.program_roots);
    let controlled_environment = controlled_environment(&roots);
    let mut resolved_programs = BTreeMap::new();
    let resolved_gpgv = resolve_program(&config.approvals.gpgv_program, &roots, &repository).ok();
    if let Some(program) = &resolved_gpgv {
        resolved_programs.insert(
            "approval:gpgv".to_owned(),
            executable_file_identity("approval:gpgv", program)
                .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?,
        );
    }
    let commands = config
        .checks
        .iter()
        .map(|check| {
            let program = resolve_program(&check.program, &roots, &repository)
                .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?;
            resolved_programs.insert(
                check.id.clone(),
                executable_file_identity(&check.id, &program)
                    .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?,
            );
            let mut command_environment = check.env.clone();
            for name in &check.inherit_env {
                if name == "PATH" {
                    if let Some(path) = controlled_environment.get("PATH") {
                        command_environment.insert(name.clone(), path.clone());
                    }
                } else if let Ok(value) = env::var(name) {
                    command_environment.insert(name.clone(), value);
                }
            }
            Ok(CommandSpec {
                program,
                args: check.args.iter().map(Into::into).collect(),
                cwd: PathBuf::from(&check.cwd),
                env: command_environment.into_iter().collect(),
                timeout: Duration::from_secs(check.timeout_seconds),
                termination_grace: Duration::from_secs(2),
            })
        })
        .collect::<Result<Vec<_>, GateRunError>>()?;
    let redactions = config
        .checks
        .iter()
        .flat_map(|check| check.env.values())
        .filter(|value| !value.is_empty())
        .cloned()
        .collect();
    let outcomes = run_guarded_checks(
        &repository,
        &snapshot,
        &commands,
        &OutputPolicy {
            max_bytes_per_stream: config.max_output_bytes,
            redactions,
        },
        Duration::from_secs(config.whole_gate_timeout_seconds),
    )
    .map_err(|error| GateRunError::Check(error.to_string()))?;

    let mut reasons = Vec::new();
    let (environment, identity_complete, versions_match) =
        capture_builder_environment(&config, &roots, &repository, &controlled_environment)?;
    if !identity_complete {
        reasons.push(GateReasonCode::BuilderIdentityUnavailable);
    }
    if !versions_match {
        reasons.push(GateReasonCode::BuilderVersionMismatch);
    }
    let artifacts = collect_artifacts(
        &repository,
        &config
            .artifacts
            .iter()
            .map(|artifact| ArtifactRequest {
                logical_name: artifact.id.clone(),
                relative_path: PathBuf::from(&artifact.path),
                kind: artifact.kind.clone(),
                producer: artifact.producer.clone(),
                target_triple: if artifact.target_triple == "builder-host" {
                    environment.target_triple.clone()
                } else {
                    artifact.target_triple.clone()
                },
                profile: artifact.profile.clone(),
                expected_sha256: artifact.expected_sha256.clone(),
            })
            .collect::<Vec<_>>(),
    )
    .map_err(|error| GateRunError::Artifact(error.to_string()))?;
    snapshot
        .verify_unchanged(&repository)
        .map_err(|error| GateRunError::Repository(error.to_string()))?;

    let command_log_hashes = config
        .checks
        .iter()
        .zip(&outcomes)
        .map(|(check, outcome)| {
            let mut bytes = outcome.stdout.text.as_bytes().to_vec();
            bytes.extend_from_slice(outcome.stderr.text.as_bytes());
            (check.id.clone(), sha256(&bytes))
        })
        .collect();
    let check_results = config
        .checks
        .iter()
        .map(|check| (check.id.clone(), GateResult::Pass))
        .collect();
    let builder_id =
        env::var("STAGE_GATE_BUILDER_ID").unwrap_or_else(|_| "local-unidentified".to_owned());
    if !valid_builder_id(&builder_id) {
        reasons.push(GateReasonCode::BuilderIdentityUnavailable);
    }
    let builder_report = BuilderReport {
        schema_version: 1,
        stage_id: config.stage_id.clone(),
        implementation_commit: snapshot.head().to_owned(),
        design_tag_object: config.design.object.clone(),
        design_commit: config.design.commit.clone(),
        config_sha256: input_hashes.config_sha256,
        schema_sha256: input_hashes.schema_sha256,
        builder_id,
        environment,
        resolved_programs,
        command_log_hashes,
        artifacts,
        check_results,
    };
    let builder_report_sha256 = builder_report
        .full_hash()
        .map_err(|error| GateRunError::Report(error.to_string()))?;
    let comparison_projection_sha256 = builder_report
        .projection_hash()
        .map_err(|error| GateRunError::Report(error.to_string()))?;

    let mut aggregate =
        aggregate_builder_reports(&repository, &config, &builder_report, &mut reasons)?;
    let policy = load_trust_policy(&repository, &config, &mut reasons);
    let remote_path = repository.join(&config.remote.proof_path);
    let remote = verify_remote_proof(
        &remote_path,
        &RemoteRequirement {
            implementation_commit: builder_report.implementation_commit.clone(),
            app_source: config.remote.app_source.clone(),
            required_checks: config.remote.required_checks.clone(),
        },
    );
    let remote_hash = fs::read(&remote_path).ok().map(|bytes| sha256(&bytes));
    let remote_result = if remote.status == GateStatus::Pass {
        GateResult::Pass
    } else {
        reasons.push(GateReasonCode::RequiredGithubChecksUnavailable);
        GateResult::Blocked
    };
    aggregate = aggregate.bind_external_inputs_with_status(
        remote_hash,
        remote_result,
        trust_registry_sha256,
    );
    let aggregate_evidence_sha256 = aggregate
        .full_hash()
        .map_err(|error| GateRunError::Report(error.to_string()))?;
    let comparison_manifest_sha256 = aggregate
        .comparison_manifest_sha256()
        .map_err(|error| GateRunError::Report(error.to_string()))?;
    let known_limitations_sha256 = sha256(
        &canonicalize(&config.approvals.known_limitations)
            .map_err(|error| GateRunError::Report(error.to_string()))?,
    );

    verify_external_approvals(
        &repository,
        &config,
        policy.as_ref(),
        resolved_gpgv,
        ApprovalBinding {
            stage_id: config.stage_id.clone(),
            implementation_commit: builder_report.implementation_commit.clone(),
            design_tag_object: builder_report.design_tag_object.clone(),
            design_commit: builder_report.design_commit.clone(),
            aggregate_evidence_sha256: aggregate_evidence_sha256.clone(),
            comparison_manifest_sha256: comparison_manifest_sha256.clone(),
            known_limitations: config.approvals.known_limitations.clone(),
            known_limitations_sha256,
        },
        &mut reasons,
    );
    snapshot
        .verify_unchanged(&repository)
        .map_err(|error| GateRunError::Repository(error.to_string()))?;

    reasons.sort_by_key(reason_order);
    reasons.dedup();
    let overall_result = if reasons.is_empty() {
        GateStatus::Pass
    } else if reasons.contains(&GateReasonCode::SecondBuilderMismatch) {
        GateStatus::Fail
    } else {
        GateStatus::Blocked
    };
    let report = GateRunReport {
        schema_version: 1,
        stage_id: config.stage_id,
        stage_outcome: if overall_result == GateStatus::Pass {
            StageOutcome::Accepted
        } else {
            StageOutcome::Hold
        },
        overall_result,
        reason_codes: reasons,
        builder_report,
        builder_report_sha256,
        comparison_projection_sha256,
        aggregate_manifest: aggregate,
        aggregate_evidence_sha256,
        comparison_manifest_sha256,
    };
    let bytes = canonicalize(&report).map_err(|error| GateRunError::Report(error.to_string()))?;
    let builder_bytes = canonicalize(&report.builder_report)
        .map_err(|error| GateRunError::Report(error.to_string()))?;
    write_atomic(&builder_output, &builder_bytes)?;
    write_atomic(&output, &bytes)?;
    Ok(report)
}

fn capture_builder_environment(
    config: &GateConfig,
    roots: &[PathBuf],
    repository: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<(BuilderEnvironment, bool, bool), GateRunError> {
    let mut identities = BTreeMap::new();
    let mut complete = true;
    let mut versions_match = true;
    for tool in &config.builder.tools {
        let identity = resolve_program(&tool.program, roots, repository).and_then(|program| {
            capture_executable_identity_with_env(&tool.id, &program, &tool.args, environment)
        });
        match identity {
            Ok(identity) => {
                if !version_output_matches(
                    &identity.version_output,
                    tool.expected_output_contains.as_deref(),
                ) {
                    versions_match = false;
                }
                identities.insert(tool.id.clone(), identity);
            }
            Err(_) => complete = false,
        }
    }
    let target_triple = identities
        .get(&config.builder.target_tool)
        .and_then(|identity| parse_rustc_host(&identity.version_output).ok())
        .unwrap_or_else(|| {
            complete = false;
            "unavailable".to_owned()
        });
    let os_version = identities
        .get("os")
        .or_else(|| identities.get(&config.builder.target_tool))
        .map(|identity| identity.version_output.clone())
        .filter(|output| !output.is_empty())
        .unwrap_or_else(|| {
            complete = false;
            "unavailable".to_owned()
        });
    let toolchain_fingerprint = sha256(
        &canonicalize(&identities).map_err(|error| GateRunError::Report(error.to_string()))?,
    );
    Ok((
        BuilderEnvironment {
            os_version,
            target_triple,
            toolchains: identities,
            toolchain_fingerprint,
        },
        complete,
        versions_match,
    ))
}

fn aggregate_builder_reports(
    repository: &Path,
    config: &GateConfig,
    builder_report: &BuilderReport,
    reasons: &mut Vec<GateReasonCode>,
) -> Result<AggregateManifest, GateRunError> {
    let path = repository.join(&config.comparison.second_builder_report_path);
    let aggregate = match fs::read(&path) {
        Err(_) => {
            reasons.push(GateReasonCode::SecondBuilderUnavailable);
            single_builder_aggregate(builder_report)
        }
        Ok(bytes) => match serde_json::from_slice::<BuilderReport>(&bytes) {
            Ok(second) => {
                let aggregate = aggregate_reports(builder_report, &second);
                if !builder_ids_are_independent(&builder_report.builder_id, &second.builder_id) {
                    reasons.push(GateReasonCode::SecondBuilderIdentityInvalid);
                }
                match validate_builder_evidence(&config.builder, &second) {
                    BuilderEvidenceValidation::Valid => {}
                    BuilderEvidenceValidation::IdentityInvalid => {
                        reasons.push(GateReasonCode::SecondBuilderIdentityInvalid);
                    }
                    BuilderEvidenceValidation::VersionMismatch => {
                        reasons.push(GateReasonCode::SecondBuilderVersionMismatch);
                    }
                }
                if let Ok(value) = &aggregate
                    && !comparison_satisfies_reproducibility(value.comparison)
                    && let Some(reason) = comparison_gate_reason(value.comparison)
                {
                    reasons.push(reason);
                }
                aggregate
            }
            Err(_) => {
                reasons.push(GateReasonCode::SecondBuilderMismatch);
                single_builder_aggregate(builder_report)
            }
        },
    };
    aggregate.map_err(|error| GateRunError::Report(error.to_string()))
}

fn load_trust_policy(
    repository: &Path,
    config: &GateConfig,
    reasons: &mut Vec<GateReasonCode>,
) -> Option<TrustPolicy> {
    let bytes =
        match read_committed_file_bytes(repository, Path::new(&config.approvals.policy_path)) {
            Ok(bytes) => bytes,
            Err(_) => {
                reasons.push(GateReasonCode::TrustRegistryUnconfigured);
                return None;
            }
        };
    let source = match std::str::from_utf8(&bytes) {
        Ok(source) => source,
        Err(_) => {
            reasons.push(GateReasonCode::TrustRegistryUnconfigured);
            return None;
        }
    };
    let mut policy = match toml::from_str::<TrustPolicy>(source) {
        Ok(policy) if policy.validate(&ApprovalRequirements::stage_zero()).is_ok() => policy,
        _ => {
            reasons.push(GateReasonCode::TrustRegistryUnconfigured);
            return None;
        }
    };
    if policy.keyring_path.is_relative() {
        policy.keyring_path = repository.join(&policy.keyring_path);
    }
    Some(policy)
}

fn verify_external_approvals(
    repository: &Path,
    config: &GateConfig,
    policy: Option<&TrustPolicy>,
    resolved_gpgv: Option<PathBuf>,
    binding: ApprovalBinding,
    reasons: &mut Vec<GateReasonCode>,
) {
    for role in ["platform-data", "independent"] {
        let missing = config
            .approvals
            .evidence
            .iter()
            .find(|evidence| evidence.role == role)
            .is_none_or(|evidence| {
                !repository.join(&evidence.statement_path).is_file()
                    || !repository.join(&evidence.signature_path).is_file()
            });
        if missing {
            reasons.push(match role {
                "platform-data" => GateReasonCode::PlatformDataApprovalMissing,
                _ => GateReasonCode::IndependentReviewMissing,
            });
        }
    }
    if resolved_gpgv.is_none() {
        reasons.push(GateReasonCode::OpenpgpToolingUnavailable);
    }
    let Some(policy) = policy else {
        return;
    };
    let all_inputs_exist = config.approvals.evidence.iter().all(|evidence| {
        repository.join(&evidence.statement_path).is_file()
            && repository.join(&evidence.signature_path).is_file()
    });
    if !all_inputs_exist {
        return;
    }
    let Some(gpgv) = resolved_gpgv else {
        return;
    };
    let evidence = config
        .approvals
        .evidence
        .iter()
        .map(|item| ApprovalEvidence {
            role: item.role.clone(),
            claimed_fingerprint: policy
                .reviewers
                .iter()
                .find(|reviewer| reviewer.role == item.role)
                .map(|reviewer| reviewer.fingerprint.clone())
                .unwrap_or_default(),
            statement_path: repository.join(&item.statement_path),
            signature_path: repository.join(&item.signature_path),
        })
        .collect::<Vec<_>>();
    let approval = verify_approvals(&binding, policy, &evidence, gpgv);
    if approval.status != GateStatus::Pass {
        if approval
            .reasons
            .contains(&ApprovalReasonCode::OpenPgpToolingUnavailable)
        {
            reasons.push(GateReasonCode::OpenpgpToolingUnavailable);
        } else {
            reasons.push(GateReasonCode::ApprovalVerificationUnavailable);
        }
    }
}

fn controlled_environment(roots: &[PathBuf]) -> BTreeMap<String, String> {
    let mut environment = BTreeMap::new();
    if let Ok(path) = env::join_paths(roots) {
        environment.insert("PATH".to_owned(), path.to_string_lossy().into_owned());
    }
    for name in ["HOME", "CARGO_HOME", "RUSTUP_HOME", "TMPDIR"] {
        if let Ok(value) = env::var(name) {
            environment.insert(name.to_owned(), value);
        }
    }
    environment
}

fn prepare_output_path(
    repository: &Path,
    config: &GateConfig,
    requested: &Path,
) -> Result<PathBuf, GateRunError> {
    let output_root = repository.join(&config.output_root);
    let unresolved_output = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        repository.join(requested)
    };
    let output = normalize_nonexistent_path(&unresolved_output)?;
    if !lexically_inside(&output_root, &output) || output == output_root {
        return Err(GateRunError::UnsafeOutput);
    }
    let relative = output
        .strip_prefix(repository)
        .map_err(|_| GateRunError::UnsafeOutput)?;
    let ignored = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(["check-ignore", "-q", "--"])
        .arg(relative)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| GateRunError::UnsafeOutput)?;
    if !ignored.success() {
        return Err(GateRunError::UnsafeOutput);
    }
    fs::create_dir_all(&output_root).map_err(|error| GateRunError::Output(error.to_string()))?;
    let canonical_root = output_root
        .canonicalize()
        .map_err(|_| GateRunError::UnsafeOutput)?;
    if !canonical_root.starts_with(repository) {
        return Err(GateRunError::UnsafeOutput);
    }
    Ok(output)
}

fn normalize_nonexistent_path(path: &Path) -> Result<PathBuf, GateRunError> {
    if path
        .components()
        .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        return Err(GateRunError::UnsafeOutput);
    }
    let mut ancestor = path;
    let mut suffix = Vec::new();
    while !ancestor.exists() {
        let name = ancestor
            .file_name()
            .ok_or(GateRunError::UnsafeOutput)?
            .to_owned();
        suffix.push(name);
        ancestor = ancestor.parent().ok_or(GateRunError::UnsafeOutput)?;
    }
    let mut normalized = ancestor
        .canonicalize()
        .map_err(|_| GateRunError::UnsafeOutput)?;
    for component in suffix.into_iter().rev() {
        normalized.push(component);
    }
    Ok(normalized)
}

fn repository_relative(repository: &Path, path: &Path) -> Result<PathBuf, GateRunError> {
    let relative = if path.is_absolute() {
        path.strip_prefix(repository)
            .map_err(|_| GateRunError::ConfigRead("config is outside repository".to_owned()))?
            .to_path_buf()
    } else {
        path.to_path_buf()
    };
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(GateRunError::ConfigRead(
            "config path is not repository-relative".to_owned(),
        ));
    }
    Ok(relative)
}

fn lexically_inside(root: &Path, path: &Path) -> bool {
    path.starts_with(root)
        && !path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), GateRunError> {
    let parent = path.parent().ok_or(GateRunError::UnsafeOutput)?;
    fs::create_dir_all(parent).map_err(|error| GateRunError::Output(error.to_string()))?;
    let temporary = parent.join(format!(".stage-gate-{}.tmp", std::process::id()));
    fs::write(&temporary, bytes).map_err(|error| GateRunError::Output(error.to_string()))?;
    fs::rename(&temporary, path).map_err(|error| GateRunError::Output(error.to_string()))
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

const fn reason_order(reason: &GateReasonCode) -> u8 {
    match reason {
        GateReasonCode::BuilderIdentityUnavailable => 0,
        GateReasonCode::BuilderVersionMismatch => 1,
        GateReasonCode::SecondBuilderUnavailable => 2,
        GateReasonCode::SecondBuilderIdentityInvalid => 3,
        GateReasonCode::SecondBuilderVersionMismatch => 4,
        GateReasonCode::SecondBuilderNotIdentical => 5,
        GateReasonCode::SecondBuilderMismatch => 6,
        GateReasonCode::TrustRegistryUnconfigured => 7,
        GateReasonCode::PlatformDataApprovalMissing => 8,
        GateReasonCode::IndependentReviewMissing => 9,
        GateReasonCode::OpenpgpToolingUnavailable => 10,
        GateReasonCode::ApprovalVerificationUnavailable => 11,
        GateReasonCode::RequiredGithubChecksUnavailable => 12,
    }
}
