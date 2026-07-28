use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Component, Path},
};

use serde::Deserialize;

const REQUIRED_SCHEMA_VERSION: u32 = 1;
const REQUIRED_REVIEWER_ROLES: [&str; 2] = ["platform-data", "independent"];
const FORBIDDEN_SHELLS: [&str; 9] = [
    "bash",
    "cmd",
    "dash",
    "fish",
    "ksh",
    "powershell",
    "pwsh",
    "sh",
    "zsh",
];

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct GateConfig {
    pub schema_version: u32,
    pub stage_id: String,
    pub schema_path: String,
    pub output_root: String,
    pub builder_report_output_path: String,
    pub whole_gate_timeout_seconds: u64,
    pub max_output_bytes: usize,
    pub allowed_programs: Vec<String>,
    pub program_roots: Vec<String>,
    pub design: DesignConfig,
    pub builder: BuilderConfig,
    pub comparison: ComparisonConfig,
    pub approvals: ApprovalConfig,
    pub remote: RemoteConfig,
    #[serde(default)]
    pub artifacts: Vec<ArtifactConfig>,
    pub checks: Vec<CheckConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuilderConfig {
    pub target_tool: String,
    pub tools: Vec<BuilderToolConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct BuilderToolConfig {
    pub id: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub expected_output_contains: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DesignConfig {
    pub tag: String,
    pub object: String,
    pub commit: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonConfig {
    pub second_builder_report_path: String,
    #[serde(default)]
    pub second_builder_signature_path: String,
    #[serde(default)]
    pub signer_role: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApprovalConfig {
    pub policy_path: String,
    pub required_roles: Vec<String>,
    pub gpgv_program: String,
    pub known_limitations: Vec<String>,
    pub evidence: Vec<ApprovalEvidenceConfig>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ApprovalEvidenceConfig {
    pub role: String,
    pub statement_path: String,
    pub signature_path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct RemoteConfig {
    pub proof_path: String,
    #[serde(default)]
    pub signature_path: String,
    #[serde(default)]
    pub signer_role: String,
    #[serde(default)]
    pub repository: String,
    #[serde(default)]
    pub repository_id: u64,
    #[serde(default)]
    pub repository_owner_id: u64,
    #[serde(default)]
    pub workflow: String,
    #[serde(default)]
    pub workflow_ref: String,
    #[serde(default)]
    pub workflow_sha: String,
    #[serde(default = "default_event_name")]
    pub event_name: String,
    #[serde(default = "default_git_ref")]
    pub git_ref: String,
    #[serde(default = "default_signing_check_name")]
    pub signing_check_name: String,
    pub required_checks: Vec<String>,
}

fn default_event_name() -> String {
    "push".to_owned()
}

fn default_git_ref() -> String {
    "refs/heads/main".to_owned()
}

fn default_signing_check_name() -> String {
    "Stage 0 evidence signing".to_owned()
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ArtifactConfig {
    pub id: String,
    pub path: String,
    pub kind: String,
    pub producer: String,
    pub target_triple: String,
    pub profile: String,
    pub expected_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct CheckConfig {
    pub id: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
    pub cwd: String,
    pub timeout_seconds: u64,
    #[serde(default = "default_termination_grace_seconds")]
    pub termination_grace_seconds: u64,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub inherit_env: Vec<String>,
}

const fn default_termination_grace_seconds() -> u64 {
    2
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ConfigErrorCode {
    InvalidToml,
    UnsupportedSchemaVersion,
    DuplicateCheckId,
    DuplicateArtifactId,
    MissingCommand,
    MissingArtifact,
    MalformedHash,
    UnsafeProgram,
    UnsafeArgument,
    UnsafeWorkingDirectory,
    UnsafeOutput,
    MissingReviewerRole,
    InvalidValue,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("configuration is not valid TOML: {0}")]
    InvalidToml(#[from] toml::de::Error),
    #[error("unsupported configuration schema version {0}")]
    UnsupportedSchemaVersion(u32),
    #[error("configuration repeats check ID {0}")]
    DuplicateCheckId(String),
    #[error("configuration repeats artifact ID {0}")]
    DuplicateArtifactId(String),
    #[error("check {0} has no command")]
    MissingCommand(String),
    #[error("configuration declares no artifacts")]
    MissingArtifact,
    #[error("artifact {0} has a malformed expected SHA-256")]
    MalformedHash(String),
    #[error("check {check_id} uses unsafe or unapproved program {program}")]
    UnsafeProgram { check_id: String, program: String },
    #[error("check {check_id} has an unsafe argument")]
    UnsafeArgument { check_id: String },
    #[error("unsafe repository-relative path for {field}: {path}")]
    UnsafePath { field: &'static str, path: String },
    #[error("output root must be target/stage-gates or its descendant")]
    UnsafeOutput,
    #[error("required reviewer role is missing: {0}")]
    MissingReviewerRole(String),
    #[error("invalid configuration value: {0}")]
    InvalidValue(String),
}

impl ConfigError {
    #[must_use]
    pub fn code(&self) -> ConfigErrorCode {
        match self {
            Self::InvalidToml(_) => ConfigErrorCode::InvalidToml,
            Self::UnsupportedSchemaVersion(_) => ConfigErrorCode::UnsupportedSchemaVersion,
            Self::DuplicateCheckId(_) => ConfigErrorCode::DuplicateCheckId,
            Self::DuplicateArtifactId(_) => ConfigErrorCode::DuplicateArtifactId,
            Self::MissingCommand(_) => ConfigErrorCode::MissingCommand,
            Self::MissingArtifact => ConfigErrorCode::MissingArtifact,
            Self::MalformedHash(_) => ConfigErrorCode::MalformedHash,
            Self::UnsafeProgram { .. } => ConfigErrorCode::UnsafeProgram,
            Self::UnsafeArgument { .. } => ConfigErrorCode::UnsafeArgument,
            Self::UnsafePath {
                field: "check cwd", ..
            } => ConfigErrorCode::UnsafeWorkingDirectory,
            Self::UnsafePath { .. } => ConfigErrorCode::InvalidValue,
            Self::UnsafeOutput => ConfigErrorCode::UnsafeOutput,
            Self::MissingReviewerRole(_) => ConfigErrorCode::MissingReviewerRole,
            Self::InvalidValue(_) => ConfigErrorCode::InvalidValue,
        }
    }
}

impl GateConfig {
    pub fn parse(source: &str) -> Result<Self, ConfigError> {
        let config: Self = toml::from_str(source)?;
        config.validate()?;
        Ok(config)
    }

    fn validate(&self) -> Result<(), ConfigError> {
        if self.schema_version != REQUIRED_SCHEMA_VERSION {
            return Err(ConfigError::UnsupportedSchemaVersion(self.schema_version));
        }
        require_token("stage_id", &self.stage_id)?;
        validate_relative_path("schema path", &self.schema_path)?;
        validate_relative_path(
            "builder report output path",
            &self.builder_report_output_path,
        )?;
        require_token("design tag", &self.design.tag)?;
        require_sha1_object("design tag object", &self.design.object)?;
        require_sha1_object("design commit", &self.design.commit)?;
        validate_relative_path(
            "second builder report path",
            &self.comparison.second_builder_report_path,
        )?;
        validate_relative_path(
            "second builder signature path",
            &self.comparison.second_builder_signature_path,
        )?;
        if self.comparison.signer_role != "builder-b" {
            return Err(ConfigError::InvalidValue(
                "comparison.signer_role must be builder-b".to_owned(),
            ));
        }
        validate_relative_path("approval policy path", &self.approvals.policy_path)?;
        validate_relative_path("remote proof path", &self.remote.proof_path)?;
        validate_relative_path("remote signature path", &self.remote.signature_path)?;
        if self.remote.signer_role != "github-ci" {
            return Err(ConfigError::InvalidValue(
                "remote.signer_role must be github-ci".to_owned(),
            ));
        }
        require_token("remote repository", &self.remote.repository)?;
        require_token("remote workflow", &self.remote.workflow)?;
        require_token("remote workflow ref", &self.remote.workflow_ref)?;
        require_token("remote event", &self.remote.event_name)?;
        require_token("remote git ref", &self.remote.git_ref)?;
        require_token("remote signing check name", &self.remote.signing_check_name)?;
        if self.remote.repository_id == 0 || self.remote.repository_owner_id == 0 {
            return Err(ConfigError::InvalidValue(
                "remote repository numeric identities must be non-zero".to_owned(),
            ));
        }
        require_sha1_object("remote workflow source commit", &self.remote.workflow_sha)?;
        if self.remote.required_checks.is_empty() {
            return Err(ConfigError::InvalidValue(
                "remote.required_checks must not be empty".to_owned(),
            ));
        }
        let mut remote_checks = BTreeSet::new();
        for check in &self.remote.required_checks {
            require_token("remote required check", check)?;
            if !remote_checks.insert(check) {
                return Err(ConfigError::InvalidValue(format!(
                    "duplicate remote required check {check}"
                )));
            }
        }
        if self.whole_gate_timeout_seconds == 0 {
            return Err(ConfigError::InvalidValue(
                "whole_gate_timeout_seconds must be non-zero".to_owned(),
            ));
        }
        if self.max_output_bytes < crate::process::TRUNCATION_MARKER.len() {
            return Err(ConfigError::InvalidValue(format!(
                "max_output_bytes must be at least {}",
                crate::process::TRUNCATION_MARKER.len()
            )));
        }
        require_token("gpgv program", &self.approvals.gpgv_program)?;
        for limitation in &self.approvals.known_limitations {
            require_token("known limitation", limitation)?;
        }
        let gpgv_name = Path::new(&self.approvals.gpgv_program)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(&self.approvals.gpgv_program);
        if FORBIDDEN_SHELLS.contains(&gpgv_name) {
            return Err(ConfigError::UnsafeProgram {
                check_id: "approvals".to_owned(),
                program: self.approvals.gpgv_program.clone(),
            });
        }
        let mut evidence_roles = BTreeSet::new();
        for evidence in &self.approvals.evidence {
            require_token("approval evidence role", &evidence.role)?;
            if !evidence_roles.insert(&evidence.role) {
                return Err(ConfigError::InvalidValue(format!(
                    "duplicate approval evidence role {}",
                    evidence.role
                )));
            }
            validate_relative_path("approval statement path", &evidence.statement_path)?;
            validate_relative_path("approval signature path", &evidence.signature_path)?;
        }
        for role in REQUIRED_REVIEWER_ROLES {
            if !self
                .approvals
                .required_roles
                .iter()
                .any(|candidate| candidate == role)
            {
                return Err(ConfigError::MissingReviewerRole(role.to_owned()));
            }
            if !evidence_roles.iter().any(|candidate| *candidate == role) {
                return Err(ConfigError::MissingReviewerRole(role.to_owned()));
            }
        }
        let output_root = Path::new(&self.output_root);
        validate_relative_path("output root", &self.output_root)?;
        if output_root != Path::new("target/stage-gates")
            && !output_root.starts_with("target/stage-gates")
        {
            return Err(ConfigError::UnsafeOutput);
        }
        let builder_output = Path::new(&self.builder_report_output_path);
        if !builder_output.starts_with(output_root) || builder_output == output_root {
            return Err(ConfigError::UnsafeOutput);
        }

        if self.artifacts.is_empty() {
            return Err(ConfigError::MissingArtifact);
        }
        let mut artifact_ids = BTreeSet::new();
        let mut artifact_paths = BTreeSet::new();
        for artifact in &self.artifacts {
            require_token("artifact ID", &artifact.id)?;
            if !artifact_ids.insert(&artifact.id) {
                return Err(ConfigError::DuplicateArtifactId(artifact.id.clone()));
            }
            validate_relative_path("artifact path", &artifact.path)?;
            if !artifact_paths.insert(&artifact.path) {
                return Err(ConfigError::DuplicateArtifactId(artifact.path.clone()));
            }
            require_token("artifact kind", &artifact.kind)?;
            require_token("artifact producer", &artifact.producer)?;
            require_token("artifact target triple", &artifact.target_triple)?;
            require_token("artifact profile", &artifact.profile)?;
            if artifact
                .expected_sha256
                .as_deref()
                .is_some_and(|hash| !is_lower_hex(hash, 64))
            {
                return Err(ConfigError::MalformedHash(artifact.id.clone()));
            }
        }

        if self.allowed_programs.is_empty() {
            return Err(ConfigError::InvalidValue(
                "allowed_programs must not be empty".to_owned(),
            ));
        }
        if self.program_roots.is_empty() {
            return Err(ConfigError::InvalidValue(
                "program_roots must not be empty".to_owned(),
            ));
        }
        for root in &self.program_roots {
            if root != "$CARGO_HOME/bin" && root != "$HOME/.cargo/bin" {
                let path = Path::new(root);
                if !path.is_absolute()
                    || path
                        .components()
                        .any(|component| matches!(component, Component::ParentDir))
                {
                    return Err(ConfigError::InvalidValue(format!(
                        "program root must be absolute or a supported token: {root}"
                    )));
                }
            }
        }
        let allowed_programs = self.allowed_programs.iter().collect::<BTreeSet<_>>();
        if !allowed_programs.contains(&self.approvals.gpgv_program) {
            return Err(ConfigError::UnsafeProgram {
                check_id: "approvals".to_owned(),
                program: self.approvals.gpgv_program.clone(),
            });
        }
        let mut tool_ids = BTreeSet::new();
        for tool in &self.builder.tools {
            require_token("builder tool ID", &tool.id)?;
            if !tool_ids.insert(&tool.id) {
                return Err(ConfigError::InvalidValue(format!(
                    "duplicate builder tool ID {}",
                    tool.id
                )));
            }
            if !allowed_programs.contains(&tool.program) {
                return Err(ConfigError::UnsafeProgram {
                    check_id: format!("builder:{}", tool.id),
                    program: tool.program.clone(),
                });
            }
            if tool
                .args
                .iter()
                .any(|argument| argument.chars().any(char::is_control))
            {
                return Err(ConfigError::UnsafeArgument {
                    check_id: format!("builder:{}", tool.id),
                });
            }
            if let Some(expectation) = &tool.expected_output_contains {
                require_token("builder version expectation", expectation)?;
            }
        }
        if !tool_ids.contains(&self.builder.target_tool) {
            return Err(ConfigError::InvalidValue(
                "builder.target_tool must name a configured tool".to_owned(),
            ));
        }
        let mut check_ids = BTreeSet::new();
        if self.checks.is_empty() {
            return Err(ConfigError::MissingCommand("<all>".to_owned()));
        }
        for check in &self.checks {
            require_token("check ID", &check.id)?;
            if !check_ids.insert(&check.id) {
                return Err(ConfigError::DuplicateCheckId(check.id.clone()));
            }
            if check.program.is_empty() {
                return Err(ConfigError::MissingCommand(check.id.clone()));
            }
            let executable_name = Path::new(&check.program)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or(&check.program);
            if FORBIDDEN_SHELLS.contains(&executable_name)
                || !allowed_programs.contains(&check.program)
            {
                return Err(ConfigError::UnsafeProgram {
                    check_id: check.id.clone(),
                    program: check.program.clone(),
                });
            }
            if check
                .args
                .iter()
                .any(|argument| argument.chars().any(char::is_control))
            {
                return Err(ConfigError::UnsafeArgument {
                    check_id: check.id.clone(),
                });
            }
            validate_relative_path("check cwd", &check.cwd)?;
            if check.timeout_seconds == 0 {
                return Err(ConfigError::InvalidValue(format!(
                    "check {} timeout_seconds must be non-zero",
                    check.id
                )));
            }
            if check.termination_grace_seconds == 0 || check.termination_grace_seconds > 300 {
                return Err(ConfigError::InvalidValue(format!(
                    "check {} termination_grace_seconds must be between 1 and 300",
                    check.id
                )));
            }
            for (key, value) in &check.env {
                require_token("environment key", key)?;
                if value.chars().any(char::is_control) {
                    return Err(ConfigError::InvalidValue(format!(
                        "check {} environment value contains a control character",
                        check.id
                    )));
                }
            }
            for name in &check.inherit_env {
                if !matches!(
                    name.as_str(),
                    "PATH" | "HOME" | "CARGO_HOME" | "RUSTUP_HOME" | "TMPDIR"
                ) {
                    return Err(ConfigError::InvalidValue(format!(
                        "check {} requests unapproved inherited environment {name}",
                        check.id
                    )));
                }
            }
        }
        Ok(())
    }
}

fn validate_relative_path(field: &'static str, value: &str) -> Result<(), ConfigError> {
    let path = Path::new(value);
    if value.is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ConfigError::UnsafePath {
            field,
            path: value.to_owned(),
        });
    }
    Ok(())
}

fn require_token(field: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(ConfigError::InvalidValue(format!(
            "{field} must be a non-empty printable string"
        )));
    }
    Ok(())
}

fn require_sha1_object(field: &'static str, value: &str) -> Result<(), ConfigError> {
    if value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(ConfigError::InvalidValue(format!(
            "{field} must be a lowercase 40-character object ID"
        )))
    }
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
