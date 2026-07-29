use std::collections::BTreeMap;

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, KnownTime, ProtocolTime, SourceId};
use storage_ports::CanonicalArchive;

#[test]
fn immutable_manifest_read_does_not_follow_a_later_current_generation() {
    let temporary = tempfile::tempdir().expect("archive root");
    let archive = LocalParquetArchive::open(
        temporary.path(),
        ArchiveConfig::deterministic_fixture(
            "manifest-read-test",
            KnownTime::from_unix_micros(1_000).expect("time"),
        )
        .expect("config"),
    )
    .expect("archive");
    let first = block(500);
    let second = block(501);
    let first_receipt = archive.append_block(&first).expect("first");
    archive.append_block(&second).expect("second");

    let blocks = archive
        .read_manifest_blocks(first_receipt.manifest_id())
        .expect("exact manifest")
        .collect::<Result<Vec<_>, _>>()
        .expect("exact blocks");

    assert_eq!(blocks, vec![first]);
}

fn block(height: u64) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(
            SourceId::new("test-primary").expect("source"),
            [u8::try_from(height % 251).expect("bounded fixture byte"); 32],
        )]),
    )
    .expect("block")
}
