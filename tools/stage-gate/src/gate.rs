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
        ExecutableIdentity, capture_executable_identity_with_env, executable_file_identity,
        expand_program_roots, parse_rustc_host, resolve_program, snapshot_resolved_program,
        version_output_matches,
    },
    output::OutputRoot,
    process::{CommandSpec, OutputPolicy},
    provenance::{SignedEvidence, verify_signed_builder_report, verify_signed_remote_proof},
    remote::RemoteRequirement,
    reports::{
        AggregateManifest, BuilderEnvironment, BuilderEvidenceValidation, BuilderIdentity,
        BuilderReport, CHECK_EVIDENCE_NORMALIZATION, ComparisonResult, ExecutableEvidence,
        GateResult, aggregate_reports, builder_ids_are_independent, check_evidence_hash,
        comparison_satisfies_reproducibility, hash_committed_file_sha256, hash_committed_inputs,
        read_committed_file_bytes, single_builder_aggregate, valid_builder_id,
        validate_builder_evidence,
    },
    runner::{CheckProgress, DesignExpectation, RepositorySnapshot, run_guarded_checks_observed},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GateReasonCode {
    BuilderIdentityUnavailable,
    BuilderVersionMismatch,
    SecondBuilderUnavailable,
    SecondBuilderEvidenceInvalid,
    SecondBuilderIdentityInvalid,
    SecondBuilderVersionMismatch,
    SecondBuilderNotIdentical,
    SecondBuilderMismatch,
    TrustRegistryUnconfigured,
    PlatformDataApprovalMissing,
    IndependentReviewMissing,
    OpenpgpToolingUnavailable,
    ApprovalVerificationUnavailable,
    ApprovalEvidenceInvalid,
    RequiredGithubChecksUnavailable,
    RequiredGithubChecksInvalid,
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

#[must_use]
pub fn gate_status_for_reasons(reasons: &[GateReasonCode]) -> GateStatus {
    if reasons.is_empty() {
        return GateStatus::Pass;
    }
    if reasons.iter().any(|reason| {
        matches!(
            reason,
            GateReasonCode::SecondBuilderEvidenceInvalid
                | GateReasonCode::SecondBuilderIdentityInvalid
                | GateReasonCode::SecondBuilderVersionMismatch
                | GateReasonCode::SecondBuilderNotIdentical
                | GateReasonCode::SecondBuilderMismatch
                | GateReasonCode::ApprovalEvidenceInvalid
                | GateReasonCode::RequiredGithubChecksInvalid
        )
    }) {
        GateStatus::Fail
    } else {
        GateStatus::Blocked
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuilderProducer {
    builder_id: String,
    signer_role: String,
    signer_fingerprint: String,
}

impl BuilderProducer {
    #[must_use]
    pub fn local(builder_id: String) -> Self {
        Self {
            builder_id,
            signer_role: "local".to_owned(),
            signer_fingerprint: String::new(),
        }
    }

    pub fn builder_b(role: &str, fingerprint: &str) -> Result<Self, GateRunError> {
        if role != "builder-b"
            || fingerprint.len() != 40
            || !fingerprint
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(GateRunError::ConfigInvalid(
                "Builder B requires role builder-b and a lowercase 40-hex fingerprint".to_owned(),
            ));
        }
        Ok(Self {
            builder_id: format!("builder-b:{fingerprint}"),
            signer_role: role.to_owned(),
            signer_fingerprint: fingerprint.to_owned(),
        })
    }
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
    let builder_id = env::var("STAGE_GATE_BUILDER_ID").map_err(|_| {
        GateRunError::ConfigInvalid(
            "STAGE_GATE_BUILDER_ID must explicitly identify the local builder".to_owned(),
        )
    })?;
    if !valid_builder_id(&builder_id) {
        return Err(GateRunError::ConfigInvalid(
            "STAGE_GATE_BUILDER_ID is not a valid builder identity".to_owned(),
        ));
    }
    let producer = BuilderProducer::local(builder_id);
    run_gate_with_producer(repository, config_path, requested_output, producer)
}

pub fn run_gate_with_producer(
    repository: &Path,
    config_path: &Path,
    requested_output: &Path,
    producer: BuilderProducer,
) -> Result<GateRunReport, GateRunError> {
    let requested_output = if requested_output.is_absolute() {
        requested_output
            .strip_prefix(repository)
            .map_err(|_| GateRunError::UnsafeOutput)?
            .to_path_buf()
    } else {
        requested_output.to_path_buf()
    };
    let repository = repository
        .canonicalize()
        .map_err(|error| GateRunError::Repository(error.to_string()))?;
    let bootstrap = bootstrap_output(&repository, &requested_output);
    let relative_config = repository_relative(&repository, config_path)?;
    let config_source = match fs::read_to_string(repository.join(&relative_config)) {
        Ok(source) => source,
        Err(error) => {
            let run_error = GateRunError::ConfigRead(error.to_string());
            publish_bootstrap_failure(bootstrap.as_ref(), &run_error)?;
            return Err(run_error);
        }
    };
    let config = match GateConfig::parse(&config_source) {
        Ok(config) => config,
        Err(error) => {
            let run_error = GateRunError::ConfigInvalid(error.to_string());
            publish_bootstrap_failure(bootstrap.as_ref(), &run_error)?;
            return Err(run_error);
        }
    };
    let output = prepare_output_name(&repository, &config, &requested_output)?;
    let builder_output = prepare_output_name(
        &repository,
        &config,
        Path::new(&config.builder_report_output_path),
    )?;
    if output == builder_output {
        return Err(GateRunError::UnsafeOutput);
    }
    let output_root = if config.output_root == "target/stage-gates" {
        bootstrap
            .map(|(root, _)| root)
            .ok_or(GateRunError::UnsafeOutput)?
    } else {
        OutputRoot::open(&repository, Path::new(&config.output_root))
            .map_err(|error| GateRunError::Output(error.to_string()))?
    };
    output_root
        .remove_if_exists(&output)
        .map_err(|error| GateRunError::Output(error.to_string()))?;
    output_root
        .remove_if_exists(&builder_output)
        .map_err(|error| GateRunError::Output(error.to_string()))?;
    let invocation_id = random_invocation_id()?;
    let mut implementation_commit = repository_head(&repository);
    let mut check_states = config
        .checks
        .iter()
        .map(|check| (check.id.clone(), "NOT_RUN"))
        .collect::<BTreeMap<_, _>>();
    publish_lifecycle_report(
        &output_root,
        &output,
        &config.stage_id,
        &invocation_id,
        &implementation_commit,
        "initialized",
        "gate_in_progress",
        "NOT_RUN",
        &check_states,
    )?;

    let execution = (|| {
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
        implementation_commit = snapshot.head().to_owned();
        let roots = expand_program_roots(&config.program_roots);
        let controlled_environment = controlled_environment(&roots);
        let mut resolved_programs = BTreeMap::new();
        let mut resolved_paths = BTreeMap::new();
        let mut program_snapshots = Vec::new();
        let resolved_gpgv = if let Ok(program) =
            resolve_program(&config.approvals.gpgv_program, &roots, &repository)
        {
            let identity = executable_file_identity("approval:gpgv", &program)
                .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?;
            let verifier_snapshot = snapshot_resolved_program(&program, &identity.sha256)
                .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?;
            let verifier_path = verifier_snapshot.executable_path().to_path_buf();
            resolved_paths.insert("approval:gpgv".to_owned(), identity.resolved_path.clone());
            resolved_programs.insert("approval:gpgv".to_owned(), executable_evidence(identity));
            program_snapshots.push(verifier_snapshot);
            Some(verifier_path)
        } else {
            None
        };
        let commands = config
            .checks
            .iter()
            .map(|check| {
                let program = resolve_program(&check.program, &roots, &repository)
                    .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?;
                let identity = executable_file_identity(&check.id, &program)
                    .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?;
                let expected_sha256 = identity.sha256.clone();
                resolved_paths.insert(check.id.clone(), identity.resolved_path.clone());
                resolved_programs.insert(check.id.clone(), executable_evidence(identity));
                let evidence_program = program.executable_path.clone();
                let (execution_program, arg0) = if program.executable_path.starts_with(&repository)
                {
                    (
                        program.executable_path,
                        (program.invocation_path != evidence_program)
                            .then(|| program.invocation_path.into_os_string()),
                    )
                } else {
                    let executable_snapshot = snapshot_resolved_program(&program, &expected_sha256)
                        .map_err(|error| GateRunError::ProgramUnavailable(error.to_string()))?;
                    let execution_program = executable_snapshot.executable_path().to_path_buf();
                    let arg0 = Some(program.invocation_path.into_os_string());
                    program_snapshots.push(executable_snapshot);
                    (execution_program, arg0)
                };
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
                    program: execution_program,
                    evidence_program: Some(evidence_program),
                    arg0,
                    args: check.args.iter().map(Into::into).collect(),
                    cwd: PathBuf::from(&check.cwd),
                    env: command_environment.into_iter().collect(),
                    timeout: Duration::from_secs(check.timeout_seconds),
                    termination_grace: Duration::from_secs(check.termination_grace_seconds),
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
        let outcomes = run_guarded_checks_observed(
            &repository,
            &snapshot,
            &commands,
            &OutputPolicy {
                max_bytes_per_stream: config.max_output_bytes,
                redactions,
            },
            Duration::from_secs(config.whole_gate_timeout_seconds),
            |index, progress| {
                if let Some(check) = config.checks.get(index)
                    && let Some(state) = check_states.get_mut(&check.id)
                {
                    *state = match progress {
                        CheckProgress::Pass => "PASS",
                        CheckProgress::Fail => "FAIL",
                    };
                }
            },
        )
        .map_err(|error| GateRunError::Check(error.to_string()))?;
        if outcomes
            .iter()
            .any(|outcome| outcome.stdout.truncated || outcome.stderr.truncated)
        {
            return Err(GateRunError::Check(
                "check output exceeded the bounded evidence limit".to_owned(),
            ));
        }

        let mut reasons = Vec::new();
        let (environment, tool_paths, identity_complete, versions_match) =
            capture_builder_environment(&config, &roots, &repository, &controlled_environment)?;
        resolved_paths.extend(tool_paths);
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

        let check_evidence_hashes = config
            .checks
            .iter()
            .zip(&commands)
            .zip(&outcomes)
            .map(|((check, command), outcome)| {
                let executable_sha256 = resolved_programs
                    .get(&check.id)
                    .map(|evidence| evidence.sha256.as_str())
                    .unwrap_or_default();
                let mut executed = command.clone();
                executed.cwd = repository
                    .join(&command.cwd)
                    .canonicalize()
                    .unwrap_or_else(|_| repository.join(&command.cwd));
                (
                    check.id.clone(),
                    check_evidence_hash(
                        &check.id,
                        &executed,
                        executable_sha256,
                        &repository,
                        outcome,
                    )
                    .expect("stable check evidence is canonicalizable"),
                )
            })
            .collect();
        let check_results = config
            .checks
            .iter()
            .map(|check| (check.id.clone(), GateResult::Pass))
            .collect();
        if !valid_builder_id(&producer.builder_id) {
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
            check_evidence_normalization: CHECK_EVIDENCE_NORMALIZATION.to_owned(),
            builder_identity: BuilderIdentity {
                builder_id: producer.builder_id.clone(),
                signer_role: producer.signer_role.clone(),
                signer_fingerprint: producer.signer_fingerprint.clone(),
                resolved_paths,
            },
            environment,
            resolved_programs,
            check_evidence_hashes,
            artifacts,
            check_results,
        };
        let builder_report_sha256 = builder_report
            .full_hash()
            .map_err(|error| GateRunError::Report(error.to_string()))?;
        let comparison_projection_sha256 = builder_report
            .projection_hash()
            .map_err(|error| GateRunError::Report(error.to_string()))?;

        let policy = load_trust_policy(&repository, &config, &mut reasons);
        let mut aggregate = aggregate_builder_reports(
            &repository,
            &config,
            &builder_report,
            policy.as_ref(),
            resolved_gpgv.clone(),
            &mut reasons,
        )?;
        let remote_requirement = RemoteRequirement {
            implementation_commit: builder_report.implementation_commit.clone(),
            repository: config.remote.repository.clone(),
            repository_id: config.remote.repository_id,
            repository_owner_id: config.remote.repository_owner_id,
            workflow: config.remote.workflow.clone(),
            workflow_ref: config.remote.workflow_ref.clone(),
            workflow_sha: builder_report.implementation_commit.clone(),
            trigger_workflow_id: config.remote.trigger_workflow_id,
            trigger_workflow_name: config.remote.trigger_workflow_name.clone(),
            trigger_workflow_path: config.remote.trigger_workflow_path.clone(),
            trigger_workflow_sha: builder_report.implementation_commit.clone(),
            event_name: config.remote.event_name.clone(),
            git_ref: config.remote.git_ref.clone(),
            signing_check_name: config.remote.signing_check_name.clone(),
            required_checks: config.remote.required_checks.clone(),
        };
        let remote_payload = repository.join(&config.remote.proof_path);
        let remote_signature = repository.join(&config.remote.signature_path);
        let remote = if !remote_payload.is_file() || !remote_signature.is_file() {
            None
        } else if let Some((policy, verifier)) = policy.as_ref().zip(resolved_gpgv.clone()) {
            match verify_signed_remote_proof(
                &SignedEvidence {
                    role: config.remote.signer_role.clone(),
                    payload_path: remote_payload,
                    signature_path: remote_signature,
                },
                &remote_requirement,
                policy,
                verifier,
                config.max_output_bytes,
            ) {
                Ok(verified) => Some(Ok(verified)),
                Err(_) => Some(Err(())),
            }
        } else {
            None
        };
        let (remote_hash, remote_result) = match remote {
            Some(Ok(verified)) => (Some(verified.sha256), GateResult::Pass),
            Some(Err(())) => {
                reasons.push(GateReasonCode::RequiredGithubChecksInvalid);
                (None, GateResult::Fail)
            }
            None => {
                reasons.push(GateReasonCode::RequiredGithubChecksUnavailable);
                (None, GateResult::Blocked)
            }
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
        let overall_result = gate_status_for_reasons(&reasons);
        let report = GateRunReport {
            schema_version: 1,
            stage_id: config.stage_id.clone(),
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
        let bytes =
            canonicalize(&report).map_err(|error| GateRunError::Report(error.to_string()))?;
        let builder_bytes = canonicalize(&report.builder_report)
            .map_err(|error| GateRunError::Report(error.to_string()))?;
        output_root
            .write_atomic(&builder_output, &builder_bytes)
            .map_err(|error| GateRunError::Output(error.to_string()))?;
        output_root
            .write_atomic(&output, &bytes)
            .map_err(|error| GateRunError::Output(error.to_string()))?;
        Ok(report)
    })();
    match execution {
        Ok(report) => Ok(report),
        Err(error) => {
            let _ = output_root.remove_if_exists(&builder_output);
            let publication = publish_lifecycle_report(
                &output_root,
                &output,
                &config.stage_id,
                &invocation_id,
                &implementation_commit,
                failure_phase(&error),
                gate_error_code(&error),
                "FAIL",
                &check_states,
            );
            match publication {
                Ok(()) => Err(error),
                Err(publication_error) => Err(publication_error),
            }
        }
    }
}

fn gate_error_code(error: &GateRunError) -> &'static str {
    match error {
        GateRunError::ConfigRead(_) | GateRunError::ConfigInvalid(_) => "config_failed",
        GateRunError::UnsafeOutput | GateRunError::Output(_) => "output_failed",
        GateRunError::ProgramUnavailable(_) => "program_unavailable",
        GateRunError::Repository(_) => "repository_failed",
        GateRunError::Check(_) => "check_failed",
        GateRunError::Artifact(_) => "artifact_failed",
        GateRunError::Input(_) => "input_failed",
        GateRunError::Report(_) => "report_failed",
    }
}

fn bootstrap_output(repository: &Path, requested: &Path) -> Option<(OutputRoot, PathBuf)> {
    let relative = if requested.is_absolute() {
        requested.strip_prefix(repository).ok()?.to_path_buf()
    } else {
        requested.to_path_buf()
    };
    if relative != Path::new("target/stage-gates/stage-0.json") {
        return None;
    }
    let root = OutputRoot::open(repository, Path::new("target/stage-gates")).ok()?;
    Some((root, PathBuf::from("stage-0.json")))
}

fn publish_bootstrap_failure(
    bootstrap: Option<&(OutputRoot, PathBuf)>,
    error: &GateRunError,
) -> Result<(), GateRunError> {
    let Some((root, output)) = bootstrap else {
        return Ok(());
    };
    for stale_output in [
        output.as_path(),
        Path::new("stage-0.builder.json"),
        Path::new("stage-0-builder-report.json"),
    ] {
        root.remove_if_exists(stale_output)
            .map_err(|remove_error| GateRunError::Output(remove_error.to_string()))?;
    }
    let invocation_id = random_invocation_id().unwrap_or_else(|_| "unavailable".to_owned());
    publish_lifecycle_report(
        root,
        output,
        "stage-0-foundations",
        &invocation_id,
        "",
        failure_phase(error),
        gate_error_code(error),
        "FAIL",
        &BTreeMap::new(),
    )
}

fn failure_phase(error: &GateRunError) -> &'static str {
    match error {
        GateRunError::ConfigRead(_) | GateRunError::ConfigInvalid(_) => "configuration",
        GateRunError::UnsafeOutput | GateRunError::Output(_) => "output",
        GateRunError::ProgramUnavailable(_) => "program_resolution",
        GateRunError::Repository(_) => "repository",
        GateRunError::Check(_) => "checks",
        GateRunError::Artifact(_) => "artifacts",
        GateRunError::Input(_) => "committed_inputs",
        GateRunError::Report(_) => "report",
    }
}

#[allow(clippy::too_many_arguments)]
fn publish_lifecycle_report(
    root: &OutputRoot,
    output: &Path,
    stage_id: &str,
    invocation_id: &str,
    implementation_commit: &str,
    failure_phase: &str,
    error_code: &str,
    overall_result: &str,
    check_states: &BTreeMap<String, &str>,
) -> Result<(), GateRunError> {
    let report = serde_json::json!({
        "check_results": check_states,
        "error_code": error_code,
        "failure_phase": failure_phase,
        "implementation_commit": implementation_commit,
        "invocation_id": invocation_id,
        "overall_result": overall_result,
        "schema_version": 1,
        "stage_id": stage_id,
        "stage_outcome": "HOLD",
    });
    let bytes = canonicalize(&report).map_err(|error| GateRunError::Report(error.to_string()))?;
    root.write_atomic(output, &bytes)
        .map_err(|error| GateRunError::Output(error.to_string()))
}

fn random_invocation_id() -> Result<String, GateRunError> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| GateRunError::Output(error.to_string()))?;
    Ok(hex::encode(random))
}

fn repository_head(repository: &Path) -> String {
    Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(["rev-parse", "--verify", "HEAD^{commit}"])
        .env_clear()
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .map(|value| value.trim().to_owned())
        .unwrap_or_default()
}

fn capture_builder_environment(
    config: &GateConfig,
    roots: &[PathBuf],
    repository: &Path,
    environment: &BTreeMap<String, String>,
) -> Result<(BuilderEnvironment, BTreeMap<String, PathBuf>, bool, bool), GateRunError> {
    let mut identities = BTreeMap::new();
    let mut resolved_paths = BTreeMap::new();
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
                resolved_paths.insert(tool.id.clone(), identity.resolved_path.clone());
                identities.insert(tool.id.clone(), executable_evidence(identity));
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
        resolved_paths,
        complete,
        versions_match,
    ))
}

fn executable_evidence(identity: ExecutableIdentity) -> ExecutableEvidence {
    ExecutableEvidence {
        id: identity.id,
        sha256: identity.sha256,
        version_output: identity.version_output,
    }
}

fn aggregate_builder_reports(
    repository: &Path,
    config: &GateConfig,
    builder_report: &BuilderReport,
    policy: Option<&TrustPolicy>,
    verifier: Option<PathBuf>,
    reasons: &mut Vec<GateReasonCode>,
) -> Result<AggregateManifest, GateRunError> {
    let payload_path = repository.join(&config.comparison.second_builder_report_path);
    let signature_path = repository.join(&config.comparison.second_builder_signature_path);
    let Some((policy, verifier)) = policy.zip(verifier) else {
        reasons.push(GateReasonCode::SecondBuilderUnavailable);
        return single_builder_aggregate(builder_report)
            .map_err(|error| GateRunError::Report(error.to_string()));
    };
    if !payload_path.is_file() || !signature_path.is_file() {
        reasons.push(GateReasonCode::SecondBuilderUnavailable);
        return single_builder_aggregate(builder_report)
            .map_err(|error| GateRunError::Report(error.to_string()));
    }
    let second = match verify_signed_builder_report(
        &SignedEvidence {
            role: config.comparison.signer_role.clone(),
            payload_path,
            signature_path,
        },
        builder_report,
        &config.builder,
        policy,
        verifier,
        config.max_output_bytes,
    ) {
        Ok(verified) => verified.value,
        Err(_) => {
            reasons.push(GateReasonCode::SecondBuilderEvidenceInvalid);
            return single_builder_aggregate(builder_report)
                .map_err(|error| GateRunError::Report(error.to_string()));
        }
    };
    let aggregate = {
        let aggregate = aggregate_reports(builder_report, &second);
        if !builder_ids_are_independent(
            &builder_report.builder_identity.builder_id,
            &second.builder_identity.builder_id,
        ) {
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
    let approval = verify_approvals(&binding, policy, &evidence, resolved_gpgv);
    if approval.status == GateStatus::Fail {
        reasons.push(GateReasonCode::ApprovalEvidenceInvalid);
    } else if approval.status != GateStatus::Pass {
        if approval
            .reasons
            .contains(&ApprovalReasonCode::OpenPgpToolingUnavailable)
        {
            reasons.push(GateReasonCode::OpenpgpToolingUnavailable);
        } else if approval
            .reasons
            .iter()
            .any(|reason| *reason != ApprovalReasonCode::RequiredApprovalMissing)
        {
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

fn prepare_output_name(
    repository: &Path,
    config: &GateConfig,
    requested: &Path,
) -> Result<PathBuf, GateRunError> {
    let relative = if requested.is_absolute() {
        requested
            .strip_prefix(repository)
            .map_err(|_| GateRunError::UnsafeOutput)?
            .to_path_buf()
    } else {
        requested.to_path_buf()
    };
    if !matches!(
        relative.as_path(),
        path if path == Path::new("target/stage-gates/stage-0.json")
            || path == Path::new("target/stage-gates/stage-0.builder.json")
    ) || config.output_root != "target/stage-gates"
    {
        return Err(GateRunError::UnsafeOutput);
    }
    let name = relative
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or(GateRunError::UnsafeOutput)?;
    let ignored = Command::new("/usr/bin/git")
        .arg("-C")
        .arg(repository)
        .args(["check-ignore", "-q", "--"])
        .arg(&relative)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|_| GateRunError::UnsafeOutput)?;
    if !ignored.success() {
        return Err(GateRunError::UnsafeOutput);
    }
    Ok(PathBuf::from(name))
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

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

const fn reason_order(reason: &GateReasonCode) -> u8 {
    match reason {
        GateReasonCode::BuilderIdentityUnavailable => 0,
        GateReasonCode::BuilderVersionMismatch => 1,
        GateReasonCode::SecondBuilderUnavailable => 2,
        GateReasonCode::SecondBuilderEvidenceInvalid => 3,
        GateReasonCode::SecondBuilderIdentityInvalid => 4,
        GateReasonCode::SecondBuilderVersionMismatch => 5,
        GateReasonCode::SecondBuilderNotIdentical => 6,
        GateReasonCode::SecondBuilderMismatch => 7,
        GateReasonCode::TrustRegistryUnconfigured => 8,
        GateReasonCode::PlatformDataApprovalMissing => 9,
        GateReasonCode::IndependentReviewMissing => 10,
        GateReasonCode::OpenpgpToolingUnavailable => 11,
        GateReasonCode::ApprovalVerificationUnavailable => 12,
        GateReasonCode::ApprovalEvidenceInvalid => 13,
        GateReasonCode::RequiredGithubChecksUnavailable => 14,
        GateReasonCode::RequiredGithubChecksInvalid => 15,
    }
}
