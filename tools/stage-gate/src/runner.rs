use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::process::{
    CommandOutcome, CommandSpec, OutputPolicy, ProcessError, ProcessErrorCode, run_command,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DesignExpectation {
    pub tag: String,
    pub tag_object: String,
    pub commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RepositorySnapshot {
    head: String,
    design: DesignExpectation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RepositoryErrorCode {
    GitUnavailable,
    DirtyTree,
    HeadChanged,
    DesignTagUnavailable,
    DesignTagObjectMismatch,
    DesignCommitMismatch,
}

#[derive(Debug, thiserror::Error)]
pub enum RepositoryError {
    #[error("failed to run git: {0}")]
    GitUnavailable(String),
    #[error("repository has tracked or untracked changes")]
    DirtyTree,
    #[error("repository HEAD changed from {expected} to {actual}")]
    HeadChanged { expected: String, actual: String },
    #[error("annotated design tag is unavailable: {0}")]
    DesignTagUnavailable(String),
    #[error("design tag object mismatch: expected {expected}, got {actual}")]
    DesignTagObjectMismatch { expected: String, actual: String },
    #[error("design tag peeled commit mismatch: expected {expected}, got {actual}")]
    DesignCommitMismatch { expected: String, actual: String },
}

impl RepositoryError {
    #[must_use]
    pub const fn code(&self) -> RepositoryErrorCode {
        match self {
            Self::GitUnavailable(_) => RepositoryErrorCode::GitUnavailable,
            Self::DirtyTree => RepositoryErrorCode::DirtyTree,
            Self::HeadChanged { .. } => RepositoryErrorCode::HeadChanged,
            Self::DesignTagUnavailable(_) => RepositoryErrorCode::DesignTagUnavailable,
            Self::DesignTagObjectMismatch { .. } => RepositoryErrorCode::DesignTagObjectMismatch,
            Self::DesignCommitMismatch { .. } => RepositoryErrorCode::DesignCommitMismatch,
        }
    }
}

impl RepositorySnapshot {
    pub fn capture(repository: &Path, design: &DesignExpectation) -> Result<Self, RepositoryError> {
        ensure_clean(repository)?;
        let head = git_output(repository, ["rev-parse", "--verify", "HEAD^{commit}"])?;
        verify_design(repository, design)?;
        Ok(Self {
            head,
            design: design.clone(),
        })
    }

    pub fn verify_unchanged(&self, repository: &Path) -> Result<(), RepositoryError> {
        let actual = git_output(repository, ["rev-parse", "--verify", "HEAD^{commit}"])?;
        if actual != self.head {
            return Err(RepositoryError::HeadChanged {
                expected: self.head.clone(),
                actual,
            });
        }
        ensure_clean(repository)?;
        verify_design(repository, &self.design)
    }

    #[must_use]
    pub fn head(&self) -> &str {
        &self.head
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunnerErrorCode {
    ProcessFailed,
    RepositoryChanged,
    UnsafeWorkingDirectory,
    GateDeadlineExceeded,
    NonZeroExit,
}

#[derive(Debug, thiserror::Error)]
pub enum RunnerError {
    #[error("check process failed: {0}")]
    Process(#[from] ProcessError),
    #[error("repository invariant failed after a check: {0}")]
    Repository(#[from] RepositoryError),
    #[error("check working directory is outside the repository: {0}")]
    UnsafeWorkingDirectory(PathBuf),
    #[error("the whole-gate deadline was exceeded")]
    GateDeadlineExceeded,
    #[error("check {program} exited non-zero: {exit_code:?}")]
    NonZeroExit {
        program: PathBuf,
        exit_code: Option<i32>,
    },
}

impl RunnerError {
    #[must_use]
    pub const fn code(&self) -> RunnerErrorCode {
        match self {
            Self::Process(_) => RunnerErrorCode::ProcessFailed,
            Self::Repository(_) => RunnerErrorCode::RepositoryChanged,
            Self::UnsafeWorkingDirectory(_) => RunnerErrorCode::UnsafeWorkingDirectory,
            Self::GateDeadlineExceeded => RunnerErrorCode::GateDeadlineExceeded,
            Self::NonZeroExit { .. } => RunnerErrorCode::NonZeroExit,
        }
    }

    #[must_use]
    pub fn repository_code(&self) -> Option<RepositoryErrorCode> {
        match self {
            Self::Repository(error) => Some(error.code()),
            Self::Process(_)
            | Self::UnsafeWorkingDirectory(_)
            | Self::GateDeadlineExceeded
            | Self::NonZeroExit { .. } => None,
        }
    }
}

pub fn run_guarded_checks(
    repository: &Path,
    snapshot: &RepositorySnapshot,
    checks: &[CommandSpec],
    output_policy: &OutputPolicy,
    gate_timeout: Duration,
) -> Result<Vec<CommandOutcome>, RunnerError> {
    let canonical_repository = repository
        .canonicalize()
        .map_err(|_| RunnerError::UnsafeWorkingDirectory(repository.to_path_buf()))?;
    let deadline = Instant::now()
        .checked_add(gate_timeout)
        .ok_or(RunnerError::GateDeadlineExceeded)?;
    let mut outcomes = Vec::with_capacity(checks.len());
    for check in checks {
        let now = Instant::now();
        if now >= deadline {
            return Err(RunnerError::GateDeadlineExceeded);
        }
        let canonical_cwd = if check.cwd.is_absolute() {
            check.cwd.canonicalize()
        } else {
            canonical_repository.join(&check.cwd).canonicalize()
        }
        .map_err(|_| RunnerError::UnsafeWorkingDirectory(check.cwd.clone()))?;
        if !canonical_cwd.starts_with(&canonical_repository) {
            return Err(RunnerError::UnsafeWorkingDirectory(canonical_cwd));
        }
        let remaining = deadline.saturating_duration_since(now);
        let gate_limited = remaining <= check.timeout;
        let mut bounded_check = check.clone();
        bounded_check.cwd = canonical_cwd;
        bounded_check.timeout = remaining.min(check.timeout);
        let outcome = run_command(&bounded_check, output_policy);
        snapshot.verify_unchanged(repository)?;
        let outcome = match outcome {
            Err(error) if gate_limited && error.code() == ProcessErrorCode::TimedOut => {
                return Err(RunnerError::GateDeadlineExceeded);
            }
            Err(error) => return Err(RunnerError::Process(error)),
            Ok(outcome) => outcome,
        };
        if !outcome.success {
            return Err(RunnerError::NonZeroExit {
                program: check.program.clone(),
                exit_code: outcome.exit_code,
            });
        }
        outcomes.push(outcome);
    }
    Ok(outcomes)
}

fn ensure_clean(repository: &Path) -> Result<(), RepositoryError> {
    let status = git_bytes(
        repository,
        [
            "status",
            "--porcelain=v1",
            "--untracked-files=all",
            "--ignore-submodules=none",
            "-z",
        ],
    )?;
    if status.is_empty() {
        Ok(())
    } else {
        Err(RepositoryError::DirtyTree)
    }
}

fn verify_design(repository: &Path, design: &DesignExpectation) -> Result<(), RepositoryError> {
    let tag_expression = format!("{}^{{tag}}", design.tag);
    let actual_tag = git_output(repository, ["rev-parse", tag_expression.as_str()])
        .map_err(|error| RepositoryError::DesignTagUnavailable(error.to_string()))?;
    if actual_tag != design.tag_object {
        return Err(RepositoryError::DesignTagObjectMismatch {
            expected: design.tag_object.clone(),
            actual: actual_tag,
        });
    }

    let commit_expression = format!("{}^{{commit}}", design.tag);
    let actual_commit = git_output(repository, ["rev-parse", commit_expression.as_str()])
        .map_err(|error| RepositoryError::DesignTagUnavailable(error.to_string()))?;
    if actual_commit != design.commit {
        return Err(RepositoryError::DesignCommitMismatch {
            expected: design.commit.clone(),
            actual: actual_commit,
        });
    }
    Ok(())
}

fn git_output<const N: usize>(
    repository: &Path,
    args: [&str; N],
) -> Result<String, RepositoryError> {
    let bytes = git_bytes(repository, args)?;
    String::from_utf8(bytes)
        .map(|output| output.trim().to_owned())
        .map_err(|error| RepositoryError::GitUnavailable(error.to_string()))
}

fn git_bytes<const N: usize>(
    repository: &Path,
    args: [&str; N],
) -> Result<Vec<u8>, RepositoryError> {
    let output = Command::new(git_program())
        .args(["-C"])
        .arg(repository)
        .args(args)
        .env_clear()
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| RepositoryError::GitUnavailable(error.to_string()))?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(RepositoryError::GitUnavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ))
    }
}

fn git_program() -> PathBuf {
    PathBuf::from("/usr/bin/git")
}
