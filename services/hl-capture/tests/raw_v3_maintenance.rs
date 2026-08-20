use std::sync::Mutex;

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, RawV3Archive};
use domain_types::{ChainId, KnownTime, SourceId};
use hl_capture::{
    CaptureHealth, DiskReserveError, DiskReserveGuard, DiskSpaceProbe, RawV3MaintenanceConfig,
    restore_authorized, run_maintenance_cycle,
};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    LocalRecordSequence, RawArchiveCapacityBudgets, RawArchiveWorkloadEnvelope,
    RawObservationArchive, RawObservationBatch,
};
use tempfile::TempDir;

#[derive(Debug)]
struct Probe {
    available: Mutex<Result<u64, DiskReserveError>>,
    free_basis_points: Mutex<Result<u16, DiskReserveError>>,
}

impl DiskSpaceProbe for Probe {
    fn minimum_available_bytes(&self) -> Result<u64, DiskReserveError> {
        *self.available.lock().unwrap()
    }

    fn minimum_free_basis_points(&self) -> Result<u16, DiskReserveError> {
        *self.free_basis_points.lock().unwrap()
    }
}

fn ample_disk() -> DiskReserveGuard<Probe> {
    DiskReserveGuard::try_new(
        Probe {
            available: Mutex::new(Ok(16 * 1024 * 1024 * 1024)),
            free_basis_points: Mutex::new(Ok(2_500)),
        },
        1,
    )
    .unwrap()
}

fn tight_disk() -> DiskReserveGuard<Probe> {
    DiskReserveGuard::try_new(
        Probe {
            available: Mutex::new(Ok(1)),
            free_basis_points: Mutex::new(Ok(2_500)),
        },
        1,
    )
    .unwrap()
}

fn observation(offset: u64, payload: &[u8]) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("node-fills").unwrap(),
        "capture-v1",
        ObservationClass::AuxiliaryLedger,
        SourceCursor::new("epoch-1", offset).unwrap(),
        ReceiveTimestamps::new(1_722_000_000_000_000, offset).unwrap(),
        "raw-parser-v1",
        Bytes::copy_from_slice(payload),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn batch(first_sequence: u64, offsets: &[u64], payload: &[u8]) -> RawObservationBatch {
    let observations = offsets
        .iter()
        .map(|offset| observation(*offset, payload))
        .collect();
    RawObservationBatch::try_new_byte_offsets(
        ChainId::new("mainnet").unwrap(),
        observations,
        [0x11; 32],
        [0x22; 32],
        LocalRecordSequence::try_new(first_sequence).unwrap(),
    )
    .unwrap()
}

fn open_archive(
    path: &std::path::Path,
    micros: i64,
    retention_horizon_seconds: u64,
) -> RawV3Archive {
    let workload = RawArchiveWorkloadEnvelope::try_new(
        100,
        1,
        1_000,
        retention_horizon_seconds,
        1_024,
        1_000,
        64 * 1024 * 1024,
        64,
    )
    .unwrap();
    let budgets =
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true).unwrap();
    RawV3Archive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            "capture-v3-maintenance-test",
            KnownTime::from_unix_micros(micros).unwrap(),
        )
        .unwrap(),
        workload,
        budgets,
    )
    .unwrap()
}

fn packing_config() -> RawV3MaintenanceConfig {
    RawV3MaintenanceConfig::default()
        .with_keep_uncompacted_tail_leaves(0)
        .unwrap()
}

fn append_three_commits(archive: &RawV3Archive) {
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive.append_batch(&batch(3, &[30], b"ef")).unwrap();
}

#[test]
fn cycle_packs_index_and_logical_range_then_scrubs() {
    let root = TempDir::new().unwrap();
    let archive = open_archive(root.path(), 1_722_000_000_000_000, 3_600);
    append_three_commits(&archive);
    let before = archive
        .maintenance_statistics(
            &ChainId::new("mainnet").unwrap(),
            &SourceId::new("node-fills").unwrap(),
        )
        .unwrap();
    assert_eq!(before.pending_pack_manifest_count(), 3);
    assert_eq!(before.packed_range_count(), 0);

    let report = run_maintenance_cycle(&archive, &packing_config(), &ample_disk(), 1);
    let status = report.status();
    assert_eq!(status.health(), CaptureHealth::Yellow);
    assert_eq!(
        status.reason_code(),
        Some("capture_maintenance.retention_unauthorized")
    );
    assert!(!status.retention_authorized());
    assert!(status.last_pack_index_at_micros().is_some());
    assert!(status.last_pack_data_at_micros().is_some());
    assert!(status.last_scrub_at_micros().is_some());
    assert_eq!(status.pending_pack_manifest_count(), 0);
    assert_eq!(status.packed_range_count(), 1);
}

#[test]
fn kill_switch_file_skips_mutating_work() {
    let root = TempDir::new().unwrap();
    let archive = open_archive(root.path(), 1_722_000_000_000_000, 3_600);
    append_three_commits(&archive);
    let switch = root.path().join("maintenance.off");
    std::fs::write(&switch, b"off").unwrap();
    let config = packing_config()
        .with_kill_switch_path(Some(switch))
        .unwrap();

    let report = run_maintenance_cycle(&archive, &config, &ample_disk(), 1);
    assert!(report.status().kill_switch());
    assert_eq!(report.status().health(), CaptureHealth::Green);
    assert!(report.status().last_pack_data_at_micros().is_none());
    let stats = archive
        .maintenance_statistics(
            &ChainId::new("mainnet").unwrap(),
            &SourceId::new("node-fills").unwrap(),
        )
        .unwrap();
    assert_eq!(stats.pending_pack_manifest_count(), 3);
}

#[test]
fn disk_reserve_blocks_packing_without_failing_closed_on_capture() {
    let root = TempDir::new().unwrap();
    let archive = open_archive(root.path(), 1_722_000_000_000_000, 3_600);
    append_three_commits(&archive);

    let report = run_maintenance_cycle(&archive, &packing_config(), &tight_disk(), 1);
    assert_eq!(report.status().health(), CaptureHealth::Yellow);
    assert_eq!(
        report.status().reason_code(),
        Some("capture_disk.insufficient_space")
    );
    assert_eq!(report.status().pending_pack_manifest_count(), 3);
}

#[test]
fn authorized_retention_unlinks_only_with_backup_receipt() {
    let root = TempDir::new().unwrap();
    let archive = open_archive(root.path(), 1_722_000_000_000_000, 1);
    append_three_commits(&archive);
    run_maintenance_cycle(&archive, &packing_config(), &ample_disk(), 1);
    drop(archive);

    let unauthorized = open_archive(root.path(), 1_722_000_001_000_000, 1);
    let blocked = run_maintenance_cycle(&unauthorized, &packing_config(), &ample_disk(), 1);
    assert_eq!(
        blocked.status().reason_code(),
        Some("capture_maintenance.retention_unauthorized")
    );
    assert!(blocked.status().last_retention_at_micros().is_none());
    drop(unauthorized);

    let authorized = open_archive(root.path(), 1_722_000_002_000_000, 1);
    let config = packing_config()
        .with_backup_receipt_sha256(Some(hex::encode([0xAB; 32])))
        .unwrap();
    let report = run_maintenance_cycle(&authorized, &config, &ample_disk(), 1);
    assert!(report.status().retention_authorized());
    assert!(report.status().last_retention_at_micros().is_some());
    assert_eq!(report.status().health(), CaptureHealth::Green);
}

#[test]
fn restore_hook_fail_closes_without_a_nonzero_backup_receipt() {
    let root = TempDir::new().unwrap();
    let archive = open_archive(root.path(), 1_722_000_000_000_000, 1);
    let error = restore_authorized(
        &archive,
        &ChainId::new("mainnet").unwrap(),
        &SourceId::new("node-fills").unwrap(),
        [0x11; 32],
        [0; 32],
        root.path(),
    )
    .expect_err("zero backup receipt must fail closed");
    assert_eq!(error.reason_code(), "archive.invalid_input");
}

#[test]
fn on_demand_cycle_is_bounded_and_does_not_claim_soak_proof() {
    let root = TempDir::new().unwrap();
    let archive = open_archive(root.path(), 1_722_000_000_000_000, 3_600);
    append_three_commits(&archive);
    let report = run_maintenance_cycle(&archive, &packing_config(), &ample_disk(), 1);
    assert!(report.status().last_scrub_at_micros().is_some());
    assert_ne!(
        report.status().reason_code(),
        Some("capture_soak.runtime_proven")
    );
}
