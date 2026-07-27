use std::{
    env, fmt, fs,
    path::{Path, PathBuf},
    process::Command,
};

use sha2::{Digest, Sha256};

const PACKAGED_VCS_INFO: &str = ".cargo_vcs_info.json";
const SCHEMA_MATERIAL: &str = "schema-fingerprint-v1.material";
const SCHEMA_MATERIAL_HEADER: &str = "alpha-desk-schema-material-v1\n";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildSourceMode {
    Checkout,
    Packaged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildInputs {
    pub mode: BuildSourceMode,
    pub git_sha: String,
    pub dirty: bool,
    pub schema_fingerprint: String,
    pub cargo_lock_sha256: String,
    workspace_root: Option<PathBuf>,
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
    Ok(hex_encode(&Sha256::digest(schema_material_from_tree(
        root,
    )?)))
}

pub fn fingerprint_schema_material(path: &Path) -> Result<String, BuildSupportError> {
    let encoded = fs::read_to_string(path)
        .map_err(|_| BuildSupportError::Io("read packaged schema material".to_owned()))?;
    let payload =
        encoded
            .strip_prefix(SCHEMA_MATERIAL_HEADER)
            .ok_or(BuildSupportError::InvalidMetadata(
                "packaged schema material",
            ))?;
    let material = decode_schema_material(payload)?;
    validate_schema_material(&material)?;
    Ok(hex_encode(&Sha256::digest(material)))
}

fn schema_material_from_tree(root: &Path) -> Result<Vec<u8>, BuildSupportError> {
    let mut files = Vec::new();
    collect_regular_files(root, root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));

    let mut material = Vec::new();
    for (relative, path) in files {
        let relative_bytes = relative.as_bytes();
        let bytes =
            fs::read(&path).map_err(|_| BuildSupportError::Io("read schema file".into()))?;
        append_len_prefixed(&mut material, relative_bytes);
        append_len_prefixed(&mut material, &bytes);
    }
    Ok(material)
}

pub fn sha256_file(path: &Path) -> Result<String, BuildSupportError> {
    let bytes = fs::read(path).map_err(|_| BuildSupportError::Io("read Cargo.lock".to_owned()))?;
    Ok(hex_encode(&Sha256::digest(bytes)))
}

pub fn load_build_inputs(manifest_dir: &Path) -> Result<BuildInputs, BuildSupportError> {
    if manifest_dir.join(PACKAGED_VCS_INFO).is_file() {
        return load_packaged_inputs(manifest_dir);
    }
    load_checkout_inputs(manifest_dir)
}

pub fn emit_build_metadata(manifest_dir: &Path) -> Result<(), BuildSupportError> {
    let profile = match env::var("PROFILE").as_deref() {
        Ok("release") => BuildProfile::Release,
        _ => BuildProfile::Development,
    };
    let source_date_epoch =
        parse_source_date_epoch(env::var("SOURCE_DATE_EPOCH").ok().as_deref(), profile)?;
    let inputs = load_build_inputs(manifest_dir)?;
    let rustc_version = rustc_version::version_meta()
        .map_err(|_| BuildSupportError::CommandFailed("rustc version"))?
        .short_version_string;
    let target_triple =
        env::var("TARGET").map_err(|_| BuildSupportError::InvalidMetadata("target triple"))?;
    validate_directive_value(&rustc_version, "rustc version")?;
    validate_directive_value(&target_triple, "target triple")?;

    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    match &inputs.workspace_root {
        Some(workspace) => {
            emit_source_rerun_inputs(workspace);
            for (_, schema) in sorted_schema_files(&workspace.join("schemas/proto"))? {
                println!("cargo:rerun-if-changed={}", schema.display());
            }
            emit_git_rerun_inputs(workspace)?;
        }
        None => {
            for input in [PACKAGED_VCS_INFO, "Cargo.lock", SCHEMA_MATERIAL] {
                println!(
                    "cargo:rerun-if-changed={}",
                    manifest_dir.join(input).display()
                );
            }
        }
    }

    emit_env("ALPHA_DESK_GIT_SHA", &inputs.git_sha)?;
    emit_env(
        "ALPHA_DESK_GIT_DIRTY",
        if inputs.dirty { "true" } else { "false" },
    )?;
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
    emit_env("ALPHA_DESK_SCHEMA_FINGERPRINT", &inputs.schema_fingerprint)?;
    emit_env("ALPHA_DESK_CARGO_LOCK_SHA256", &inputs.cargo_lock_sha256)?;
    Ok(())
}

fn load_checkout_inputs(manifest_dir: &Path) -> Result<BuildInputs, BuildSupportError> {
    let workspace = find_workspace_root(manifest_dir)?;
    let git_sha = command_stdout(
        Command::new("git")
            .arg("-C")
            .arg(&workspace)
            .args(["rev-parse", "HEAD"]),
        "git rev-parse",
    )?;
    validate_git_sha(&git_sha, "git SHA")?;
    let dirty = source_dirty(&workspace)?;
    let schema_fingerprint = fingerprint_schema_tree(&workspace.join("schemas/proto"))?;
    let packaged_schema_fingerprint =
        fingerprint_schema_material(&workspace.join("crates/telemetry").join(SCHEMA_MATERIAL))?;
    if packaged_schema_fingerprint != schema_fingerprint {
        return Err(BuildSupportError::InvalidMetadata(
            "packaged schema material",
        ));
    }
    let cargo_lock_sha256 = sha256_file(&workspace.join("Cargo.lock"))?;
    Ok(BuildInputs {
        mode: BuildSourceMode::Checkout,
        git_sha,
        dirty,
        schema_fingerprint,
        cargo_lock_sha256,
        workspace_root: Some(workspace),
    })
}

fn load_packaged_inputs(manifest_dir: &Path) -> Result<BuildInputs, BuildSupportError> {
    let vcs_bytes = fs::read(manifest_dir.join(PACKAGED_VCS_INFO))
        .map_err(|_| BuildSupportError::Io("read packaged VCS metadata".to_owned()))?;
    let vcs: serde_json::Value = serde_json::from_slice(&vcs_bytes)
        .map_err(|_| BuildSupportError::InvalidMetadata("packaged VCS metadata"))?;
    let git = vcs
        .get("git")
        .and_then(serde_json::Value::as_object)
        .ok_or(BuildSupportError::InvalidMetadata("packaged VCS metadata"))?;
    let git_sha = git
        .get("sha1")
        .and_then(serde_json::Value::as_str)
        .ok_or(BuildSupportError::InvalidMetadata("packaged VCS metadata"))?;
    let dirty = git
        .get("dirty")
        .and_then(serde_json::Value::as_bool)
        .ok_or(BuildSupportError::InvalidMetadata("packaged VCS metadata"))?;
    let path_in_vcs = vcs
        .get("path_in_vcs")
        .and_then(serde_json::Value::as_str)
        .ok_or(BuildSupportError::InvalidMetadata("packaged VCS metadata"))?;
    validate_git_sha(git_sha, "packaged VCS metadata")?;
    validate_vcs_path(path_in_vcs)?;

    Ok(BuildInputs {
        mode: BuildSourceMode::Packaged,
        git_sha: git_sha.to_owned(),
        dirty,
        schema_fingerprint: fingerprint_schema_material(&manifest_dir.join(SCHEMA_MATERIAL))?,
        cargo_lock_sha256: sha256_file(&manifest_dir.join("Cargo.lock"))?,
        workspace_root: None,
    })
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

fn append_len_prefixed(material: &mut Vec<u8>, bytes: &[u8]) {
    material.extend_from_slice(
        &u64::try_from(bytes.len())
            .map_or(u64::MAX, |length| length)
            .to_be_bytes(),
    );
    material.extend_from_slice(bytes);
}

fn decode_schema_material(encoded: &str) -> Result<Vec<u8>, BuildSupportError> {
    let mut digits = Vec::new();
    for byte in encoded.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if !is_lower_hex(byte) {
            return Err(BuildSupportError::InvalidMetadata(
                "packaged schema material",
            ));
        }
        digits.push(byte);
    }
    if digits.is_empty() || !digits.len().is_multiple_of(2) {
        return Err(BuildSupportError::InvalidMetadata(
            "packaged schema material",
        ));
    }
    let mut material = Vec::with_capacity(digits.len() / 2);
    for pair in digits.chunks_exact(2) {
        let high = decode_hex_digit(pair[0]);
        let low = decode_hex_digit(pair[1]);
        material.push((high << 4) | low);
    }
    Ok(material)
}

fn decode_hex_digit(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => 0,
    }
}

fn validate_schema_material(material: &[u8]) -> Result<(), BuildSupportError> {
    let mut offset = 0;
    let mut previous_path: Option<&str> = None;
    let mut entry_count = 0usize;
    while offset < material.len() {
        let path_length = read_material_length(material, &mut offset)?;
        if path_length == 0 || path_length > material.len().saturating_sub(offset) {
            return Err(BuildSupportError::InvalidMetadata(
                "packaged schema material",
            ));
        }
        let path_bytes = &material[offset..offset + path_length];
        offset += path_length;
        let path = std::str::from_utf8(path_bytes)
            .map_err(|_| BuildSupportError::InvalidMetadata("packaged schema material"))?;
        validate_schema_path(path)?;
        if previous_path.is_some_and(|previous| previous >= path) {
            return Err(BuildSupportError::InvalidMetadata(
                "packaged schema material",
            ));
        }
        previous_path = Some(path);

        let content_length = read_material_length(material, &mut offset)?;
        if content_length > material.len().saturating_sub(offset) {
            return Err(BuildSupportError::InvalidMetadata(
                "packaged schema material",
            ));
        }
        offset += content_length;
        entry_count = entry_count.saturating_add(1);
    }
    if entry_count == 0 {
        return Err(BuildSupportError::InvalidMetadata(
            "packaged schema material",
        ));
    }
    Ok(())
}

fn read_material_length(material: &[u8], offset: &mut usize) -> Result<usize, BuildSupportError> {
    let end = offset
        .checked_add(8)
        .ok_or(BuildSupportError::InvalidMetadata(
            "packaged schema material",
        ))?;
    let bytes: [u8; 8] = material
        .get(*offset..end)
        .ok_or(BuildSupportError::InvalidMetadata(
            "packaged schema material",
        ))?
        .try_into()
        .map_err(|_| BuildSupportError::InvalidMetadata("packaged schema material"))?;
    *offset = end;
    usize::try_from(u64::from_be_bytes(bytes))
        .map_err(|_| BuildSupportError::InvalidMetadata("packaged schema material"))
}

fn validate_schema_path(path: &str) -> Result<(), BuildSupportError> {
    if path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path
            .split('/')
            .any(|segment| matches!(segment, "" | "." | ".."))
    {
        return Err(BuildSupportError::InvalidMetadata(
            "packaged schema material",
        ));
    }
    Ok(())
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

fn validate_git_sha(value: &str, field: &'static str) -> Result<(), BuildSupportError> {
    if value.len() != 40 || !value.bytes().all(is_lower_hex) {
        return Err(BuildSupportError::InvalidMetadata(field));
    }
    Ok(())
}

fn validate_vcs_path(path: &str) -> Result<(), BuildSupportError> {
    if path.starts_with('/')
        || path.contains('\\')
        || path.chars().any(char::is_control)
        || path.split('/').any(|segment| matches!(segment, "." | ".."))
    {
        return Err(BuildSupportError::InvalidMetadata("packaged VCS metadata"));
    }
    Ok(())
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
