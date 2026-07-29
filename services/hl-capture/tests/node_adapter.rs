use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use domain_types::SourceId;
use hl_capture::adapters::{
    NodeBlockDirectoryConfig, NodeBlockDirectorySource, NodeFileConfig, NodeLineFileSource,
    NodeReceiveClock,
};
use hl_protocol::node::v1::NodeStreamKind;
use hl_protocol::{
    BlockSource, ObservationClass, ReceiveTimestamps, SourceCursor, SourceError,
    SourceRequestContext,
};
use tempfile::TempDir;
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
