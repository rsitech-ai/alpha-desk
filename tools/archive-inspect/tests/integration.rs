use std::collections::BTreeMap;

use archive_inspect::{count, verify};
use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, Price, ProtocolTime, Quantity, SourceId, TransactionId,
};
use storage_ports::{CanonicalArchive, RawObservationArchive};

#[tokio::test]
async fn datafusion_count_matches_verified_manifest_on_a_real_archive() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = LocalParquetArchive::open(
        temporary.path(),
        ArchiveConfig::deterministic_fixture("archive-inspect-test", known(1_700_000_000_000_000))
            .expect("archive config"),
    )
    .expect("open archive");
    archive
        .append_block(&canonical_block())
        .expect("archive canonical block");

    let verified = verify(temporary.path()).expect("verify archive");
    assert_eq!(verified.inspection().canonical_blocks(), 1);
    assert_eq!(verified.inspection().canonical_events(), 1);

    let counted = count(temporary.path())
        .await
        .expect("count with DataFusion");
    assert_eq!(counted.canonical_objects(), 1);
    assert_eq!(counted.canonical_events(), 1);
    assert_eq!(counted.v3_sources(), 0);
    assert_eq!(counted.v3_logical_rows(), 0);
    assert_eq!(counted.v3_logical_manifests(), 0);
}

#[cfg(unix)]
#[test]
fn verify_rejects_a_dangling_raw_dataset_symlink() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary archive");
    let archive = LocalParquetArchive::open(
        temporary.path(),
        ArchiveConfig::deterministic_fixture("archive-inspect-test", known(1_700_000_000_000_000))
            .expect("archive config"),
    )
    .expect("open archive");
    archive
        .append_block(&canonical_block())
        .expect("archive canonical block");
    symlink(
        "missing-raw-dataset",
        temporary
            .path()
            .join("chain=mainnet/dataset=raw_source_observations_byte_v2"),
    )
    .expect("create dangling raw dataset symlink");

    let error = verify(temporary.path()).expect_err("unsafe dataset must fail verification");
    assert_eq!(error.reason_code(), "archive.unsafe_path");
}

#[cfg(unix)]
#[test]
fn verify_rejects_a_dangling_v3_dataset_symlink() {
    use std::os::unix::fs::symlink;

    let temporary = tempfile::tempdir().expect("temporary archive");
    write_v3_dataset(temporary.path());
    let dataset = temporary.path().join(format!(
        "chain=mainnet/dataset={}",
        canonical_archive::raw_v3::RAW_BYTE_DATASET_V3
    ));
    std::fs::rename(&dataset, temporary.path().join("v3-real")).expect("move v3 dataset");
    symlink("missing-v3-dataset", &dataset).expect("create dangling v3 dataset symlink");

    let error = verify(temporary.path()).expect_err("unsafe v3 dataset must fail verification");
    assert_eq!(error.reason_code(), "archive.unsafe_path");
}

#[tokio::test]
async fn v3_verify_and_count_replay_logical_rows() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    write_v3_dataset(temporary.path());

    let verified = verify(temporary.path()).expect("verify v3 archive");
    assert!(verified.inspection().objects().is_empty());
    let v3 = verified.v3().expect("v3 inspection");
    assert_eq!(v3.sources().len(), 1);
    assert_eq!(v3.logical_manifest_count(), 1);
    assert_eq!(v3.logical_row_count(), 1);

    let counted = count(temporary.path())
        .await
        .expect("count v3 with sequence replay");
    assert_eq!(counted.canonical_events(), 0);
    assert_eq!(counted.canonical_objects(), 0);
    assert_eq!(counted.v3_sources(), 1);
    assert_eq!(counted.v3_logical_rows(), 1);
    assert_eq!(counted.v3_logical_manifests(), 1);
}

#[test]
fn v3_scrub_stats_and_health_inspect_a_verified_dataset() {
    let temporary = tempfile::tempdir().expect("temporary archive");
    write_v3_dataset(temporary.path());

    let scrubbed = archive_inspect::scrub_v3(temporary.path()).expect("scrub v3");
    assert_eq!(scrubbed.sources().len(), 1);
    assert_eq!(scrubbed.sources()[0].scrub().logical_manifest_count(), 1);
    let stats = archive_inspect::stats_v3(temporary.path()).expect("stats v3");
    assert_eq!(stats.sources()[0].statistics().logical_row_count(), 1);
    let health = archive_inspect::health_v3(temporary.path()).expect("health v3");
    assert_eq!(health.sources().len(), 1);
}

fn write_v3_dataset(root: &std::path::Path) {
    let (workload, budgets) = (
        storage_ports::RawArchiveWorkloadEnvelope::try_new(
            100,
            1,
            1_000,
            3_600,
            1_024,
            1_000,
            64 * 1024 * 1024,
            64,
        )
        .expect("workload"),
        storage_ports::RawArchiveCapacityBudgets::try_new(
            u64::MAX,
            u64::MAX,
            u64::MAX,
            u64::MAX,
            true,
        )
        .expect("budgets"),
    );
    let archive = canonical_archive::RawV3Archive::open(
        root,
        ArchiveConfig::deterministic_fixture("archive-inspect-v3", known(1_722_000_000_000_000))
            .expect("archive config"),
        workload,
        budgets,
    )
    .expect("open v3 archive");
    let chain = ChainId::new("mainnet").expect("chain ID");
    let observation = hl_protocol::SourceObservation::new(
        SourceId::new("node-fills").expect("source"),
        "capture-v1",
        hl_protocol::ObservationClass::AuxiliaryLedger,
        hl_protocol::SourceCursor::new("epoch-1", 10).expect("cursor"),
        hl_protocol::ReceiveTimestamps::new(1_722_000_000_000_000, 10).expect("receive"),
        "raw-parser-v1",
        bytes::Bytes::from_static(b"ab"),
        Vec::new(),
        1024,
    )
    .expect("observation");
    let batch = storage_ports::RawObservationBatch::try_new_byte_offsets(
        chain,
        vec![observation],
        [0x11; 32],
        [0x22; 32],
        storage_ports::LocalRecordSequence::try_new(1).expect("sequence"),
    )
    .expect("batch");
    archive.append_batch(&batch).expect("append v3 batch");
}

fn canonical_block() -> BlockEnvelope {
    let chain = ChainId::new("mainnet").expect("chain ID");
    let source = SourceId::new("primary-node").expect("source ID");
    let time = ProtocolTime::from_unix_micros(1_700_000_000_000_000).expect("protocol time");
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: chain.clone(),
        block_height: BlockHeight::new(42),
        block_time: time,
        transaction_id: TransactionId::new("tx-42").expect("transaction ID"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: Vec::new(),
        source_evidence: vec![
            SourceEvidence::try_new(source.clone(), "node-v1", "block-42:0", [0x42; 32])
                .expect("source evidence"),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: known(1_700_000_000_000_000),
        ingested_at: known(1_700_000_000_000_001),
        canonicalized_at: known(1_700_000_000_000_002),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            1,
        )),
    })
    .expect("canonical event");
    BlockEnvelope::try_new(
        chain,
        BlockHeight::new(42),
        time,
        ConfirmationClass::CommittedPrimary,
        vec![event],
        BTreeMap::from([(source, [0x55; 32])]),
    )
    .expect("canonical block")
}

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}
