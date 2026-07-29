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
use storage_ports::CanonicalArchive;

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
