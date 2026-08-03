use domain_types::{ChainId, SourceId};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{ArchiveError, CursorPolicy, LocalRecordSequence, RawObservationBatch};

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
    assert_eq!(sequenced[0].observation().cursor().offset(), 19);
    assert_eq!(sequenced[1].observation().cursor().offset(), 20);
    assert_eq!(sequenced[2].observation().cursor().offset(), 47);
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
