use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

const CONTENT_RULES: &[(&str, &[u8])] = &[
    ("private.absolute_user_path", b"/Users/"),
    ("private.alpha_threshold", b"private_signal_threshold ="),
    ("secret.github_pat", b"ghp_"),
    ("secret.inline_api_key", b"api_key = \""),
    ("secret.private_key", b"-----BEGIN OPENSSH PRIVATE KEY-----"),
    ("execution.package", b"name = \"hl-exec\""),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Classification {
    Public,
    Private,
    GeneratedReviewRequired,
    Excluded,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuditPolicy {
    schema_version: u32,
    max_file_bytes: u64,
    #[serde(default)]
    public: Vec<String>,
    #[serde(default)]
    private: Vec<String>,
    #[serde(default)]
    generated_review_required: Vec<String>,
    #[serde(default)]
    excluded: Vec<String>,
    #[serde(default)]
    forbidden_path_prefixes: Vec<String>,
    #[serde(default)]
    allowed_binary_paths: Vec<String>,
    #[serde(default)]
    content_allowlist: Vec<ContentAllowlistEntry>,
    #[serde(default)]
    classification_overrides: Vec<ClassificationOverride>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContentAllowlistEntry {
    rule: String,
    path: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ClassificationOverride {
    path: String,
    classification: PolicyClassification,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum PolicyClassification {
    Public,
    Private,
    GeneratedReviewRequired,
    Excluded,
}

impl From<PolicyClassification> for Classification {
    fn from(value: PolicyClassification) -> Self {
        match value {
            PolicyClassification::Public => Self::Public,
            PolicyClassification::Private => Self::Private,
            PolicyClassification::GeneratedReviewRequired => Self::GeneratedReviewRequired,
            PolicyClassification::Excluded => Self::Excluded,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    reason_code: &'static str,
    path: PathBuf,
}

impl Finding {
    #[must_use]
    pub fn reason_code(&self) -> &'static str {
        self.reason_code
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Debug, Clone, Default)]
pub struct AuditReport {
    findings: Vec<Finding>,
}

impl AuditReport {
    #[must_use]
    pub fn findings(&self) -> &[Finding] {
        &self.findings
    }

    #[must_use]
    pub fn reason_codes(&self) -> Vec<&'static str> {
        self.findings.iter().map(Finding::reason_code).collect()
    }

    #[must_use]
    pub fn is_pass(&self) -> bool {
        self.findings.is_empty()
    }
}

#[derive(Debug, Error)]
pub enum AuditError {
    #[error("{reason_code}: {detail}")]
    Policy {
        reason_code: &'static str,
        detail: String,
    },
    #[error("{reason_code}: {path}")]
    Input {
        reason_code: &'static str,
        path: PathBuf,
    },
}

impl AuditError {
    #[must_use]
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::Policy { reason_code, .. } | Self::Input { reason_code, .. } => reason_code,
        }
    }
}

impl AuditPolicy {
    pub fn from_toml(source: &str) -> Result<Self, AuditError> {
        let policy: Self = toml::from_str(source).map_err(|error| AuditError::Policy {
            reason_code: "policy.invalid_toml",
            detail: error.to_string(),
        })?;
        policy.validate()?;
        Ok(policy)
    }

    fn validate(&self) -> Result<(), AuditError> {
        if self.schema_version != 1 {
            return Err(policy_error(
                "policy.unsupported_schema",
                format!("expected schema_version 1, got {}", self.schema_version),
            ));
        }
        if self.max_file_bytes == 0 {
            return Err(policy_error(
                "policy.invalid_limit",
                "max_file_bytes must be greater than zero",
            ));
        }

        let mut classifications = BTreeMap::new();
        for (classification, entries) in [
            (Classification::Public, &self.public),
            (Classification::Private, &self.private),
            (
                Classification::GeneratedReviewRequired,
                &self.generated_review_required,
            ),
            (Classification::Excluded, &self.excluded),
        ] {
            for entry in entries {
                validate_policy_path(entry)?;
                if Path::new(entry).components().count() != 1 {
                    return Err(policy_error(
                        "policy.non_top_level_classification",
                        entry.clone(),
                    ));
                }
                if classifications
                    .insert(entry.as_str(), classification)
                    .is_some()
                {
                    return Err(policy_error(
                        "policy.duplicate_classification",
                        entry.clone(),
                    ));
                }
            }
        }

        for entry in &self.classification_overrides {
            validate_policy_path(&entry.path)?;
        }
        for entry in &self.content_allowlist {
            validate_policy_path(&entry.path)?;
            if !CONTENT_RULES.iter().any(|(rule, _)| *rule == entry.rule) {
                return Err(policy_error(
                    "policy.unknown_allowlist_rule",
                    entry.rule.clone(),
                ));
            }
        }
        for prefix in &self.forbidden_path_prefixes {
            validate_policy_path(prefix.trim_end_matches('.'))?;
        }
        for path in &self.allowed_binary_paths {
            validate_policy_path(path)?;
        }
        Ok(())
    }

    #[must_use]
    pub fn classification_for(&self, path: &Path) -> Option<Classification> {
        let normalized = normalized_display(path);
        let override_match = self
            .classification_overrides
            .iter()
            .filter(|entry| path_has_prefix(&normalized, &entry.path))
            .max_by_key(|entry| entry.path.len());
        if let Some(entry) = override_match {
            return Some(entry.classification.into());
        }

        let top_level = path.components().next()?.as_os_str().to_str()?;
        [
            (Classification::Public, &self.public),
            (Classification::Private, &self.private),
            (
                Classification::GeneratedReviewRequired,
                &self.generated_review_required,
            ),
            (Classification::Excluded, &self.excluded),
        ]
        .into_iter()
        .find_map(|(classification, entries)| {
            entries
                .iter()
                .any(|entry| entry == top_level)
                .then_some(classification)
        })
    }

    fn content_allowed(&self, rule: &str, path: &Path) -> bool {
        let path = normalized_display(path);
        self.content_allowlist
            .iter()
            .any(|entry| entry.rule == rule && entry.path == path)
    }
}

pub fn audit_paths(
    root: &Path,
    paths: &[PathBuf],
    policy: &AuditPolicy,
) -> Result<AuditReport, AuditError> {
    let root = root.canonicalize().map_err(|_| AuditError::Input {
        reason_code: "input.missing_root",
        path: root.to_path_buf(),
    })?;
    let allowed_binary_paths: BTreeSet<&str> = policy
        .allowed_binary_paths
        .iter()
        .map(String::as_str)
        .collect();
    let mut findings = Vec::new();

    for relative in paths {
        validate_inventory_path(relative)?;
        let normalized = normalized_display(relative);
        let classification = policy.classification_for(relative);
        if classification.is_none() {
            findings.push(Finding {
                reason_code: "classification.unclassified_top_level",
                path: relative.clone(),
            });
        }

        if policy
            .forbidden_path_prefixes
            .iter()
            .any(|prefix| normalized.starts_with(prefix))
        {
            findings.push(Finding {
                reason_code: "path.forbidden_prefix",
                path: relative.clone(),
            });
        }

        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path).map_err(|_| AuditError::Input {
            reason_code: "input.missing_file",
            path: relative.clone(),
        })?;
        if metadata.file_type().is_symlink() {
            return Err(AuditError::Input {
                reason_code: "input.symlink",
                path: relative.clone(),
            });
        }
        if !metadata.is_file() {
            return Err(AuditError::Input {
                reason_code: "input.non_regular_file",
                path: relative.clone(),
            });
        }

        if matches!(
            classification,
            Some(Classification::Private | Classification::Excluded)
        ) {
            continue;
        }
        if metadata.len() > policy.max_file_bytes {
            findings.push(Finding {
                reason_code: "content.file_too_large",
                path: relative.clone(),
            });
            continue;
        }

        let bytes = fs::read(&path).map_err(|_| AuditError::Input {
            reason_code: "input.read_failed",
            path: relative.clone(),
        })?;
        if bytes.contains(&0) {
            if !allowed_binary_paths.contains(normalized.as_str()) {
                findings.push(Finding {
                    reason_code: "content.binary_unreviewed",
                    path: relative.clone(),
                });
            }
            continue;
        }

        for (reason_code, needle) in CONTENT_RULES {
            if contains_bytes(&bytes, needle) && !policy.content_allowed(reason_code, relative) {
                findings.push(Finding {
                    reason_code,
                    path: relative.clone(),
                });
            }
        }
    }

    findings.sort_by(|left, right| {
        left.reason_code
            .cmp(right.reason_code)
            .then_with(|| left.path.cmp(&right.path))
    });
    findings.dedup();
    Ok(AuditReport { findings })
}

fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn validate_inventory_path(path: &Path) -> Result<(), AuditError> {
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(AuditError::Input {
            reason_code: "input.unsafe_path",
            path: path.to_path_buf(),
        });
    }
    Ok(())
}

fn validate_policy_path(path: &str) -> Result<(), AuditError> {
    validate_inventory_path(Path::new(path)).map_err(|_| {
        policy_error(
            "policy.unsafe_path",
            format!("policy path is not repository-relative: {path}"),
        )
    })
}

fn policy_error(reason_code: &'static str, detail: impl Into<String>) -> AuditError {
    AuditError::Policy {
        reason_code,
        detail: detail.into(),
    }
}

fn normalized_display(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn path_has_prefix(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with('/'))
}
