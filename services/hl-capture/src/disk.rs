use std::os::fd::OwnedFd;
use std::path::PathBuf;

use rustix::fs::{Mode, OFlags, fstatvfs, open};

pub trait DiskSpaceProbe: Send + Sync {
    fn minimum_available_bytes(&self) -> Result<u64, DiskReserveError>;
}

#[derive(Debug)]
pub struct FilesystemDiskSpaceProbe {
    directories: Vec<OwnedFd>,
}

impl FilesystemDiskSpaceProbe {
    pub fn open(directories: impl IntoIterator<Item = PathBuf>) -> Result<Self, DiskReserveError> {
        let directories = directories
            .into_iter()
            .map(|path| {
                open(
                    path,
                    OFlags::RDONLY | OFlags::CLOEXEC | OFlags::DIRECTORY | OFlags::NOFOLLOW,
                    Mode::empty(),
                )
                .map_err(|_| DiskReserveError::Probe)
            })
            .collect::<Result<Vec<_>, _>>()?;
        if directories.is_empty() {
            return Err(DiskReserveError::InvalidConfig);
        }
        Ok(Self { directories })
    }
}

impl DiskSpaceProbe for FilesystemDiskSpaceProbe {
    fn minimum_available_bytes(&self) -> Result<u64, DiskReserveError> {
        self.directories
            .iter()
            .map(|directory| {
                let statistics = fstatvfs(directory).map_err(|_| DiskReserveError::Probe)?;
                let fragment_size = if statistics.f_frsize == 0 {
                    statistics.f_bsize
                } else {
                    statistics.f_frsize
                };
                statistics
                    .f_bavail
                    .checked_mul(fragment_size)
                    .ok_or(DiskReserveError::SizeOverflow)
            })
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .min()
            .ok_or(DiskReserveError::InvalidConfig)
    }
}

#[derive(Debug)]
pub struct DiskReserveGuard<P> {
    probe: P,
    reserve_bytes: u64,
}

impl<P: DiskSpaceProbe> DiskReserveGuard<P> {
    pub fn try_new(probe: P, reserve_bytes: u64) -> Result<Self, DiskReserveError> {
        if reserve_bytes == 0 {
            return Err(DiskReserveError::InvalidConfig);
        }
        Ok(Self {
            probe,
            reserve_bytes,
        })
    }

    pub fn ensure_write(
        &self,
        anticipated_write_bytes: u64,
    ) -> Result<DiskCapacity, DiskReserveError> {
        let available_bytes = self.probe.minimum_available_bytes()?;
        let required = self
            .reserve_bytes
            .checked_add(anticipated_write_bytes)
            .ok_or(DiskReserveError::SizeOverflow)?;
        if available_bytes < required {
            return Err(DiskReserveError::InsufficientSpace {
                available: available_bytes,
                required,
            });
        }
        Ok(DiskCapacity {
            available_bytes,
            remaining_after_write_bytes: available_bytes - anticipated_write_bytes,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiskCapacity {
    available_bytes: u64,
    remaining_after_write_bytes: u64,
}

impl DiskCapacity {
    #[must_use]
    pub const fn available_bytes(self) -> u64 {
        self.available_bytes
    }

    #[must_use]
    pub const fn remaining_after_write_bytes(self) -> u64 {
        self.remaining_after_write_bytes
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DiskReserveError {
    #[error("disk reserve policy configuration is invalid")]
    InvalidConfig,
    #[error("filesystem free space cannot be determined")]
    Probe,
    #[error("disk reserve size calculation overflowed")]
    SizeOverflow,
    #[error("filesystem cannot preserve the configured disk reserve")]
    InsufficientSpace { available: u64, required: u64 },
}

impl DiskReserveError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidConfig => "capture_disk.invalid_config",
            Self::Probe => "capture_disk.probe",
            Self::SizeOverflow => "capture_disk.size_overflow",
            Self::InsufficientSpace { .. } => "capture_disk.insufficient_space",
        }
    }
}
