use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
};

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
