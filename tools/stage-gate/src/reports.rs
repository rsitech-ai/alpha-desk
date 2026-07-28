use std::{
    cmp::Reverse,
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
    process::{CapturedOutput, CommandOutcome, CommandSpec},
};

pub const CHECK_EVIDENCE_NORMALIZATION: &str = "stage-gate-semantic-v1";

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
    pub check_evidence_normalization: String,
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
    pub check_evidence_normalization: String,
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
            check_evidence_normalization: self.check_evidence_normalization.clone(),
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

#[derive(Serialize)]
struct CheckEvidence<'a> {
    normalization: &'static str,
    command: CheckCommandEvidence,
    executable_sha256: &'a str,
    success: bool,
    exit_code: Option<i32>,
    stdout: StreamEvidence,
    stderr: StreamEvidence,
}

#[derive(Serialize)]
struct CheckCommandEvidence {
    check_id: String,
    program: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    arg0: Option<String>,
    args: Vec<String>,
    cwd: String,
    env: BTreeMap<String, String>,
    timeout_seconds: u64,
    termination_grace_seconds: u64,
}

#[derive(Serialize)]
struct StreamEvidence {
    semantic_sha256: String,
    semantic_bytes: usize,
    semantic_lines: usize,
    source_truncated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    truncated_source_total_bytes: Option<usize>,
}

impl StreamEvidence {
    fn normalized(output: &CapturedOutput, normalization: &NormalizationContext) -> Self {
        let semantic = semantic_output(&output.text, normalization);
        Self {
            semantic_sha256: sha256(semantic.as_bytes()),
            semantic_bytes: semantic.len(),
            semantic_lines: semantic.lines().count(),
            source_truncated: output.truncated,
            truncated_source_total_bytes: output.truncated.then_some(output.total_bytes),
        }
    }
}

#[doc(hidden)]
pub fn check_evidence_hash(
    check_id: &str,
    command: &CommandSpec,
    executable_sha256: &str,
    repository: &Path,
    outcome: &CommandOutcome,
) -> Result<String, CanonicalError> {
    let normalization = NormalizationContext::new(repository, command);
    canonicalize(&CheckEvidence {
        normalization: CHECK_EVIDENCE_NORMALIZATION,
        command: command_evidence(check_id, command, &normalization),
        executable_sha256,
        success: outcome.success,
        exit_code: outcome.exit_code,
        stdout: StreamEvidence::normalized(&outcome.stdout, &normalization),
        stderr: StreamEvidence::normalized(&outcome.stderr, &normalization),
    })
    .map(|bytes| sha256(&bytes))
}

#[derive(Clone, Debug)]
struct NormalizationContext {
    owned_prefixes: Vec<(String, &'static str)>,
}

impl NormalizationContext {
    fn new(repository: &Path, command: &CommandSpec) -> Self {
        let mut owned_prefixes = vec![(repository.to_string_lossy().into_owned(), "<REPOSITORY>")];
        for (key, value) in &command.env {
            let token = match key.as_str() {
                "HOME" => Some("<HOME>"),
                "CARGO_HOME" => Some("<CARGO_HOME>"),
                "RUSTUP_HOME" => Some("<RUSTUP_HOME>"),
                "TMPDIR" => Some("<TMPDIR>"),
                _ => None,
            };
            if let Some(token) = token
                && !value.is_empty()
            {
                owned_prefixes.push((value.clone(), token));
            }
        }
        owned_prefixes.sort_by_key(|prefix| Reverse(prefix.0.len()));
        Self { owned_prefixes }
    }

    fn tokenize(&self, source: &str) -> String {
        let mut tokenized = source.to_owned();
        for (prefix, replacement) in &self.owned_prefixes {
            tokenized = replace_path_prefix(&tokenized, prefix, replacement);
        }
        replace_stage_project_tokens(&tokenized)
    }
}

fn command_evidence(
    check_id: &str,
    command: &CommandSpec,
    normalization: &NormalizationContext,
) -> CheckCommandEvidence {
    let env = command
        .env
        .iter()
        .map(|(key, value)| (key.clone(), normalization.tokenize(value)))
        .collect();
    CheckCommandEvidence {
        check_id: check_id.to_owned(),
        program: normalization.tokenize(&command.program.to_string_lossy()),
        arg0: command
            .arg0
            .as_ref()
            .map(|arg0| normalization.tokenize(&arg0.to_string_lossy())),
        args: command
            .args
            .iter()
            .map(|argument| normalization.tokenize(&argument.to_string_lossy()))
            .collect(),
        cwd: normalization.tokenize(&command.cwd.to_string_lossy()),
        env,
        timeout_seconds: command.timeout.as_secs(),
        termination_grace_seconds: command.termination_grace.as_secs(),
    }
}

fn semantic_output(source: &str, normalization: &NormalizationContext) -> String {
    strip_ansi(source)
        .split_inclusive('\n')
        .filter_map(|line| {
            let (content, newline) = line
                .strip_suffix('\n')
                .map_or((line, ""), |content| (content, "\n"));
            let protected = contains_diagnostic(content);
            if !protected && is_known_build_progress(content) {
                return None;
            }
            if protected {
                if let Some(summary) = normalize_exact_test_summary(content) {
                    return Some(format!("{summary}{newline}"));
                }
                return Some(format!("{content}{newline}"));
            }
            let tokenized = normalization.tokenize(content);
            Some(format!("{}{newline}", normalize_durations(&tokenized)))
        })
        .collect()
}

fn normalize_exact_test_summary(line: &str) -> Option<String> {
    normalize_cargo_test_summary(line).or_else(|| normalize_swift_test_summary(line))
}

fn normalize_cargo_test_summary(line: &str) -> Option<String> {
    let (counts, duration) = line.rsplit_once("; finished in ")?;
    if !is_duration(duration) {
        return None;
    }
    let counts = counts.strip_prefix("test result: ")?;
    let (status, counts) = counts.split_once(". ")?;
    if !matches!(status, "ok" | "FAILED") {
        return None;
    }
    let fields = counts.split("; ").collect::<Vec<_>>();
    if !(4..=5).contains(&fields.len())
        || !count_field(fields[0], "passed")
        || !count_field(fields[1], "failed")
        || !count_field(fields[2], "ignored")
        || !count_field(fields[3], "measured")
        || fields
            .get(4)
            .is_some_and(|field| !count_field(field, "filtered out"))
    {
        return None;
    }
    Some(format!(
        "{line_prefix}; finished in <DURATION>",
        line_prefix = line.split_once("; finished in ")?.0
    ))
}

fn count_field(field: &str, label: &str) -> bool {
    field
        .strip_suffix(label)
        .and_then(|value| value.strip_suffix(' '))
        .is_some_and(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn normalize_swift_test_summary(line: &str) -> Option<String> {
    let after_executed = line.strip_prefix("Executed ")?;
    let (test_count, rest) = after_executed.split_once(" tests, with ")?;
    if !ascii_digits(test_count) {
        return None;
    }
    let (failure_count, rest) = rest.split_once(" failures (")?;
    if !ascii_digits(failure_count) {
        return None;
    }
    let (unexpected_count, timings) = rest.split_once(" unexpected) in ")?;
    if !ascii_digits(unexpected_count) {
        return None;
    }
    let timings = timings.strip_suffix(" seconds")?;
    let (wall, cpu_parenthesized) = timings.split_once(" (")?;
    let cpu = cpu_parenthesized.strip_suffix(')')?;
    if !decimal_number(wall) || !decimal_number(cpu) {
        return None;
    }
    Some(format!(
        "Executed {test_count} tests, with {failure_count} failures ({unexpected_count} unexpected) in <DURATION> (<DURATION>) seconds"
    ))
}

fn ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn decimal_number(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
        && value.bytes().filter(|byte| *byte == b'.').count() <= 1
}

fn contains_diagnostic(line: &str) -> bool {
    let lowercase = line.to_ascii_lowercase();
    ["warning", "error", "fail", "skip", "ignore", "panic"]
        .iter()
        .any(|marker| lowercase.contains(marker))
}

fn is_known_build_progress(line: &str) -> bool {
    let trimmed = line.trim_start();
    if cargo_package_progress(trimmed) || cargo_finished_progress(trimmed) {
        return true;
    }
    if matches!(
        trimmed,
        "Building for debugging..." | "Building for production..."
    ) || swift_build_complete(trimmed)
    {
        return true;
    }
    if let Some(after_counter) = swift_progress_suffix(trimmed) {
        const SWIFT_PREFIXES: [&str; 8] = [
            "Compiling ",
            "Emitting module ",
            "Linking ",
            "Write sources",
            "Write swift-version",
            "Wrapping AST",
            "Copying ",
            "Planning build",
        ];
        if SWIFT_PREFIXES
            .iter()
            .any(|prefix| after_counter.starts_with(prefix))
        {
            return true;
        }
    }
    docker_progress(trimmed)
}

fn cargo_package_progress(line: &str) -> bool {
    let Some(rest) = ["Compiling ", "Checking ", "Fresh "]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))
    else {
        return false;
    };
    let Some((package, version_and_source)) = rest.split_once(' ') else {
        return false;
    };
    let (version, source) = version_and_source
        .split_once(' ')
        .map_or((version_and_source, None), |(version, source)| {
            (version, Some(source))
        });
    !package.is_empty()
        && version.strip_prefix('v').is_some_and(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'+'))
        })
        && source.is_none_or(|source| {
            source.len() >= 3
                && source.starts_with('(')
                && source.ends_with(')')
                && source[1..source.len() - 1].starts_with('/')
                && !source[1..source.len() - 1].contains(['(', ')'])
        })
}

fn cargo_finished_progress(line: &str) -> bool {
    line.strip_prefix("Finished `").is_some_and(|rest| {
        rest.contains("` profile [")
            && rest.contains(" target(s) in ")
            && rest
                .split_ascii_whitespace()
                .next_back()
                .is_some_and(is_duration)
    })
}

fn swift_build_complete(line: &str) -> bool {
    line.strip_prefix("Build complete! (")
        .and_then(|duration| duration.strip_suffix(')'))
        .is_some_and(is_duration)
}

fn swift_progress_suffix(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let (counter, suffix) = rest.split_once("] ")?;
    let (done, total) = counter.split_once('/')?;
    if done.bytes().all(|byte| byte.is_ascii_digit())
        && total.bytes().all(|byte| byte.is_ascii_digit())
    {
        Some(suffix)
    } else {
        None
    }
}

fn docker_progress(line: &str) -> bool {
    let Some((kind, rest)) = line.split_once(' ') else {
        return false;
    };
    if !matches!(kind, "Container" | "Network" | "Volume") {
        return false;
    }
    [
        " Creating",
        " Created",
        " Starting",
        " Started",
        " Stopping",
        " Stopped",
        " Removing",
        " Removed",
    ]
    .iter()
    .any(|status| rest.ends_with(status))
}

fn strip_ansi(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == 0x1b && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}

fn replace_path_prefix(line: &str, prefix: &str, replacement: &str) -> String {
    if prefix.is_empty() {
        return line.to_owned();
    }
    let mut result = String::with_capacity(line.len());
    let mut cursor = 0;
    while let Some(offset) = line[cursor..].find(prefix) {
        let start = cursor + offset;
        let end = start + prefix.len();
        let before_ok = start == 0
            || line[..start].chars().next_back().is_some_and(|character| {
                character.is_whitespace()
                    || matches!(character, '(' | '[' | '{' | '"' | '\'' | '=' | ':')
            });
        let after_ok = end == line.len()
            || line[end..].chars().next().is_some_and(|character| {
                character == '/' || character.is_whitespace() || matches!(character, ')' | ']')
            });
        result.push_str(&line[cursor..start]);
        if before_ok && after_ok {
            result.push_str(replacement);
        } else {
            result.push_str(prefix);
        }
        cursor = end;
    }
    result.push_str(&line[cursor..]);
    result
}

fn replace_stage_project_tokens(line: &str) -> String {
    const PREFIX: &str = "alpha-desk-stage0-";
    let mut result = String::with_capacity(line.len());
    let mut cursor = 0;
    while let Some(offset) = line[cursor..].find(PREFIX) {
        let start = cursor + offset;
        let nonce_start = start + PREFIX.len();
        let nonce_end = nonce_start.saturating_add(8);
        let left_boundary = start == 0
            || line[..start]
                .chars()
                .next_back()
                .is_some_and(|character| !character.is_ascii_alphanumeric() && character != '_');
        let valid_nonce = line.get(nonce_start..nonce_end).is_some_and(|nonce| {
            nonce
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        });
        let boundary = line
            .get(nonce_end..)
            .and_then(|suffix| suffix.chars().next())
            .is_none_or(|character| !character.is_ascii_alphanumeric());
        result.push_str(&line[cursor..start]);
        if left_boundary && valid_nonce && boundary {
            result.push_str("<STAGE0_PROJECT>");
            cursor = nonce_end;
        } else {
            result.push_str(PREFIX);
            cursor = nonce_start;
        }
    }
    result.push_str(&line[cursor..]);
    result
}

fn normalize_durations(line: &str) -> String {
    let mut output = String::with_capacity(line.len());
    let mut cursor = 0;
    while cursor < line.len() {
        let Some((offset, character)) = line[cursor..].char_indices().next() else {
            break;
        };
        let start = cursor + offset;
        let left_boundary = start == 0
            || line[..start]
                .chars()
                .next_back()
                .is_some_and(|previous| !previous.is_ascii_alphanumeric() && previous != '_');
        if left_boundary && character.is_ascii_digit() {
            let mut end = start;
            let mut dots = 0;
            for (relative, candidate) in line[start..].char_indices() {
                if candidate.is_ascii_digit() {
                    end = start + relative + candidate.len_utf8();
                } else if candidate == '.' && dots == 0 {
                    dots += 1;
                    end = start + relative + 1;
                } else {
                    break;
                }
            }
            for unit in ["ns", "us", "µs", "ms", "s"] {
                if line[end..].starts_with(unit) {
                    let duration_end = end + unit.len();
                    let right_boundary = duration_end == line.len()
                        || line[duration_end..]
                            .chars()
                            .next()
                            .is_some_and(|next| !next.is_ascii_alphanumeric() && next != '_');
                    if right_boundary {
                        output.push_str(&line[cursor..start]);
                        output.push_str("<DURATION>");
                        cursor = duration_end;
                        break;
                    }
                }
            }
            if cursor == end || cursor == start {
                output.push(character);
                cursor = start + character.len_utf8();
            }
        } else {
            output.push(character);
            cursor = start + character.len_utf8();
        }
    }
    output
}

fn is_duration(core: &str) -> bool {
    for suffix in ["ns", "us", "µs", "ms", "s"] {
        if let Some(number) = core.strip_suffix(suffix)
            && !number.is_empty()
            && number
                .bytes()
                .all(|byte| byte.is_ascii_digit() || byte == b'.')
            && number.bytes().filter(|byte| *byte == b'.').count() <= 1
        {
            return true;
        }
    }
    false
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
