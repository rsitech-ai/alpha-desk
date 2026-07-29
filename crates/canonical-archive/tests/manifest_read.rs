use std::collections::BTreeMap;

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, BlockRange, ChainId, KnownTime, ProtocolTime, SourceId};
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

#[test]
fn range_plan_resolves_current_catalog_to_ordered_immutable_manifests() {
    let temporary = tempfile::tempdir().expect("archive root");
    let archive = LocalParquetArchive::open(
        temporary.path(),
        ArchiveConfig::deterministic_fixture(
            "manifest-plan-test",
            KnownTime::from_unix_micros(1_000).expect("time"),
        )
        .expect("config"),
    )
    .expect("archive");
    let first = archive.append_block(&block(600)).expect("first");
    let second = archive.append_block(&block(601)).expect("second");
    archive.append_block(&block(602)).expect("outside range");
    let chain = ChainId::new("mainnet").expect("chain");

    let plan = archive
        .plan_range(
            &chain,
            BlockRange::new(BlockHeight::new(600), BlockHeight::new(601)).expect("range"),
        )
        .expect("plan");

    assert_eq!(plan.len(), 2);
    assert_eq!(plan[0].manifest_id(), first.manifest_id());
    assert_eq!(plan[1].manifest_id(), second.manifest_id());
    assert_eq!(
        plan[0].block_range(),
        BlockRange::new(BlockHeight::new(600), BlockHeight::new(600)).expect("first range")
    );
    assert_eq!(
        plan[1].block_range(),
        BlockRange::new(BlockHeight::new(601), BlockHeight::new(601)).expect("second range")
    );
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
