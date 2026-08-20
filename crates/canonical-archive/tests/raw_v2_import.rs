use std::{
    fs,
    path::{Path, PathBuf},
};

use canonical_archive::{ArchiveConfig, LocalParquetArchive, RawArchiveCheckpoint, RawV3Archive};
use domain_types::{ChainId, KnownTime, SourceId};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    ArchiveError, LocalRecordSequence, LocalRecordSequenceRange, RawArchiveCapacityBudgets,
    RawArchiveWorkloadEnvelope, RawObservationArchive, RawObservationBatch,
};

fn observation(offset: u64, payload: &[u8]) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("node-fills").unwrap(),
        "capture-v1",
        ObservationClass::AuxiliaryLedger,
        SourceCursor::new("epoch-1", offset).unwrap(),
        ReceiveTimestamps::new(1_722_000_000_000_000, offset).unwrap(),
        "raw-parser-v1",
        bytes::Bytes::copy_from_slice(payload),
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

fn generous_capacity() -> (RawArchiveWorkloadEnvelope, RawArchiveCapacityBudgets) {
    (
        RawArchiveWorkloadEnvelope::try_new(
            100,
            1,
            1_000,
            3_600,
            1_024,
            1_000,
            64 * 1024 * 1024,
            64,
        )
        .unwrap(),
        RawArchiveCapacityBudgets::try_new(u64::MAX, u64::MAX, u64::MAX, u64::MAX, true).unwrap(),
    )
}

fn open_v2(path: &Path) -> LocalParquetArchive {
    LocalParquetArchive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            "raw-v2-import-test",
            KnownTime::from_unix_micros(1_722_000_000_000_000).unwrap(),
        )
        .unwrap(),
    )
    .unwrap()
}

fn open_v3(path: &Path) -> RawV3Archive {
    let (workload, budgets) = generous_capacity();
    RawV3Archive::open(
        path,
        ArchiveConfig::deterministic_fixture(
            "raw-v2-import-test",
            KnownTime::from_unix_micros(1_722_000_000_000_000).unwrap(),
        )
        .unwrap(),
        workload,
        budgets,
    )
    .unwrap()
}

fn chain() -> ChainId {
    ChainId::new("mainnet").unwrap()
}

fn source() -> SourceId {
    SourceId::new("node-fills").unwrap()
}

fn v2_current_relative() -> PathBuf {
    PathBuf::from("chain=mainnet/dataset=raw_source_observations_byte_v2/source=node-fills/CURRENT")
}

fn v3_current_relative() -> PathBuf {
    PathBuf::from("chain=mainnet/dataset=raw_source_observations_byte_v3/source=node-fills/CURRENT")
}

fn v3_import_relative() -> PathBuf {
    PathBuf::from("chain=mainnet/dataset=raw_source_observations_byte_v3/source=node-fills/IMPORT")
}

fn cutover_relative() -> PathBuf {
    PathBuf::from("chain=mainnet/dataset=raw_source_observations/source=node-fills/CUTOVER")
}

fn read_file(root: &Path, relative: &Path) -> Vec<u8> {
    fs::read(root.join(relative)).expect("read archive file")
}

fn file_exists(root: &Path, relative: &Path) -> bool {
    root.join(relative).is_file()
}

fn collect_parquets(dir: &Path, output: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_parquets(&path, output);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("parquet") {
            output.push(path);
        }
    }
}

fn v2_object_parquets(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_parquets(
        &root.join("chain=mainnet/dataset=raw_source_observations_byte_v2"),
        &mut files,
    );
    files
}

fn backup_dir() -> tempfile::TempDir {
    tempfile::tempdir().expect("backup root")
}

fn mutate_first_v2_parquet(root: &Path) {
    let mut files = Vec::new();
    collect_parquets(
        &root.join("chain=mainnet/dataset=raw_source_observations_byte_v2"),
        &mut files,
    );
    let path = files.first().expect("V2 parquet object");
    let mut bytes = fs::read(path).expect("read V2 parquet");
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    fs::write(path, bytes).expect("mutate V2 parquet");
}

fn replay_payloads(archive: &RawV3Archive, last: u64) -> Vec<Vec<u8>> {
    archive
        .read_observations_by_sequence(
            &chain(),
            &source(),
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(1).unwrap(),
                LocalRecordSequence::try_new(last).unwrap(),
            )
            .unwrap(),
        )
        .unwrap()
        .map(|item| item.unwrap().observation().payload().to_vec())
        .collect()
}

#[test]
fn plan_publish_approve_replays_v2_identically_without_mutating_v2() {
    let temporary = tempfile::tempdir().unwrap();
    let v2 = open_v2(temporary.path());
    let first = v2.append_batch(&batch(1, &[10, 11], b"ab")).unwrap();
    let second = v2.append_batch(&batch(3, &[20], b"cd")).unwrap();
    let v2_current_before = read_file(temporary.path(), &v2_current_relative());

    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).expect("plan");
    assert_eq!(plan.batches().len(), 2);
    assert_eq!(plan.first_local_sequence(), 1);
    assert_eq!(plan.last_local_sequence(), 3);

    let report = archive
        .publish_v2_import(&chain(), &source(), &plan)
        .expect("publish");
    assert_eq!(report.packed_logical_manifest_count(), 2);
    assert!(file_exists(temporary.path(), &v3_import_relative()));
    assert!(!file_exists(temporary.path(), &v3_current_relative()));
    assert_eq!(
        read_file(temporary.path(), &v2_current_relative()),
        v2_current_before
    );

    let republished = archive
        .publish_v2_import(&chain(), &source(), &plan)
        .expect("idempotent publish");
    assert_eq!(republished.v3_root_sha256(), report.v3_root_sha256());

    let approval = archive
        .approve_v2_import(&chain(), &source(), &plan)
        .expect("approve");
    assert_eq!(approval.v3_root_sha256(), report.v3_root_sha256());
    assert!(file_exists(temporary.path(), &v3_current_relative()));
    assert!(file_exists(temporary.path(), &cutover_relative()));
    assert_eq!(
        read_file(temporary.path(), &v2_current_relative()),
        v2_current_before
    );

    assert_eq!(
        replay_payloads(&archive, 3),
        [b"ab".to_vec(), b"ab".to_vec(), b"cd".to_vec()]
    );

    archive
        .verify_raw_manifest_at_sequence(first.manifest_id(), first.local_sequence_range().unwrap())
        .expect("original first V2 receipt verifies through packed import");
    archive
        .verify_raw_manifest_at_sequence(
            second.manifest_id(),
            second.local_sequence_range().unwrap(),
        )
        .expect("original second V2 receipt verifies through packed import");

    match archive.load_checkpoint(&chain(), &source()).unwrap() {
        Some(RawArchiveCheckpoint::V2(checkpoint)) => {
            assert_eq!(checkpoint.root_hash(), report.v3_root_sha256());
            assert_eq!(checkpoint.sha256(), approval.checkpoint_sha256());
        }
        other => panic!("expected checkpoint V2, got {other:?}"),
    }

    let again = archive
        .approve_v2_import(&chain(), &source(), &plan)
        .expect("idempotent approve");
    assert_eq!(again.checkpoint_sha256(), approval.checkpoint_sha256());
    assert_eq!(v2_object_parquets(temporary.path()).len(), 2);
}

#[test]
fn single_batch_import_allows_underfilled_pack() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"xy"))
        .unwrap();
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    let report = archive
        .publish_v2_import(&chain(), &source(), &plan)
        .unwrap();
    assert_eq!(report.pack_count(), 1);
    assert_eq!(report.packed_logical_manifest_count(), 1);
    archive
        .approve_v2_import(&chain(), &source(), &plan)
        .unwrap();
    assert_eq!(replay_payloads(&archive, 1), [b"xy".to_vec()]);
}

#[test]
fn mutated_v2_object_after_plan_fails_closed_without_current_switch() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(2, &[20], b"cd"))
        .unwrap();
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    mutate_first_v2_parquet(temporary.path());
    let error = archive
        .publish_v2_import(&chain(), &source(), &plan)
        .expect_err("mutated V2 object must fail closed");
    assert!(matches!(
        error,
        ArchiveError::ManifestVerification(_) | ArchiveError::CorruptObject(_)
    ));
    assert!(!file_exists(temporary.path(), &v3_current_relative()));
    assert!(!file_exists(temporary.path(), &cutover_relative()));
}

#[test]
fn extra_v2_append_after_plan_fails_closed() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(2, &[20], b"cd"))
        .unwrap();
    let error = archive
        .publish_v2_import(&chain(), &source(), &plan)
        .expect_err("extra V2 append must fail closed");
    assert!(matches!(error, ArchiveError::ManifestVerification(_)));
    assert!(!file_exists(temporary.path(), &v3_current_relative()));
}

#[test]
fn failed_import_never_publishes_v3_current() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    mutate_first_v2_parquet(temporary.path());
    let _ = archive.publish_v2_import(&chain(), &source(), &plan);
    let _ = archive.approve_v2_import(&chain(), &source(), &plan);
    assert!(!file_exists(temporary.path(), &v3_current_relative()));
}

#[test]
fn v2_originals_stay_unreclaimed_until_import_is_approved() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(2, &[20], b"cd"))
        .unwrap();
    let v2_current_before = read_file(temporary.path(), &v2_current_relative());
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    archive
        .publish_v2_import(&chain(), &source(), &plan)
        .unwrap();
    let backup = backup_dir();

    let backup_error = archive
        .backup_v2_import_originals(&chain(), &source(), backup.path())
        .expect_err("backup before approve must fail closed");
    assert!(matches!(
        backup_error,
        ArchiveError::ManifestVerification(_)
    ));
    let plan_error = archive
        .plan_v2_import_reclaim(&chain(), &source(), [0xAB; 32])
        .expect_err("reclaim plan before approve must fail closed");
    assert!(matches!(plan_error, ArchiveError::ManifestVerification(_)));
    let execute_error = archive
        .execute_v2_import_reclaim(&chain(), &source(), [0xAB; 32], [0xCD; 32])
        .expect_err("reclaim execute before approve must fail closed");
    assert!(matches!(
        execute_error,
        ArchiveError::ManifestVerification(_) | ArchiveError::InvalidInput(_)
    ));

    assert_eq!(v2_object_parquets(temporary.path()).len(), 2);
    assert_eq!(
        read_file(temporary.path(), &v2_current_relative()),
        v2_current_before
    );
    assert!(!file_exists(temporary.path(), &v3_current_relative()));
}

#[test]
fn random_hex_backup_receipt_fails_closed_without_unlinking_v2_originals() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    let v2_current_before = read_file(temporary.path(), &v2_current_relative());
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    archive
        .publish_v2_import(&chain(), &source(), &plan)
        .unwrap();
    archive
        .approve_v2_import(&chain(), &source(), &plan)
        .unwrap();

    let error = archive
        .plan_v2_import_reclaim(&chain(), &source(), [0xAB; 32])
        .expect_err("random hex must not authorize reclaim");
    assert!(matches!(
        error,
        ArchiveError::ManifestVerification(_) | ArchiveError::InvalidInput(_)
    ));
    let execute_error = archive
        .execute_v2_import_reclaim(&chain(), &source(), [0xAB; 32], [0xCD; 32])
        .expect_err("random hex must not unlink");
    assert!(matches!(
        execute_error,
        ArchiveError::ManifestVerification(_) | ArchiveError::InvalidInput(_)
    ));

    assert_eq!(v2_object_parquets(temporary.path()).len(), 1);
    assert_eq!(
        read_file(temporary.path(), &v2_current_relative()),
        v2_current_before
    );
}

#[test]
fn reclaim_receipt_mismatch_fails_closed_without_unlinking() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    let v2_current_before = read_file(temporary.path(), &v2_current_relative());
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    archive
        .publish_v2_import(&chain(), &source(), &plan)
        .unwrap();
    archive
        .approve_v2_import(&chain(), &source(), &plan)
        .unwrap();
    let backup = backup_dir();
    let receipt = archive
        .backup_v2_import_originals(&chain(), &source(), backup.path())
        .expect("library backup receipt");
    let reclaim = archive
        .plan_v2_import_reclaim(&chain(), &source(), receipt.digest())
        .expect("reclaim plan");
    let mut mismatched = receipt.digest();
    mismatched[0] ^= 0xff;

    let error = archive
        .execute_v2_import_reclaim(&chain(), &source(), reclaim.digest(), mismatched)
        .expect_err("receipt mismatch must fail closed");
    assert!(matches!(
        error,
        ArchiveError::ManifestVerification(_) | ArchiveError::InvalidInput(_)
    ));
    assert_eq!(v2_object_parquets(temporary.path()).len(), 1);
    assert_eq!(
        read_file(temporary.path(), &v2_current_relative()),
        v2_current_before
    );
}

#[test]
fn v2_catalog_mismatch_after_approve_fails_closed_without_unlinking() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    archive
        .publish_v2_import(&chain(), &source(), &plan)
        .unwrap();
    archive
        .approve_v2_import(&chain(), &source(), &plan)
        .unwrap();
    let mut current = read_file(temporary.path(), &v2_current_relative());
    let last = current.len() - 1;
    current[last] ^= 0xff;
    fs::write(temporary.path().join(v2_current_relative()), current).expect("tamper V2 CURRENT");
    let backup = backup_dir();

    let error = archive
        .backup_v2_import_originals(&chain(), &source(), backup.path())
        .expect_err("catalog mismatch must fail closed");
    assert!(matches!(
        error,
        ArchiveError::ManifestVerification(_) | ArchiveError::CorruptObject(_)
    ));
    assert_eq!(v2_object_parquets(temporary.path()).len(), 1);
}

#[test]
fn mutated_covered_original_fails_closed_without_unlink() {
    let temporary = tempfile::tempdir().unwrap();
    open_v2(temporary.path())
        .append_batch(&batch(1, &[10], b"ab"))
        .unwrap();
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    archive
        .publish_v2_import(&chain(), &source(), &plan)
        .unwrap();
    archive
        .approve_v2_import(&chain(), &source(), &plan)
        .unwrap();
    mutate_first_v2_parquet(temporary.path());
    let backup = backup_dir();

    let error = archive
        .backup_v2_import_originals(&chain(), &source(), backup.path())
        .expect_err("mutated original must fail closed");
    assert!(matches!(
        error,
        ArchiveError::ManifestVerification(_) | ArchiveError::CorruptObject(_)
    ));
    assert_eq!(v2_object_parquets(temporary.path()).len(), 1);
}

#[test]
fn approved_import_reclaims_v2_originals_covered_by_verified_packs() {
    let temporary = tempfile::tempdir().unwrap();
    let v2 = open_v2(temporary.path());
    let first = v2.append_batch(&batch(1, &[10, 11], b"ab")).unwrap();
    let second = v2.append_batch(&batch(3, &[20], b"cd")).unwrap();
    let v2_current_before = read_file(temporary.path(), &v2_current_relative());
    let archive = open_v3(temporary.path());
    let plan = archive.plan_v2_import(&chain(), &source()).unwrap();
    archive
        .publish_v2_import(&chain(), &source(), &plan)
        .unwrap();
    let approval = archive
        .approve_v2_import(&chain(), &source(), &plan)
        .unwrap();
    assert_eq!(
        read_file(temporary.path(), &v2_current_relative()),
        v2_current_before
    );
    assert_eq!(v2_object_parquets(temporary.path()).len(), 2);

    let backup = backup_dir();
    let receipt = archive
        .backup_v2_import_originals(&chain(), &source(), backup.path())
        .expect("library backup receipt");
    let reclaim = archive
        .plan_v2_import_reclaim(&chain(), &source(), receipt.digest())
        .expect("reclaim plan");
    assert!(!reclaim.is_empty());
    let executed = archive
        .execute_v2_import_reclaim(&chain(), &source(), reclaim.digest(), receipt.digest())
        .expect("reclaim execute");
    assert_eq!(executed.unlinked_files(), 2);

    assert!(v2_object_parquets(temporary.path()).is_empty());
    assert_eq!(
        read_file(temporary.path(), &v2_current_relative()),
        v2_current_before
    );
    assert_eq!(
        replay_payloads(&archive, 3),
        [b"ab".to_vec(), b"ab".to_vec(), b"cd".to_vec()]
    );
    archive
        .verify_raw_manifest_at_sequence(first.manifest_id(), first.local_sequence_range().unwrap())
        .expect("checkpoint still verifies through packs after reclaim");
    archive
        .verify_raw_manifest_at_sequence(
            second.manifest_id(),
            second.local_sequence_range().unwrap(),
        )
        .expect("second checkpoint still verifies through packs after reclaim");
    match archive.load_checkpoint(&chain(), &source()).unwrap() {
        Some(RawArchiveCheckpoint::V2(checkpoint)) => {
            assert_eq!(checkpoint.root_hash(), approval.v3_root_sha256());
        }
        other => panic!("expected checkpoint V2, got {other:?}"),
    }

    let again = archive
        .execute_v2_import_reclaim(&chain(), &source(), reclaim.digest(), receipt.digest())
        .expect("idempotent reclaim");
    assert_eq!(again.unlinked_files(), 2);
}
