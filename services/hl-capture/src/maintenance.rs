use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use canonical_archive::{RawArchiveRetentionRequest, RawV3Archive, UncompactedLogicalLeafV3};
use domain_types::{ChainId, KnownTime, SourceId};
use storage_ports::{ArchiveError, LocalRecordSequenceRange, RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES};
use tokio::time::{MissedTickBehavior, interval};
use tokio_util::sync::CancellationToken;

use crate::app::CaptureRuntimeHealth;
use crate::{
    AppError, CaptureHealth, CaptureMaintenanceStatus, DiskReserveGuard, DiskSpaceProbe,
    FilesystemDiskSpaceProbe, OwnedTask, RawV3MaintenanceConfig,
};

const TASK_NAME: &str = "raw-v3-maintenance";
const RESTORE_TASK: &str = "raw-v3-restore";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MaintenanceCycleReport {
    status: CaptureMaintenanceStatus,
}

impl MaintenanceCycleReport {
    #[must_use]
    pub const fn status(&self) -> &CaptureMaintenanceStatus {
        &self.status
    }
}

pub(crate) fn maintenance_task<P>(
    archive: Arc<RawV3Archive>,
    config: RawV3MaintenanceConfig,
    disk: DiskReserveGuard<P>,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> OwnedTask
where
    P: DiskSpaceProbe + 'static,
{
    OwnedTask::new(TASK_NAME, async move {
        let disk = Arc::new(disk);
        let mut heartbeat = interval(Duration::from_millis(config.interval_millis()));
        heartbeat.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let mut cycle_index = 0_u64;
        loop {
            tokio::select! {
                () = cancellation.cancelled() => return Ok(()),
                _ = heartbeat.tick() => {
                    cycle_index = cycle_index.saturating_add(1);
                    let archive = Arc::clone(&archive);
                    let config = config.clone();
                    let disk = Arc::clone(&disk);
                    let report = tokio::task::spawn_blocking(move || {
                        run_maintenance_cycle(archive.as_ref(), &config, disk.as_ref(), cycle_index)
                    })
                    .await
                    .map_err(|_| AppError::TaskPanicked { task: TASK_NAME })?;
                    health.record_maintenance(report.status);
                }
            }
        }
    })
}

pub(crate) fn filesystem_maintenance_task(
    archive: Arc<RawV3Archive>,
    config: RawV3MaintenanceConfig,
    archive_path: PathBuf,
    disk_reserve_bytes: u64,
    health: Arc<CaptureRuntimeHealth>,
    cancellation: CancellationToken,
) -> Result<OwnedTask, AppError> {
    let disk = filesystem_disk_guard(archive_path, disk_reserve_bytes)?;
    Ok(maintenance_task(
        archive,
        config,
        disk,
        health,
        cancellation,
    ))
}

pub fn run_configured_maintenance_cycle(
    capture: &crate::CaptureConfig,
) -> Result<MaintenanceCycleReport, AppError> {
    let runtime = capture.runtime();
    if runtime.raw_archive_format() != crate::RawArchiveFormat::V3 {
        return Err(AppError::TaskFailed {
            task: TASK_NAME,
            reason_code: "capture_maintenance.v2_has_no_maintenance",
        });
    }
    let raw_v3 = runtime.raw_v3().ok_or(AppError::TaskFailed {
        task: TASK_NAME,
        reason_code: "capture_config.missing_raw_v3_capacity",
    })?;
    let archive_config = canonical_archive::ArchiveConfig::production("hl-capture/maintain")
        .map_err(|_| AppError::TaskFailed {
            task: TASK_NAME,
            reason_code: "capture_connect.archive",
        })?;
    let workload = raw_v3.workload().map_err(|_| AppError::TaskFailed {
        task: TASK_NAME,
        reason_code: "capture_config.invalid_raw_v3_capacity",
    })?;
    let budgets = raw_v3.budgets().map_err(|_| AppError::TaskFailed {
        task: TASK_NAME,
        reason_code: "capture_config.invalid_raw_v3_capacity",
    })?;
    let archive = RawV3Archive::open(runtime.archive_path(), archive_config, workload, budgets)
        .map_err(|_| AppError::TaskFailed {
            task: TASK_NAME,
            reason_code: "capture_connect.archive",
        })?;
    let disk = filesystem_disk_guard(
        runtime.archive_path().to_path_buf(),
        runtime.disk_reserve_bytes(),
    )?;
    Ok(run_maintenance_cycle(
        &archive,
        raw_v3.maintenance(),
        &disk,
        1,
    ))
}

fn filesystem_disk_guard(
    archive_path: PathBuf,
    disk_reserve_bytes: u64,
) -> Result<DiskReserveGuard<FilesystemDiskSpaceProbe>, AppError> {
    let probe =
        FilesystemDiskSpaceProbe::open([archive_path]).map_err(|_| AppError::TaskFailed {
            task: TASK_NAME,
            reason_code: "capture_disk.probe",
        })?;
    DiskReserveGuard::try_new(probe, disk_reserve_bytes).map_err(|_| AppError::TaskFailed {
        task: TASK_NAME,
        reason_code: "capture_disk.invalid_config",
    })
}

pub fn run_maintenance_cycle<P: DiskSpaceProbe>(
    archive: &RawV3Archive,
    config: &RawV3MaintenanceConfig,
    disk: &DiskReserveGuard<P>,
    cycle_index: u64,
) -> MaintenanceCycleReport {
    let mut status = CaptureMaintenanceStatus::idle(config.enabled(), false);
    let backup_receipt = match config.backup_receipt() {
        Ok(receipt) => {
            status.set_retention_authorized(receipt.is_some());
            receipt
        }
        Err(_) => {
            status.degrade(
                CaptureHealth::Red,
                "capture_maintenance.retention_unauthorized",
            );
            None
        }
    };
    match kill_switch_latched(config.kill_switch_path()) {
        Ok(true) => {
            status = CaptureMaintenanceStatus::idle(config.enabled(), true);
            status.set_retention_authorized(backup_receipt.is_some());
            collect_statistics(archive, &mut status);
            return MaintenanceCycleReport { status };
        }
        Ok(false) => {}
        Err(reason) => {
            status.degrade(CaptureHealth::Yellow, reason);
            collect_statistics(archive, &mut status);
            return MaintenanceCycleReport { status };
        }
    }
    if let Err(reason) = ensure_disk_reserve(disk) {
        status.degrade(CaptureHealth::Yellow, reason);
        collect_statistics(archive, &mut status);
        return MaintenanceCycleReport { status };
    }
    let sources = match archive.list_sources() {
        Ok(sources) => sources,
        Err(error) => {
            status.degrade(CaptureHealth::Red, maintenance_archive_reason(&error));
            return MaintenanceCycleReport { status };
        }
    };
    let should_scrub = cycle_index.is_multiple_of(config.scrub_every_cycles());
    let mut remaining_index_packs = config.max_index_packs_per_cycle();
    let mut remaining_data_packs = config.max_pack_ranges_per_cycle();
    for (chain, source) in sources {
        maintain_source(
            archive,
            config,
            backup_receipt,
            should_scrub,
            &chain,
            &source,
            &mut remaining_index_packs,
            &mut remaining_data_packs,
            &mut status,
        );
    }
    refresh_counts(archive, &mut status);
    if status.packed_range_count() > 0 && backup_receipt.is_none() {
        status.degrade(
            CaptureHealth::Yellow,
            "capture_maintenance.retention_unauthorized",
        );
    }
    MaintenanceCycleReport { status }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthorizedRestoreRequest {
    dest: PathBuf,
    backup_root: PathBuf,
    chain: ChainId,
    source: SourceId,
    plan_digest: [u8; 32],
    backup_receipt: [u8; 32],
}

impl AuthorizedRestoreRequest {
    pub fn try_new(
        dest: impl Into<PathBuf>,
        backup_root: impl Into<PathBuf>,
        chain: ChainId,
        source: SourceId,
        plan_digest: [u8; 32],
        backup_receipt: [u8; 32],
    ) -> Result<Self, AppError> {
        let dest = dest.into();
        let backup_root = backup_root.into();
        require_operator_directory(
            &dest,
            "capture_restore.dest_missing",
            "capture_restore.dest_unsafe",
        )?;
        require_operator_directory(
            &backup_root,
            "capture_restore.backup_missing",
            "capture_restore.backup_unsafe",
        )?;
        if same_path(&dest, &backup_root) {
            return Err(restore_failed("capture_restore.backup_is_dest"));
        }
        Ok(Self {
            dest,
            backup_root,
            chain,
            source,
            plan_digest,
            backup_receipt,
        })
    }

    #[must_use]
    pub fn dest(&self) -> &Path {
        &self.dest
    }

    #[must_use]
    pub fn backup_root(&self) -> &Path {
        &self.backup_root
    }

    #[must_use]
    pub const fn plan_digest(&self) -> [u8; 32] {
        self.plan_digest
    }

    #[must_use]
    pub const fn backup_receipt(&self) -> [u8; 32] {
        self.backup_receipt
    }
}

pub fn restore_authorized(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    plan_digest: [u8; 32],
    backup_receipt: [u8; 32],
    backup_root: impl AsRef<Path>,
) -> Result<canonical_archive::RawArchiveRestoreReceipt, ArchiveError> {
    if backup_receipt == [0; 32] {
        return Err(ArchiveError::InvalidInput(
            "restore backup receipt must be a nonzero digest",
        ));
    }
    archive.restore_planned_files_from_backup(
        chain,
        source,
        plan_digest,
        backup_receipt,
        backup_root,
    )
}

pub fn run_configured_restore(
    capture: &crate::CaptureConfig,
    request: &AuthorizedRestoreRequest,
) -> Result<canonical_archive::RawArchiveRestoreReceipt, AppError> {
    let runtime = capture.runtime();
    if runtime.raw_archive_format() != crate::RawArchiveFormat::V3 {
        return Err(restore_failed("capture_restore.v2_has_no_restore"));
    }
    if same_path(request.dest(), runtime.archive_path()) {
        return Err(restore_failed("capture_restore.live_current_refused"));
    }
    let raw_v3 = runtime
        .raw_v3()
        .ok_or(restore_failed("capture_config.missing_raw_v3_capacity"))?;
    let archive_config = canonical_archive::ArchiveConfig::production("hl-capture/restore")
        .map_err(|_| restore_failed("capture_connect.archive"))?;
    let workload = raw_v3
        .workload()
        .map_err(|_| restore_failed("capture_config.invalid_raw_v3_capacity"))?;
    let budgets = raw_v3
        .budgets()
        .map_err(|_| restore_failed("capture_config.invalid_raw_v3_capacity"))?;
    let archive = RawV3Archive::open(request.dest(), archive_config, workload, budgets)
        .map_err(|error| restore_failed(error.reason_code()))?;
    restore_authorized(
        &archive,
        &request.chain,
        &request.source,
        request.plan_digest,
        request.backup_receipt,
        request.backup_root(),
    )
    .map_err(|error| restore_failed(error.reason_code()))
}

#[allow(clippy::too_many_arguments)]
fn maintain_source(
    archive: &RawV3Archive,
    config: &RawV3MaintenanceConfig,
    backup_receipt: Option<[u8; 32]>,
    should_scrub: bool,
    chain: &ChainId,
    source: &SourceId,
    remaining_index_packs: &mut u64,
    remaining_data_packs: &mut u64,
    status: &mut CaptureMaintenanceStatus,
) {
    match archive.maintenance_statistics(chain, source) {
        Ok(stats) => {
            if let Err(rejection) =
                archive
                    .workload()
                    .validate_backlog(stats.pending_pack_manifest_count(), 0, 0)
            {
                status.degrade(CaptureHealth::Red, rejection.reason_code());
            }
        }
        Err(ArchiveError::RangeUnavailable) => {}
        Err(error) => {
            status.degrade(CaptureHealth::Red, maintenance_archive_reason(&error));
            return;
        }
    }
    if *remaining_index_packs > 0 {
        match archive.pack_index(chain, source) {
            Ok(_) => {
                *remaining_index_packs = remaining_index_packs.saturating_sub(1);
                if let Some(now) = unix_micros() {
                    status.set_last_pack_index_at_micros(now);
                }
            }
            Err(error) => status.degrade(CaptureHealth::Yellow, maintenance_archive_reason(&error)),
        }
    }
    if *remaining_data_packs > 0 {
        match pack_data_ranges(
            archive,
            chain,
            source,
            config.keep_uncompacted_tail_leaves(),
            remaining_data_packs,
        ) {
            Ok(packed) => {
                if packed > 0
                    && let Some(now) = unix_micros()
                {
                    status.set_last_pack_data_at_micros(now);
                }
            }
            Err(error) => status.degrade(CaptureHealth::Yellow, maintenance_archive_reason(&error)),
        }
    }
    if should_scrub {
        match archive.scrub(chain, source) {
            Ok(_) => {
                if let Some(now) = unix_micros() {
                    status.set_last_scrub_at_micros(now);
                }
            }
            Err(error) => status.degrade(CaptureHealth::Red, maintenance_archive_reason(&error)),
        }
    }
    apply_retention(archive, chain, source, backup_receipt, status);
}

fn apply_retention(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    backup_receipt: Option<[u8; 32]>,
    status: &mut CaptureMaintenanceStatus,
) {
    let Some(backup_receipt) = backup_receipt else {
        return;
    };
    let plan = match archive.plan_packed_object_gc(chain, source, backup_receipt) {
        Ok(plan) => plan,
        Err(error) => {
            status.degrade(CaptureHealth::Yellow, maintenance_archive_reason(&error));
            return;
        }
    };
    if plan.is_empty() {
        return;
    }
    let request = match RawArchiveRetentionRequest::try_new(backup_receipt, plan.digest()) {
        Ok(request) => request,
        Err(_) => {
            status.degrade(
                CaptureHealth::Red,
                "capture_maintenance.retention_unauthorized",
            );
            return;
        }
    };
    match archive.apply_authorized_retention(chain, source, request) {
        Ok(_) => {
            if let Some(now) = unix_micros() {
                status.set_last_retention_at_micros(now);
            }
        }
        Err(error) => status.degrade(CaptureHealth::Red, maintenance_archive_reason(&error)),
    }
}

fn pack_data_ranges(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    keep_tail: u64,
    remaining_data_packs: &mut u64,
) -> Result<u64, ArchiveError> {
    let leaves = archive.pending_uncompacted_logical_leaves(chain, source)?;
    let ranges = packable_ranges(&leaves, keep_tail, *remaining_data_packs)?;
    let mut packed = 0_u64;
    for range in ranges {
        archive.pack_logical_range(chain, source, range)?;
        packed = packed.saturating_add(1);
        *remaining_data_packs = remaining_data_packs.saturating_sub(1);
        if *remaining_data_packs == 0 {
            break;
        }
    }
    Ok(packed)
}

fn packable_ranges(
    leaves: &[UncompactedLogicalLeafV3],
    keep_tail: u64,
    max_ranges: u64,
) -> Result<Vec<LocalRecordSequenceRange>, ArchiveError> {
    let mut groups: Vec<(String, Vec<LocalRecordSequenceRange>)> = Vec::new();
    for leaf in leaves {
        match groups.last_mut() {
            Some((partition, ranges))
                if partition == leaf.partition()
                    && ranges.last().is_some_and(|previous| {
                        previous.end().get().checked_add(1) == Some(leaf.range().start().get())
                    }) =>
            {
                ranges.push(leaf.range());
            }
            _ => groups.push((leaf.partition().to_owned(), vec![leaf.range()])),
        }
    }
    if keep_tail > 0
        && let Some((_, ranges)) = groups.last_mut()
    {
        let retain = ranges
            .len()
            .saturating_sub(usize::try_from(keep_tail).unwrap_or(usize::MAX));
        ranges.truncate(retain);
        if ranges.len() < 2 {
            groups.pop();
        }
    }
    let mut selected = Vec::new();
    for (_, ranges) in groups {
        if ranges.len() < 2 || u64::try_from(selected.len()).unwrap_or(u64::MAX) >= max_ranges {
            continue;
        }
        let Some(first) = ranges.first() else {
            continue;
        };
        let Some(last) = ranges.last() else {
            continue;
        };
        selected.push(LocalRecordSequenceRange::try_new(
            first.start(),
            last.end(),
        )?);
    }
    Ok(selected)
}

fn refresh_counts(archive: &RawV3Archive, status: &mut CaptureMaintenanceStatus) {
    status.set_counts(0, 0, 0, 0);
    collect_statistics(archive, status);
}

fn collect_statistics(archive: &RawV3Archive, status: &mut CaptureMaintenanceStatus) {
    let Ok(sources) = archive.list_sources() else {
        return;
    };
    for (chain, source) in sources {
        if let Ok(stats) = archive.maintenance_statistics(&chain, &source) {
            add_counts(status, stats);
        }
    }
}

fn add_counts(
    status: &mut CaptureMaintenanceStatus,
    stats: storage_ports::RawArchiveMaintenanceStatistics,
) {
    status.set_counts(
        status
            .pending_pack_manifest_count()
            .saturating_add(stats.pending_pack_manifest_count()),
        status
            .packed_range_count()
            .saturating_add(stats.packed_range_count()),
        status
            .logical_manifest_count()
            .saturating_add(stats.logical_manifest_count()),
        status
            .physical_data_object_count()
            .saturating_add(stats.physical_data_object_count()),
    );
}

fn ensure_disk_reserve<P: DiskSpaceProbe>(disk: &DiskReserveGuard<P>) -> Result<(), &'static str> {
    disk.ensure_write(RAW_ARCHIVE_MAXIMUM_DATA_PACK_BYTES)
        .map(|_| ())
        .map_err(|error| error.reason_code())
}

fn kill_switch_latched(path: Option<&Path>) -> Result<bool, &'static str> {
    let Some(path) = path else {
        return Ok(false);
    };
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(_) => Err("capture_maintenance.kill_switch_unsafe"),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err("capture_maintenance.kill_switch_unsafe")
        }
        Ok(_) => Ok(true),
    }
}

fn maintenance_archive_reason(error: &ArchiveError) -> &'static str {
    match error {
        ArchiveError::WriterBusy => "capture_maintenance.writer_busy",
        ArchiveError::Capacity(_) => error.reason_code(),
        _ => error.reason_code(),
    }
}

fn restore_failed(reason_code: &'static str) -> AppError {
    AppError::TaskFailed {
        task: RESTORE_TASK,
        reason_code,
    }
}

fn require_operator_directory(
    path: &Path,
    missing: &'static str,
    unsafe_code: &'static str,
) -> Result<(), AppError> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(restore_failed(unsafe_code));
    }
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == ErrorKind::NotFound => Err(restore_failed(missing)),
        Err(_) => Err(restore_failed(unsafe_code)),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(restore_failed(unsafe_code))
        }
        Ok(_) => Ok(()),
    }
}

fn same_path(left: &Path, right: &Path) -> bool {
    if left == right {
        return true;
    }
    match (fs::canonicalize(left), fs::canonicalize(right)) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

fn unix_micros() -> Option<i64> {
    let micros = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()?
        .as_micros();
    let micros = i64::try_from(micros).ok()?;
    KnownTime::from_unix_micros(micros)
        .ok()
        .map(KnownTime::unix_micros)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_owned_tasks;
    use storage_ports::{RawArchiveCapacityBudgets, RawArchiveWorkloadEnvelope};
    use tokio::time::timeout;

    #[derive(Debug)]
    struct Probe {
        available: std::sync::Mutex<Result<u64, crate::DiskReserveError>>,
        free_basis_points: std::sync::Mutex<Result<u16, crate::DiskReserveError>>,
    }

    impl DiskSpaceProbe for Probe {
        fn minimum_available_bytes(&self) -> Result<u64, crate::DiskReserveError> {
            *self.available.lock().unwrap()
        }

        fn minimum_free_basis_points(&self) -> Result<u16, crate::DiskReserveError> {
            *self.free_basis_points.lock().unwrap()
        }
    }

    #[tokio::test]
    async fn shutdown_joins_the_owned_maintenance_task() {
        let root = tempfile::tempdir().unwrap();
        let workload = RawArchiveWorkloadEnvelope::try_new(
            100,
            1,
            1_000,
            3_600,
            1_024,
            1_000,
            64 * 1024 * 1024,
            64,
        )
        .unwrap();
        let budgets =
            RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true)
                .unwrap();
        let archive = Arc::new(
            RawV3Archive::open(
                root.path(),
                canonical_archive::ArchiveConfig::deterministic_fixture(
                    "capture-v3-maintenance-shutdown",
                    KnownTime::from_unix_micros(1_000).unwrap(),
                )
                .unwrap(),
                workload,
                budgets,
            )
            .unwrap(),
        );
        let disk = DiskReserveGuard::try_new(
            Probe {
                available: std::sync::Mutex::new(Ok(16 * 1024 * 1024 * 1024)),
                free_basis_points: std::sync::Mutex::new(Ok(2_500)),
            },
            1,
        )
        .unwrap();
        let cancellation = CancellationToken::new();
        let health = Arc::new(CaptureRuntimeHealth::new());
        let config = RawV3MaintenanceConfig::default()
            .with_interval_millis(50)
            .unwrap();
        let task = maintenance_task(archive, config, disk, health, cancellation.child_token());
        let supervisor = tokio::spawn(run_owned_tasks(
            cancellation.clone(),
            Duration::from_secs(1),
            vec![task],
        ));
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancellation.cancel();
        timeout(Duration::from_secs(1), supervisor)
            .await
            .expect("maintenance task joined before timeout")
            .expect("supervisor task joined")
            .expect("clean maintenance shutdown");
    }
}
