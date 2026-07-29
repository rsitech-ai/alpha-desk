use std::{collections::BTreeMap, fs};

use bytes::Bytes;
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, BlockRange, ChainId, KnownTime, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};
use hl_analytics::archive::{ArchiveConfig, LocalParquetArchive};
use hl_protocol::{
    ObservationClass, ParseWarning, ReceiveTimestamps, SourceCursor, SourceObservation,
};
use storage_ports::{
    ArchiveError, CanonicalArchive, CanonicalArchiveMaintenance, RawObservationArchive,
    RawObservationBatch, RawObservationRange,
};

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}

fn canonical_block(height: u64, payload_seed: u64, event_count: usize) -> BlockEnvelope {
    let block_time_micros = 1_721_779_200_000_000_i64
        .checked_add(i64::try_from(height).expect("fixture height fits i64"))
        .expect("fixture block time");
    let block_time =
        ProtocolTime::from_unix_micros(block_time_micros).expect("protocol block time");
    let source_id = SourceId::new("primary-node").expect("source ID");
    let events = (0..event_count)
        .map(|index| {
            let index = u32::try_from(index).expect("fixture event count fits u32");
            CanonicalEventEnvelope::from_input(CanonicalEventInput {
                schema_version: "1.0.0".to_owned(),
                chain_id: ChainId::new("mainnet").expect("chain ID"),
                block_height: BlockHeight::new(height),
                block_time,
                transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction ID"),
                transaction_index: 0,
                canonical_event_index: index,
                market_ids: Vec::new(),
                account_ids: Vec::new(),
                source_evidence: vec![
                    SourceEvidence::try_new(
                        source_id.clone(),
                        "node-v1",
                        format!("block-{height}:{index}"),
                        [u8::try_from(payload_seed).unwrap_or(0x7f); 32],
                    )
                    .expect("source evidence"),
                ],
                confirmation_class: ConfirmationClass::CommittedPrimary,
                observed_at: known(block_time_micros),
                ingested_at: known(block_time_micros + 1),
                canonicalized_at: known(block_time_micros + 2),
                parser_version: "canonical-parser-v1".to_owned(),
                payload: EventPayload::TradeMatched(TradeMatched::without_identities(
                    Price::parse_at_scale("65000", 6).expect("price"),
                    Quantity::parse_at_scale("0.01", 8).expect("quantity"),
                    payload_seed + u64::from(index),
                )),
            })
            .expect("canonical event")
        })
        .collect();

    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        BlockHeight::new(height),
        block_time,
        ConfirmationClass::CommittedPrimary,
        events,
        BTreeMap::from([(source_id, [0x55; 32])]),
    )
    .expect("canonical block")
}

fn archive(temporary: &tempfile::TempDir) -> Result<LocalParquetArchive, ArchiveError> {
    LocalParquetArchive::open(
        temporary.path(),
        ArchiveConfig::deterministic_fixture("archive-test-build", known(1_721_779_300_000_000))?,
    )
}

fn raw_observation(source: &str, epoch: &str, offset: u64, payload: &[u8]) -> SourceObservation {
    raw_observation_at(
        source,
        epoch,
        offset,
        payload,
        1_721_779_200_000_000_i64 + i64::try_from(offset).expect("offset fits i64"),
    )
}

fn raw_observation_at(
    source: &str,
    epoch: &str,
    offset: u64,
    payload: &[u8],
    wall_micros: i64,
) -> SourceObservation {
    SourceObservation::new(
        SourceId::new(source).expect("source ID"),
        "capture-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new(epoch, offset).expect("source cursor"),
        ReceiveTimestamps::new(wall_micros, 9_000_000 + offset).expect("receive timestamps"),
        "raw-parser-v1",
        Bytes::copy_from_slice(payload),
        vec![ParseWarning::new("fixture-warning", format!("offset={offset}")).expect("warning")],
        1024,
    )
    .expect("source observation")
}

fn raw_batch(source: &str, epoch: &str, start: u64, payloads: &[&[u8]]) -> RawObservationBatch {
    let observations = payloads
        .iter()
        .enumerate()
        .map(|(index, payload)| {
            raw_observation(
                source,
                epoch,
                start + u64::try_from(index).expect("fixture index fits u64"),
                payload,
            )
        })
        .collect();
    RawObservationBatch::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        observations,
        [0xa1; 32],
        [0xb2; 32],
    )
    .expect("raw observation batch")
}

#[test]
fn append_is_idempotent_and_conflicting_height_fails_closed() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let original = canonical_block(42, 7, 1);

    let first = archive.append_block(&original).expect("first append");
    let duplicate = archive.append_block(&original).expect("matching duplicate");
    assert_eq!(first, duplicate);

    let conflicting = canonical_block(42, 8, 1);
    let error = archive
        .append_block(&conflicting)
        .expect_err("same height with different content must fail");
    assert!(matches!(
        error,
        ArchiveError::ConflictingBlock(height) if height == BlockHeight::new(42)
    ));
}

#[test]
fn orphan_objects_are_invisible_and_empty_blocks_round_trip() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let empty = canonical_block(43, 0, 0);
    archive.append_block(&empty).expect("archive empty block");

    fs::write(temporary.path().join("orphan.parquet"), b"not referenced")
        .expect("write unreferenced orphan");

    let range = BlockRange::new(BlockHeight::new(43), BlockHeight::new(43)).expect("valid range");
    let blocks = archive
        .read_range(&ChainId::new("mainnet").expect("chain ID"), range)
        .expect("open verified range")
        .collect::<Result<Vec<_>, _>>()
        .expect("read verified range");

    assert_eq!(blocks, vec![empty]);
}

#[test]
fn range_read_reconstructs_ordered_blocks_exactly() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let first = canonical_block(100, 10, 2);
    let second = canonical_block(101, 11, 1);
    archive.append_block(&first).expect("first block");
    archive.append_block(&second).expect("second block");

    let range = BlockRange::new(BlockHeight::new(100), BlockHeight::new(101)).expect("valid range");
    let blocks = archive
        .read_range(&ChainId::new("mainnet").expect("chain ID"), range)
        .expect("open verified range")
        .collect::<Result<Vec<_>, _>>()
        .expect("read verified range");

    assert_eq!(blocks, vec![first, second]);
}

#[test]
fn corrupt_object_is_rejected_before_an_iterator_is_returned() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let block = canonical_block(200, 20, 2);
    let receipt = archive.append_block(&block).expect("archive block");
    let manifest = archive
        .verify_manifest(receipt.manifest_id())
        .expect("verify manifest");
    let object = manifest.objects().first().expect("manifest object");
    let object_path = temporary.path().join(object.relative_path());
    let mut bytes = fs::read(&object_path).expect("read object");
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0xff;
    fs::write(&object_path, bytes).expect("corrupt object");

    let range = BlockRange::new(BlockHeight::new(200), BlockHeight::new(200)).expect("valid range");
    let error = match archive.read_range(&ChainId::new("mainnet").expect("chain ID"), range) {
        Ok(_) => panic!("corruption must fail before returning an iterator"),
        Err(error) => error,
    };
    assert!(matches!(error, ArchiveError::CorruptObject(_)));
}

#[test]
fn current_pointer_cannot_alias_a_manifest_at_another_path() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    archive
        .append_block(&canonical_block(300, 30, 1))
        .expect("archive block");
    let dataset = temporary
        .path()
        .join("chain=mainnet/dataset=canonical_events");
    let current_path = dataset.join("CURRENT");
    let mut pointer: serde_json::Value =
        serde_json::from_slice(&fs::read(&current_path).expect("read current pointer"))
            .expect("parse current pointer");
    let original_relative = pointer["manifest_relative_path"]
        .as_str()
        .expect("manifest path");
    let alias_relative = "chain=mainnet/dataset=canonical_events/alias-catalog.json";
    fs::copy(
        temporary.path().join(original_relative),
        temporary.path().join(alias_relative),
    )
    .expect("copy catalog to alias");
    pointer["manifest_relative_path"] = serde_json::Value::String(alias_relative.to_owned());
    fs::write(
        &current_path,
        serde_json::to_vec(&pointer).expect("serialize pointer"),
    )
    .expect("replace current pointer");

    let error = read_range_error(&archive, 300, 300);
    assert!(matches!(error, ArchiveError::ManifestVerification(_)));
}

#[cfg(unix)]
#[test]
fn symlinked_archive_object_is_rejected() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let receipt = archive
        .append_block(&canonical_block(400, 40, 1))
        .expect("archive block");
    let verified = archive
        .verify_manifest(receipt.manifest_id())
        .expect("verify block manifest");
    let relative = verified.objects()[0].relative_path();
    let object_path = temporary.path().join(relative);
    let backing_path = object_path.with_extension("backing");
    fs::rename(&object_path, &backing_path).expect("move object to backing path");
    symlink(
        backing_path.file_name().expect("backing file name"),
        &object_path,
    )
    .expect("create object symlink");

    let error = read_range_error(&archive, 400, 400);
    assert!(matches!(error, ArchiveError::UnsafePath));
}

#[test]
fn raw_batch_rejects_cursor_gaps_and_mixed_sources() {
    let gapped = vec![
        raw_observation("primary-node", "epoch-a", 1, b"one"),
        raw_observation("primary-node", "epoch-a", 3, b"three"),
    ];
    assert!(matches!(
        RawObservationBatch::try_new(
            ChainId::new("mainnet").expect("chain ID"),
            gapped,
            [1; 32],
            [2; 32]
        ),
        Err(ArchiveError::InvalidInput(_))
    ));

    let mixed = vec![
        raw_observation("primary-node", "epoch-a", 1, b"one"),
        raw_observation("independent-node", "epoch-a", 2, b"two"),
    ];
    assert!(matches!(
        RawObservationBatch::try_new(
            ChainId::new("mainnet").expect("chain ID"),
            mixed,
            [1; 32],
            [2; 32]
        ),
        Err(ArchiveError::InvalidInput(_))
    ));
}

#[test]
fn raw_archive_rejects_a_batch_that_crosses_an_hour_partition() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let batch = RawObservationBatch::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        vec![
            raw_observation_at(
                "primary-node",
                "epoch-a",
                1,
                b"last-in-hour",
                1_721_782_799_999_999,
            ),
            raw_observation_at(
                "primary-node",
                "epoch-a",
                2,
                b"first-in-next-hour",
                1_721_782_800_000_000,
            ),
        ],
        [0xa1; 32],
        [0xb2; 32],
    )
    .expect("valid contiguous source batch");

    let error = archive
        .append_batch(&batch)
        .expect_err("one archive object cannot cross an hour partition");
    assert!(matches!(error, ArchiveError::InvalidInput(_)));
    assert!(
        temporary
            .path()
            .join("chain=mainnet/dataset=raw_source_observations")
            .read_dir()
            .is_err(),
        "rejected batch must publish no archive state"
    );
}

#[test]
fn configured_read_limits_fail_closed_before_returning_an_iterator() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let config =
        ArchiveConfig::deterministic_fixture("archive-test-build", known(1_721_779_300_000_000))
            .expect("archive config")
            .with_read_limits(1, 1024 * 1024)
            .expect("bounded archive config");
    let archive = LocalParquetArchive::open(temporary.path(), config).expect("open archive");
    archive
        .append_block(&canonical_block(500, 50, 1))
        .expect("first block");
    archive
        .append_block(&canonical_block(501, 51, 1))
        .expect("second block");

    let error = read_range_error(&archive, 500, 501);
    assert!(matches!(error, ArchiveError::InvalidInput(_)));
}

#[test]
fn compaction_is_idempotent_preserves_replay_and_keeps_prior_objects() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let blocks = vec![
        canonical_block(600, 60, 2),
        canonical_block(601, 61, 0),
        canonical_block(602, 62, 1),
    ];
    let old_objects = blocks
        .iter()
        .map(|block| {
            let receipt = archive.append_block(block).expect("archive input block");
            archive
                .verify_manifest(receipt.manifest_id())
                .expect("verify input manifest")
                .objects()[0]
                .relative_path()
                .to_path_buf()
        })
        .collect::<Vec<_>>();
    let range = BlockRange::new(BlockHeight::new(600), BlockHeight::new(602)).expect("range");
    let chain = ChainId::new("mainnet").expect("chain ID");

    let first = archive
        .compact_range(&chain, range)
        .expect("compact canonical range");
    let duplicate = archive
        .compact_range(&chain, range)
        .expect("repeat identical compaction");
    assert_eq!(first, duplicate);
    assert_eq!(first.input_object_count(), 3);
    assert_eq!(first.block_range(), range);
    assert_eq!(first.row_count(), 3);

    let replayed = archive
        .read_range(&chain, range)
        .expect("open compacted range")
        .collect::<Result<Vec<_>, _>>()
        .expect("read compacted range");
    assert_eq!(replayed, blocks);
    for relative in old_objects {
        assert!(
            temporary.path().join(relative).is_file(),
            "compaction must not delete prior immutable objects"
        );
    }
}

#[test]
fn archive_inspection_verifies_reachable_canonical_and_raw_objects() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    archive
        .append_block(&canonical_block(700, 70, 2))
        .expect("archive canonical block");
    archive
        .append_batch(&raw_batch(
            "primary-node",
            "epoch-a",
            700,
            &[b"raw-seven-hundred"],
        ))
        .expect("archive raw observation");

    let inspection = archive.inspect().expect("inspect archive");
    assert_eq!(inspection.canonical_chains(), 1);
    assert_eq!(inspection.raw_sources(), 1);
    assert_eq!(inspection.canonical_blocks(), 1);
    assert_eq!(inspection.canonical_events(), 2);
    assert_eq!(inspection.raw_observations(), 1);
    assert_eq!(inspection.objects().len(), 2);
}

#[test]
fn raw_partition_manifest_chain_is_verified_to_its_root() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    archive
        .append_batch(&raw_batch(
            "primary-node",
            "epoch-a",
            1,
            &[b"raw-one", b"raw-two"],
        ))
        .expect("first raw batch");
    archive
        .append_batch(&raw_batch(
            "primary-node",
            "epoch-a",
            3,
            &[b"raw-three", b"raw-four"],
        ))
        .expect("second raw batch");

    let manifests = temporary
        .path()
        .join(
            "chain=mainnet/dataset=raw_source_observations/source=primary-node/date=2024-07-24/hour=00/manifests",
        );
    let head = fs::read_dir(&manifests)
        .expect("list raw partition manifests")
        .filter_map(Result::ok)
        .find_map(|entry| {
            let bytes = fs::read(entry.path()).ok()?;
            let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            value["previous_manifest_sha256"]
                .as_str()
                .map(ToOwned::to_owned)
        })
        .expect("second generation references partition root");
    fs::remove_file(manifests.join(format!("partition-{head}.json")))
        .expect("remove partition root manifest");

    assert!(
        archive.inspect().is_err(),
        "inspection must reject a partition chain whose root is missing"
    );
}

#[test]
fn raw_batch_append_is_idempotent_and_round_trips_exact_observations() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let batch = raw_batch(
        "primary-node",
        "epoch-a",
        10,
        &[b"{\"height\":10}", b"{\"height\":11}"],
    );

    let first = archive.append_batch(&batch).expect("append raw batch");
    let duplicate = archive.append_batch(&batch).expect("append matching batch");
    assert_eq!(first, duplicate);

    let source = SourceId::new("primary-node").expect("source ID");
    let chain = ChainId::new("mainnet").expect("chain ID");
    let range = RawObservationRange::try_new("epoch-a", 10, 11).expect("raw range");
    let observations = archive
        .read_observations(&chain, &source, range)
        .expect("open verified raw range")
        .collect::<Result<Vec<_>, _>>()
        .expect("read raw range");
    assert_eq!(observations.len(), 2);
    for (actual, expected) in observations.iter().zip(batch.observations()) {
        assert_observation_eq(actual, expected);
    }

    let verified = archive
        .verify_raw_manifest(first.manifest_id())
        .expect("verify raw manifest");
    assert_eq!(verified.object().row_count(), 2);
    assert_eq!(verified.spool_manifest_blake3(), [0xa1; 32]);
    assert_eq!(verified.spool_segment_blake3(), [0xb2; 32]);
}

#[test]
fn overlapping_raw_cursor_ranges_with_different_content_fail_closed() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    archive
        .append_batch(&raw_batch(
            "primary-node",
            "epoch-a",
            30,
            &[b"thirty", b"thirty-one"],
        ))
        .expect("append first raw batch");

    let error = archive
        .append_batch(&raw_batch(
            "primary-node",
            "epoch-a",
            31,
            &[b"changed", b"thirty-two"],
        ))
        .expect_err("overlapping cursor range must fail");
    assert!(matches!(error, ArchiveError::ConflictingRawRange { .. }));
}

#[test]
fn corrupt_raw_object_is_rejected_before_an_iterator_is_returned() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = archive(&temporary).expect("open archive");
    let batch = raw_batch("primary-node", "epoch-a", 20, &[b"twenty", b"twenty-one"]);
    let receipt = archive.append_batch(&batch).expect("append raw batch");
    let verified = archive
        .verify_raw_manifest(receipt.manifest_id())
        .expect("verify raw manifest");
    let object_path = temporary.path().join(verified.object().relative_path());
    let mut bytes = fs::read(&object_path).expect("read raw object");
    let middle = bytes.len() / 2;
    bytes[middle] ^= 0x55;
    fs::write(&object_path, bytes).expect("corrupt raw object");

    let source = SourceId::new("primary-node").expect("source ID");
    let chain = ChainId::new("mainnet").expect("chain ID");
    let range = RawObservationRange::try_new("epoch-a", 20, 21).expect("raw range");
    let error = match archive.read_observations(&chain, &source, range) {
        Ok(_) => panic!("corruption must fail before returning an iterator"),
        Err(error) => error,
    };
    assert!(matches!(error, ArchiveError::CorruptObject(_)));
}

fn assert_observation_eq(actual: &SourceObservation, expected: &SourceObservation) {
    assert_eq!(actual.source_id(), expected.source_id());
    assert_eq!(actual.source_version(), expected.source_version());
    assert_eq!(actual.observation_class(), expected.observation_class());
    assert_eq!(actual.cursor(), expected.cursor());
    assert_eq!(actual.received(), expected.received());
    assert_eq!(
        actual.parser_schema_version(),
        expected.parser_schema_version()
    );
    assert_eq!(actual.payload(), expected.payload());
    assert_eq!(actual.content_hash(), expected.content_hash());
    assert_eq!(actual.warnings(), expected.warnings());
}

fn read_range_error(archive: &LocalParquetArchive, start: u64, end: u64) -> ArchiveError {
    let range =
        BlockRange::new(BlockHeight::new(start), BlockHeight::new(end)).expect("valid range");
    match archive.read_range(&ChainId::new("mainnet").expect("chain ID"), range) {
        Ok(_) => panic!("archive read was expected to fail before returning an iterator"),
        Err(error) => error,
    }
}
