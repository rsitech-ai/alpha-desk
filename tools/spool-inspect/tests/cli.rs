use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use bytes::Bytes;
use domain_types::SourceId;
use hl_capture::spool::{DurabilityPolicy, SegmentHeaderV1, SpoolWriter};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use tempfile::TempDir;

const BUILD_HASH: [u8; 32] = [0x24; 32];

fn header(sequence: u64) -> SegmentHeaderV1 {
    SegmentHeaderV1::new(
        SourceId::new("primary-node").unwrap(),
        "node-v1.2.3",
        "spool-v1",
        sequence,
        1_721_000_000_000_000,
        BUILD_HASH,
    )
    .unwrap()
}

fn observation(offset: u64) -> SourceObservation {
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "node-v1.2.3",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-session-17", offset).unwrap(),
        ReceiveTimestamps::new(1_721_000_000_000_000 + offset as i64, offset).unwrap(),
        "parser-v1",
        Bytes::from(format!("payload-{offset}")),
        Vec::new(),
        1024,
    )
    .unwrap()
}

fn append_closed(root: &Path, sequence: u64, offset: u64, previous: Option<[u8; 32]>) -> [u8; 32] {
    let mut writer =
        SpoolWriter::create(root, header(sequence), DurabilityPolicy::FsyncEveryRecord).unwrap();
    writer
        .append(&observation(offset), 1_721_000_000_000_100 + offset as i64)
        .unwrap();
    writer
        .close(1_721_000_000_000_200 + offset as i64, previous)
        .unwrap()
        .manifest_hash()
}

fn verify(root: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_spool-inspect"))
        .arg("verify")
        .arg(root)
        .output()
        .unwrap()
}

#[test]
fn verify_reports_a_stable_summary_for_a_valid_closed_chain() {
    let fixture = TempDir::new().unwrap();
    let first = append_closed(fixture.path(), 1, 10, None);
    let tip = append_closed(fixture.path(), 2, 11, Some(first));

    let output = verify(fixture.path());

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            "PASS closed_segments=2 open_segments=0 records=2 chain_tip={}\n",
            hex_string(tip)
        )
    );
    assert!(output.stderr.is_empty());
}

#[test]
fn verify_fails_closed_when_segment_bytes_do_not_match_the_manifest() {
    let fixture = TempDir::new().unwrap();
    append_closed(fixture.path(), 1, 10, None);
    let segment = fixture.path().join("segment-0000000001.hlsp");
    let mut bytes = fs::read(&segment).unwrap();
    let last = bytes.last_mut().unwrap();
    *last ^= 0x80;
    fs::write(segment, bytes).unwrap();

    let output = verify(fixture.path());

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ERROR spool.segment_hash_mismatch\n"
    );
}

#[test]
fn verify_fails_closed_when_the_manifest_chain_is_broken() {
    let fixture = TempDir::new().unwrap();
    let first = append_closed(fixture.path(), 1, 10, None);
    append_closed(fixture.path(), 2, 11, Some(first));
    let manifest_path = fixture.path().join("segment-0000000002.hlsp.manifest");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["previous_manifest_blake3"] = serde_json::Value::String("00".repeat(32));
    fs::write(
        manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();

    let output = verify(fixture.path());

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ERROR spool.manifest_chain_broken\n"
    );
}

#[test]
fn verify_accepts_one_complete_open_tail_but_rejects_an_incomplete_tail() {
    let fixture = TempDir::new().unwrap();
    let mut writer = SpoolWriter::create(
        fixture.path(),
        header(1),
        DurabilityPolicy::FsyncEveryRecord,
    )
    .unwrap();
    writer.append(&observation(10), 100).unwrap();
    let segment = writer.segment_path().to_owned();
    drop(writer);

    let complete = verify(fixture.path());
    assert!(
        complete.status.success(),
        "{}",
        String::from_utf8_lossy(&complete.stderr)
    );
    assert_eq!(
        String::from_utf8(complete.stdout).unwrap(),
        "PASS closed_segments=0 open_segments=1 records=1 chain_tip=none\n"
    );

    let length = fs::metadata(&segment).unwrap().len();
    fs::OpenOptions::new()
        .write(true)
        .open(segment)
        .unwrap()
        .set_len(length - 1)
        .unwrap();
    let incomplete = verify(fixture.path());
    assert!(!incomplete.status.success());
    assert_eq!(
        String::from_utf8(incomplete.stderr).unwrap(),
        "ERROR spool.incomplete_tail\n"
    );
}

#[test]
fn verify_rejects_incomplete_publication_malformed_names_and_sequence_gaps() {
    let temporary_manifest = TempDir::new().unwrap();
    append_closed(temporary_manifest.path(), 1, 10, None);
    fs::write(
        temporary_manifest
            .path()
            .join("segment-0000000002.hlsp.manifest.tmp"),
        b"partial",
    )
    .unwrap();
    let output = verify(temporary_manifest.path());
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ERROR spool.incomplete_manifest_publication\n"
    );

    let malformed = TempDir::new().unwrap();
    fs::write(
        malformed.path().join("segment-invalid.hlsp"),
        b"not-a-segment",
    )
    .unwrap();
    let output = verify(malformed.path());
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ERROR spool.unsafe_entry\n"
    );

    let gap = TempDir::new().unwrap();
    let first = append_closed(gap.path(), 1, 10, None);
    let mut writer =
        SpoolWriter::create(gap.path(), header(3), DurabilityPolicy::FsyncEveryRecord).unwrap();
    writer.append(&observation(11), 100).unwrap();
    drop(writer);
    let output = verify(gap.path());
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ERROR spool.unexpected_open_segment\n"
    );
    assert_ne!(first, [0; 32]);
}

#[test]
fn verify_rejects_noncanonical_or_extended_manifest_json() {
    let unknown_field = TempDir::new().unwrap();
    append_closed(unknown_field.path(), 1, 10, None);
    let manifest_path = unknown_field
        .path()
        .join("segment-0000000001.hlsp.manifest");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["unexpected"] = serde_json::Value::Bool(true);
    fs::write(
        &manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
    let output = verify(unknown_field.path());
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ERROR spool.invalid_manifest\n"
    );

    let uppercase_hash = TempDir::new().unwrap();
    append_closed(uppercase_hash.path(), 1, 10, None);
    let manifest_path = uppercase_hash
        .path()
        .join("segment-0000000001.hlsp.manifest");
    let mut manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    manifest["segment_blake3"] =
        serde_json::Value::String(manifest["segment_blake3"].as_str().unwrap().to_uppercase());
    fs::write(
        manifest_path,
        format!("{}\n", serde_json::to_string_pretty(&manifest).unwrap()),
    )
    .unwrap();
    let output = verify(uppercase_hash.path());
    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "ERROR spool.invalid_manifest\n"
    );
}

#[test]
fn invalid_arguments_return_usage_without_panicking() {
    let output = Command::new(env!("CARGO_BIN_EXE_spool-inspect"))
        .arg("unknown")
        .output()
        .unwrap();

    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).unwrap(),
        "usage: spool-inspect verify <directory-or-segment>\n"
    );
}

#[test]
fn committed_v1_fixture_remains_readable_by_the_current_inspector() {
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/spool/valid-v1");

    let output = verify(&fixture);

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    let chain_tip = stdout
        .trim()
        .strip_prefix("PASS closed_segments=1 open_segments=0 records=3 chain_tip=")
        .expect("stable fixture summary");
    assert_eq!(chain_tip.len(), 64);
    assert!(chain_tip.bytes().all(|byte| byte.is_ascii_hexdigit()));
}

fn hex_string(value: [u8; 32]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}
