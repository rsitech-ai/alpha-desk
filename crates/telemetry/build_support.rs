use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

const SOURCE_INPUTS: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    "rust-toolchain.toml",
    "justfile",
    "crates",
    "services",
    "tools",
    "schemas",
    "apps/AlphaDesk/Package.swift",
    "apps/AlphaDesk/Sources",
    "apps/AlphaDesk/Tests",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildProfile {
    Development,
    Release,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildSupportError {
    InvalidSourceDateEpoch,
    ReleaseEpochRequired,
    WorkspaceRootNotFound,
    Io(String),
    CommandFailed(&'static str),
    InvalidMetadata(&'static str),
}

impl fmt::Display for BuildSupportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSourceDateEpoch => {
                formatter.write_str("SOURCE_DATE_EPOCH must be an unsigned 64-bit integer")
            }
            Self::ReleaseEpochRequired => {
                formatter.write_str("release builds require SOURCE_DATE_EPOCH")
            }
            Self::WorkspaceRootNotFound => formatter.write_str("workspace root was not found"),
            Self::Io(operation) => write!(formatter, "I/O operation failed: {operation}"),
            Self::CommandFailed(command) => write!(formatter, "{command} command failed"),
            Self::InvalidMetadata(field) => write!(formatter, "invalid build metadata: {field}"),
        }
    }
}

pub fn parse_source_date_epoch(
    value: Option<&str>,
    profile: BuildProfile,
) -> Result<Option<u64>, BuildSupportError> {
    match value {
        Some(raw) if !raw.is_empty() && raw.as_bytes().iter().all(u8::is_ascii_digit) => raw
            .parse::<u64>()
            .map(Some)
            .map_err(|_| BuildSupportError::InvalidSourceDateEpoch),
        Some(_) => Err(BuildSupportError::InvalidSourceDateEpoch),
        None if profile == BuildProfile::Release => Err(BuildSupportError::ReleaseEpochRequired),
        None => Ok(None),
    }
}

pub fn fingerprint_schema_tree(root: &Path) -> Result<String, BuildSupportError> {
    let mut files = Vec::new();
    collect_regular_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut hasher = Sha256::new();
    for (relative, path) in files {
        let relative_bytes = relative.as_bytes();
        let bytes =
            fs::read(&path).map_err(|_| BuildSupportError::Io("read schema file".into()))?;
        hash_len_prefixed(&mut hasher, relative_bytes);
        hash_len_prefixed(&mut hasher, &bytes);
    }
    Ok(hex_encode(&hasher.finalize()))
}

pub fn sha256_file(path: &Path) -> Result<String, BuildSupportError> {
    let bytes = fs::read(path).map_err(|_| BuildSupportError::Io("read Cargo.lock".to_owned()))?;
    Ok(hex_encode(&Sha256::digest(bytes)))
}

pub fn emit_build_metadata(manifest_dir: &Path) -> Result<(), BuildSupportError> {
    let workspace = find_workspace_root(manifest_dir)?;
    let profile = match env::var("PROFILE").as_deref() {
        Ok("release") => BuildProfile::Release,
        _ => BuildProfile::Development,
    };
    let source_date_epoch =
        parse_source_date_epoch(env::var("SOURCE_DATE_EPOCH").ok().as_deref(), profile)?;
    let git_sha = command_stdout(
        Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["rev-parse", "HEAD"]),
        "git rev-parse",
    )?;
    if git_sha.len() != 40 || !git_sha.bytes().all(is_lower_hex) {
        return Err(BuildSupportError::InvalidMetadata("git SHA"));
    }
    let dirty = source_dirty(&workspace)?;
    let rustc_version = rustc_version::version_meta()
        .map_err(|_| BuildSupportError::CommandFailed("rustc version"))?
        .short_version_string;
    let target_triple =
        env::var("TARGET").map_err(|_| BuildSupportError::InvalidMetadata("target triple"))?;
    validate_directive_value(&rustc_version, "rustc version")?;
    validate_directive_value(&target_triple, "target triple")?;

    let schema_root = workspace.join("schemas/proto");
    let schema_fingerprint = fingerprint_schema_tree(&schema_root)?;
    let cargo_lock = workspace.join("Cargo.lock");
    let cargo_lock_sha256 = sha256_file(&cargo_lock)?;

    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    emit_source_rerun_inputs(&workspace);
    for (_, schema) in sorted_schema_files(&schema_root)? {
        println!("cargo:rerun-if-changed={}", schema.display());
    }
    emit_git_rerun_inputs(&workspace)?;

    emit_env("ALPHA_DESK_GIT_SHA", &git_sha)?;
    emit_env("ALPHA_DESK_GIT_DIRTY", if dirty { "true" } else { "false" })?;
    emit_env("ALPHA_DESK_RUSTC_VERSION", &rustc_version)?;
    emit_env("ALPHA_DESK_TARGET_TRIPLE", &target_triple)?;
    emit_env(
        "ALPHA_DESK_BUILD_EPOCH",
        &source_date_epoch.map_or_else(String::new, |epoch| epoch.to_string()),
    )?;
    emit_env(
        "ALPHA_DESK_REPRODUCIBLE",
        if source_date_epoch.is_some() {
            "true"
        } else {
            "false"
        },
    )?;
    emit_env("ALPHA_DESK_SCHEMA_FINGERPRINT", &schema_fingerprint)?;
    emit_env("ALPHA_DESK_CARGO_LOCK_SHA256", &cargo_lock_sha256)?;
    Ok(())
}

pub fn source_dirty(workspace: &Path) -> Result<bool, BuildSupportError> {
    let mut command = Command::new("git");
    command.arg("-C").arg(workspace).args([
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--",
    ]);
    command.args(SOURCE_INPUTS);
    command_has_output(&mut command, "git status")
}

fn emit_source_rerun_inputs(workspace: &Path) {
    for source in SOURCE_INPUTS {
        println!(
            "cargo:rerun-if-changed={}",
            workspace.join(source).display()
        );
    }
}

fn find_workspace_root(start: &Path) -> Result<PathBuf, BuildSupportError> {
    for candidate in start.ancestors() {
        if candidate.join("Cargo.lock").is_file()
            && candidate.join("schemas/proto").is_dir()
            && candidate.join("Cargo.toml").is_file()
        {
            return Ok(candidate.to_owned());
        }
    }
    Err(BuildSupportError::WorkspaceRootNotFound)
}

fn collect_regular_files(
    root: &Path,
    directory: &Path,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<(), BuildSupportError> {
    let entries = fs::read_dir(directory)
        .map_err(|_| BuildSupportError::Io("read schema tree".to_owned()))?;
    for entry in entries {
        let entry = entry.map_err(|_| BuildSupportError::Io("read schema entry".to_owned()))?;
        let file_type = entry
            .file_type()
            .map_err(|_| BuildSupportError::Io("read schema file type".to_owned()))?;
        if file_type.is_dir() {
            collect_regular_files(root, &entry.path(), files)?;
        } else if file_type.is_file() {
            let relative = entry
                .path()
                .strip_prefix(root)
                .map_err(|_| BuildSupportError::InvalidMetadata("schema path"))?
                .to_str()
                .ok_or(BuildSupportError::InvalidMetadata("schema path encoding"))?
                .replace(std::path::MAIN_SEPARATOR, "/");
            validate_directive_value(&relative, "schema path")?;
            files.push((relative, entry.path()));
        } else {
            return Err(BuildSupportError::InvalidMetadata(
                "schema tree contains a symlink or special file",
            ));
        }
    }
    Ok(())
}

fn sorted_schema_files(root: &Path) -> Result<Vec<(String, PathBuf)>, BuildSupportError> {
    let mut files = Vec::new();
    collect_regular_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn hash_len_prefixed(hasher: &mut Sha256, bytes: &[u8]) {
    hasher.update(
        u64::try_from(bytes.len())
            .map_or(u64::MAX, |length| length)
            .to_be_bytes(),
    );
    hasher.update(bytes);
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn command_stdout(command: &mut Command, name: &'static str) -> Result<String, BuildSupportError> {
    let output = command
        .output()
        .map_err(|_| BuildSupportError::CommandFailed(name))?;
    if !output.status.success() {
        return Err(BuildSupportError::CommandFailed(name));
    }
    let stdout =
        String::from_utf8(output.stdout).map_err(|_| BuildSupportError::InvalidMetadata(name))?;
    let value = stdout.trim_end_matches(['\r', '\n']).to_owned();
    validate_directive_value(&value, name)?;
    Ok(value)
}

fn command_has_output(
    command: &mut Command,
    name: &'static str,
) -> Result<bool, BuildSupportError> {
    let output = command
        .output()
        .map_err(|_| BuildSupportError::CommandFailed(name))?;
    if !output.status.success() {
        return Err(BuildSupportError::CommandFailed(name));
    }
    Ok(!output.stdout.is_empty())
}

fn emit_git_rerun_inputs(workspace: &Path) -> Result<(), BuildSupportError> {
    for git_path in ["HEAD", "index", "packed-refs"] {
        let path = command_stdout(
            Command::new("git").arg("-C").arg(workspace).args([
                "rev-parse",
                "--git-path",
                git_path,
            ]),
            "git rev-parse --git-path",
        )?;
        let path = absolute_git_path(workspace, &path);
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let head_path = absolute_git_path(
        workspace,
        &command_stdout(
            Command::new("git")
                .arg("-C")
                .arg(workspace)
                .args(["rev-parse", "--git-path", "HEAD"]),
            "git rev-parse --git-path",
        )?,
    );
    let head = fs::read_to_string(head_path)
        .map_err(|_| BuildSupportError::Io("read Git HEAD".to_owned()))?;
    if let Some(reference) = head.trim().strip_prefix("ref: ") {
        validate_directive_value(reference, "Git reference")?;
        let path = command_stdout(
            Command::new("git").arg("-C").arg(workspace).args([
                "rev-parse",
                "--git-path",
                reference,
            ]),
            "git rev-parse --git-path",
        )?;
        println!(
            "cargo:rerun-if-changed={}",
            absolute_git_path(workspace, &path).display()
        );
    }
    Ok(())
}

fn absolute_git_path(workspace: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_owned()
    } else {
        workspace.join(path)
    }
}

fn validate_directive_value(value: &str, field: &'static str) -> Result<(), BuildSupportError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(BuildSupportError::InvalidMetadata(field));
    }
    Ok(())
}

fn emit_env(key: &'static str, value: &str) -> Result<(), BuildSupportError> {
    if value.chars().any(char::is_control) {
        return Err(BuildSupportError::InvalidMetadata(key));
    }
    println!("cargo:rustc-env={key}={value}");
    Ok(())
}
