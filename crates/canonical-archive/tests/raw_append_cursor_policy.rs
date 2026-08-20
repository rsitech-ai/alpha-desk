use std::path::Path;

use bytes::Bytes;
use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use domain_types::{ChainId, KnownTime, SourceId};
use hl_protocol::{ObservationClass, ReceiveTimestamps, SourceCursor, SourceObservation};
use storage_ports::{
    CursorPolicy, LocalRecordSequence, RawObservationArchive, RawObservationBatch,
};

#[test]
fn append_batch_covers_every_constructible_cursor_policy() {
    for cursor_policy in [
        CursorPolicy::ContiguousNativeOffset,
        CursorPolicy::MonotonicByteOffset,
    ] {
        let temporary = tempfile::tempdir().expect("archive root");
        let archive = LocalParquetArchive::open(
            temporary.path(),
            ArchiveConfig::deterministic_fixture(
                "raw-append-policy-test",
                KnownTime::from_unix_micros(1_721_779_300_000_000).expect("time"),
            )
            .expect("config"),
        )
        .expect("archive");
        let batch = match cursor_policy {
            CursorPolicy::ContiguousNativeOffset => contiguous_batch(),
            CursorPolicy::MonotonicByteOffset => byte_offset_batch(),
        };
        let source = batch
            .observations()
            .first()
            .expect("fixture batch is nonempty")
            .source_id();
        let receipt = archive
            .append_batch(&batch)
            .expect("append still admits both constructible policies");

        match cursor_policy {
            CursorPolicy::MonotonicByteOffset => {
                assert_eq!(receipt.cursor_policy(), CursorPolicy::MonotonicByteOffset);
                assert!(
                    receipt.receipt_id().starts_with("raw-archive-receipt-v2-"),
                    "MonotonicByteOffset still takes the V2 byte-offset append path"
                );
                assert!(receipt.local_sequence_range().is_some());
                assert!(
                    current_pointer(temporary.path(), "raw_source_observations_byte_v2", source)
                        .is_file(),
                    "byte-offset append still publishes the V2 CURRENT pointer"
                );
                assert!(
                    !current_pointer(temporary.path(), "raw_source_observations", source).exists(),
                    "byte-offset append still does not inherit the legacy contiguous dataset"
                );
            }
            CursorPolicy::ContiguousNativeOffset => {
                assert_eq!(
                    receipt.cursor_policy(),
                    CursorPolicy::ContiguousNativeOffset
                );
                assert!(
                    receipt.receipt_id().starts_with("raw-archive-receipt-v1-"),
                    "ContiguousNativeOffset still takes the legacy contiguous append path"
                );
                assert!(receipt.local_sequence_range().is_none());
                assert!(
                    current_pointer(temporary.path(), "raw_source_observations", source).is_file(),
                    "contiguous append still publishes the legacy CURRENT pointer"
                );
                assert!(
                    !current_pointer(temporary.path(), "raw_source_observations_byte_v2", source)
                        .exists(),
                    "contiguous append still does not take the V2 byte-offset dataset"
                );
            }
        }
    }
}

fn contiguous_batch() -> RawObservationBatch {
    RawObservationBatch::try_new(
        ChainId::new("mainnet").expect("chain"),
        vec![observation(
            "primary-node",
            ObservationClass::CommittedBlock,
            10,
            b"legacy",
        )],
        [0xa1; 32],
        [0xb2; 32],
    )
    .expect("contiguous batch")
}

fn byte_offset_batch() -> RawObservationBatch {
    RawObservationBatch::try_new_byte_offsets(
        ChainId::new("mainnet").expect("chain"),
        vec![observation(
            "node-trades",
            ObservationClass::AuxiliaryLedger,
            19,
            b"byte",
        )],
        [0xa1; 32],
        [0xb2; 32],
        LocalRecordSequence::try_new(41).expect("local sequence"),
    )
    .expect("byte-offset batch")
}

fn observation(
    source: &str,
    observation_class: ObservationClass,
    offset: u64,
    payload: &[u8],
) -> SourceObservation {
    SourceObservation::new(
        SourceId::new(source).expect("source"),
        "capture-v1",
        observation_class,
        SourceCursor::new("epoch-a", offset).expect("cursor"),
        ReceiveTimestamps::new(1_721_779_200_000_000, 9_000_000).expect("timestamps"),
        "raw-parser-v1",
        Bytes::copy_from_slice(payload),
        Vec::new(),
        1024,
    )
    .expect("observation")
}

fn current_pointer(root: &Path, dataset: &str, source: &SourceId) -> std::path::PathBuf {
    root.join(format!(
        "chain=mainnet/dataset={dataset}/source={}/CURRENT",
        source.as_str()
    ))
}
