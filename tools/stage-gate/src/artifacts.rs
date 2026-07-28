use std::{
    collections::BTreeSet,
    fs::{self, File, Metadata},
    io::{self, Read, Seek as _},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactRequest {
    pub logical_name: String,
    pub relative_path: PathBuf,
    pub kind: String,
    pub producer: String,
    pub target_triple: String,
    pub profile: String,
    pub expected_sha256: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactRecord {
    pub logical_name: String,
    pub relative_path: String,
    pub kind: String,
    pub size_bytes: u64,
    pub sha256: String,
    pub producer: String,
    pub target_triple: String,
    pub profile: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactManifest {
    pub schema_version: u32,
    pub artifacts: Vec<ArtifactRecord>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationPhase {
    AfterHash,
}

impl ObservationPhase {
    #[must_use]
    pub const fn is_after_hash(self) -> bool {
        matches!(self, Self::AfterHash)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactErrorCode {
    InvalidRoot,
    OutsideRoot,
    Duplicate,
    Missing,
    Symlink,
    NonRegular,
    ReadFailed,
    HashMismatch,
    ChangedDuringRead,
}

#[derive(Debug, thiserror::Error)]
pub enum ArtifactError {
    #[error("artifact root is invalid: {0}")]
    InvalidRoot(String),
    #[error("artifact path is outside the declared root: {0}")]
    OutsideRoot(PathBuf),
    #[error("artifact logical name or path is duplicated: {0}")]
    Duplicate(String),
    #[error("artifact is missing: {0}")]
    Missing(PathBuf),
    #[error("artifact is a symbolic link: {0}")]
    Symlink(PathBuf),
    #[error("artifact is not a regular file: {0}")]
    NonRegular(PathBuf),
    #[error("artifact could not be read: {path}: {source}")]
    ReadFailed {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("artifact hash differs from the configured hash: {0}")]
    HashMismatch(String),
    #[error("artifact changed while it was being hashed: {0}")]
    ChangedDuringRead(PathBuf),
}

impl ArtifactError {
    #[must_use]
    pub const fn code(&self) -> ArtifactErrorCode {
        match self {
            Self::InvalidRoot(_) => ArtifactErrorCode::InvalidRoot,
            Self::OutsideRoot(_) => ArtifactErrorCode::OutsideRoot,
            Self::Duplicate(_) => ArtifactErrorCode::Duplicate,
            Self::Missing(_) => ArtifactErrorCode::Missing,
            Self::Symlink(_) => ArtifactErrorCode::Symlink,
            Self::NonRegular(_) => ArtifactErrorCode::NonRegular,
            Self::ReadFailed { .. } => ArtifactErrorCode::ReadFailed,
            Self::HashMismatch(_) => ArtifactErrorCode::HashMismatch,
            Self::ChangedDuringRead(_) => ArtifactErrorCode::ChangedDuringRead,
        }
    }
}

pub fn collect_artifacts(
    root: &Path,
    requests: &[ArtifactRequest],
) -> Result<ArtifactManifest, ArtifactError> {
    collect_artifacts_observed(root, requests, |_, _| {})
}

#[doc(hidden)]
pub fn collect_artifacts_observed<F>(
    root: &Path,
    requests: &[ArtifactRequest],
    mut observer: F,
) -> Result<ArtifactManifest, ArtifactError>
where
    F: FnMut(&str, ObservationPhase),
{
    let canonical_root = root
        .canonicalize()
        .map_err(|error| ArtifactError::InvalidRoot(error.to_string()))?;
    let mut logical_names = BTreeSet::new();
    let mut relative_paths = BTreeSet::new();
    for request in requests {
        if !logical_names.insert(request.logical_name.clone()) {
            return Err(ArtifactError::Duplicate(request.logical_name.clone()));
        }
        if !is_safe_relative(&request.relative_path) {
            return Err(ArtifactError::OutsideRoot(request.relative_path.clone()));
        }
        if !relative_paths.insert(request.relative_path.clone()) {
            return Err(ArtifactError::Duplicate(
                request.relative_path.display().to_string(),
            ));
        }
    }

    let mut artifacts = Vec::with_capacity(requests.len());
    for request in requests {
        artifacts.push(collect_one(&canonical_root, request, &mut observer)?);
    }
    artifacts.sort_by(|left, right| left.logical_name.cmp(&right.logical_name));
    Ok(ArtifactManifest {
        schema_version: 1,
        artifacts,
    })
}

fn collect_one<F>(
    root: &Path,
    request: &ArtifactRequest,
    observer: &mut F,
) -> Result<ArtifactRecord, ArtifactError>
where
    F: FnMut(&str, ObservationPhase),
{
    let path = root.join(&request.relative_path);
    let before_path = symlink_metadata(&path)?;
    if before_path.file_type().is_symlink() {
        return Err(ArtifactError::Symlink(path));
    }
    if !before_path.is_file() {
        return Err(ArtifactError::NonRegular(path));
    }
    let canonical_path = path
        .canonicalize()
        .map_err(|source| ArtifactError::ReadFailed {
            path: path.clone(),
            source,
        })?;
    if !canonical_path.starts_with(root) {
        return Err(ArtifactError::OutsideRoot(request.relative_path.clone()));
    }

    let mut file = File::open(&path).map_err(|source| ArtifactError::ReadFailed {
        path: path.clone(),
        source,
    })?;
    let before_file = file
        .metadata()
        .map_err(|source| ArtifactError::ReadFailed {
            path: path.clone(),
            source,
        })?;
    if !same_file(&before_path, &before_file) {
        return Err(ArtifactError::ChangedDuringRead(path));
    }

    let (sha256, size_bytes) = hash_reader(&mut file, &path)?;
    observer(&request.logical_name, ObservationPhase::AfterHash);
    file.rewind().map_err(|source| ArtifactError::ReadFailed {
        path: path.clone(),
        source,
    })?;
    let (confirmed_sha256, confirmed_size) = hash_reader(&mut file, &path)?;

    let after_file = file
        .metadata()
        .map_err(|source| ArtifactError::ReadFailed {
            path: path.clone(),
            source,
        })?;
    let after_path = symlink_metadata(&path)?;
    if after_path.file_type().is_symlink()
        || !after_path.is_file()
        || !same_file(&before_file, &after_file)
        || !same_file(&after_file, &after_path)
        || metadata_changed(&before_file, &after_file)
        || confirmed_sha256 != sha256
        || confirmed_size != size_bytes
    {
        return Err(ArtifactError::ChangedDuringRead(path));
    }
    if request
        .expected_sha256
        .as_ref()
        .is_some_and(|expected| expected != &sha256)
    {
        return Err(ArtifactError::HashMismatch(request.logical_name.clone()));
    }

    Ok(ArtifactRecord {
        logical_name: request.logical_name.clone(),
        relative_path: path_string(&request.relative_path)?,
        kind: request.kind.clone(),
        size_bytes,
        sha256,
        producer: request.producer.clone(),
        target_triple: request.target_triple.clone(),
        profile: request.profile.clone(),
    })
}

fn symlink_metadata(path: &Path) -> Result<Metadata, ArtifactError> {
    fs::symlink_metadata(path).map_err(|source| {
        if source.kind() == io::ErrorKind::NotFound {
            ArtifactError::Missing(path.to_path_buf())
        } else {
            ArtifactError::ReadFailed {
                path: path.to_path_buf(),
                source,
            }
        }
    })
}

fn hash_reader(reader: &mut impl Read, path: &Path) -> Result<(String, u64), ArtifactError> {
    let mut hasher = Sha256::new();
    let mut size_bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader
            .read(&mut buffer)
            .map_err(|source| ArtifactError::ReadFailed {
                path: path.to_path_buf(),
                source,
            })?;
        if count == 0 {
            break;
        }
        size_bytes = size_bytes.saturating_add(count as u64);
        hasher.update(&buffer[..count]);
    }
    Ok((hex::encode(hasher.finalize()), size_bytes))
}

fn metadata_changed(before: &Metadata, after: &Metadata) -> bool {
    before.len() != after.len()
        || modified(before) != modified(after)
        || created(before) != created(after)
}

fn modified(metadata: &Metadata) -> Option<SystemTime> {
    metadata.modified().ok()
}

fn created(metadata: &Metadata) -> Option<SystemTime> {
    metadata.created().ok()
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len() && modified(left) == modified(right)
}

fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

fn path_string(path: &Path) -> Result<String, ArtifactError> {
    path.to_str()
        .map(ToOwned::to_owned)
        .ok_or_else(|| ArtifactError::OutsideRoot(path.to_path_buf()))
}
