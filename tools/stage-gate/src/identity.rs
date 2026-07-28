use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

const VERSION_OUTPUT_LIMIT: usize = 64 * 1024;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutableIdentity {
    pub id: String,
    pub resolved_path: PathBuf,
    pub sha256: String,
    pub version_output: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResolvedProgram {
    pub invocation_path: PathBuf,
    pub executable_path: PathBuf,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityErrorCode {
    ProgramUnavailable,
    OutsideApprovedRoot,
    NotExecutable,
    IdentityCommandFailed,
    TargetTripleUnavailable,
}

#[derive(Debug, thiserror::Error)]
pub enum IdentityError {
    #[error("program is unavailable in approved roots: {0}")]
    ProgramUnavailable(String),
    #[error("program resolves outside its approved root: {0}")]
    OutsideApprovedRoot(PathBuf),
    #[error("program is not an executable regular file: {0}")]
    NotExecutable(PathBuf),
    #[error("identity command failed for {0}")]
    IdentityCommandFailed(PathBuf),
    #[error("rustc host target is unavailable")]
    TargetTripleUnavailable,
}

impl IdentityError {
    #[must_use]
    pub const fn code(&self) -> IdentityErrorCode {
        match self {
            Self::ProgramUnavailable(_) => IdentityErrorCode::ProgramUnavailable,
            Self::OutsideApprovedRoot(_) => IdentityErrorCode::OutsideApprovedRoot,
            Self::NotExecutable(_) => IdentityErrorCode::NotExecutable,
            Self::IdentityCommandFailed(_) => IdentityErrorCode::IdentityCommandFailed,
            Self::TargetTripleUnavailable => IdentityErrorCode::TargetTripleUnavailable,
        }
    }
}

pub fn expand_program_roots(configured: &[String]) -> Vec<PathBuf> {
    configured
        .iter()
        .filter_map(|root| match root.as_str() {
            "$CARGO_HOME/bin" => std::env::var_os("CARGO_HOME")
                .map(PathBuf::from)
                .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo")))
                .map(|root| root.join("bin")),
            "$HOME/.cargo/bin" => {
                std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo/bin"))
            }
            _ => Some(PathBuf::from(root)),
        })
        .collect()
}

pub fn resolve_program(
    configured: &str,
    approved_roots: &[PathBuf],
    repository: &Path,
) -> Result<ResolvedProgram, IdentityError> {
    let requested = Path::new(configured);
    if requested.components().count() > 1 && !requested.is_absolute() {
        let candidate = repository.join(requested);
        return validate_repository_program(&candidate, repository);
    }
    if requested.is_absolute() {
        return validate_root_program(requested, approved_roots);
    }
    for root in approved_roots {
        let candidate = root.join(requested);
        if candidate.exists() {
            return validate_root_program(&candidate, approved_roots);
        }
    }
    Err(IdentityError::ProgramUnavailable(configured.to_owned()))
}

pub fn capture_executable_identity(
    id: &str,
    program: &ResolvedProgram,
    args: &[String],
) -> Result<ExecutableIdentity, IdentityError> {
    capture_executable_identity_with_env(id, program, args, &BTreeMap::new())
}

pub fn executable_file_identity(
    id: &str,
    program: &ResolvedProgram,
) -> Result<ExecutableIdentity, IdentityError> {
    let bytes = fs::read(&program.executable_path)
        .map_err(|_| IdentityError::ProgramUnavailable(id.to_owned()))?;
    Ok(ExecutableIdentity {
        id: id.to_owned(),
        resolved_path: program.executable_path.clone(),
        sha256: hex::encode(Sha256::digest(bytes)),
        version_output: String::new(),
    })
}

pub fn capture_executable_identity_with_env(
    id: &str,
    program: &ResolvedProgram,
    args: &[String],
    environment: &BTreeMap<String, String>,
) -> Result<ExecutableIdentity, IdentityError> {
    let bytes = fs::read(&program.executable_path)
        .map_err(|_| IdentityError::ProgramUnavailable(id.to_owned()))?;
    let output = Command::new(&program.invocation_path)
        .args(args)
        .env_clear()
        .envs(environment)
        .stdin(Stdio::null())
        .output()
        .map_err(|_| IdentityError::IdentityCommandFailed(program.invocation_path.clone()))?;
    if !output.status.success() {
        return Err(IdentityError::IdentityCommandFailed(
            program.invocation_path.clone(),
        ));
    }
    let mut version = output.stdout;
    version.extend_from_slice(&output.stderr);
    version.truncate(VERSION_OUTPUT_LIMIT);
    Ok(ExecutableIdentity {
        id: id.to_owned(),
        resolved_path: program.executable_path.clone(),
        sha256: hex::encode(Sha256::digest(bytes)),
        version_output: String::from_utf8_lossy(&version).trim().to_owned(),
    })
}

pub fn parse_rustc_host(output: &str) -> Result<String, IdentityError> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .filter(|host| !host.is_empty() && !host.chars().any(char::is_whitespace))
        .map(ToOwned::to_owned)
        .ok_or(IdentityError::TargetTripleUnavailable)
}

#[must_use]
pub fn version_output_matches(output: &str, expected_contains: Option<&str>) -> bool {
    expected_contains.is_none_or(|expected| output.contains(expected))
}

fn validate_repository_program(
    candidate: &Path,
    repository: &Path,
) -> Result<ResolvedProgram, IdentityError> {
    let canonical_repository = repository
        .canonicalize()
        .map_err(|_| IdentityError::ProgramUnavailable(candidate.display().to_string()))?;
    let canonical = candidate
        .canonicalize()
        .map_err(|_| IdentityError::ProgramUnavailable(candidate.display().to_string()))?;
    if !canonical.starts_with(canonical_repository) {
        return Err(IdentityError::OutsideApprovedRoot(canonical));
    }
    require_executable(candidate, canonical)
}

fn validate_root_program(
    candidate: &Path,
    approved_roots: &[PathBuf],
) -> Result<ResolvedProgram, IdentityError> {
    let canonical = candidate
        .canonicalize()
        .map_err(|_| IdentityError::ProgramUnavailable(candidate.display().to_string()))?;
    let inside = approved_roots.iter().any(|root| {
        root.canonicalize()
            .is_ok_and(|canonical_root| canonical.starts_with(canonical_root))
    });
    if !inside {
        return Err(IdentityError::OutsideApprovedRoot(canonical));
    }
    require_executable(candidate, canonical)
}

fn require_executable(
    invocation_path: &Path,
    executable_path: PathBuf,
) -> Result<ResolvedProgram, IdentityError> {
    let metadata = fs::metadata(&executable_path)
        .map_err(|_| IdentityError::ProgramUnavailable(executable_path.display().to_string()))?;
    #[cfg(unix)]
    let executable = metadata.is_file() && metadata.permissions().mode() & 0o111 != 0;
    #[cfg(not(unix))]
    let executable = false;
    if executable {
        Ok(ResolvedProgram {
            invocation_path: invocation_path.to_path_buf(),
            executable_path,
        })
    } else {
        Err(IdentityError::NotExecutable(executable_path))
    }
}
