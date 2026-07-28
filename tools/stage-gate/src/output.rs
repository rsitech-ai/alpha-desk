use std::{
    fs::File,
    io::Write as _,
    os::unix::ffi::OsStrExt as _,
    path::{Component, Path, PathBuf},
};

use rustix::{
    fd::OwnedFd,
    fs::{
        AtFlags, FileType, Mode, OFlags, fsync, mkdirat, open, openat, renameat, statat, unlinkat,
    },
    io::Errno,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputObservation {
    BeforeRename,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputErrorCode {
    UnsafeRoot,
    UnsafeTarget,
    CreateFailed,
    WriteFailed,
    RenameFailed,
}

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    #[error("output root is unsafe: {0}")]
    UnsafeRoot(String),
    #[error("output target must be a safe direct child: {0}")]
    UnsafeTarget(PathBuf),
    #[error("could not create an exclusive output temporary: {0}")]
    CreateFailed(String),
    #[error("could not write or synchronize output: {0}")]
    WriteFailed(String),
    #[error("could not atomically publish output: {0}")]
    RenameFailed(String),
}

impl OutputError {
    #[must_use]
    pub const fn code(&self) -> OutputErrorCode {
        match self {
            Self::UnsafeRoot(_) => OutputErrorCode::UnsafeRoot,
            Self::UnsafeTarget(_) => OutputErrorCode::UnsafeTarget,
            Self::CreateFailed(_) => OutputErrorCode::CreateFailed,
            Self::WriteFailed(_) => OutputErrorCode::WriteFailed,
            Self::RenameFailed(_) => OutputErrorCode::RenameFailed,
        }
    }
}

#[derive(Debug)]
pub struct OutputRoot {
    directory: OwnedFd,
}

impl OutputRoot {
    pub fn open(repository: &Path, relative_root: &Path) -> Result<Self, OutputError> {
        let components = safe_relative_components(relative_root)?;
        let mut directory = open(
            repository,
            OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|error| OutputError::UnsafeRoot(error.to_string()))?;

        for component in components {
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC | OFlags::NOFOLLOW;
            match openat(&directory, component, flags, Mode::empty()) {
                Ok(next) => directory = next,
                Err(Errno::NOENT) => {
                    match mkdirat(&directory, component, Mode::RUSR | Mode::WUSR | Mode::XUSR) {
                        Ok(()) | Err(Errno::EXIST) => {}
                        Err(error) => {
                            return Err(OutputError::UnsafeRoot(error.to_string()));
                        }
                    }
                    directory = openat(&directory, component, flags, Mode::empty())
                        .map_err(|error| OutputError::UnsafeRoot(error.to_string()))?;
                }
                Err(error) => return Err(OutputError::UnsafeRoot(error.to_string())),
            }
        }

        Ok(Self { directory })
    }

    pub fn write_atomic(&self, target: &Path, bytes: &[u8]) -> Result<(), OutputError> {
        self.write_atomic_observed(target, bytes, |_| {})
    }

    #[doc(hidden)]
    pub fn write_atomic_observed<F>(
        &self,
        target: &Path,
        bytes: &[u8],
        observer: F,
    ) -> Result<(), OutputError>
    where
        F: FnOnce(OutputObservation),
    {
        let target_name = direct_child(target)?;
        let mut original_target = self.target_identity(target_name)?;
        if original_target.is_some_and(|identity| identity.file_type.is_symlink()) {
            self.unlink_target(target_name, target)?;
            original_target = None;
        }

        let (temporary_name, temporary_fd) = self.create_temporary()?;
        let result = (|| {
            let mut temporary = File::from(temporary_fd);
            temporary
                .write_all(bytes)
                .and_then(|()| temporary.sync_all())
                .map_err(|error| OutputError::WriteFailed(error.to_string()))?;
            observer(OutputObservation::BeforeRename);
            if self.target_identity(target_name)? != original_target {
                self.remove_changed_target(target_name, target)?;
                return Err(OutputError::UnsafeTarget(target.to_path_buf()));
            }
            renameat(
                &self.directory,
                temporary_name.as_str(),
                &self.directory,
                target_name,
            )
            .map_err(|error| OutputError::RenameFailed(error.to_string()))?;
            fsync(&self.directory).map_err(|error| OutputError::WriteFailed(error.to_string()))?;
            Ok(())
        })();
        if result.is_err()
            && unlinkat(&self.directory, temporary_name.as_str(), AtFlags::empty()).is_ok()
        {
            let _ = fsync(&self.directory);
        }
        result
    }

    pub fn remove_if_exists(&self, target: &Path) -> Result<(), OutputError> {
        let target_name = direct_child(target)?;
        match self.target_identity(target_name)? {
            None => Ok(()),
            Some(identity) if identity.file_type.is_file() || identity.file_type.is_symlink() => {
                self.unlink_target(target_name, target)
            }
            Some(_) => Err(OutputError::UnsafeTarget(target.to_path_buf())),
        }
    }

    fn create_temporary(&self) -> Result<(String, OwnedFd), OutputError> {
        for _ in 0..128 {
            let mut random = [0_u8; 16];
            getrandom::getrandom(&mut random)
                .map_err(|error| OutputError::CreateFailed(error.to_string()))?;
            let mut random_hex = String::with_capacity(random.len() * 2);
            for byte in random {
                use std::fmt::Write as _;
                write!(&mut random_hex, "{byte:02x}")
                    .map_err(|error| OutputError::CreateFailed(error.to_string()))?;
            }
            let name = format!(".stage-gate-{random_hex}.tmp");
            match openat(
                &self.directory,
                name.as_str(),
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC | OFlags::NOFOLLOW,
                Mode::RUSR | Mode::WUSR,
            ) {
                Ok(fd) => return Ok((name, fd)),
                Err(Errno::EXIST) => {}
                Err(error) => return Err(OutputError::CreateFailed(error.to_string())),
            }
        }
        Err(OutputError::CreateFailed(
            "exclusive random temporary name attempts exhausted".to_owned(),
        ))
    }

    fn unlink_target(&self, target_name: &str, target: &Path) -> Result<(), OutputError> {
        unlinkat(&self.directory, target_name, AtFlags::empty()).map_err(|error| {
            OutputError::UnsafeTarget(PathBuf::from(format!("{}: {error}", target.display())))
        })?;
        fsync(&self.directory).map_err(|error| OutputError::WriteFailed(error.to_string()))
    }

    fn remove_changed_target(&self, target_name: &str, target: &Path) -> Result<(), OutputError> {
        match self.target_identity(target_name)? {
            None => Ok(()),
            Some(identity) if identity.file_type.is_file() || identity.file_type.is_symlink() => {
                self.unlink_target(target_name, target)
            }
            Some(_) => Err(OutputError::UnsafeTarget(target.to_path_buf())),
        }
    }

    fn target_identity(&self, target: &str) -> Result<Option<TargetIdentity>, OutputError> {
        match statat(&self.directory, target, AtFlags::SYMLINK_NOFOLLOW) {
            Ok(stat) => Ok(Some(TargetIdentity {
                device: stat.st_dev as u64,
                inode: stat.st_ino,
                file_type: FileType::from_raw_mode(stat.st_mode),
            })),
            Err(Errno::NOENT) => Ok(None),
            Err(error) => Err(OutputError::UnsafeTarget(PathBuf::from(format!(
                "{target}: {error}"
            )))),
        }
    }
}

fn safe_relative_components(path: &Path) -> Result<Vec<&str>, OutputError> {
    let lexical = path.as_os_str().as_bytes();
    if lexical.is_empty()
        || path.is_absolute()
        || lexical
            .split(|byte| *byte == b'/')
            .any(|component| component.is_empty() || component == b"." || component == b"..")
    {
        return Err(OutputError::UnsafeRoot(path.display().to_string()));
    }
    let mut safe = Vec::new();
    for component in path.components() {
        let Component::Normal(name) = component else {
            return Err(OutputError::UnsafeRoot(path.display().to_string()));
        };
        let Some(name) = name.to_str().filter(|name| !name.is_empty()) else {
            return Err(OutputError::UnsafeRoot(path.display().to_string()));
        };
        safe.push(name);
    }
    if safe.is_empty() {
        return Err(OutputError::UnsafeRoot(path.display().to_string()));
    }
    Ok(safe)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TargetIdentity {
    device: u64,
    inode: u64,
    file_type: FileType,
}

fn direct_child(path: &Path) -> Result<&str, OutputError> {
    let mut components = path.components();
    let Some(Component::Normal(name)) = components.next() else {
        return Err(OutputError::UnsafeTarget(path.to_path_buf()));
    };
    if components.next().is_some() {
        return Err(OutputError::UnsafeTarget(path.to_path_buf()));
    }
    name.to_str()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| OutputError::UnsafeTarget(path.to_path_buf()))
}
