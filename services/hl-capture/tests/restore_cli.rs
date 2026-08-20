use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, RawV3Archive};
use domain_types::{ChainId, KnownTime, SourceId};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    LocalRecordSequence, LocalRecordSequenceRange, RawArchiveCapacityBudgets,
    RawArchiveWorkloadEnvelope, RawObservationArchive, RawObservationBatch,
};
use tempfile::TempDir;

const FIXTURE_TIME_MICROS: i64 = 1_722_000_000_000_000;
const BACKUP_RECEIPT: [u8; 32] = [0xAB; 32];
const ZERO_DIGEST: [u8; 32] = [0; 32];

fn binary() -> Command {
    Command::new(env!("CARGO_BIN_EXE_hl-capture"))
}

fn observation(offset: u64, payload: &[u8]) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("node-fills").unwrap(),
        "capture-v1",
        ObservationClass::AuxiliaryLedger,
        SourceCursor::new("epoch-1", offset).unwrap(),
        ReceiveTimestamps::new(FIXTURE_TIME_MICROS, offset).unwrap(),
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

fn open_retention_archive(path: &Path, micros: i64) -> RawV3Archive {
    let workload =
        RawArchiveWorkloadEnvelope::try_new(100, 1, 1_000, 1, 1_024, 1_000, 64 * 1024 * 1024, 64)
            .unwrap();
    let budgets =
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true).unwrap();
    RawV3Archive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            "capture-v3-restore-cli",
            KnownTime::from_unix_micros(micros).unwrap(),
        )
        .unwrap(),
        workload,
        budgets,
    )
    .unwrap()
}

fn dataset_dir(root: &Path) -> PathBuf {
    root.join("chain=mainnet")
        .join(format!(
            "dataset={}",
            canonical_archive::raw_v3::RAW_BYTE_DATASET_V3
        ))
        .join("source=node-fills")
}

fn current_pointer(root: &Path) -> PathBuf {
    dataset_dir(root).join("CURRENT")
}

fn v3_config(live_archive: &Path) -> String {
    let with_path = include_str!("../../../config/capture.example.toml").replace(
        "archive_path = \"state/canonical-archive\"",
        &format!("archive_path = \"{}\"", live_archive.display()),
    );
    let with_v3 = with_path.replace(
        "disk_reserve_bytes = 10737418240",
        "disk_reserve_bytes = 10737418240\nraw_archive_format = \"v3\"",
    );
    format!(
        "{with_v3}

[runtime.raw_v3]
maximum_records_per_second = 100
minimum_group_records = 1
maximum_group_delay_millis = 1000
retention_horizon_seconds = 3600
maximum_encoded_record_bytes = 1024
maximum_uncompacted_commits = 1000
maximum_eligible_bytes = 67108864
maximum_eligible_inodes = 64
raw_data_budget_bytes = 18446744073709551615
metadata_budget_bytes = 18446744073709551615
total_storage_budget_bytes = 18446744073709551615
inode_budget = 18446744073709551615
digest_confirmed_purge_workflow_configured = true
"
    )
}

struct RestoreFixture {
    backup: TempDir,
    plan_digest: [u8; 32],
    restored_relative: PathBuf,
    current_before: Vec<u8>,
}

fn prepare_isolated_dest(dest: &Path) -> RestoreFixture {
    assert!(
        dest.read_dir().expect("empty dest").next().is_none(),
        "restore dest must start empty"
    );
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-fills").unwrap();
    let archive = open_retention_archive(dest, FIXTURE_TIME_MICROS);
    archive.append_batch(&batch(1, &[10], b"ab")).unwrap();
    archive.append_batch(&batch(2, &[20], b"cd")).unwrap();
    archive
        .pack_logical_range(
            &chain,
            &source,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(2).unwrap(),
            )
            .unwrap(),
        )
        .unwrap();
    drop(archive);

    let later = open_retention_archive(dest, FIXTURE_TIME_MICROS + 1_000_000);
    let plan = later
        .plan_packed_object_gc(&chain, &source, BACKUP_RECEIPT)
        .unwrap();
    let (relative, object_sha256, byte_len) = plan
        .files()
        .find(|(path, _, _)| !path.contains("/leases/"))
        .map(|(path, hash, len)| (path.to_owned(), hash.to_owned(), len))
        .expect("eligible file");
    let backup = TempDir::new().unwrap();
    let backup_file = backup.path().join(&relative);
    fs::create_dir_all(backup_file.parent().unwrap()).unwrap();
    fs::copy(dest.join(&relative), &backup_file).unwrap();
    let journal = dataset_dir(dest)
        .join("gc")
        .join(format!("deletion-{}.log", hex::encode(plan.digest())));
    fs::create_dir_all(journal.parent().unwrap()).unwrap();
    fs::write(
        &journal,
        format!(
            "{{\"kind\":\"planned\",\"relative_path\":\"{relative}\",\"object_sha256\":\"{object_sha256}\",\"byte_len\":{byte_len}}}\n"
        ),
    )
    .unwrap();
    fs::remove_file(dest.join(&relative)).unwrap();
    let current_before = fs::read(current_pointer(dest)).unwrap();
    drop(later);
    RestoreFixture {
        backup,
        plan_digest: plan.digest(),
        restored_relative: PathBuf::from(relative),
        current_before,
    }
}

fn write_config(directory: &Path, live_archive: &Path) -> PathBuf {
    let config_path = directory.join("capture.toml");
    fs::write(&config_path, v3_config(live_archive)).unwrap();
    config_path
}

fn restore_args(
    config_path: &Path,
    dest: &Path,
    backup_root: &Path,
    plan_digest: [u8; 32],
    backup_receipt: Option<[u8; 32]>,
) -> Vec<String> {
    let mut args = vec![
        "restore".to_owned(),
        "--config".to_owned(),
        config_path.display().to_string(),
        "--dest".to_owned(),
        dest.display().to_string(),
        "--backup-root".to_owned(),
        backup_root.display().to_string(),
        "--chain".to_owned(),
        "mainnet".to_owned(),
        "--source".to_owned(),
        "node-fills".to_owned(),
        "--plan-digest".to_owned(),
        hex::encode(plan_digest),
        "--i-approve-restore".to_owned(),
    ];
    if let Some(receipt) = backup_receipt {
        args.push("--backup-receipt".to_owned());
        args.push(hex::encode(receipt));
    }
    args
}

fn reason_code(stderr: &[u8]) -> String {
    let stderr = String::from_utf8(stderr.to_vec()).expect("UTF-8 stderr");
    let value: serde_json::Value = serde_json::from_str(stderr.trim()).expect("error JSON");
    value["reason_code"]
        .as_str()
        .expect("reason_code")
        .to_owned()
}

#[test]
fn restore_without_operator_approval_or_dest_is_usage() {
    let output = binary()
        .args(["restore", "--config", "capture.toml"])
        .output()
        .expect("run restore");
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("UTF-8 stderr")
            .contains("usage: hl-capture")
    );
}

#[test]
fn restore_fail_closes_without_an_authorized_backup_receipt() {
    let root = TempDir::new().unwrap();
    let live = root.path().join("live");
    let dest = root.path().join("dest");
    fs::create_dir_all(&live).unwrap();
    fs::create_dir_all(&dest).unwrap();
    let marker = live.join("CURRENT");
    fs::write(&marker, b"live-current").unwrap();
    let config_path = write_config(root.path(), &live);
    let backup = TempDir::new().unwrap();

    let output = binary()
        .args(restore_args(
            &config_path,
            &dest,
            backup.path(),
            [0x11; 32],
            None,
        ))
        .output()
        .expect("run restore");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        reason_code(&output.stderr),
        "capture_restore.receipt_required"
    );
    assert_eq!(fs::read(&marker).unwrap(), b"live-current");
    assert!(dest.read_dir().unwrap().next().is_none());
}

#[test]
fn restore_fail_closes_on_backup_receipt_mismatch() {
    let root = TempDir::new().unwrap();
    let live = root.path().join("live");
    let dest = root.path().join("dest");
    fs::create_dir_all(&live).unwrap();
    fs::create_dir_all(&dest).unwrap();
    let fixture = prepare_isolated_dest(&dest);
    let config_path = write_config(root.path(), &live);
    let live_entries = fs::read_dir(&live).unwrap().count();

    let output = binary()
        .args(restore_args(
            &config_path,
            &dest,
            fixture.backup.path(),
            fixture.plan_digest,
            Some([0xCD; 32]),
        ))
        .output()
        .expect("run restore");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(reason_code(&output.stderr), "archive.manifest_verification");
    assert!(!dest.join(&fixture.restored_relative).is_file());
    assert_eq!(
        fs::read(current_pointer(&dest)).unwrap(),
        fixture.current_before
    );
    assert_eq!(fs::read_dir(&live).unwrap().count(), live_entries);
}

#[test]
fn restore_replays_backup_into_empty_dest_from_fixtures_without_touching_live_current() {
    let root = TempDir::new().unwrap();
    let live = root.path().join("live");
    let dest = root.path().join("dest");
    fs::create_dir_all(&live).unwrap();
    fs::create_dir_all(&dest).unwrap();
    let marker = live.join("CURRENT");
    fs::write(&marker, b"live-current").unwrap();
    let fixture = prepare_isolated_dest(&dest);
    let config_path = write_config(root.path(), &live);

    let output = binary()
        .args(restore_args(
            &config_path,
            &dest,
            fixture.backup.path(),
            fixture.plan_digest,
            Some(BACKUP_RECEIPT),
        ))
        .output()
        .expect("run restore");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).expect("restore JSON");
    assert_eq!(value["schema_version"], "hl.capture.restore.v1");
    assert_eq!(value["plan_digest"], hex::encode(fixture.plan_digest));
    assert_eq!(value["restored_files"], 1);
    assert!(dest.join(&fixture.restored_relative).is_file());
    assert_eq!(
        fs::read(current_pointer(&dest)).unwrap(),
        fixture.current_before
    );
    assert_eq!(fs::read(&marker).unwrap(), b"live-current");
}

#[test]
fn restore_refuses_the_live_current_archive_path() {
    let root = TempDir::new().unwrap();
    let live = root.path().join("live");
    fs::create_dir_all(&live).unwrap();
    let marker = live.join("CURRENT");
    fs::write(&marker, b"live-current").unwrap();
    let backup = TempDir::new().unwrap();
    let config_path = write_config(root.path(), &live);

    let output = binary()
        .args(restore_args(
            &config_path,
            &live,
            backup.path(),
            [0x11; 32],
            Some(BACKUP_RECEIPT),
        ))
        .output()
        .expect("run restore");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(
        reason_code(&output.stderr),
        "capture_restore.live_current_refused"
    );
    assert_eq!(fs::read(&marker).unwrap(), b"live-current");
}

#[test]
fn restore_fail_closes_on_a_zero_backup_receipt() {
    let root = TempDir::new().unwrap();
    let live = root.path().join("live");
    let dest = root.path().join("dest");
    fs::create_dir_all(&live).unwrap();
    fs::create_dir_all(&dest).unwrap();
    let fixture = prepare_isolated_dest(&dest);
    let config_path = write_config(root.path(), &live);

    let output = binary()
        .args(restore_args(
            &config_path,
            &dest,
            fixture.backup.path(),
            fixture.plan_digest,
            Some(ZERO_DIGEST),
        ))
        .output()
        .expect("run restore");

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert_eq!(reason_code(&output.stderr), "archive.invalid_input");
    assert!(!dest.join(&fixture.restored_relative).is_file());
    assert_eq!(
        fs::read(current_pointer(&dest)).unwrap(),
        fixture.current_before
    );
}
