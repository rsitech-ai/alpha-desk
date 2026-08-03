use std::path::PathBuf;

use domain_types::{ChainId, KnownTime, ManifestId, SourceId};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    ArchiveError, CursorPolicy, LocalRecordSequence, LocalRecordSequenceRange,
    OwnedSequencedSourceObservation, RawArchiveObject, RawObservationBatch, RawObservationRange,
    RawObservationReceipt,
};

fn observation(epoch: &str, offset: u64) -> SourceObservation {
    observation_with(
        "node-trades",
        "node-v1",
        ObservationClass::AuxiliaryLedger,
        epoch,
        offset,
        "node-trades-v1",
    )
}

fn observation_with(
    source_id: &str,
    source_version: &str,
    observation_class: ObservationClass,
    epoch: &str,
    offset: u64,
    parser_schema_version: &str,
) -> SourceObservation {
    SourceObservation::new(
        SourceId::new(source_id).expect("source ID"),
        source_version,
        observation_class,
        SourceCursor::new(epoch, offset).expect("cursor"),
        ReceiveTimestamps::new(1_722_000_000_000_000, offset).expect("receive timestamps"),
        parser_schema_version,
        format!("trade-at-{offset}").into(),
        Vec::new(),
        1024,
    )
    .expect("observation")
}

#[test]
fn byte_offset_batches_accept_gaps_and_derive_contiguous_local_sequences() {
    let batch = RawObservationBatch::try_new_byte_offsets(
        ChainId::new("mainnet").expect("chain ID"),
        vec![
            observation("rotation-7", 19),
            observation("rotation-7", 20),
            observation("rotation-7", 47),
        ],
        [1; 32],
        [2; 32],
        LocalRecordSequence::try_new(41).expect("nonzero local sequence"),
    )
    .expect("strictly increasing byte offsets");

    assert_eq!(batch.cursor_policy(), CursorPolicy::MonotonicByteOffset);
    let sequenced = batch
        .sequenced_observations()
        .expect("byte-offset policy has local sequences")
        .map(|item| item.expect("constructor prevalidated sequence range"))
        .collect::<Vec<_>>();
    assert_eq!(sequenced.len(), 3);
    assert_eq!(sequenced[0].local_sequence().get(), 41);
    assert_eq!(sequenced[1].local_sequence().get(), 42);
    assert_eq!(sequenced[2].local_sequence().get(), 43);
    assert_eq!(batch.last_local_sequence().unwrap().get(), 43);
    assert_eq!(batch.local_sequence_range().unwrap().len(), 3);
    assert_eq!(sequenced[0].observation().cursor().offset(), 19);
    assert_eq!(sequenced[1].observation().cursor().offset(), 20);
    assert_eq!(sequenced[2].observation().cursor().offset(), 47);
}

#[test]
fn local_sequence_ranges_are_inclusive_checked_and_preserve_owned_observations() {
    let range = LocalRecordSequenceRange::try_new(
        LocalRecordSequence::try_new(41).unwrap(),
        LocalRecordSequence::try_new(43).unwrap(),
    )
    .expect("ordered inclusive range");
    assert_eq!(range.start().get(), 41);
    assert_eq!(range.end().get(), 43);
    assert_eq!(range.len(), 3);

    let observation = observation("rotation-7", 47);
    let sequenced = OwnedSequencedSourceObservation::new(observation.clone(), range.end());
    assert_eq!(sequenced.local_sequence(), range.end());
    assert_eq!(sequenced.observation().cursor(), observation.cursor());
    assert_eq!(sequenced.observation().payload(), observation.payload());
    let recovered = sequenced.into_observation();
    assert_eq!(recovered.cursor(), observation.cursor());
    assert_eq!(recovered.payload(), observation.payload());
    let (sequence, recovered) =
        OwnedSequencedSourceObservation::new(observation.clone(), range.start()).into_parts();
    assert_eq!(sequence, range.start());
    assert_eq!(recovered.payload(), observation.payload());

    assert_invalid(
        LocalRecordSequenceRange::try_new(
            LocalRecordSequence::try_new(43).unwrap(),
            LocalRecordSequence::try_new(41).unwrap(),
        )
        .expect_err("sequence range cannot regress"),
        "local record sequence range",
    );
}

#[test]
fn byte_archive_evidence_exposes_policy_and_authenticated_sequence_range() {
    let chain = ChainId::new("mainnet").unwrap();
    let source = SourceId::new("node-trades").unwrap();
    let legacy_object = RawArchiveObject::try_new(
        PathBuf::from("raw/legacy.parquet"),
        [0; 32],
        128,
        3,
        chain.clone(),
        source.clone(),
        RawObservationRange::try_new("legacy", 1, 3).unwrap(),
    )
    .unwrap();
    assert_eq!(
        legacy_object.cursor_policy(),
        CursorPolicy::ContiguousNativeOffset
    );
    assert!(legacy_object.local_sequence_range().is_none());
    let legacy_receipt = RawObservationReceipt::try_new(
        "legacy-receipt",
        ManifestId::new("legacy-manifest").unwrap(),
        chain.clone(),
        source.clone(),
        "legacy",
        1,
        3,
        [1; 32],
        [2; 32],
        [3; 32],
        [4; 32],
        [5; 32],
        [6; 32],
        KnownTime::from_unix_micros(1_722_000_000_000_000).unwrap(),
    )
    .unwrap();
    assert_eq!(
        legacy_receipt.cursor_policy(),
        CursorPolicy::ContiguousNativeOffset
    );
    assert!(legacy_receipt.local_sequence_range().is_none());

    let cursor_range = RawObservationRange::try_new("rotation-7", 19, 47).unwrap();
    let sequence_range = LocalRecordSequenceRange::try_new(
        LocalRecordSequence::try_new(41).unwrap(),
        LocalRecordSequence::try_new(43).unwrap(),
    )
    .unwrap();
    let object = RawArchiveObject::try_new_byte_offsets(
        PathBuf::from("raw/object.parquet"),
        [1; 32],
        128,
        3,
        chain.clone(),
        source.clone(),
        cursor_range,
        sequence_range,
    )
    .expect("byte object evidence");
    assert_eq!(object.cursor_policy(), CursorPolicy::MonotonicByteOffset);
    assert_eq!(object.local_sequence_range(), Some(sequence_range));

    let error = RawArchiveObject::try_new_byte_offsets(
        PathBuf::from("raw/bad.parquet"),
        [1; 32],
        128,
        2,
        chain.clone(),
        source.clone(),
        RawObservationRange::try_new("rotation-7", 19, 47).unwrap(),
        sequence_range,
    )
    .expect_err("object rows must cover the exact local sequence span");
    assert_invalid(error, "raw archive object local sequence span");

    let receipt = RawObservationReceipt::try_new_byte_offsets(
        "receipt-41-43",
        ManifestId::new("manifest-41-43").unwrap(),
        chain,
        source,
        "rotation-7",
        19,
        47,
        sequence_range,
        [2; 32],
        [3; 32],
        [4; 32],
        [5; 32],
        [6; 32],
        [7; 32],
        KnownTime::from_unix_micros(1_722_000_000_000_000).unwrap(),
    )
    .expect("byte receipt evidence");
    assert_eq!(receipt.cursor_policy(), CursorPolicy::MonotonicByteOffset);
    assert_eq!(receipt.local_sequence_range(), Some(sequence_range));
}

#[test]
fn local_record_sequences_are_nonzero_and_checked() {
    assert_invalid(
        LocalRecordSequence::try_new(0).expect_err("zero has no durable ordering meaning"),
        "local record sequence is zero",
    );

    let maximum = LocalRecordSequence::try_new(u64::MAX).expect("nonzero maximum");
    assert_eq!(maximum.get(), u64::MAX);
    assert_invalid(
        maximum.checked_next().expect_err("maximum cannot advance"),
        "local record sequence overflows",
    );
}

#[test]
fn legacy_constructor_keeps_native_offsets_contiguous() {
    let batch = RawObservationBatch::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        vec![
            observation("block-height", 0),
            observation("block-height", 1),
        ],
        [3; 32],
        [4; 32],
    )
    .expect("legacy contiguous offsets");

    assert_eq!(batch.cursor_policy(), CursorPolicy::ContiguousNativeOffset);
    assert_eq!(batch.observations()[0].cursor().offset(), 0);
    assert_eq!(batch.observations()[1].cursor().offset(), 1);

    let error = RawObservationBatch::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        vec![
            observation("block-height", 0),
            observation("block-height", 2),
        ],
        [3; 32],
        [4; 32],
    )
    .expect_err("legacy gaps must remain invalid");
    assert_invalid(error, "raw observation cursors are not contiguous");
}

#[test]
fn independent_legacy_batches_make_no_local_sequence_claim() {
    let first = RawObservationBatch::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        vec![
            observation("block-height", 40),
            observation("block-height", 41),
        ],
        [3; 32],
        [4; 32],
    )
    .expect("first legacy batch");
    let second = RawObservationBatch::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        vec![
            observation("block-height", 90),
            observation("block-height", 91),
        ],
        [5; 32],
        [6; 32],
    )
    .expect("independent legacy batch");

    for batch in [&first, &second] {
        assert_eq!(batch.cursor_policy(), CursorPolicy::ContiguousNativeOffset);
        assert!(batch.first_local_sequence().is_none());
        assert!(batch.last_local_sequence().is_none());
        assert!(batch.local_sequence_range().is_none());
        assert!(batch.sequenced_observations().is_none());
    }
}

#[test]
fn byte_offset_batches_reject_duplicate_and_regressing_native_offsets() {
    for (offsets, expected) in [
        ([19, 19], "raw observation byte offsets are duplicated"),
        ([19, 18], "raw observation byte offsets regress"),
    ] {
        let error = byte_offset_batch(
            offsets
                .into_iter()
                .map(|offset| observation("rotation-7", offset))
                .collect(),
            1,
        )
        .expect_err("native byte positions must strictly increase");
        assert_invalid(error, expected);
    }
}

#[test]
fn byte_offset_batches_reject_mixed_epochs_and_metadata() {
    let mixed_epoch = byte_offset_batch(
        vec![observation("rotation-7", 19), observation("rotation-8", 47)],
        1,
    )
    .expect_err("one durable batch cannot cross rotation epochs");
    assert_invalid(
        mixed_epoch,
        "raw observation batch cursor epochs are inconsistent",
    );

    for changed in [
        observation_with(
            "other-source",
            "node-v1",
            ObservationClass::AuxiliaryLedger,
            "rotation-7",
            47,
            "node-trades-v1",
        ),
        observation_with(
            "node-trades",
            "node-v2",
            ObservationClass::AuxiliaryLedger,
            "rotation-7",
            47,
            "node-trades-v1",
        ),
        observation_with(
            "node-trades",
            "node-v1",
            ObservationClass::AuxiliaryBookDiff,
            "rotation-7",
            47,
            "node-trades-v1",
        ),
        observation_with(
            "node-trades",
            "node-v1",
            ObservationClass::AuxiliaryLedger,
            "rotation-7",
            47,
            "node-trades-v2",
        ),
    ] {
        let error = byte_offset_batch(vec![observation("rotation-7", 19), changed], 1)
            .expect_err("batch metadata must identify one source projection");
        assert_invalid(error, "raw observation batch metadata is inconsistent");
    }
}

#[test]
fn byte_offset_batches_reject_local_sequence_overflow() {
    let error = byte_offset_batch(
        vec![observation("rotation-7", 19), observation("rotation-7", 47)],
        u64::MAX,
    )
    .expect_err("every observation requires a representable local sequence");
    assert_invalid(error, "local record sequence overflows");
}

#[test]
fn byte_offset_policy_rejects_block_height_observation_classes() {
    for observation_class in [
        ObservationClass::CommittedBlock,
        ObservationClass::HistoricalBlock,
    ] {
        let error = byte_offset_batch(
            vec![observation_with(
                "node-blocks",
                "node-v1",
                observation_class,
                "node-session-7",
                19,
                "node-block-v1",
            )],
            1,
        )
        .expect_err("block-height classes cannot claim byte-offset cursor semantics");
        assert_invalid(
            error,
            "byte-offset cursor policy is incompatible with block-height observation class",
        );
    }
}

fn byte_offset_batch(
    observations: Vec<SourceObservation>,
    first_local_sequence: u64,
) -> Result<RawObservationBatch, ArchiveError> {
    RawObservationBatch::try_new_byte_offsets(
        ChainId::new("mainnet").expect("chain ID"),
        observations,
        [1; 32],
        [2; 32],
        LocalRecordSequence::try_new(first_local_sequence).expect("nonzero sequence fixture"),
    )
}

fn assert_invalid(error: ArchiveError, detail: &str) {
    assert_eq!(error.reason_code(), "archive.invalid_input");
    assert_eq!(
        error.to_string(),
        format!("archive input is invalid: {detail}")
    );
}
