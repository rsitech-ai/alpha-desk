use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

use sha2::{Digest, Sha256};
use storage_ports::ArchiveError;

const MAX_MANIFEST_BYTES: u64 = 64 * 1024 * 1024;

pub fn validate_relative(path: &Path) -> Result<(), ArchiveError> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(ArchiveError::UnsafePath);
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            _ => return Err(ArchiveError::UnsafePath),
        }
    }
    Ok(())
}

pub fn ensure_directory(root: &Path, relative: &Path) -> Result<PathBuf, ArchiveError> {
    validate_relative(relative)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(ArchiveError::UnsafePath);
        };
        current.push(component);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(ArchiveError::UnsafePath);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current)
                    .map_err(|_| ArchiveError::Io("creating archive directory"))?;
                let parent = current.parent().ok_or(ArchiveError::UnsafePath)?;
                sync_directory(parent)?;
            }
            Err(_) => return Err(ArchiveError::Io("inspecting archive directory")),
        }
    }
    Ok(current)
}

pub fn read_regular(root: &Path, relative: &Path, max_bytes: u64) -> Result<Vec<u8>, ArchiveError> {
    let (mut file, length) = open_regular(root, relative, max_bytes)?;
    let capacity = usize::try_from(length)
        .map_err(|_| ArchiveError::InvalidInput("archive file exceeds address space"))?;
    let mut bytes = Vec::with_capacity(capacity);
    file.read_to_end(&mut bytes)
        .map_err(|_| ArchiveError::Io("reading archive file"))?;
    if u64::try_from(bytes.len()).ok() != Some(length) {
        return Err(ArchiveError::Io("archive file changed while reading"));
    }
    Ok(bytes)
}

pub fn open_regular(
    root: &Path,
    relative: &Path,
    max_bytes: u64,
) -> Result<(File, u64), ArchiveError> {
    validate_relative(relative)?;
    validate_existing_components(root, relative)?;
    let path = root.join(relative);
    let before =
        fs::symlink_metadata(&path).map_err(|_| ArchiveError::Io("inspecting archive file"))?;
    if before.file_type().is_symlink() || !before.is_file() || before.len() > max_bytes {
        return Err(ArchiveError::UnsafePath);
    }
    let file = File::open(&path).map_err(|_| ArchiveError::Io("opening archive file"))?;
    let opened = file
        .metadata()
        .map_err(|_| ArchiveError::Io("inspecting opened archive file"))?;
    if !same_file(&before, &opened) {
        return Err(ArchiveError::UnsafePath);
    }
    Ok((file, opened.len()))
}

pub fn read_manifest(root: &Path, relative: &Path) -> Result<Vec<u8>, ArchiveError> {
    read_regular(root, relative, MAX_MANIFEST_BYTES)
}

pub fn publish_immutable(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), ArchiveError> {
    validate_relative(relative)?;
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    let parent = ensure_directory(root, parent_relative)?;
    let destination = root.join(relative);

    if destination.exists() {
        let existing = read_regular(
            root,
            relative,
            u64::try_from(bytes.len())
                .map_err(|_| ArchiveError::InvalidInput("archive object exceeds u64"))?,
        )?;
        return if existing == bytes {
            Ok(())
        } else {
            Err(ArchiveError::CorruptObject(
                relative.to_string_lossy().into_owned(),
            ))
        };
    }

    let mut staged = tempfile::Builder::new()
        .prefix(".staged-")
        .tempfile_in(&parent)
        .map_err(|_| ArchiveError::Io("creating staged archive file"))?;
    staged
        .write_all(bytes)
        .map_err(|_| ArchiveError::Io("writing staged archive file"))?;
    staged
        .as_file_mut()
        .sync_all()
        .map_err(|_| ArchiveError::Io("syncing staged archive file"))?;
    match staged.persist_noclobber(&destination) {
        Ok(_) => {}
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
            let existing = read_regular(
                root,
                relative,
                u64::try_from(bytes.len())
                    .map_err(|_| ArchiveError::InvalidInput("archive object exceeds u64"))?,
            )?;
            if existing != bytes {
                return Err(ArchiveError::CorruptObject(
                    relative.to_string_lossy().into_owned(),
                ));
            }
        }
        Err(_) => return Err(ArchiveError::Io("publishing immutable archive file")),
    }
    sync_directory(&parent)
}

pub fn try_read_regular(
    root: &Path,
    relative: &Path,
    max_bytes: u64,
) -> Result<Option<Vec<u8>>, ArchiveError> {
    validate_relative(relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(_) => read_regular(root, relative, max_bytes).map(Some),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(ArchiveError::Io("inspecting archive file")),
    }
}

pub fn exists_regular(root: &Path, relative: &Path) -> Result<bool, ArchiveError> {
    validate_relative(relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                Err(ArchiveError::UnsafePath)
            } else {
                Ok(true)
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(_) => Err(ArchiveError::Io("inspecting archive file")),
    }
}

pub fn extend_append_only(
    root: &Path,
    relative: &Path,
    committed_prefix: &[u8],
    full_bytes: &[u8],
    max_bytes: u64,
) -> Result<(), ArchiveError> {
    let full_size = u64::try_from(full_bytes.len())
        .map_err(|_| ArchiveError::InvalidInput("append-only object exceeds u64"))?;
    if full_bytes.is_empty()
        || full_size > max_bytes
        || !full_bytes.starts_with(committed_prefix)
        || full_bytes.len() < committed_prefix.len()
    {
        return Err(ArchiveError::InvalidInput(
            "append-only object does not extend the committed prefix",
        ));
    }
    match try_read_regular(root, relative, max_bytes)? {
        None => publish_immutable(root, relative, full_bytes),
        Some(existing) => {
            if existing == full_bytes {
                return Ok(());
            }
            if !existing.starts_with(committed_prefix) {
                return Err(ArchiveError::ManifestVerification(
                    "append-only object does not authenticate its committed prefix",
                ));
            }
            if existing.len() == committed_prefix.len() {
                append_suffix(root, relative, &full_bytes[committed_prefix.len()..])
            } else if full_bytes.starts_with(&existing) {
                append_suffix(root, relative, &full_bytes[existing.len()..])
            } else {
                Err(ArchiveError::ManifestVerification(
                    "append-only object has a divergent uncommitted suffix",
                ))
            }
        }
    }
}

fn append_suffix(root: &Path, relative: &Path, suffix: &[u8]) -> Result<(), ArchiveError> {
    if suffix.is_empty() {
        return Ok(());
    }
    validate_relative(relative)?;
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    let parent = ensure_directory(root, parent_relative)?;
    let path = root.join(relative);
    let mut file = OpenOptions::new()
        .append(true)
        .open(&path)
        .map_err(|_| ArchiveError::Io("opening append-only archive file"))?;
    file.write_all(suffix)
        .map_err(|_| ArchiveError::Io("appending archive file"))?;
    file.sync_all()
        .map_err(|_| ArchiveError::Io("syncing append-only archive file"))?;
    sync_directory(&parent)
}

pub fn publish_current_cas(
    root: &Path,
    relative: &Path,
    expected_existing: Option<&[u8]>,
    new_bytes: &[u8],
) -> Result<(), ArchiveError> {
    let existing = try_read_regular(root, relative, MAX_MANIFEST_BYTES)?;
    match (expected_existing, existing.as_deref()) {
        (None, None) => {}
        (Some(expected), Some(actual)) if expected == actual => {}
        _ => {
            return Err(ArchiveError::ManifestVerification(
                "CURRENT pointer does not match the expected exact root",
            ));
        }
    }
    publish_current(root, relative, new_bytes)?;
    let readback = read_regular(root, relative, MAX_MANIFEST_BYTES)?;
    if readback != new_bytes {
        return Err(ArchiveError::ManifestVerification(
            "CURRENT pointer readback does not match the published exact root",
        ));
    }
    Ok(())
}

pub fn publish_current(root: &Path, relative: &Path, bytes: &[u8]) -> Result<(), ArchiveError> {
    validate_relative(relative)?;
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    let parent = ensure_directory(root, parent_relative)?;
    let destination = root.join(relative);
    if let Ok(metadata) = fs::symlink_metadata(&destination)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(ArchiveError::UnsafePath);
    }

    let mut staged = tempfile::Builder::new()
        .prefix(".current-")
        .tempfile_in(&parent)
        .map_err(|_| ArchiveError::Io("creating staged current pointer"))?;
    staged
        .write_all(bytes)
        .map_err(|_| ArchiveError::Io("writing staged current pointer"))?;
    staged
        .as_file_mut()
        .sync_all()
        .map_err(|_| ArchiveError::Io("syncing staged current pointer"))?;
    staged
        .persist(&destination)
        .map_err(|_| ArchiveError::Io("publishing current pointer"))?;
    sync_directory(&parent)
}

pub fn create_parquet_staging_file(
    root: &Path,
    parent_relative: &Path,
) -> Result<tempfile::NamedTempFile, ArchiveError> {
    let parent = ensure_directory(root, parent_relative)?;
    tempfile::Builder::new()
        .prefix(".parquet-")
        .tempfile_in(parent)
        .map_err(|_| ArchiveError::Io("creating staged Parquet object"))
}

pub fn publish_staged_immutable(
    root: &Path,
    relative: &Path,
    staged: tempfile::NamedTempFile,
) -> Result<(), ArchiveError> {
    validate_relative(relative)?;
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    let parent = ensure_directory(root, parent_relative)?;
    let destination = root.join(relative);
    match staged.persist_noclobber(&destination) {
        Ok(_) => sync_directory(&parent),
        Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => Err(ArchiveError::Io("publishing immutable Parquet object")),
    }
}

pub fn open_writer_lock(root: &Path, relative: &Path) -> Result<File, ArchiveError> {
    validate_relative(relative)?;
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    ensure_directory(root, parent_relative)?;
    let path = root.join(relative);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|_| ArchiveError::Io("opening archive writer lock"))?;
    rustix::fs::flock(&file, rustix::fs::FlockOperation::NonBlockingLockExclusive)
        .map_err(|_| ArchiveError::WriterBusy)?;
    Ok(file)
}

pub fn open_shared_lease(root: &Path, relative: &Path) -> Result<File, ArchiveError> {
    open_lease(
        root,
        relative,
        rustix::fs::FlockOperation::NonBlockingLockShared,
    )
}

pub fn open_exclusive_lease(root: &Path, relative: &Path) -> Result<File, ArchiveError> {
    open_lease(
        root,
        relative,
        rustix::fs::FlockOperation::NonBlockingLockExclusive,
    )
}

fn open_lease(
    root: &Path,
    relative: &Path,
    operation: rustix::fs::FlockOperation,
) -> Result<File, ArchiveError> {
    validate_relative(relative)?;
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    ensure_directory(root, parent_relative)?;
    let path = root.join(relative);
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&path)
        .map_err(|_| ArchiveError::Io("opening archive root lease"))?;
    let metadata = fs::symlink_metadata(&path)
        .map_err(|_| ArchiveError::Io("inspecting archive root lease"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ArchiveError::UnsafePath);
    }
    rustix::fs::flock(&file, operation).map_err(|_| ArchiveError::WriterBusy)?;
    Ok(file)
}

pub fn list_regular_names(root: &Path, relative: &Path) -> Result<Vec<String>, ArchiveError> {
    validate_relative(relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ArchiveError::UnsafePath);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(_) => return Err(ArchiveError::Io("inspecting archive directory")),
    }
    let mut names = Vec::new();
    let entries = fs::read_dir(&path).map_err(|_| ArchiveError::Io("listing archive directory"))?;
    for entry in entries {
        let entry = entry.map_err(|_| ArchiveError::Io("reading archive directory entry"))?;
        let name = entry.file_name();
        let child = path.join(&name);
        let metadata = fs::symlink_metadata(&child)
            .map_err(|_| ArchiveError::Io("inspecting archive directory entry"))?;
        if metadata.file_type().is_symlink() {
            return Err(ArchiveError::UnsafePath);
        }
        if metadata.is_file() {
            let name = name.to_str().ok_or(ArchiveError::UnsafePath)?.to_owned();
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

pub fn regular_digest(
    root: &Path,
    relative: &Path,
    max_bytes: u64,
) -> Result<([u8; 32], u64), ArchiveError> {
    let (mut file, length) = open_regular(root, relative, max_bytes)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut read_total = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| ArchiveError::Io("hashing archive file"))?;
        if read == 0 {
            break;
        }
        read_total = read_total
            .checked_add(
                u64::try_from(read)
                    .map_err(|_| ArchiveError::InvalidInput("archive file exceeds u64"))?,
            )
            .ok_or(ArchiveError::InvalidInput("archive file size overflows"))?;
        hasher.update(&buffer[..read]);
    }
    if read_total != length {
        return Err(ArchiveError::Io("archive file changed while hashing"));
    }
    Ok((hasher.finalize().into(), length))
}

pub fn unlink_regular_matching(
    root: &Path,
    relative: &Path,
    expected_sha256: [u8; 32],
    expected_len: u64,
) -> Result<(), ArchiveError> {
    let (digest, length) = regular_digest(root, relative, expected_len.max(1))?;
    if digest != expected_sha256 || length != expected_len {
        return Err(ArchiveError::ManifestVerification(
            "eligible object digest or length does not match the deletion plan",
        ));
    }
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    let parent = ensure_directory(root, parent_relative)?;
    fs::remove_file(root.join(relative)).map_err(|_| ArchiveError::Io("unlinking archive file"))?;
    sync_directory(&parent)
}

pub fn append_journal_line(
    root: &Path,
    relative: &Path,
    line: &[u8],
    max_bytes: u64,
) -> Result<(), ArchiveError> {
    if line.is_empty() || line.contains(&b'\n') {
        return Err(ArchiveError::InvalidInput(
            "deletion journal line must be nonempty and single-line",
        ));
    }
    let added = u64::try_from(line.len().checked_add(1).ok_or(ArchiveError::InvalidInput(
        "deletion journal line overflows",
    ))?)
    .map_err(|_| ArchiveError::InvalidInput("deletion journal line exceeds u64"))?;
    validate_relative(relative)?;
    let parent_relative = relative.parent().ok_or(ArchiveError::UnsafePath)?;
    let parent = ensure_directory(root, parent_relative)?;
    let path = root.join(relative);
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(ArchiveError::UnsafePath);
            }
            if metadata
                .len()
                .checked_add(added)
                .is_none_or(|total| total > max_bytes)
            {
                return Err(ArchiveError::InvalidInput(
                    "deletion journal exceeds the reserved bound",
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if added > max_bytes {
                return Err(ArchiveError::InvalidInput(
                    "deletion journal exceeds the reserved bound",
                ));
            }
        }
        Err(_) => return Err(ArchiveError::Io("inspecting deletion journal")),
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|_| ArchiveError::Io("opening deletion journal"))?;
    file.write_all(line)
        .and_then(|_| file.write_all(b"\n"))
        .map_err(|_| ArchiveError::Io("appending deletion journal"))?;
    file.sync_all()
        .map_err(|_| ArchiveError::Io("syncing deletion journal"))?;
    sync_directory(&parent)
}

fn validate_existing_components(root: &Path, relative: &Path) -> Result<(), ArchiveError> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(ArchiveError::UnsafePath);
        };
        current.push(component);
        let metadata = fs::symlink_metadata(&current)
            .map_err(|_| ArchiveError::Io("inspecting archive path component"))?;
        if metadata.file_type().is_symlink() {
            return Err(ArchiveError::UnsafePath);
        }
    }
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), ArchiveError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| ArchiveError::Io("syncing archive directory"))
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}

#[cfg(not(unix))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && left.is_file() == right.is_file()
        && left.modified().ok() == right.modified().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn current_cas_rejects_stale_expected_root_and_readback_matches() {
        let root = tempfile::tempdir().unwrap();
        let relative = PathBuf::from("dataset/CURRENT");
        let first = b"{\"root\":1}";
        let second = b"{\"root\":2}";
        assert!(publish_current_cas(root.path(), &relative, None, first).is_ok());
        assert!(publish_current_cas(root.path(), &relative, Some(b"stale"), second).is_err());
        assert!(publish_current_cas(root.path(), &relative, Some(first), second).is_ok());
        assert_eq!(read_regular(root.path(), &relative, 1024).unwrap(), second);
    }

    #[test]
    fn append_only_extension_keeps_committed_prefix_and_rejects_divergence() {
        let root = tempfile::tempdir().unwrap();
        let relative = PathBuf::from("journals/generation-1.log");
        let first = b"prefix-bytes";
        let extended = b"prefix-bytes-and-suffix";
        extend_append_only(root.path(), &relative, &[], first, 1024).unwrap();
        extend_append_only(root.path(), &relative, first, extended, 1024).unwrap();
        assert_eq!(
            read_regular(root.path(), &relative, 1024).unwrap(),
            extended
        );
        assert!(
            extend_append_only(
                root.path(),
                &relative,
                first,
                b"prefix-bytes-DIFFERENT",
                1024
            )
            .is_err()
        );
    }

    #[test]
    fn exclusive_lease_fails_closed_while_a_shared_lease_is_held() {
        let root = tempfile::tempdir().unwrap();
        let relative = PathBuf::from("dataset/leases/root-ab.lease");
        let shared = open_shared_lease(root.path(), &relative).unwrap();
        assert!(matches!(
            open_exclusive_lease(root.path(), &relative),
            Err(ArchiveError::WriterBusy)
        ));
        drop(shared);
        assert!(open_exclusive_lease(root.path(), &relative).is_ok());
    }

    #[test]
    fn unlink_regular_matching_requires_digest_and_fsyncs_parent() {
        let root = tempfile::tempdir().unwrap();
        let relative = PathBuf::from("objects/part.bin");
        publish_immutable(root.path(), &relative, b"payload").unwrap();
        assert!(unlink_regular_matching(root.path(), &relative, [0x11; 32], 7).is_err());
        let (digest, length) = regular_digest(root.path(), &relative, 1024).unwrap();
        unlink_regular_matching(root.path(), &relative, digest, length).unwrap();
        assert!(
            try_read_regular(root.path(), &relative, 1024)
                .unwrap()
                .is_none()
        );
    }
}
