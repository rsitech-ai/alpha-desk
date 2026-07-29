use std::sync::Mutex;

use hl_capture::{DiskReserveError, DiskReserveGuard, DiskSpaceProbe, FilesystemDiskSpaceProbe};

#[derive(Debug)]
struct Probe {
    available: Mutex<Result<u64, DiskReserveError>>,
}

impl DiskSpaceProbe for Probe {
    fn minimum_available_bytes(&self) -> Result<u64, DiskReserveError> {
        *self.available.lock().unwrap()
    }
}

fn guard(available: Result<u64, DiskReserveError>) -> DiskReserveGuard<Probe> {
    DiskReserveGuard::try_new(
        Probe {
            available: Mutex::new(available),
        },
        1_000,
    )
    .unwrap()
}

#[test]
fn exact_reserve_plus_write_boundary_is_allowed() {
    let capacity = guard(Ok(1_250)).ensure_write(250).unwrap();

    assert_eq!(capacity.available_bytes(), 1_250);
    assert_eq!(capacity.remaining_after_write_bytes(), 1_000);
}

#[test]
fn one_byte_below_reserve_plus_write_fails_closed() {
    let error = guard(Ok(1_249)).ensure_write(250).unwrap_err();

    assert!(matches!(
        error,
        DiskReserveError::InsufficientSpace {
            available: 1_249,
            required: 1_250
        }
    ));
    assert_eq!(error.reason_code(), "capture_disk.insufficient_space");
}

#[test]
fn overflow_and_probe_failure_have_stable_distinct_failures() {
    let overflow = guard(Ok(u64::MAX)).ensure_write(u64::MAX).unwrap_err();
    assert!(matches!(overflow, DiskReserveError::SizeOverflow));
    assert_eq!(overflow.reason_code(), "capture_disk.size_overflow");

    let probe = guard(Err(DiskReserveError::Probe));
    assert!(matches!(
        probe.ensure_write(1).unwrap_err(),
        DiskReserveError::Probe
    ));
}

#[test]
fn filesystem_probe_reads_an_open_directory_without_following_a_symlink() {
    let root = tempfile::tempdir().unwrap();
    let probe = FilesystemDiskSpaceProbe::open([root.path().to_path_buf()]).unwrap();
    assert!(probe.minimum_available_bytes().unwrap() > 0);

    let alias = root.path().join("alias");
    std::os::unix::fs::symlink(root.path(), &alias).unwrap();
    let error = FilesystemDiskSpaceProbe::open([alias.clone()]).unwrap_err();
    assert!(matches!(error, DiskReserveError::Probe));
    std::fs::remove_file(alias).unwrap();
}
