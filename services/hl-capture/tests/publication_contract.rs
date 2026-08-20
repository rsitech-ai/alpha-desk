use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventKind,
    EventPayload, SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, ManifestId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};
use hl_capture::bus::{
    CANONICAL_STREAM, CommittedPublicationBatch, PublicationDisposition, PublicationError,
    PublicationLedger, Subject, subject_for_event_kind,
};
use storage_ports::ArchiveReceipt;

const FROZEN_COMMITTED_PRIMARY_MARKER_SHA256: &str =
    "f4ec375bab6832c6ca3c06b5b736ab827a0b96024e5af0f6578d1a4d2abc8704";
const FROZEN_COMMITTED_INDEPENDENT_MARKER_SHA256: &str =
    "544a2aab99608aa308ce25a064c0eca22ef586338fd640942183012e7f619f6e";

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}

fn canonical_block(height: u64, seed: u64) -> BlockEnvelope {
    classified_block(height, seed, ConfirmationClass::CommittedPrimary)
}

fn classified_block(height: u64, seed: u64, confirmation: ConfirmationClass) -> BlockEnvelope {
    let block_time_micros = 1_721_779_200_000_000_i64
        .checked_add(i64::try_from(height).expect("height fits i64"))
        .expect("block time");
    let block_time =
        ProtocolTime::from_unix_micros(block_time_micros).expect("protocol block time");
    let source_id = SourceId::new("primary-node").expect("source ID");
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain ID"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction ID"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: Vec::new(),
        source_evidence: vec![
            SourceEvidence::try_new(
                source_id.clone(),
                "node-v1",
                format!("block-{height}:0"),
                [u8::try_from(seed).unwrap_or(0x7f); 32],
            )
            .expect("source evidence"),
        ],
        confirmation_class: confirmation,
        observed_at: known(block_time_micros),
        ingested_at: known(block_time_micros + 1),
        canonicalized_at: known(block_time_micros + 2),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            seed,
        )),
    })
    .expect("canonical event");

    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        BlockHeight::new(height),
        block_time,
        confirmation,
        vec![event],
        BTreeMap::from([(source_id, [0x55; 32])]),
    )
    .expect("canonical block")
}

fn archive_receipt(block: &BlockEnvelope) -> ArchiveReceipt {
    ArchiveReceipt::try_new(
        format!("receipt-{}", block.block_height().get()),
        ManifestId::new(format!(
            "manifest-{}",
            hex::encode(block.canonical_block_hash())
        ))
        .expect("manifest ID"),
        block.block_height(),
        block.canonical_block_hash(),
        [0x11; 32],
        [0x22; 32],
        [0x33; 32],
        known(1_721_779_300_000_000),
    )
    .expect("archive receipt")
}

#[test]
fn every_event_kind_routes_to_one_frozen_canonical_subject() {
    let fill = [
        EventKind::OrderPartiallyFilled,
        EventKind::OrderFilled,
        EventKind::TwapSliceFilled,
        EventKind::TradeMatched,
        EventKind::LiquidationFill,
        EventKind::BackstopLiquidation,
    ];
    let order = [
        EventKind::OrderAccepted,
        EventKind::OrderRested,
        EventKind::OrderModified,
        EventKind::OrderCancelled,
        EventKind::OrderRejected,
        EventKind::TriggerOrderActivated,
        EventKind::TwapStarted,
        EventKind::TwapCompleted,
    ];
    let oracle = [EventKind::OracleUpdated, EventKind::FundingRateUpdated];
    let market_meta = [
        EventKind::MarketHalted,
        EventKind::MarketResumed,
        EventKind::OpenInterestCapChanged,
        EventKind::MarginTableChanged,
        EventKind::MarketCreated,
        EventKind::MarketMetadataChanged,
        EventKind::AssetContextUpdated,
        EventKind::DexCreated,
        EventKind::OutcomeCreated,
        EventKind::OutcomeResolved,
    ];

    for kind in EventKind::ALL {
        let expected = if fill.contains(&kind) {
            Subject::EventFill
        } else if order.contains(&kind) {
            Subject::EventOrder
        } else if oracle.contains(&kind) {
            Subject::EventOracle
        } else if market_meta.contains(&kind) {
            Subject::EventMarketMeta
        } else {
            Subject::EventLedger
        };
        assert_eq!(subject_for_event_kind(kind), expected, "{kind:?}");
        assert_eq!(expected.stream(), CANONICAL_STREAM);
    }
}

#[test]
fn committed_publications_are_exact_archive_bound_and_deterministic() {
    let block = canonical_block(42, 7);
    let receipt = archive_receipt(&block);

    let first = CommittedPublicationBatch::try_new(&block, &receipt).expect("publication contract");
    let second =
        CommittedPublicationBatch::try_new(&block, &receipt).expect("repeat publication contract");

    assert_eq!(first, second);
    assert_eq!(first.block().subject(), Subject::BlockCommitted);
    assert_eq!(first.block().stream(), CANONICAL_STREAM);
    assert_eq!(first.block().block_height(), BlockHeight::new(42));
    assert_eq!(
        first.block().canonical_block_hash(),
        block.canonical_block_hash()
    );
    assert_eq!(first.block().archive_receipt_id(), receipt.receipt_id());
    assert_eq!(
        first.block().archive_manifest_sha256(),
        receipt.manifest_sha256()
    );
    assert_eq!(first.events().len(), 1);

    let event = &block.events()[0];
    let published = &first.events()[0];
    assert_eq!(published.subject(), Subject::EventFill);
    assert_eq!(published.message_id(), event.event_id().as_str());
    assert_eq!(
        published.payload(),
        event
            .encode_to_vec()
            .expect("canonical event bytes")
            .as_slice()
    );
    assert_eq!(
        published.message_id(),
        published.message_id().to_lowercase()
    );
}

#[test]
fn a_receipt_for_another_block_is_rejected_before_publication() {
    let block = canonical_block(42, 7);
    let another = canonical_block(43, 7);
    let error = CommittedPublicationBatch::try_new(&block, &archive_receipt(&another))
        .expect_err("mismatched archive receipt");

    assert_eq!(error, PublicationError::ArchiveReceiptMismatch);
    assert_eq!(error.reason_code(), "publication.archive_receipt_mismatch");
}

#[test]
fn duplicate_ids_require_identical_publication_bytes_and_binding() {
    let original = canonical_block(42, 7);
    let original_batch = CommittedPublicationBatch::try_new(&original, &archive_receipt(&original))
        .expect("original publication");
    let duplicate_batch =
        CommittedPublicationBatch::try_new(&original, &archive_receipt(&original))
            .expect("duplicate publication");
    let conflicting = canonical_block(42, 8);
    let conflicting_batch =
        CommittedPublicationBatch::try_new(&conflicting, &archive_receipt(&conflicting))
            .expect("conflicting publication contract");
    let mut ledger = PublicationLedger::new(8).expect("bounded ledger");

    assert_eq!(
        ledger
            .record(&original_batch.events()[0])
            .expect("first publication"),
        PublicationDisposition::New
    );
    assert_eq!(
        ledger
            .record(&duplicate_batch.events()[0])
            .expect("identical duplicate"),
        PublicationDisposition::IdenticalDuplicate
    );
    let error = ledger
        .record(&conflicting_batch.events()[0])
        .expect_err("same event ID with different bytes must diverge");
    assert!(matches!(
        error,
        PublicationError::DivergentMessageId { ref message_id }
            if message_id == original.events()[0].event_id().as_str()
    ));
    assert_eq!(ledger.len(), 1);
}

#[test]
fn publication_ledger_is_explicitly_bounded() {
    let first = canonical_block(42, 7);
    let second = canonical_block(43, 7);
    let first_batch = CommittedPublicationBatch::try_new(&first, &archive_receipt(&first))
        .expect("first publication");
    let second_batch = CommittedPublicationBatch::try_new(&second, &archive_receipt(&second))
        .expect("second publication");
    let mut ledger = PublicationLedger::new(1).expect("bounded ledger");

    ledger
        .record(&first_batch.events()[0])
        .expect("first publication");
    let error = ledger
        .record(&second_batch.events()[0])
        .expect_err("ledger capacity must fail closed");
    assert_eq!(error, PublicationError::LedgerCapacityExceeded { limit: 1 });
    assert_eq!(ledger.len(), 1);
}

#[test]
fn marker_encoding_covers_every_confirmation_class() {
    for class in [
        ConfirmationClass::ProvisionalSource,
        ConfirmationClass::CommittedPrimary,
        ConfirmationClass::CommittedIndependent,
        ConfirmationClass::ReconciledSnapshot,
        ConfirmationClass::Corrected,
        ConfirmationClass::Expired,
    ] {
        let block = classified_block(42, 7, class);
        let receipt = archive_receipt(&block);
        match class {
            ConfirmationClass::CommittedPrimary => {
                let batch = CommittedPublicationBatch::try_new(&block, &receipt)
                    .expect("committed primary still encodes");
                assert_eq!(
                    hex::encode(batch.block().publication_sha256()),
                    FROZEN_COMMITTED_PRIMARY_MARKER_SHA256,
                    "committed-primary marker bytes must stay frozen"
                );
            }
            ConfirmationClass::CommittedIndependent => {
                let batch = CommittedPublicationBatch::try_new(&block, &receipt)
                    .expect("committed independent still encodes");
                assert_eq!(
                    hex::encode(batch.block().publication_sha256()),
                    FROZEN_COMMITTED_INDEPENDENT_MARKER_SHA256,
                    "committed-independent marker bytes must stay frozen"
                );
            }
            ConfirmationClass::ProvisionalSource
            | ConfirmationClass::ReconciledSnapshot
            | ConfirmationClass::Corrected
            | ConfirmationClass::Expired => {
                let error = CommittedPublicationBatch::try_new(&block, &receipt)
                    .expect_err("non-committed lanes fail closed");
                assert_eq!(
                    error,
                    PublicationError::NotCommitted,
                    "{class:?} must not blur into the committed publication lane"
                );
                assert_eq!(
                    error.reason_code(),
                    "publication.not_committed",
                    "{class:?} must reuse the existing not-committed reason"
                );
            }
        }
    }
}
