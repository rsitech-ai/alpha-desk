use std::fs::{self, OpenOptions};
use std::io::{Seek, SeekFrom, Write};
use std::path::Path;
use std::time::{Duration, Instant};

use bytes::Bytes;
use domain_types::SourceId;
use hl_capture::spool::{
    DurabilityPolicy, SegmentHeaderV1, SpoolError, SpoolRead, SpoolReader, SpoolWriter,
    recover_open_segment, recover_spool_tail, validate_segment_bytes,
};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use tempfile::TempDir;

const BUILD_HASH: [u8; 32] = [0x42; 32];

fn header(sequence: u64) -> SegmentHeaderV1 {
    SegmentHeaderV1::new(
        SourceId::new("primary-node").expect("valid source id"),
        "node-v1.2.3",
        "spool-v1",
        sequence,
        1_721_000_000_000_000,
        BUILD_HASH,
    )
    .expect("valid segment header")
}

fn observation(offset: u64, payload: impl Into<Bytes>) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").expect("valid source id"),
        "node-v1.2.3",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-session-17", offset).expect("valid cursor"),
        ReceiveTimestamps::new(
            1_721_000_000_000_000 + i64::try_from(offset).expect("fixture offset"),
            99_000 + offset,
        )
        .expect("valid timestamps"),
        "parser-v1",
        payload.into(),
        Vec::new(),
        1024 * 1024,
    )
    .expect("valid observation")
}

fn write_three_records(root: &Path) -> (std::path::PathBuf, [u64; 3]) {
    let mut writer = SpoolWriter::create(root, header(1), DurabilityPolicy::FsyncEveryRecord)
        .expect("create spool writer");
    let first = writer
        .append(
            &observation(40, Bytes::from_static(b"first")),
            1_721_000_000_000_040,
        )
        .expect("append first")
        .expect("strict durability receipt");
    let second = writer
        .append(
            &observation(41, Bytes::from_static(b"second")),
            1_721_000_000_000_041,
        )
        .expect("append second")
        .expect("strict durability receipt");
    let third = writer
        .append(
            &observation(42, Bytes::from_static(b"third")),
            1_721_000_000_000_042,
        )
        .expect("append third")
        .expect("strict durability receipt");
    let path = writer.segment_path().to_owned();
    drop(writer);
    (
        path,
        [
            first.record_offset,
            second.record_offset,
            third.record_offset,
        ],
    )
}

#[test]
fn recovery_truncates_only_an_incomplete_final_record_at_every_byte_boundary() {
    let fixture = TempDir::new().expect("fixture");
    let (complete_path, offsets) = write_three_records(fixture.path());
    let complete = fs::read(&complete_path).expect("complete segment");
    let final_record_start = usize::try_from(offsets[2]).expect("fixture offset fits usize");

    for cut in final_record_start..complete.len() {
        let case = TempDir::new().expect("case");
        let segment = case.path().join("segment-0000000001.hlsp");
        fs::write(&segment, &complete[..cut]).expect("write truncated segment");

        let report = recover_open_segment(&segment).expect("recover incomplete final frame");
        assert_eq!(report.valid_records, 2, "cut={cut}");
        assert_eq!(
            report.truncated_bytes,
            u64::try_from(cut - final_record_start).expect("fixture size"),
            "cut={cut}"
        );
        assert_eq!(
            fs::metadata(&segment).expect("metadata").len(),
            offsets[2],
            "cut={cut}"
        );

        let records = SpoolReader::open(&segment)
            .expect("open recovered segment")
            .read_all()
            .expect("read recovered records");
        assert_eq!(records.len(), 2, "cut={cut}");
        assert_eq!(records[0].cursor().offset(), 40);
        assert_eq!(records[1].cursor().offset(), 41);
    }
}

#[test]
fn recovery_preserves_a_complete_segment_when_a_middle_record_is_corrupt() {
    let fixture = TempDir::new().expect("fixture");
    let (segment, offsets) = write_three_records(fixture.path());
    let mut before = fs::read(&segment).expect("segment bytes");
    let corrupt_at = usize::try_from(offsets[1]).expect("fixture offset") + 24;
    before[corrupt_at] ^= 0x80;

    let mut file = OpenOptions::new()
        .write(true)
        .open(&segment)
        .expect("open segment");
    file.seek(SeekFrom::Start(
        u64::try_from(corrupt_at).expect("fixture offset"),
    ))
    .expect("seek");
    file.write_all(&before[corrupt_at..=corrupt_at])
        .expect("corrupt one byte");
    file.sync_all().expect("sync corruption");

    let error = recover_open_segment(&segment).expect_err("middle corruption must fail closed");
    assert!(matches!(
        error,
        SpoolError::CorruptRecord { record_offset } if record_offset == offsets[1]
    ));
    assert_eq!(
        fs::read(&segment).expect("preserved corrupt segment"),
        before,
        "recovery must preserve evidence for quarantine"
    );
}

#[test]
fn reader_round_trips_framing_metadata_and_source_payload_bytes() {
    let fixture = TempDir::new().expect("fixture");
    let (segment, _) = write_three_records(fixture.path());
    let reader = SpoolReader::open(&segment).expect("open segment");

    assert_eq!(reader.header().source_id().as_str(), "primary-node");
    assert_eq!(reader.header().source_version(), "node-v1.2.3");
    assert_eq!(reader.header().schema_version(), "spool-v1");
    assert_eq!(reader.header().segment_sequence(), 1);
    assert_eq!(reader.header().producer_build_hash(), BUILD_HASH);

    let records = reader.read_all().expect("read complete segment");
    assert_eq!(records.len(), 3);
    assert_eq!(records[2].cursor().epoch(), "node-session-17");
    assert_eq!(records[2].cursor().offset(), 42);
    assert_eq!(
        records[2].observation_class(),
        ObservationClass::CommittedBlock
    );
    assert_eq!(records[2].payload(), b"third");
    assert_eq!(records[2].content_hash(), blake3::hash(b"third"));
}

#[test]
fn incremental_reader_keeps_a_bounded_cursor_and_retries_an_incomplete_active_tail() {
    let fixture = TempDir::new().expect("fixture");
    let (segment, offsets) = write_three_records(fixture.path());
    let complete = fs::read(&segment).expect("complete segment");
    let third_offset = usize::try_from(offsets[2]).expect("fixture offset");
    let cut = third_offset + 8;
    OpenOptions::new()
        .write(true)
        .open(&segment)
        .expect("open segment")
        .set_len(u64::try_from(cut).expect("fixture cut"))
        .expect("truncate into final record");

    let reader = SpoolReader::open(&segment).expect("open incremental reader");
    let mut records = reader.stream().expect("open record stream");
    assert!(matches!(
        records.next_record().expect("first record"),
        SpoolRead::Record(record) if record.cursor().offset() == 40
    ));
    assert!(matches!(
        records.next_record().expect("second record"),
        SpoolRead::Record(record) if record.cursor().offset() == 41
    ));
    assert_eq!(records.next_offset(), offsets[2]);
    assert!(matches!(
        records.next_record().expect("incomplete active tail"),
        SpoolRead::IncompleteTail { record_offset } if record_offset == offsets[2]
    ));
    assert_eq!(records.next_offset(), offsets[2]);

    OpenOptions::new()
        .append(true)
        .open(&segment)
        .expect("reopen active tail")
        .write_all(&complete[cut..])
        .expect("finish active record");

    assert!(matches!(
        records.next_record().expect("completed final record"),
        SpoolRead::Record(record) if record.cursor().offset() == 42
    ));
    assert!(matches!(
        records.next_record().expect("complete eof"),
        SpoolRead::EndOfFile
    ));
}

#[test]
fn durability_receipts_cover_only_records_in_the_completed_sync_batch() {
    let fixture = TempDir::new().expect("fixture");
    let mut writer = SpoolWriter::create(
        fixture.path(),
        header(11),
        DurabilityPolicy::FsyncEvery {
            max_records: 2,
            max_delay: Duration::from_secs(60),
        },
    )
    .expect("create batched writer");

    assert!(
        writer
            .append(&observation(50, Bytes::from_static(b"first")), 100)
            .expect("append first")
            .is_none(),
        "an unsynced record must not receive a durability receipt"
    );
    let deadline = writer
        .next_sync_deadline()
        .expect("a pending batch must expose its sync deadline");
    assert!(deadline > Instant::now());
    let batch = writer
        .append(&observation(51, Bytes::from_static(b"second")), 101)
        .expect("append second")
        .expect("record-count boundary must sync");
    assert_eq!(batch.segment_sequence, 11);
    assert_eq!(batch.durable_cursor.offset(), 51);
    assert_eq!(batch.durable_at_micros, 101);
    assert_eq!(writer.next_sync_deadline(), None);

    assert!(
        writer
            .append(&observation(52, Bytes::from_static(b"third")), 102)
            .expect("append third")
            .is_none()
    );
    let flushed = writer
        .flush(103)
        .expect("flush pending record")
        .expect("flush durability receipt");
    assert_eq!(flushed.durable_cursor.offset(), 52);
    assert_eq!(flushed.durable_at_micros, 103);
    assert_eq!(writer.next_sync_deadline(), None);
    assert!(writer.flush(104).expect("empty flush").is_none());
}

#[test]
fn next_sync_deadline_covers_every_constructible_writer_policy() {
    for policy in [
        DurabilityPolicy::FsyncEveryRecord,
        DurabilityPolicy::FsyncEvery {
            max_records: 2,
            max_delay: Duration::from_secs(60),
        },
    ] {
        let fixture = TempDir::new().expect("fixture");
        let mut writer =
            SpoolWriter::create(fixture.path(), header(15), policy).expect("create writer");
        let receipt = writer
            .append(&observation(70, Bytes::from_static(b"pending")), 300)
            .expect("append");
        match policy {
            DurabilityPolicy::FsyncEveryRecord => {
                assert!(
                    receipt.is_some(),
                    "fsync-every-record still commits immediately"
                );
                assert_eq!(writer.next_sync_deadline(), None);
            }
            DurabilityPolicy::FsyncEvery {
                max_records: _,
                max_delay: _,
            } => {
                assert!(
                    receipt.is_none(),
                    "bounded FsyncEvery still defers durability until the batch bound"
                );
                writer
                    .next_sync_deadline()
                    .expect("a pending FsyncEvery batch must expose its sync deadline");
            }
        }
    }
}

#[test]
fn a_due_batch_can_be_synced_by_the_runtime_timer_without_another_append() {
    let fixture = TempDir::new().expect("fixture");
    let mut writer = SpoolWriter::create(
        fixture.path(),
        header(13),
        DurabilityPolicy::FsyncEvery {
            max_records: 100,
            max_delay: Duration::from_millis(10),
        },
    )
    .expect("create batched writer");
    writer
        .append(&observation(60, Bytes::from_static(b"pending")), 200)
        .expect("append pending record");
    let deadline = writer.next_sync_deadline().expect("sync deadline");

    assert!(
        writer
            .flush_due(deadline - Duration::from_nanos(1), 201)
            .expect("not due")
            .is_none()
    );
    let receipt = writer
        .flush_due(deadline, 202)
        .expect("due flush")
        .expect("durability receipt");
    assert_eq!(receipt.durable_cursor.offset(), 60);
    assert_eq!(receipt.durable_at_micros, 202);
}

#[test]
fn negative_durability_timestamps_fail_before_mutating_the_segment() {
    let fixture = TempDir::new().expect("fixture");
    let mut writer = SpoolWriter::create(
        fixture.path(),
        header(14),
        DurabilityPolicy::FsyncEveryRecord,
    )
    .expect("create writer");
    let size_before = fs::metadata(writer.segment_path())
        .expect("header metadata")
        .len();

    let error = writer
        .append(&observation(61, Bytes::from_static(b"rejected")), -1)
        .expect_err("negative durability timestamp");
    assert!(matches!(error, SpoolError::InvalidTimestamp));
    assert_eq!(
        fs::metadata(writer.segment_path())
            .expect("unchanged metadata")
            .len(),
        size_before
    );
}

#[test]
fn invalid_batching_policy_is_rejected_before_a_segment_is_created() {
    let fixture = TempDir::new().expect("fixture");

    let error = SpoolWriter::create(
        fixture.path(),
        header(12),
        DurabilityPolicy::FsyncEvery {
            max_records: 0,
            max_delay: Duration::from_secs(1),
        },
    )
    .expect_err("zero records is not a bounded durability policy");
    assert!(matches!(error, SpoolError::InvalidDurabilityPolicy));
    assert!(
        fs::read_dir(fixture.path())
            .expect("read fixture")
            .next()
            .is_none()
    );
}

#[test]
fn close_atomically_publishes_a_complete_hash_chained_manifest() {
    let fixture = TempDir::new().expect("fixture");
    let mut first_writer = SpoolWriter::create(
        fixture.path(),
        header(20),
        DurabilityPolicy::FsyncEveryRecord,
    )
    .expect("create first writer");
    first_writer
        .append(&observation(70, Bytes::from_static(b"alpha")), 200)
        .expect("append alpha");
    first_writer
        .append(&observation(71, Bytes::from_static(b"beta")), 201)
        .expect("append beta");
    let first_segment = first_writer.segment_path().to_owned();
    let first_close = first_writer.close(202, None).expect("close first segment");

    assert!(first_close.manifest_path().is_file());
    let mut temporary_manifest = first_close.manifest_path().as_os_str().to_owned();
    temporary_manifest.push(".tmp");
    assert!(
        !Path::new(&temporary_manifest).exists(),
        "temporary manifests must not remain after a successful close"
    );
    assert_eq!(first_close.manifest().record_count(), 2);
    assert_eq!(first_close.manifest().min_cursor().offset(), 70);
    assert_eq!(first_close.manifest().max_cursor().offset(), 71);
    assert_eq!(
        first_close.manifest().file_size_bytes(),
        fs::metadata(&first_segment)
            .expect("segment metadata")
            .len()
    );
    assert_eq!(
        first_close.manifest().segment_blake3(),
        *blake3::hash(&fs::read(&first_segment).expect("segment bytes")).as_bytes()
    );
    assert_eq!(first_close.manifest().producer_build_hash(), BUILD_HASH);
    assert_eq!(first_close.manifest().previous_manifest_blake3(), None);
    assert_eq!(
        first_close.manifest_hash(),
        *blake3::hash(&fs::read(first_close.manifest_path()).expect("first manifest bytes"))
            .as_bytes()
    );

    let mut second_writer = SpoolWriter::create(
        fixture.path(),
        header(21),
        DurabilityPolicy::FsyncEveryRecord,
    )
    .expect("create second writer");
    second_writer
        .append(&observation(72, Bytes::from_static(b"gamma")), 203)
        .expect("append gamma");
    let second_close = second_writer
        .close(204, Some(first_close.manifest_hash()))
        .expect("close second segment");
    assert_eq!(
        second_close.manifest().previous_manifest_blake3(),
        Some(first_close.manifest_hash())
    );

    let manifest_json: serde_json::Value = serde_json::from_slice(
        &fs::read(second_close.manifest_path()).expect("second manifest bytes"),
    )
    .expect("valid manifest JSON");
    assert_eq!(manifest_json["schema_version"], "hl-spool-manifest-v1");
    assert_eq!(manifest_json["segment_sequence"], 21);
    assert_eq!(manifest_json["record_count"], 1);
    assert_eq!(manifest_json["min_cursor"]["offset"], 72);
    assert_eq!(manifest_json["max_cursor"]["offset"], 72);
    assert_eq!(
        manifest_json["previous_manifest_blake3"]
            .as_str()
            .map(str::len),
        Some(64)
    );
}

#[test]
fn close_never_overwrites_an_existing_manifest() {
    let fixture = TempDir::new().expect("fixture");
    let mut writer = SpoolWriter::create(
        fixture.path(),
        header(22),
        DurabilityPolicy::FsyncEveryRecord,
    )
    .expect("create writer");
    writer
        .append(&observation(80, Bytes::from_static(b"payload")), 300)
        .expect("append payload");
    let manifest_path = fixture.path().join("segment-0000000022.hlsp.manifest");
    fs::write(&manifest_path, b"sentinel").expect("seed existing manifest");

    let error = writer
        .close(301, None)
        .expect_err("close must not replace a manifest");
    assert!(matches!(error, SpoolError::ManifestAlreadyExists));
    assert_eq!(
        fs::read(manifest_path).expect("sentinel manifest"),
        b"sentinel"
    );
}

#[test]
fn recovery_refuses_to_modify_a_segment_with_a_published_manifest() {
    let fixture = TempDir::new().expect("fixture");
    let mut writer = SpoolWriter::create(
        fixture.path(),
        header(23),
        DurabilityPolicy::FsyncEveryRecord,
    )
    .expect("create writer");
    writer
        .append(&observation(81, Bytes::from_static(b"closed")), 310)
        .expect("append record");
    let segment = writer.segment_path().to_owned();
    writer.close(311, None).expect("close segment");
    let before = fs::read(&segment).expect("closed bytes");

    let error = recover_open_segment(&segment).expect_err("closed recovery must fail");
    assert!(matches!(error, SpoolError::ClosedSegment));
    assert_eq!(fs::read(segment).expect("preserved segment"), before);
}

#[test]
fn recovered_writer_resumes_cursor_and_manifest_accounting() {
    let fixture = TempDir::new().expect("fixture");
    let mut original = SpoolWriter::create(
        fixture.path(),
        header(24),
        DurabilityPolicy::FsyncEveryRecord,
    )
    .expect("create writer");
    original
        .append(&observation(90, Bytes::from_static(b"before-crash-a")), 320)
        .expect("append first record");
    original
        .append(&observation(91, Bytes::from_static(b"before-crash-b")), 321)
        .expect("append second record");
    let segment = original.segment_path().to_owned();
    drop(original);

    let (mut recovered, report) =
        SpoolWriter::open_recovered(&segment, DurabilityPolicy::FsyncEveryRecord)
            .expect("recover writer");
    assert_eq!(report.valid_records, 2);
    assert_eq!(report.truncated_bytes, 0);

    let regression = recovered
        .append(
            &observation(91, Bytes::from_static(b"duplicate-after-restart")),
            322,
        )
        .expect_err("recovered cursor must reject duplicates");
    assert!(matches!(regression, SpoolError::CursorRegression));

    recovered
        .append(&observation(92, Bytes::from_static(b"after-restart")), 323)
        .expect("append successor");
    let closed = recovered.close(324, None).expect("close recovered segment");

    assert_eq!(closed.manifest().record_count(), 3);
    assert_eq!(closed.manifest().min_cursor().offset(), 90);
    assert_eq!(closed.manifest().max_cursor().offset(), 92);
    let records = SpoolReader::open(&segment)
        .expect("open segment")
        .read_all()
        .expect("read segment");
    assert_eq!(
        records
            .iter()
            .map(|record| record.cursor().offset())
            .collect::<Vec<_>>(),
        vec![90, 91, 92]
    );
}

#[test]
fn directory_tail_recovery_repairs_only_the_verified_open_segment() {
    let fixture = TempDir::new().expect("fixture");
    let (segment, offsets) = write_three_records(fixture.path());
    OpenOptions::new()
        .write(true)
        .open(&segment)
        .expect("open segment")
        .set_len(offsets[2] + 3)
        .expect("truncate final record");

    let report = recover_spool_tail(fixture.path())
        .expect("recover directory tail")
        .expect("tail was repaired");

    assert_eq!(report.valid_records, 2);
    assert_eq!(report.final_size, offsets[2]);
    assert!(
        recover_spool_tail(fixture.path())
            .expect("complete tail")
            .is_none()
    );
}

#[test]
fn bounded_in_memory_validation_matches_the_file_reader_failure_modes() {
    let fixture = TempDir::new().expect("fixture");
    let (segment, offsets) = write_three_records(fixture.path());
    let complete = fs::read(segment).expect("segment bytes");

    assert_eq!(
        validate_segment_bytes(&complete).expect("complete segment"),
        3
    );
    let incomplete = validate_segment_bytes(&complete[..complete.len() - 1])
        .expect_err("truncated final record");
    assert!(matches!(
        incomplete,
        SpoolError::IncompleteTail { record_offset } if record_offset == offsets[2]
    ));

    let mut corrupt = complete;
    corrupt[usize::try_from(offsets[1]).expect("fixture offset") + 24] ^= 0x80;
    let error = validate_segment_bytes(&corrupt).expect_err("corrupt middle record");
    assert!(matches!(
        error,
        SpoolError::CorruptRecord { record_offset } if record_offset == offsets[1]
    ));

    let mut malicious_length = Vec::from(&b"HLSPV001"[..]);
    malicious_length.extend_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(
        validate_segment_bytes(&malicious_length),
        Err(SpoolError::InvalidHeader)
    ));
}
