use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use domain_types::SourceId;
use hl_capture::adapters::{
    NodeBlockDirectoryConfig, NodeBlockDirectorySource, NodeFileConfig, NodeLineFileSource,
    NodeReceiveClock, NodeSnapshotDirectoryConfig, NodeSnapshotDirectorySource,
};
use hl_protocol::node::state_snapshot::PERIODIC_SNAPSHOT_STRIDE;
use hl_protocol::node::v1::{NodeRecordKind, NodeStreamKind, parse_node_record};
use hl_protocol::{
    BlockSource, ObservationClass, ReceiveTimestamps, SourceCursor, SourceError,
    SourceRequestContext,
};
use tempfile::TempDir;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct TestClock {
    next: Mutex<u64>,
}

impl TestClock {
    const fn new() -> Self {
        Self {
            next: Mutex::new(1),
        }
    }
}

impl NodeReceiveClock for TestClock {
    fn now(&self) -> Result<ReceiveTimestamps, SourceError> {
        let mut next = self
            .next
            .lock()
            .map_err(|_| SourceError::Configuration("test clock poisoned".to_owned()))?;
        let value = *next;
        *next += 1;
        ReceiveTimestamps::new(1_721_000_000_000_000 + value as i64, value)
            .map_err(|_| SourceError::Configuration("test clock invalid".to_owned()))
    }
}

fn fixture(name: &str) -> Vec<u8> {
    let mut bytes = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/source/node-v1")
            .join(name),
    )
    .expect("node fixture");
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
    }
    bytes
}

fn config(path: PathBuf) -> NodeFileConfig {
    stream_config(path, NodeStreamKind::Fills, "node-fills")
}

fn stream_config(path: PathBuf, stream: NodeStreamKind, stream_name: &str) -> NodeFileConfig {
    NodeFileConfig::new(
        path,
        stream_name,
        stream,
        SourceId::new("primary-node-fills").expect("source id"),
        "hl-node-v1",
        "node-v1",
        1024 * 1024,
        Duration::from_millis(5),
    )
    .expect("valid node file config")
}

fn bounded_stream_config(
    path: PathBuf,
    stream: NodeStreamKind,
    stream_name: &str,
    max_inflight_observations: usize,
) -> NodeFileConfig {
    NodeFileConfig::new_bounded(
        path,
        stream_name,
        stream,
        SourceId::new("primary-node-fills").expect("source id"),
        "hl-node-v1",
        "node-v1",
        1024 * 1024,
        Duration::from_millis(5),
        max_inflight_observations,
    )
    .expect("valid bounded node file config")
}

fn context(cancellation: CancellationToken, timeout: Duration) -> SourceRequestContext {
    SourceRequestContext::new(cancellation, Instant::now() + timeout)
}

fn write_line(path: &Path, payload: &[u8]) {
    let mut bytes = payload.to_vec();
    bytes.push(b'\n');
    fs::write(path, bytes).expect("write line");
}

#[tokio::test]
async fn partial_final_line_is_not_emitted_until_complete() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    let first = fixture("fill.json");
    write_line(&path, &first);
    let second = fixture("fill.json");
    let split = second.len() / 2;
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("open append")
        .write_all(&second[..split])
        .expect("write partial line");

    let mut source =
        NodeLineFileSource::open_with_clock(config(path.clone()), None, TestClock::new())
            .expect("open source");
    let cancellation = CancellationToken::new();
    let observed = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("first observation");
    assert_eq!(observed.payload().as_ref(), first);
    source
        .acknowledge_durable(observed.cursor())
        .expect("first line durable");

    let error = source
        .next_observation(&context(cancellation.clone(), Duration::from_millis(20)))
        .await
        .expect_err("partial line must remain pending");
    assert_eq!(error, SourceError::BackpressureTimeout);
    let partial = source.tail_state().expect("partial tail state");
    assert!(partial.partial_line());
    assert_eq!(
        partial.unread_bytes(),
        u64::try_from(split).expect("fixture length fits u64")
    );

    let mut append = OpenOptions::new()
        .append(true)
        .open(path)
        .expect("reopen append");
    append
        .write_all(&second[split..])
        .expect("complete payload");
    append.write_all(b"\n").expect("complete line");
    let observed = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("completed second observation");
    assert_eq!(observed.payload().as_ref(), second);
    let complete = source.tail_state().expect("completed tail state");
    assert!(!complete.partial_line());
    assert_eq!(complete.unread_bytes(), 0);
}

#[tokio::test]
async fn restart_replays_only_progress_after_the_durable_cursor() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    let payload = fixture("fill.json");
    let mut contents = Vec::new();
    for _ in 0..3 {
        contents.extend_from_slice(&payload);
        contents.push(b'\n');
    }
    fs::write(&path, contents).expect("three lines");

    let cancellation = CancellationToken::new();
    let mut first =
        NodeLineFileSource::open_with_clock(config(path.clone()), None, TestClock::new())
            .expect("open first source");
    let durable = first
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("first")
        .cursor()
        .clone();
    first
        .acknowledge_durable(&durable)
        .expect("acknowledge first record");
    let speculative = first
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("speculative second")
        .cursor()
        .clone();
    drop(first);

    let mut restarted =
        NodeLineFileSource::open_with_clock(config(path), Some(durable.clone()), TestClock::new())
            .expect("restart from durable cursor");
    assert_eq!(restarted.committed_cursor(), Some(&durable));
    let replayed = restarted
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("replayed second");
    assert_eq!(replayed.cursor(), &speculative);
}

#[tokio::test]
async fn replacing_the_path_drains_old_file_then_starts_a_new_epoch() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    let rotated = directory.path().join("fills.1");
    let payload = fixture("fill.json");
    write_line(&path, &payload);

    let cancellation = CancellationToken::new();
    let mut source =
        NodeLineFileSource::open_with_clock(config(path.clone()), None, TestClock::new())
            .expect("open source");
    let first = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("old file record");
    source
        .acknowledge_durable(first.cursor())
        .expect("old file record durable");

    fs::rename(&path, rotated).expect("rotate old file");
    write_line(&path, &payload);
    let second = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("replacement record");

    assert_ne!(first.cursor().epoch(), second.cursor().epoch());
    assert_eq!(second.payload().as_ref(), payload);
}

#[tokio::test]
async fn truncation_within_one_file_identity_is_fatal() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    write_line(&path, &fixture("fill.json"));
    let cancellation = CancellationToken::new();
    let mut source =
        NodeLineFileSource::open_with_clock(config(path.clone()), None, TestClock::new())
            .expect("open source");
    let first = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("first observation");
    source
        .acknowledge_durable(first.cursor())
        .expect("first record durable");
    OpenOptions::new()
        .write(true)
        .open(path)
        .expect("open for truncate")
        .set_len(0)
        .expect("truncate");

    let error = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect_err("same-identity truncation must fail");
    assert_eq!(error, SourceError::CursorRegression);
}

#[tokio::test]
async fn cancellation_stops_a_waiting_tail_without_detached_work() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    write_line(&path, &fixture("fill.json"));
    let cancellation = CancellationToken::new();
    let mut source = NodeLineFileSource::open_with_clock(config(path), None, TestClock::new())
        .expect("open source");
    source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("first observation");
    cancellation.cancel();

    let error = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect_err("cancelled tail");
    assert_eq!(error, SourceError::Cancelled);
}

#[tokio::test]
async fn durable_acknowledgement_must_match_an_emitted_cursor() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    write_line(&path, &fixture("fill.json"));
    let cancellation = CancellationToken::new();
    let mut source = NodeLineFileSource::open_with_clock(config(path), None, TestClock::new())
        .expect("open source");
    let emitted = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("observation")
        .cursor()
        .clone();
    let invalid = SourceCursor::new(emitted.epoch(), emitted.offset() + 1).expect("invalid cursor");

    assert_eq!(
        source
            .acknowledge_durable(&invalid)
            .expect_err("unemitted cursor"),
        SourceError::CursorRegression
    );
    assert!(source.committed_cursor().is_none());
    source
        .acknowledge_durable(&emitted)
        .expect("emitted cursor");
    assert_eq!(source.committed_cursor(), Some(&emitted));
}

#[tokio::test]
async fn restart_after_rotation_opens_the_successor_epoch_at_offset_zero() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    let rotated = directory.path().join("fills.1");
    let payload = fixture("fill.json");
    write_line(&path, &payload);
    let cancellation = CancellationToken::new();
    let mut source =
        NodeLineFileSource::open_with_clock(config(path.clone()), None, TestClock::new())
            .expect("open source");
    let first = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("first epoch record");
    source
        .acknowledge_durable(first.cursor())
        .expect("first epoch durable");
    let durable = first.cursor().clone();
    drop(source);

    fs::rename(&path, rotated).expect("rotate old file");
    write_line(&path, &payload);
    let mut restarted =
        NodeLineFileSource::open_with_clock(config(path), Some(durable.clone()), TestClock::new())
            .expect("successor epoch must open after rotation");
    assert_eq!(restarted.committed_cursor(), Some(&durable));
    let second = restarted
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("successor epoch record");
    assert_ne!(second.cursor().epoch(), durable.epoch());
    assert!(second.cursor().offset() > 0);
    assert_eq!(second.payload().as_ref(), payload);
}

#[tokio::test]
async fn adapter_never_reads_a_second_record_before_first_is_durable() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    let payload = fixture("fill.json");
    let mut contents = payload.clone();
    contents.push(b'\n');
    contents.extend_from_slice(&payload);
    contents.push(b'\n');
    fs::write(&path, contents).expect("two records");
    let cancellation = CancellationToken::new();
    let mut source = NodeLineFileSource::open_with_clock(config(path), None, TestClock::new())
        .expect("open source");
    let first = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("first observation");

    assert_eq!(
        source
            .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
            .await
            .expect_err("durability acknowledgement is required"),
        SourceError::BackpressureTimeout
    );
    source
        .acknowledge_durable(first.cursor())
        .expect("first record durable");
    let second = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("second observation after durability");
    assert!(second.cursor().offset() > first.cursor().offset());
}

#[tokio::test]
async fn bounded_adapter_allows_ordered_group_commit_without_unbounded_read_ahead() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("fills");
    let payload = fixture("fill.json");
    let mut contents = Vec::new();
    for _ in 0..3 {
        contents.extend_from_slice(&payload);
        contents.push(b'\n');
    }
    fs::write(&path, contents).expect("three lines");
    let cancellation = CancellationToken::new();
    let mut source = NodeLineFileSource::open_with_clock(
        bounded_stream_config(path, NodeStreamKind::Fills, "node-fills", 2),
        None,
        TestClock::new(),
    )
    .expect("open bounded source");

    let first = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("first observation");
    let second = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("second observation before group acknowledgement");
    assert!(second.cursor().offset() > first.cursor().offset());
    assert_eq!(
        source
            .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
            .await
            .expect_err("bounded in-flight window"),
        SourceError::BackpressureTimeout
    );
    assert_eq!(
        source.acknowledge_durable(second.cursor()),
        Err(SourceError::CursorRegression),
        "acknowledgements must remain ordered"
    );
    source
        .acknowledge_durable(first.cursor())
        .expect("first group member durable");
    let third = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("window reopens after ordered acknowledgement");
    assert!(third.cursor().offset() > second.cursor().offset());
    source
        .acknowledge_durable(second.cursor())
        .expect("second group member durable");
    source
        .acknowledge_durable(third.cursor())
        .expect("third group member durable");
}

#[tokio::test]
async fn unknown_complete_variant_is_available_for_durable_quarantine() {
    let directory = TempDir::new().expect("temp directory");
    let path = directory.path().join("misc-events");
    let payload = fixture("unknown-variant.json");
    write_line(&path, &payload);
    let cancellation = CancellationToken::new();
    let mut source = NodeLineFileSource::open_with_clock(
        stream_config(path, NodeStreamKind::MiscEvents, "node-misc-events"),
        None,
        TestClock::new(),
    )
    .expect("open source");

    let error = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect_err("unknown variant");
    assert!(matches!(error, SourceError::SchemaDrift(_)));
    let quarantined = source
        .pending_quarantine()
        .expect("quarantine evidence retained")
        .clone();
    assert_eq!(quarantined.payload().as_ref(), payload);
    assert_eq!(quarantined.content_hash(), blake3::hash(&payload));
    assert_eq!(quarantined.reason_code(), "source.schema_drift");
    source
        .acknowledge_quarantine_durable(quarantined.cursor())
        .expect("quarantine is durable");
    assert_eq!(source.committed_cursor(), Some(quarantined.cursor()));
    assert!(source.pending_quarantine().is_none());
}

fn block_directory_config(root: PathBuf) -> NodeBlockDirectoryConfig {
    NodeBlockDirectoryConfig::new(
        root,
        "replica-cmds",
        SourceId::new("primary-node-blocks").expect("source id"),
        "hl-node-v1",
        "node-v1",
        100,
        1024 * 1024,
        Duration::from_millis(5),
    )
    .expect("block directory config")
}

fn write_block(root: &Path, session: &str, height: u64, payload: &[u8]) {
    let directory = root.join(session).join("20260728");
    fs::create_dir_all(&directory).expect("block directory");
    fs::write(directory.join(height.to_string()), payload).expect("block file");
}

#[tokio::test]
async fn per_height_block_directory_restarts_from_durable_height() {
    let directory = TempDir::new().expect("temp directory");
    let payload = fixture("transaction-block.json");
    write_block(directory.path(), "1721000000", 100, &payload);
    write_block(directory.path(), "1721000000", 101, &payload);
    let cancellation = CancellationToken::new();
    let mut first = NodeBlockDirectorySource::open_with_clock(
        block_directory_config(directory.path().to_path_buf()),
        None,
        TestClock::new(),
    )
    .expect("open block source");
    let first_observation = first
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("height 100");
    assert_eq!(first_observation.cursor().offset(), 100);
    assert_eq!(first_observation.payload().as_ref(), payload);
    let durable = first_observation.cursor().clone();
    first.acknowledge_durable(&durable).expect("ack height 100");
    drop(first);

    let mut restarted = NodeBlockDirectorySource::open_with_clock(
        block_directory_config(directory.path().to_path_buf()),
        Some(durable),
        TestClock::new(),
    )
    .expect("restart block source");
    let next = restarted
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("height 101");
    assert_eq!(next.cursor().offset(), 101);
}

#[tokio::test]
async fn per_height_block_directory_emits_unparsed_bytes_for_raw_first_durability() {
    let directory = TempDir::new().expect("temp directory");
    let payload = b"{not-yet-valid-json";
    write_block(directory.path(), "1721000000", 100, payload);
    let cancellation = CancellationToken::new();
    let mut source = NodeBlockDirectorySource::open_with_clock(
        block_directory_config(directory.path().to_path_buf()),
        None,
        TestClock::new(),
    )
    .expect("open block source");

    let observation = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("raw block observation");

    assert_eq!(observation.cursor().offset(), 100);
    assert_eq!(observation.payload().as_ref(), payload);
    assert_eq!(
        observation.observation_class(),
        ObservationClass::CommittedBlock
    );
}

#[tokio::test]
async fn per_height_block_directory_fails_closed_on_a_visible_gap() {
    let directory = TempDir::new().expect("temp directory");
    let payload = fixture("transaction-block.json");
    write_block(directory.path(), "1721000000", 100, &payload);
    write_block(directory.path(), "1721000000", 102, &payload);
    let cancellation = CancellationToken::new();
    let mut source = NodeBlockDirectorySource::open_with_clock(
        block_directory_config(directory.path().to_path_buf()),
        None,
        TestClock::new(),
    )
    .expect("open block source");
    let first = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("height 100");
    source
        .acknowledge_durable(first.cursor())
        .expect("height 100 durable");

    assert_eq!(
        source
            .next_observation(&context(cancellation, Duration::from_secs(1)))
            .await
            .expect_err("height 101 is missing"),
        SourceError::RangeUnavailable
    );
}

#[test]
fn conflicting_duplicate_block_height_is_schema_drift() {
    let directory = TempDir::new().expect("temp directory");
    let payload = fixture("transaction-block.json");
    write_block(directory.path(), "1721000000", 100, &payload);
    let mut conflicting = payload;
    conflicting.extend_from_slice(b" ");
    write_block(directory.path(), "1721000001", 100, &conflicting);

    let error = NodeBlockDirectorySource::open_with_clock(
        block_directory_config(directory.path().to_path_buf()),
        None,
        TestClock::new(),
    )
    .expect_err("conflicting duplicate height");
    assert!(matches!(error, SourceError::SchemaDrift(_)));
}

fn snapshot_fixture(name: &str) -> Vec<u8> {
    fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("fixtures/source/node-v1/snapshots")
            .join(name),
    )
    .expect("snapshot fixture")
}

fn snapshot_directory_config(
    root: PathBuf,
    stream: NodeStreamKind,
    stream_name: &str,
) -> NodeSnapshotDirectoryConfig {
    NodeSnapshotDirectoryConfig::new(
        root,
        stream_name,
        stream,
        SourceId::new("primary-node-snapshots").expect("source id"),
        "hl-node-v1",
        "node-v1",
        PERIODIC_SNAPSHOT_STRIDE,
        1024 * 1024,
        Duration::from_millis(5),
    )
    .expect("snapshot directory config")
}

fn write_snapshot(root: &Path, date: &str, height: u64, extension: &str, payload: &[u8]) {
    let directory = root.join(date);
    fs::create_dir_all(&directory).expect("snapshot date directory");
    fs::write(directory.join(format!("{height}.{extension}")), payload).expect("snapshot file");
}

#[test]
fn line_file_config_rejects_whole_file_snapshot_streams() {
    let path = PathBuf::from("/tmp/node-abci.ndjson");
    let error = NodeFileConfig::new(
        path,
        "node-abci",
        NodeStreamKind::AbciStateSnapshots,
        SourceId::new("primary-node-abci").expect("source id"),
        "hl-node-v1",
        "node-v1",
        1024 * 1024,
        Duration::from_millis(5),
    )
    .expect_err("abci is not ndjson");
    assert!(matches!(error, SourceError::Configuration(_)));
}

#[test]
fn snapshot_directory_rejects_unaligned_start_height() {
    let directory = TempDir::new().expect("temp directory");
    let error = NodeSnapshotDirectoryConfig::new(
        directory.path().to_path_buf(),
        "periodic-abci",
        NodeStreamKind::AbciStateSnapshots,
        SourceId::new("primary-node-snapshots").expect("source id"),
        "hl-node-v1",
        "node-v1",
        100,
        1024 * 1024,
        Duration::from_millis(5),
    )
    .expect_err("start height must follow the 10_000 stride");
    assert!(matches!(error, SourceError::Configuration(_)));
}

#[tokio::test]
async fn periodic_abci_directory_restarts_from_durable_height() {
    let directory = TempDir::new().expect("temp directory");
    let payload = snapshot_fixture("abci-10000.rmp");
    write_snapshot(directory.path(), "20260728", 10_000, "rmp", &payload);
    write_snapshot(directory.path(), "20260728", 20_000, "rmp", &payload);
    let cancellation = CancellationToken::new();
    let mut first = NodeSnapshotDirectorySource::open_with_clock(
        snapshot_directory_config(
            directory.path().to_path_buf(),
            NodeStreamKind::AbciStateSnapshots,
            "periodic-abci",
        ),
        None,
        TestClock::new(),
    )
    .expect("open abci source");
    let first_observation = first
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("height 10000");
    assert_eq!(first_observation.cursor().offset(), 10_000);
    assert_eq!(first_observation.payload().as_ref(), payload);
    assert_eq!(
        first_observation.observation_class(),
        ObservationClass::AuxiliaryLedger
    );
    let parsed = parse_node_record(
        NodeStreamKind::AbciStateSnapshots,
        first_observation.payload().clone(),
    )
    .expect("abci parse");
    assert_eq!(parsed.kind(), NodeRecordKind::AbciStateSnapshot);
    assert_eq!(parsed.content_hash(), blake3::hash(&payload));
    let durable = first_observation.cursor().clone();
    first
        .acknowledge_durable(&durable)
        .expect("ack height 10000");
    drop(first);

    let mut restarted = NodeSnapshotDirectorySource::open_with_clock(
        snapshot_directory_config(
            directory.path().to_path_buf(),
            NodeStreamKind::AbciStateSnapshots,
            "periodic-abci",
        ),
        Some(durable),
        TestClock::new(),
    )
    .expect("restart abci source");
    let next = restarted
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("height 20000");
    assert_eq!(next.cursor().offset(), 20_000);
}

#[tokio::test]
async fn periodic_abci_directory_fails_closed_on_a_visible_stride_gap() {
    let directory = TempDir::new().expect("temp directory");
    let payload = snapshot_fixture("abci-10000.rmp");
    write_snapshot(directory.path(), "20260728", 10_000, "rmp", &payload);
    write_snapshot(directory.path(), "20260728", 30_000, "rmp", &payload);
    let cancellation = CancellationToken::new();
    let mut source = NodeSnapshotDirectorySource::open_with_clock(
        snapshot_directory_config(
            directory.path().to_path_buf(),
            NodeStreamKind::AbciStateSnapshots,
            "periodic-abci",
        ),
        None,
        TestClock::new(),
    )
    .expect("open abci source");
    let first = source
        .next_observation(&context(cancellation.clone(), Duration::from_secs(1)))
        .await
        .expect("height 10000");
    source
        .acknowledge_durable(first.cursor())
        .expect("height 10000 durable");

    assert_eq!(
        source
            .next_observation(&context(cancellation, Duration::from_secs(1)))
            .await
            .expect_err("height 20000 is missing"),
        SourceError::RangeUnavailable
    );
}

#[test]
fn identical_duplicate_snapshot_height_is_idempotent() {
    let directory = TempDir::new().expect("temp directory");
    let payload = snapshot_fixture("abci-10000.rmp");
    write_snapshot(directory.path(), "20260728", 10_000, "rmp", &payload);
    write_snapshot(directory.path(), "20260729", 10_000, "rmp", &payload);
    NodeSnapshotDirectorySource::open_with_clock(
        snapshot_directory_config(
            directory.path().to_path_buf(),
            NodeStreamKind::AbciStateSnapshots,
            "periodic-abci",
        ),
        None,
        TestClock::new(),
    )
    .expect("identical duplicate height");
}

#[test]
fn conflicting_duplicate_snapshot_height_is_schema_drift() {
    let directory = TempDir::new().expect("temp directory");
    let payload = snapshot_fixture("abci-10000.rmp");
    write_snapshot(directory.path(), "20260728", 10_000, "rmp", &payload);
    let mut conflicting = payload;
    conflicting.extend_from_slice(&[0xff]);
    write_snapshot(directory.path(), "20260729", 10_000, "rmp", &conflicting);
    let error = NodeSnapshotDirectorySource::open_with_clock(
        snapshot_directory_config(
            directory.path().to_path_buf(),
            NodeStreamKind::AbciStateSnapshots,
            "periodic-abci",
        ),
        None,
        TestClock::new(),
    )
    .expect_err("conflicting duplicate height");
    assert!(matches!(error, SourceError::SchemaDrift(_)));
}

#[tokio::test]
async fn periodic_l4_directory_emits_raw_bytes_with_book_observation_class() {
    let directory = TempDir::new().expect("temp directory");
    let payload = snapshot_fixture("l4-10000.json");
    write_snapshot(directory.path(), "20260728", 10_000, "json", &payload);
    let cancellation = CancellationToken::new();
    let mut source = NodeSnapshotDirectorySource::open_with_clock(
        snapshot_directory_config(
            directory.path().to_path_buf(),
            NodeStreamKind::L4Snapshots,
            "periodic-l4",
        ),
        None,
        TestClock::new(),
    )
    .expect("open l4 source");
    let observation = source
        .next_observation(&context(cancellation, Duration::from_secs(1)))
        .await
        .expect("l4 snapshot");
    assert_eq!(observation.cursor().offset(), 10_000);
    assert_eq!(observation.payload().as_ref(), payload);
    assert_eq!(
        observation.observation_class(),
        ObservationClass::AuxiliaryBookDiff
    );
    let parsed = parse_node_record(NodeStreamKind::L4Snapshots, observation.payload().clone())
        .expect("l4 parse");
    assert_eq!(parsed.kind(), NodeRecordKind::L4Snapshot);
    assert_eq!(parsed.content_hash(), blake3::hash(&payload));
}
