//! Crash-safe atomic state adapter.
//!
//! Stage 2 names RocksDB 11.1.x with a synced WAL WriteBatch. This workspace
//! cannot vendor that line: `cmake` is absent, crates.io `rust-rocksdb` wraps
//! ~10.x, and `librocksdb-sys` is dual-licensed GPL-2.0 which `deny.toml` does
//! not allow. Domain crates also must not depend on a native engine.
//!
//! This adapter still implements [`storage_ports::AtomicStateStore`]: one
//! fsynced generation plus an atomic HEAD pointer. Success means a later
//! process can `load_latest` the complete image. It does not claim a RocksDB
//! 11.1 deployment.

use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
};

use canonical_ledger::{StateImage, StateImageError, StateImageLimits};
use rustix::fs::{
    AtFlags, FlockOperation, Mode, OFlags, RenameFlags, flock, mkdirat, open, openat,
    renameat_with, unlinkat,
};
use storage_ports::{
    AtomicStateCommit, AtomicStateStore, STATE_STORE_SCHEMA, StateCommitDisposition,
    StateCommitReceipt, StateStoreError,
};

const HEAD_FILE: &str = "HEAD";
const LOCK_FILE: &str = "LOCK";
const STATE_FILE: &str = "state.bin";
const SCHEMA_FILE: &str = "SCHEMA";
const GENERATION_PREFIX: &str = "gen-";
const MAX_PATH_BYTES: usize = 4_096;
const STAGING_ATTEMPTS: usize = 16;

#[derive(Debug)]
pub struct SyncedWriteBatchStore {
    root_path: PathBuf,
    root: File,
    lock: File,
    state_limits: StateImageLimits,
}

impl SyncedWriteBatchStore {
    pub fn open(
        root: impl AsRef<Path>,
        state_limits: StateImageLimits,
    ) -> Result<Self, StateStoreError> {
        let root_path = prepare_root(root.as_ref())?;
        let root = File::from(
            open(
                &root_path,
                OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::empty(),
            )
            .map_err(|_| StateStoreError::Io("opening atomic state root"))?,
        );
        validate_directory(&root)?;
        let lock = File::from(
            openat(
                &root,
                LOCK_FILE,
                OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|_| StateStoreError::Io("creating atomic state lock"))?,
        );
        lock.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| StateStoreError::Io("setting atomic state lock permissions"))?;
        flock(&lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| StateStoreError::Locked)?;
        Ok(Self {
            root_path,
            root,
            lock,
            state_limits,
        })
    }

    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    fn load_current(&self) -> Result<Option<StateImage>, StateStoreError> {
        let Some(generation) = read_head(&self.root)? else {
            return Ok(None);
        };
        let directory = open_generation(&self.root, &generation)?;
        require_schema(&directory)?;
        let bytes = read_regular(&directory, STATE_FILE, self.state_limits.max_state_bytes())?;
        decode_image(&bytes, self.state_limits).map(Some)
    }
}

impl AtomicStateStore for SyncedWriteBatchStore {
    fn commit(
        &self,
        commit: &AtomicStateCommit<'_>,
    ) -> Result<StateCommitDisposition, StateStoreError> {
        let bytes = commit.state_image().canonical_bytes();
        if bytes.len() > self.state_limits.max_state_bytes() {
            return Err(StateStoreError::ResourceLimit);
        }
        let receipt = StateCommitReceipt::new(
            commit.block_height(),
            commit.canonical_block_hash(),
            commit.after_state_hash(),
        );
        if let Some(existing) = self.load_current()? {
            let existing_height = existing.block_height().ok_or(StateStoreError::Corrupt)?;
            if existing.chain_id() != commit.state_image().chain_id() {
                return Err(StateStoreError::Conflict);
            }
            if existing_height.get() == commit.block_height().get() {
                if existing.state_hash() == commit.after_state_hash()
                    && existing.canonical_block_hash() == Some(commit.canonical_block_hash())
                {
                    return Ok(StateCommitDisposition::AlreadyCommitted(receipt));
                }
                return Err(StateStoreError::Conflict);
            }
            if existing.state_hash() != commit.before_state_hash()
                || existing_height.get() + 1 != commit.block_height().get()
            {
                return Err(StateStoreError::Conflict);
            }
        }

        let generation = generation_name(commit.after_state_hash());
        let mut staged = StagedGeneration::create(&self.root)?;
        staged.write_file(STATE_FILE, &bytes)?;
        staged.write_file(SCHEMA_FILE, STATE_STORE_SCHEMA.as_bytes())?;
        staged.sync()?;
        renameat_with(
            &self.root,
            staged.name.as_str(),
            &self.root,
            generation.as_str(),
            RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            if error == rustix::io::Errno::EXIST {
                StateStoreError::Conflict
            } else {
                StateStoreError::Io("publishing atomic state generation")
            }
        })?;
        staged.published = true;
        write_head(&self.root, &generation)?;
        self.root
            .sync_all()
            .map_err(|_| StateStoreError::Io("syncing atomic state root"))?;
        Ok(StateCommitDisposition::Committed(receipt))
    }

    fn load_latest(&self, limits: StateImageLimits) -> Result<Option<StateImage>, StateStoreError> {
        if limits.max_state_bytes() > self.state_limits.max_state_bytes() {
            return Err(StateStoreError::ResourceLimit);
        }
        let Some(generation) = read_head(&self.root)? else {
            return Ok(None);
        };
        let directory = open_generation(&self.root, &generation)?;
        require_schema(&directory)?;
        let bytes = read_regular(&directory, STATE_FILE, limits.max_state_bytes())?;
        decode_image(&bytes, limits).map(Some)
    }
}

impl Drop for SyncedWriteBatchStore {
    fn drop(&mut self) {
        let _ = flock(&self.lock, FlockOperation::Unlock);
    }
}

struct StagedGeneration<'a> {
    root: &'a File,
    name: String,
    directory: File,
    published: bool,
}

impl<'a> StagedGeneration<'a> {
    fn create(root: &'a File) -> Result<Self, StateStoreError> {
        for _ in 0..STAGING_ATTEMPTS {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random)
                .map_err(|_| StateStoreError::Io("generating staging identity"))?;
            let name = format!(".staged-{}", hex::encode(random));
            match mkdirat(root, name.as_str(), Mode::RWXU) {
                Ok(()) => {
                    let directory = File::from(
                        openat(
                            root,
                            name.as_str(),
                            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                            Mode::empty(),
                        )
                        .map_err(|_| StateStoreError::Io("opening staged atomic generation"))?,
                    );
                    directory
                        .set_permissions(fs::Permissions::from_mode(0o700))
                        .map_err(|_| {
                            StateStoreError::Io("setting staged generation permissions")
                        })?;
                    validate_directory(&directory)?;
                    return Ok(Self {
                        root,
                        name,
                        directory,
                        published: false,
                    });
                }
                Err(error) if error == rustix::io::Errno::EXIST => {}
                Err(_) => {
                    return Err(StateStoreError::Io("creating staged atomic generation"));
                }
            }
        }
        Err(StateStoreError::Io(
            "allocating unique atomic staging identity",
        ))
    }

    fn write_file(&self, name: &'static str, bytes: &[u8]) -> Result<(), StateStoreError> {
        let descriptor = openat(
            &self.directory,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| StateStoreError::Io("creating staged atomic file"))?;
        let mut file = File::from(descriptor);
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| StateStoreError::Io("setting staged atomic file permissions"))?;
        file.write_all(bytes)
            .map_err(|_| StateStoreError::Io("writing staged atomic file"))?;
        file.sync_all()
            .map_err(|_| StateStoreError::Io("syncing staged atomic file"))
    }

    fn sync(&self) -> Result<(), StateStoreError> {
        self.directory
            .sync_all()
            .map_err(|_| StateStoreError::Io("syncing staged atomic directory"))
    }
}

impl Drop for StagedGeneration<'_> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let _ = unlinkat(&self.directory, STATE_FILE, AtFlags::empty());
        let _ = unlinkat(&self.directory, SCHEMA_FILE, AtFlags::empty());
        let _ = unlinkat(self.root, self.name.as_str(), AtFlags::REMOVEDIR);
    }
}

fn prepare_root(root: &Path) -> Result<PathBuf, StateStoreError> {
    if root.as_os_str().is_empty()
        || root.as_os_str().as_bytes().len() > MAX_PATH_BYTES
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(StateStoreError::Io("unsafe atomic state path"));
    }
    let name = root
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or(StateStoreError::Io("unsafe atomic state path"))?;
    let parent = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| StateStoreError::Io("unsafe atomic state path"))?;
    let canonical_root = canonical_parent.join(name);
    match fs::symlink_metadata(&canonical_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.permissions().mode() & 0o777 != 0o700
            {
                return Err(StateStoreError::Io("unsafe atomic state path"));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder
                .create(&canonical_root)
                .map_err(|_| StateStoreError::Io("creating atomic state root"))?;
            fs::set_permissions(&canonical_root, fs::Permissions::from_mode(0o700))
                .map_err(|_| StateStoreError::Io("setting atomic state root permissions"))?;
            File::open(&canonical_parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| StateStoreError::Io("syncing atomic state root parent"))?;
        }
        Err(_) => return Err(StateStoreError::Io("unsafe atomic state path")),
    }
    Ok(canonical_root)
}

fn validate_directory(directory: &File) -> Result<(), StateStoreError> {
    let metadata = directory
        .metadata()
        .map_err(|_| StateStoreError::Io("inspecting atomic directory"))?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(StateStoreError::Corrupt);
    }
    Ok(())
}

fn read_regular(
    directory: &File,
    name: &'static str,
    max_bytes: usize,
) -> Result<Vec<u8>, StateStoreError> {
    let descriptor = openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(map_open_file)?;
    let file = File::from(descriptor);
    let before = file
        .metadata()
        .map_err(|_| StateStoreError::Io("inspecting atomic state file"))?;
    if !before.is_file() || before.permissions().mode() & 0o777 != 0o600 {
        return Err(StateStoreError::Corrupt);
    }
    let length = usize::try_from(before.len()).map_err(|_| StateStoreError::ResourceLimit)?;
    if length > max_bytes {
        return Err(StateStoreError::ResourceLimit);
    }
    let mut bytes = Vec::with_capacity(length);
    file.take(
        u64::try_from(max_bytes)
            .map_err(|_| StateStoreError::ResourceLimit)?
            .saturating_add(1),
    )
    .read_to_end(&mut bytes)
    .map_err(|_| StateStoreError::Io("reading atomic state file"))?;
    if bytes.len() != length {
        return Err(StateStoreError::Corrupt);
    }
    Ok(bytes)
}

fn require_schema(directory: &File) -> Result<(), StateStoreError> {
    match openat(
        directory,
        SCHEMA_FILE,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let file = File::from(descriptor);
            let metadata = file
                .metadata()
                .map_err(|_| StateStoreError::Io("inspecting atomic schema file"))?;
            if !metadata.is_file() || metadata.permissions().mode() & 0o777 != 0o600 {
                return Err(StateStoreError::Corrupt);
            }
            let mut bytes = Vec::new();
            file.take(256)
                .read_to_end(&mut bytes)
                .map_err(|_| StateStoreError::Io("reading atomic schema file"))?;
            if bytes == STATE_STORE_SCHEMA.as_bytes() {
                Ok(())
            } else {
                Err(StateStoreError::RebuildRequired)
            }
        }
        Err(error) if error == rustix::io::Errno::NOENT => Err(StateStoreError::RebuildRequired),
        Err(_) => Err(StateStoreError::Io("opening atomic schema file")),
    }
}

fn read_head(root: &File) -> Result<Option<String>, StateStoreError> {
    match openat(
        root,
        HEAD_FILE,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    ) {
        Ok(descriptor) => {
            let file = File::from(descriptor);
            let mut bytes = Vec::new();
            file.take(80)
                .read_to_end(&mut bytes)
                .map_err(|_| StateStoreError::Io("reading atomic HEAD"))?;
            let name = std::str::from_utf8(&bytes).map_err(|_| StateStoreError::Corrupt)?;
            validate_generation_name(name)?;
            Ok(Some(name.to_owned()))
        }
        Err(error) if error == rustix::io::Errno::NOENT => Ok(None),
        Err(_) => Err(StateStoreError::Io("opening atomic HEAD")),
    }
}

fn write_head(root: &File, generation: &str) -> Result<(), StateStoreError> {
    validate_generation_name(generation)?;
    let staging = format!(".{HEAD_FILE}.staged");
    let descriptor = openat(
        root,
        staging.as_str(),
        OFlags::WRONLY | OFlags::CREATE | OFlags::TRUNC | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )
    .map_err(|_| StateStoreError::Io("creating staged HEAD"))?;
    let mut file = File::from(descriptor);
    file.set_permissions(fs::Permissions::from_mode(0o600))
        .map_err(|_| StateStoreError::Io("setting staged HEAD permissions"))?;
    file.write_all(generation.as_bytes())
        .map_err(|_| StateStoreError::Io("writing staged HEAD"))?;
    file.sync_all()
        .map_err(|_| StateStoreError::Io("syncing staged HEAD"))?;
    rustix::fs::renameat(root, staging.as_str(), root, HEAD_FILE)
        .map_err(|_| StateStoreError::Io("publishing atomic HEAD"))
}

fn open_generation(root: &File, generation: &str) -> Result<File, StateStoreError> {
    validate_generation_name(generation)?;
    let directory = File::from(
        openat(
            root,
            generation,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(map_open_generation)?,
    );
    validate_directory(&directory)?;
    Ok(directory)
}

fn generation_name(state_hash: [u8; 32]) -> String {
    format!("{GENERATION_PREFIX}{}", hex::encode(state_hash))
}

fn validate_generation_name(name: &str) -> Result<(), StateStoreError> {
    let Some(hash) = name.strip_prefix(GENERATION_PREFIX) else {
        return Err(StateStoreError::Corrupt);
    };
    if hash.len() != 64
        || hash
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(StateStoreError::Corrupt);
    }
    Ok(())
}

fn decode_image(bytes: &[u8], limits: StateImageLimits) -> Result<StateImage, StateStoreError> {
    match StateImage::decode_canonical(bytes, limits) {
        Ok(image) => {
            if image.canonical_bytes() != bytes {
                return Err(StateStoreError::Corrupt);
            }
            Ok(image)
        }
        Err(StateImageError::LimitExceeded) => Err(StateStoreError::ResourceLimit),
        Err(_) => Err(StateStoreError::Corrupt),
    }
}

fn map_open_generation(error: rustix::io::Errno) -> StateStoreError {
    if error == rustix::io::Errno::NOENT {
        StateStoreError::Corrupt
    } else {
        StateStoreError::Io("opening atomic generation")
    }
}

fn map_open_file(error: rustix::io::Errno) -> StateStoreError {
    if error == rustix::io::Errno::NOENT {
        StateStoreError::Corrupt
    } else {
        StateStoreError::Io("opening atomic state file")
    }
}
