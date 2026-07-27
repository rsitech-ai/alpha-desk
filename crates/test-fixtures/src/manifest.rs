use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashSet};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

const MANIFEST_VERSION: u32 = 1;
const SHA256_HEX_LENGTH: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureManifest {
    pub version: u32,
    pub fixture: Vec<FixtureEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FixtureEntry {
    pub id: String,
    pub source_path: String,
    pub source_sha256: String,
    pub source_schema: String,
    pub expected_path: String,
    pub expected_sha256: String,
    pub expected_schema: String,
}

#[derive(Debug, thiserror::Error)]
pub enum FixtureError {
    #[error("read fixture manifest {path}: {source}")]
    ReadManifest {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("parse fixture manifest {path}: {source}")]
    ParseManifest {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("fixture manifest is not valid UTF-8: {0}")]
    InvalidManifestUtf8(PathBuf),
    #[error("serialize fixture manifest: {0}")]
    SerializeManifest(#[from] toml::ser::Error),
    #[error("unsupported fixture manifest version {0}")]
    UnsupportedVersion(u32),
    #[error("duplicate fixture id {0}")]
    DuplicateId(String),
    #[error("duplicate fixture path {0}")]
    DuplicatePath(String),
    #[error("unsafe fixture path {0}")]
    UnsafePath(String),
    #[error("fixture path is outside blocks/ or expected/: {0}")]
    PathOutsideFixtureTrees(String),
    #[error("missing fixture file {path}: {source}")]
    MissingFile {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("fixture path contains a symlink: {0}")]
    Symlink(PathBuf),
    #[error("fixture path is not a regular file: {0}")]
    NotRegularFile(PathBuf),
    #[error("invalid lowercase SHA-256 digest for {field}: {digest}")]
    InvalidDigest { field: &'static str, digest: String },
    #[error("fixture digest mismatch for {path}: expected {expected}, found {actual}")]
    DigestMismatch {
        path: String,
        expected: String,
        actual: String,
    },
    #[error("undeclared fixture file {0}")]
    UndeclaredFile(String),
    #[error("fixture file changed while it was being verified: {0}")]
    FileChanged(PathBuf),
    #[error("fixture filesystem operation failed for {path}: {source}")]
    Filesystem {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("fixture path is not valid UTF-8: {0:?}")]
    NonUtf8Path(PathBuf),
    #[error("fixture source has no deterministic expected pair: {0}")]
    MissingExpectedPair(String),
    #[error("expected fixture has no deterministic source pair: {0}")]
    MissingSourcePair(String),
    #[error("unsupported fixture filename {0}")]
    UnsupportedFilename(String),
    #[error("parse fixture JSON {path}: {source}")]
    ParseJson {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
    #[error("fixture JSON {path} is missing non-empty string field {field}")]
    MissingSchemaField { path: PathBuf, field: &'static str },
    #[error("write fixture manifest {path}: {source}")]
    WriteManifest {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl FixtureManifest {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            version: MANIFEST_VERSION,
            fixture: Vec::new(),
        }
    }

    pub fn load(path: impl AsRef<Path>) -> Result<Self, FixtureError> {
        let path = path.as_ref();
        let before = fs::symlink_metadata(path).map_err(|source| FixtureError::ReadManifest {
            path: path.to_path_buf(),
            source,
        })?;
        reject_symlink_or_non_regular(path, &before)?;
        let bytes = read_stable_file(path, &before)?;
        let text = std::str::from_utf8(&bytes)
            .map_err(|_| FixtureError::InvalidManifestUtf8(path.to_path_buf()))?;
        toml::from_str(text).map_err(|source| FixtureError::ParseManifest {
            path: path.to_path_buf(),
            source,
        })
    }

    pub fn verify(&self, root: impl AsRef<Path>) -> Result<(), FixtureError> {
        let root = root.as_ref();
        validate_root(root)?;
        if self.version != MANIFEST_VERSION {
            return Err(FixtureError::UnsupportedVersion(self.version));
        }

        let mut ids = HashSet::new();
        let mut declared_paths = HashSet::new();
        for entry in &self.fixture {
            if !ids.insert(entry.id.as_str()) {
                return Err(FixtureError::DuplicateId(entry.id.clone()));
            }
            validate_digest("source_sha256", &entry.source_sha256)?;
            validate_digest("expected_sha256", &entry.expected_sha256)?;
            verify_declared_file(
                root,
                &entry.source_path,
                "blocks",
                &entry.source_sha256,
                &mut declared_paths,
            )?;
            verify_declared_file(
                root,
                &entry.expected_path,
                "expected",
                &entry.expected_sha256,
                &mut declared_paths,
            )?;
        }

        for directory in ["blocks", "expected"] {
            for relative in scan_fixture_tree(root, directory)? {
                if !declared_paths.contains(relative.as_str()) {
                    return Err(FixtureError::UndeclaredFile(relative));
                }
            }
        }
        Ok(())
    }

    pub fn generate(root: impl AsRef<Path>) -> Result<Self, FixtureError> {
        let root = root.as_ref();
        validate_root(root)?;
        let sources = scan_fixture_tree(root, "blocks")?;
        let expected_files: BTreeSet<String> =
            scan_fixture_tree(root, "expected")?.into_iter().collect();
        let mut paired_expected = BTreeSet::new();
        let mut entries = Vec::with_capacity(sources.len());

        for source_path in sources {
            let relative_source = source_path
                .strip_prefix("blocks/")
                .ok_or_else(|| FixtureError::PathOutsideFixtureTrees(source_path.clone()))?;
            let id = relative_source
                .strip_suffix(".json")
                .ok_or_else(|| FixtureError::UnsupportedFilename(source_path.clone()))?;
            if id.is_empty() || id.ends_with(".canonical") {
                return Err(FixtureError::UnsupportedFilename(source_path));
            }
            let expected_path = format!("expected/{id}.canonical.json");
            if !expected_files.contains(&expected_path) {
                return Err(FixtureError::MissingExpectedPair(source_path));
            }
            paired_expected.insert(expected_path.clone());

            let source_bytes = read_verified_file(root, &source_path, "blocks")?;
            let expected_bytes = read_verified_file(root, &expected_path, "expected")?;
            entries.push(FixtureEntry {
                id: id.to_owned(),
                source_path: source_path.clone(),
                source_sha256: sha256_hex(&source_bytes),
                source_schema: json_string_field(root.join(&source_path), &source_bytes, "schema")?,
                expected_path: expected_path.clone(),
                expected_sha256: sha256_hex(&expected_bytes),
                expected_schema: json_string_field(
                    root.join(&expected_path),
                    &expected_bytes,
                    "schema_version",
                )?,
            });
        }

        if let Some(unpaired) = expected_files.difference(&paired_expected).next() {
            return Err(FixtureError::MissingSourcePair(unpaired.clone()));
        }
        entries.sort_by(|left, right| {
            left.source_path
                .as_bytes()
                .cmp(right.source_path.as_bytes())
        });
        Ok(Self {
            version: MANIFEST_VERSION,
            fixture: entries,
        })
    }

    pub fn write_atomic(&self, root: impl AsRef<Path>) -> Result<String, FixtureError> {
        let root = root.as_ref();
        validate_root(root)?;
        let mut rendered = toml::to_string(self)?;
        if !rendered.ends_with('\n') {
            rendered.push('\n');
        }
        let bytes = rendered.as_bytes();
        let target = root.join("manifest.toml");
        let (temporary_path, mut temporary) = create_temporary_manifest(root)?;
        let mut cleanup = TemporaryCleanup::new(temporary_path.clone());

        temporary
            .write_all(bytes)
            .and_then(|()| temporary.sync_all())
            .map_err(|source| FixtureError::WriteManifest {
                path: temporary_path.clone(),
                source,
            })?;
        drop(temporary);
        fs::rename(&temporary_path, &target).map_err(|source| FixtureError::WriteManifest {
            path: target.clone(),
            source,
        })?;
        cleanup.disarm();
        sync_directory_if_supported(root)?;
        Ok(sha256_hex(bytes))
    }

    #[must_use]
    pub fn fixture_ids_sorted(&self) -> Vec<&str> {
        let mut ids: Vec<&str> = self.fixture.iter().map(|entry| entry.id.as_str()).collect();
        ids.sort_unstable_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
        ids
    }
}

fn validate_root(root: &Path) -> Result<(), FixtureError> {
    let metadata = fs::symlink_metadata(root).map_err(|source| FixtureError::Filesystem {
        path: root.to_path_buf(),
        source,
    })?;
    if metadata.file_type().is_symlink() {
        return Err(FixtureError::Symlink(root.to_path_buf()));
    }
    if !metadata.is_dir() {
        return Err(FixtureError::NotRegularFile(root.to_path_buf()));
    }
    Ok(())
}

fn verify_declared_file<'a>(
    root: &Path,
    relative: &'a str,
    expected_tree: &str,
    expected_digest: &str,
    declared_paths: &mut HashSet<&'a str>,
) -> Result<(), FixtureError> {
    if !declared_paths.insert(relative) {
        return Err(FixtureError::DuplicatePath(relative.to_owned()));
    }
    let bytes = read_verified_file(root, relative, expected_tree)?;
    let actual = sha256_hex(&bytes);
    if actual != expected_digest {
        return Err(FixtureError::DigestMismatch {
            path: relative.to_owned(),
            expected: expected_digest.to_owned(),
            actual,
        });
    }
    Ok(())
}

fn validate_digest(field: &'static str, digest: &str) -> Result<(), FixtureError> {
    if digest.len() != SHA256_HEX_LENGTH
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(FixtureError::InvalidDigest {
            field,
            digest: digest.to_owned(),
        });
    }
    Ok(())
}

fn read_verified_file(
    root: &Path,
    relative: &str,
    expected_tree: &str,
) -> Result<Vec<u8>, FixtureError> {
    validate_root(root)?;
    let path = validate_relative_path(root, relative, expected_tree)?;
    let before = validated_path_metadata(root, relative)?;
    let bytes = read_stable_file(&path, &before)?;
    validate_root(root)?;
    let after_path = validated_path_metadata(root, relative)?;
    if !same_file(&before, &after_path) {
        return Err(FixtureError::FileChanged(path));
    }
    Ok(bytes)
}

fn read_stable_file(path: &Path, before: &Metadata) -> Result<Vec<u8>, FixtureError> {
    let mut file = File::open(path).map_err(|source| FixtureError::MissingFile {
        path: path.to_path_buf(),
        source,
    })?;
    let opened = file.metadata().map_err(|source| FixtureError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;
    if !opened.is_file() || !same_file(before, &opened) {
        return Err(FixtureError::FileChanged(path.to_path_buf()));
    }
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|source| FixtureError::Filesystem {
            path: path.to_path_buf(),
            source,
        })?;
    let after_open = file.metadata().map_err(|source| FixtureError::Filesystem {
        path: path.to_path_buf(),
        source,
    })?;
    let after_path = fs::symlink_metadata(path).map_err(|source| FixtureError::MissingFile {
        path: path.to_path_buf(),
        source,
    })?;
    reject_symlink_or_non_regular(path, &after_path)?;
    let byte_length_matches = u64::try_from(bytes.len())
        .ok()
        .is_some_and(|length| length == before.len());
    if !same_file(before, &after_open)
        || !same_file(before, &after_path)
        || before.len() != after_open.len()
        || !byte_length_matches
    {
        return Err(FixtureError::FileChanged(path.to_path_buf()));
    }
    Ok(bytes)
}

fn reject_symlink_or_non_regular(path: &Path, metadata: &Metadata) -> Result<(), FixtureError> {
    if metadata.file_type().is_symlink() {
        return Err(FixtureError::Symlink(path.to_path_buf()));
    }
    if !metadata.is_file() {
        return Err(FixtureError::NotRegularFile(path.to_path_buf()));
    }
    Ok(())
}

fn validate_relative_path(
    root: &Path,
    relative: &str,
    expected_tree: &str,
) -> Result<PathBuf, FixtureError> {
    if relative.is_empty()
        || relative.contains('\\')
        || relative.starts_with("//")
        || looks_like_windows_prefix(relative)
    {
        return Err(FixtureError::UnsafePath(relative.to_owned()));
    }
    let path = Path::new(relative);
    let mut components = path.components();
    match components.next() {
        Some(Component::Normal(first)) if first == expected_tree => {}
        Some(Component::Normal(_)) => {
            return Err(FixtureError::PathOutsideFixtureTrees(relative.to_owned()));
        }
        _ => return Err(FixtureError::UnsafePath(relative.to_owned())),
    }
    if components.clone().next().is_none()
        || components.any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(FixtureError::UnsafePath(relative.to_owned()));
    }
    Ok(root.join(path))
}

fn looks_like_windows_prefix(path: &str) -> bool {
    let bytes = path.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn validated_path_metadata(root: &Path, relative: &str) -> Result<Metadata, FixtureError> {
    let mut current = root.to_path_buf();
    let path = Path::new(relative);
    let component_count = path.components().count();
    for (index, component) in path.components().enumerate() {
        let Component::Normal(component) = component else {
            return Err(FixtureError::UnsafePath(relative.to_owned()));
        };
        current.push(component);
        let metadata =
            fs::symlink_metadata(&current).map_err(|source| FixtureError::MissingFile {
                path: current.clone(),
                source,
            })?;
        if metadata.file_type().is_symlink() {
            return Err(FixtureError::Symlink(current));
        }
        if index + 1 == component_count {
            if !metadata.is_file() {
                return Err(FixtureError::NotRegularFile(current));
            }
            return Ok(metadata);
        }
        if !metadata.is_dir() {
            return Err(FixtureError::NotRegularFile(current));
        }
    }
    Err(FixtureError::UnsafePath(relative.to_owned()))
}

#[cfg(unix)]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev()
        && left.ino() == right.ino()
        && left.len() == right.len()
        && left.mtime() == right.mtime()
        && left.mtime_nsec() == right.mtime_nsec()
}

#[cfg(not(unix))]
fn same_file(left: &Metadata, right: &Metadata) -> bool {
    left.len() == right.len() && left.modified().ok() == right.modified().ok()
}

fn scan_fixture_tree(root: &Path, directory: &str) -> Result<Vec<String>, FixtureError> {
    let directory_path = root.join(directory);
    let metadata =
        fs::symlink_metadata(&directory_path).map_err(|source| FixtureError::MissingFile {
            path: directory_path.clone(),
            source,
        })?;
    if metadata.file_type().is_symlink() {
        return Err(FixtureError::Symlink(directory_path));
    }
    if !metadata.is_dir() {
        return Err(FixtureError::NotRegularFile(directory_path));
    }

    let mut files = Vec::new();
    scan_directory(root, &directory_path, &mut files)?;
    files.sort_by(|left, right| left.as_bytes().cmp(right.as_bytes()));
    Ok(files)
}

fn scan_directory(
    root: &Path,
    directory: &Path,
    files: &mut Vec<String>,
) -> Result<(), FixtureError> {
    let entries = fs::read_dir(directory).map_err(|source| FixtureError::Filesystem {
        path: directory.to_path_buf(),
        source,
    })?;
    for entry in entries {
        let entry = entry.map_err(|source| FixtureError::Filesystem {
            path: directory.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| FixtureError::Filesystem {
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(FixtureError::Symlink(path));
        }
        if metadata.is_dir() {
            scan_directory(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(relative_utf8(root, &path)?);
        }
    }
    Ok(())
}

fn relative_utf8(root: &Path, path: &Path) -> Result<String, FixtureError> {
    path.strip_prefix(root)
        .map_err(|_| FixtureError::UnsafePath(path.display().to_string()))?
        .to_str()
        .map(str::to_owned)
        .ok_or_else(|| FixtureError::NonUtf8Path(path.to_path_buf()))
}

fn json_string_field(
    path: PathBuf,
    bytes: &[u8],
    field: &'static str,
) -> Result<String, FixtureError> {
    let value: serde_json::Value =
        serde_json::from_slice(bytes).map_err(|source| FixtureError::ParseJson {
            path: path.clone(),
            source,
        })?;
    value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .ok_or(FixtureError::MissingSchemaField { path, field })
}

fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn create_temporary_manifest(root: &Path) -> Result<(PathBuf, File), FixtureError> {
    for attempt in 0..1_024_u32 {
        let path = root.join(format!(
            ".manifest.toml.tmp-{}-{attempt}",
            std::process::id()
        ));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => return Err(FixtureError::WriteManifest { path, source }),
        }
    }
    let path = root.join(".manifest.toml.tmp");
    Err(FixtureError::WriteManifest {
        path,
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "no unique same-directory manifest temporary path available",
        ),
    })
}

fn sync_directory_if_supported(directory: &Path) -> Result<(), FixtureError> {
    match File::open(directory).and_then(|file| file.sync_all()) {
        Ok(()) => Ok(()),
        Err(source)
            if matches!(
                source.kind(),
                io::ErrorKind::Unsupported | io::ErrorKind::InvalidInput
            ) =>
        {
            Ok(())
        }
        Err(source) => Err(FixtureError::WriteManifest {
            path: directory.to_path_buf(),
            source,
        }),
    }
}

struct TemporaryCleanup {
    path: PathBuf,
    armed: bool,
}

impl TemporaryCleanup {
    const fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    const fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TemporaryCleanup {
    fn drop(&mut self) {
        if self.armed {
            // The primary error is returned by the write path; Drop cannot safely report
            // a secondary cleanup failure.
            drop(fs::remove_file(&self.path));
        }
    }
}
