#![forbid(unsafe_code)]

mod atomic;

use std::{
    fs::{self, File},
    io::{Read, Write},
    os::unix::{
        ffi::OsStrExt,
        fs::{DirBuilderExt, PermissionsExt},
    },
    path::{Component, Path, PathBuf},
};

use canonical_ledger::{CheckpointArtifact, CheckpointCompatibility, StateImageLimits};
use domain_types::CheckpointId;
use rustix::fs::{
    AtFlags, Mode, OFlags, RenameFlags, mkdirat, open, openat, renameat_with, unlinkat,
};
use storage_ports::{
    CheckpointPublishDisposition, CheckpointReceipt, CheckpointStoreError, StateCheckpointStore,
};

pub use atomic::SyncedWriteBatchStore;

const MANIFEST_FILE: &str = "manifest.json";
const STATE_FILE: &str = "state.bin";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;
const MAX_PATH_BYTES: usize = 4_096;
const STAGING_ATTEMPTS: usize = 16;
const MANIFEST_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/state-checkpoint-manifest-file/v1";

#[derive(Debug)]
pub struct LocalCheckpointStore {
    root_path: PathBuf,
    root: File,
    state_limits: StateImageLimits,
}

impl LocalCheckpointStore {
    pub fn open(
        root: impl AsRef<Path>,
        state_limits: StateImageLimits,
    ) -> Result<Self, CheckpointStoreError> {
        let root = prepare_root(root.as_ref())?;
        let descriptor = open(
            &root,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(|_| CheckpointStoreError::UnsafePath)?;
        let root_file = File::from(descriptor);
        validate_directory(&root_file)?;
        Ok(Self {
            root_path: root,
            root: root_file,
            state_limits,
        })
    }

    #[must_use]
    pub fn root_path(&self) -> &Path {
        &self.root_path
    }

    fn load_raw(
        &self,
        checkpoint_id: &CheckpointId,
    ) -> Result<CheckpointArtifact, CheckpointStoreError> {
        validate_checkpoint_id(checkpoint_id)?;
        let generation = openat(
            &self.root,
            checkpoint_id.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_open_generation)?;
        validate_directory(&generation)?;
        let manifest = read_regular(&generation, MANIFEST_FILE, MAX_MANIFEST_BYTES)?;
        let state = read_regular(&generation, STATE_FILE, self.state_limits.max_state_bytes())?;
        let artifact = CheckpointArtifact::decode(&manifest, &state, self.state_limits)?;
        if artifact.checkpoint_id() != checkpoint_id {
            return Err(CheckpointStoreError::Conflict);
        }
        Ok(artifact)
    }

    fn receipt(artifact: &CheckpointArtifact, manifest: &[u8]) -> CheckpointReceipt {
        let mut hasher = blake3::Hasher::new_derive_key(MANIFEST_HASH_CONTEXT);
        hasher.update(manifest);
        CheckpointReceipt::new(
            artifact.checkpoint_id().clone(),
            artifact.checkpoint().block_height(),
            artifact.checkpoint().state_hash(),
            *hasher.finalize().as_bytes(),
        )
    }
}

impl StateCheckpointStore for LocalCheckpointStore {
    fn publish(
        &self,
        artifact: &CheckpointArtifact,
    ) -> Result<CheckpointPublishDisposition, CheckpointStoreError> {
        validate_checkpoint_id(artifact.checkpoint_id())?;
        if artifact.state_image_bytes().len() > self.state_limits.max_state_bytes() {
            return Err(CheckpointStoreError::TooLarge);
        }
        let manifest = artifact.encode_manifest()?;
        if manifest.len() > MAX_MANIFEST_BYTES {
            return Err(CheckpointStoreError::TooLarge);
        }
        let receipt = Self::receipt(artifact, &manifest);
        let mut staged = StagedGeneration::create(&self.root)?;
        staged.write_file(STATE_FILE, artifact.state_image_bytes())?;
        staged.write_file(MANIFEST_FILE, &manifest)?;
        staged.sync()?;

        match renameat_with(
            &self.root,
            staged.name.as_str(),
            &self.root,
            artifact.checkpoint_id().as_str(),
            RenameFlags::NOREPLACE,
        ) {
            Ok(()) => {
                staged.published = true;
                self.root
                    .sync_all()
                    .map_err(|_| CheckpointStoreError::Io("syncing checkpoint root"))?;
                Ok(CheckpointPublishDisposition::Published(receipt))
            }
            Err(error) if error == rustix::io::Errno::EXIST => {
                let existing = self.load_raw(artifact.checkpoint_id())?;
                if existing == *artifact {
                    Ok(CheckpointPublishDisposition::Identical(receipt))
                } else {
                    Err(CheckpointStoreError::Conflict)
                }
            }
            Err(_) => Err(CheckpointStoreError::Io("publishing checkpoint generation")),
        }
    }

    fn load(
        &self,
        checkpoint_id: &CheckpointId,
        compatibility: &CheckpointCompatibility,
        limits: StateImageLimits,
    ) -> Result<CheckpointArtifact, CheckpointStoreError> {
        if limits.max_state_bytes() > self.state_limits.max_state_bytes() {
            return Err(CheckpointStoreError::TooLarge);
        }
        validate_checkpoint_id(checkpoint_id)?;
        let generation = openat(
            &self.root,
            checkpoint_id.as_str(),
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map(File::from)
        .map_err(map_open_generation)?;
        validate_directory(&generation)?;
        let manifest = read_regular(&generation, MANIFEST_FILE, MAX_MANIFEST_BYTES)?;
        let state = read_regular(&generation, STATE_FILE, limits.max_state_bytes())?;
        let artifact = CheckpointArtifact::decode(&manifest, &state, limits)?;
        if artifact.checkpoint_id() != checkpoint_id {
            return Err(CheckpointStoreError::Conflict);
        }
        artifact.validate_compatibility(compatibility)?;
        Ok(artifact)
    }
}

struct StagedGeneration<'a> {
    root: &'a File,
    name: String,
    directory: File,
    published: bool,
}

impl<'a> StagedGeneration<'a> {
    fn create(root: &'a File) -> Result<Self, CheckpointStoreError> {
        for _ in 0..STAGING_ATTEMPTS {
            let mut random = [0_u8; 16];
            getrandom::fill(&mut random)
                .map_err(|_| CheckpointStoreError::Io("generating staging identity"))?;
            let name = format!(".staged-{}", hex::encode(random));
            match mkdirat(root, name.as_str(), Mode::RWXU) {
                Ok(()) => {
                    let directory = match openat(
                        root,
                        name.as_str(),
                        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
                        Mode::empty(),
                    ) {
                        Ok(descriptor) => File::from(descriptor),
                        Err(_) => {
                            let _ = unlinkat(root, name.as_str(), AtFlags::REMOVEDIR);
                            return Err(CheckpointStoreError::Io(
                                "opening staged checkpoint generation",
                            ));
                        }
                    };
                    directory
                        .set_permissions(fs::Permissions::from_mode(0o700))
                        .map_err(|_| {
                            let _ = unlinkat(root, name.as_str(), AtFlags::REMOVEDIR);
                            CheckpointStoreError::Io("setting staged checkpoint permissions")
                        })?;
                    if let Err(error) = validate_directory(&directory) {
                        let _ = unlinkat(root, name.as_str(), AtFlags::REMOVEDIR);
                        return Err(error);
                    }
                    return Ok(Self {
                        root,
                        name,
                        directory,
                        published: false,
                    });
                }
                Err(error) if error == rustix::io::Errno::EXIST => {}
                Err(_) => {
                    return Err(CheckpointStoreError::Io(
                        "creating staged checkpoint generation",
                    ));
                }
            }
        }
        Err(CheckpointStoreError::Io(
            "allocating unique checkpoint staging identity",
        ))
    }

    fn write_file(&self, name: &'static str, bytes: &[u8]) -> Result<(), CheckpointStoreError> {
        let descriptor = openat(
            &self.directory,
            name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|_| CheckpointStoreError::Io("creating staged checkpoint file"))?;
        let mut file = File::from(descriptor);
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| CheckpointStoreError::Io("setting staged checkpoint file permissions"))?;
        file.write_all(bytes)
            .map_err(|_| CheckpointStoreError::Io("writing staged checkpoint file"))?;
        file.sync_all()
            .map_err(|_| CheckpointStoreError::Io("syncing staged checkpoint file"))
    }

    fn sync(&self) -> Result<(), CheckpointStoreError> {
        self.directory
            .sync_all()
            .map_err(|_| CheckpointStoreError::Io("syncing staged checkpoint directory"))
    }
}

impl Drop for StagedGeneration<'_> {
    fn drop(&mut self) {
        if self.published {
            return;
        }
        let _ = unlinkat(&self.directory, MANIFEST_FILE, AtFlags::empty());
        let _ = unlinkat(&self.directory, STATE_FILE, AtFlags::empty());
        let _ = unlinkat(self.root, self.name.as_str(), AtFlags::REMOVEDIR);
    }
}

fn prepare_root(root: &Path) -> Result<PathBuf, CheckpointStoreError> {
    if root.as_os_str().is_empty()
        || root.as_os_str().as_bytes().len() > MAX_PATH_BYTES
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(CheckpointStoreError::UnsafePath);
    }
    let name = root
        .file_name()
        .filter(|name| !name.is_empty())
        .ok_or(CheckpointStoreError::UnsafePath)?;
    let parent = root
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = parent
        .canonicalize()
        .map_err(|_| CheckpointStoreError::UnsafePath)?;
    let canonical_root = canonical_parent.join(name);
    match fs::symlink_metadata(&canonical_root) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || !metadata.is_dir()
                || metadata.permissions().mode() & 0o777 != 0o700
            {
                return Err(CheckpointStoreError::UnsafePath);
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.mode(0o700);
            builder
                .create(&canonical_root)
                .map_err(|_| CheckpointStoreError::Io("creating checkpoint root"))?;
            fs::set_permissions(&canonical_root, fs::Permissions::from_mode(0o700))
                .map_err(|_| CheckpointStoreError::Io("setting checkpoint root permissions"))?;
            File::open(&canonical_parent)
                .and_then(|directory| directory.sync_all())
                .map_err(|_| CheckpointStoreError::Io("syncing checkpoint root parent"))?;
        }
        Err(_) => return Err(CheckpointStoreError::UnsafePath),
    }
    Ok(canonical_root)
}

fn validate_directory(directory: &File) -> Result<(), CheckpointStoreError> {
    let metadata = directory
        .metadata()
        .map_err(|_| CheckpointStoreError::Io("inspecting checkpoint directory"))?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o777 != 0o700 {
        return Err(CheckpointStoreError::UnsafeObject);
    }
    Ok(())
}

fn read_regular(
    directory: &File,
    name: &'static str,
    max_bytes: usize,
) -> Result<Vec<u8>, CheckpointStoreError> {
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
        .map_err(|_| CheckpointStoreError::Io("inspecting checkpoint file"))?;
    if !before.is_file() || before.permissions().mode() & 0o777 != 0o600 {
        return Err(CheckpointStoreError::UnsafeObject);
    }
    let length = usize::try_from(before.len()).map_err(|_| CheckpointStoreError::TooLarge)?;
    if length > max_bytes {
        return Err(CheckpointStoreError::TooLarge);
    }
    let mut bytes = Vec::with_capacity(length);
    file.take(
        u64::try_from(max_bytes)
            .map_err(|_| CheckpointStoreError::TooLarge)?
            .saturating_add(1),
    )
    .read_to_end(&mut bytes)
    .map_err(|_| CheckpointStoreError::Io("reading checkpoint file"))?;
    if bytes.len() != length {
        return Err(CheckpointStoreError::Io(
            "checkpoint file changed while reading",
        ));
    }
    Ok(bytes)
}

fn validate_checkpoint_id(checkpoint_id: &CheckpointId) -> Result<(), CheckpointStoreError> {
    let Some(hash) = checkpoint_id.as_str().strip_prefix("state-checkpoint-v1-") else {
        return Err(CheckpointStoreError::UnsafePath);
    };
    if hash.len() != 64
        || hash
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointStoreError::UnsafePath);
    }
    Ok(())
}

fn map_open_generation(error: rustix::io::Errno) -> CheckpointStoreError {
    if error == rustix::io::Errno::NOENT {
        CheckpointStoreError::NotFound
    } else if error == rustix::io::Errno::LOOP || error == rustix::io::Errno::NOTDIR {
        CheckpointStoreError::UnsafeObject
    } else {
        CheckpointStoreError::Io("opening checkpoint generation")
    }
}

fn map_open_file(error: rustix::io::Errno) -> CheckpointStoreError {
    if error == rustix::io::Errno::NOENT {
        CheckpointStoreError::NotFound
    } else if error == rustix::io::Errno::LOOP {
        CheckpointStoreError::UnsafeObject
    } else {
        CheckpointStoreError::Io("opening checkpoint file")
    }
}

pub const CRATE_BOOTSTRAPPED: bool = true;
