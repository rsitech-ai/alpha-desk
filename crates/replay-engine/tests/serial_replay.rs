use std::{cell::Cell, collections::BTreeMap};

use canonical_archive::{ArchiveConfig, LocalParquetArchive};
use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::{
    CanonicalLedger, CanonicalTradeReducerV1, LedgerLimits, TradeReconciliationRecordV1,
    WatermarkOnlyReducerV1,
};
use domain_types::{
    Address, BlockHeight, BlockRange, ChainId, KnownTime, ManifestId, MarketId, Price,
    ProtocolTime, Quantity, SourceId, TradeId, TransactionId,
};
use replay_engine::{
    ReplayCancellation, ReplayLimits, ReplayOutcome, ReplayRequest, SerialReplayEngine,
};
use storage_ports::CanonicalArchive;

#[test]
fn two_clean_replays_produce_byte_identical_receipts_and_final_state() {
    let fixture = ReplayFixture::empty(&[500, 501, 502]);
    let mut first_ledger = ledger(500);
    let mut second_ledger = ledger(500);
    let first_request = fixture.request(
        500,
        502,
        fixture.manifests.clone(),
        first_ledger.state_hash(),
    );
    let second_request = fixture.request(
        500,
        502,
        fixture.manifests.clone(),
        second_ledger.state_hash(),
    );

    let first = SerialReplayEngine::new(
        &fixture.archive,
        &mut first_ledger,
        ReplayLimits::production(),
    )
    .run(&first_request, &NeverCancel)
    .expect("first replay");
    let second = SerialReplayEngine::new(
        &fixture.archive,
        &mut second_ledger,
        ReplayLimits::production(),
    )
    .run(&second_request, &NeverCancel)
    .expect("second replay");

    let (ReplayOutcome::Completed(first), ReplayOutcome::Completed(second)) = (first, second)
    else {
        panic!("both replays must complete");
    };
    assert_eq!(first, second);
    assert_eq!(first.canonical_bytes(), second.canonical_bytes());
    assert_eq!(first.receipt_hash(), second.receipt_hash());
    assert_eq!(first_ledger.state_hash(), second_ledger.state_hash());
    assert_eq!(first.applied_block_count(), 3);
    assert_eq!(first.last_applied_height(), Some(BlockHeight::new(502)));
}

#[test]
fn completed_receipt_hash_matches_the_v1_golden_vector() {
    let fixture = ReplayFixture::empty(&[505]);
    let mut ledger = ledger(505);
    let request = fixture.request(505, 505, fixture.manifests.clone(), ledger.state_hash());

    let outcome =
        SerialReplayEngine::new(&fixture.archive, &mut ledger, ReplayLimits::production())
            .run(&request, &NeverCancel)
            .expect("replay");
    let ReplayOutcome::Completed(receipt) = outcome else {
        panic!("replay must complete");
    };

    assert_eq!(
        receipt.receipt_hash(),
        [
            0xfe, 0x45, 0xd3, 0x97, 0x17, 0x19, 0xfd, 0x3b, 0xb0, 0x9a, 0xa9, 0xeb, 0x15, 0x01,
            0xe6, 0xe2, 0xca, 0x85, 0xc6, 0xc0, 0xf7, 0x7e, 0x2b, 0x0b, 0x7e, 0x8f, 0x27, 0x20,
            0xdf, 0x9b, 0x25, 0x29,
        ]
    );
}

#[test]
fn checkpoint_resume_equals_uninterrupted_replay() {
    let fixture = ReplayFixture::empty(&[510, 511, 512]);
    let mut uninterrupted = ledger(510);
    let full_request = fixture.request(
        510,
        512,
        fixture.manifests.clone(),
        uninterrupted.state_hash(),
    );
    SerialReplayEngine::new(
        &fixture.archive,
        &mut uninterrupted,
        ReplayLimits::production(),
    )
    .run(&full_request, &NeverCancel)
    .expect("full replay");

    let mut partial = ledger(510);
    let partial_request = fixture.request(
        510,
        511,
        fixture.manifests[..2].to_vec(),
        partial.state_hash(),
    );
    SerialReplayEngine::new(&fixture.archive, &mut partial, ReplayLimits::production())
        .run(&partial_request, &NeverCancel)
        .expect("partial replay");
    let mut resumed = CanonicalLedger::try_from_state_image(
        partial.state_image().clone(),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("restored state");
    let resume_request = fixture.request(
        512,
        512,
        fixture.manifests[2..].to_vec(),
        resumed.state_hash(),
    );
    SerialReplayEngine::new(&fixture.archive, &mut resumed, ReplayLimits::production())
        .run(&resume_request, &NeverCancel)
        .expect("resume replay");

    assert_eq!(resumed.state_hash(), uninterrupted.state_hash());
    assert_eq!(
        resumed.state_image().canonical_bytes(),
        uninterrupted.state_image().canonical_bytes()
    );
}

#[test]
fn canonical_trade_state_and_reconciliation_replay_identically_from_checkpoint() {
    let fixture =
        ReplayFixture::with_blocks(vec![trade_block(513), trade_block(514), trade_block(515)]);
    let mut uninterrupted = trade_ledger(513);
    let full_request = fixture.request(
        513,
        515,
        fixture.manifests.clone(),
        uninterrupted.state_hash(),
    );
    SerialReplayEngine::new(
        &fixture.archive,
        &mut uninterrupted,
        ReplayLimits::production(),
    )
    .run(&full_request, &NeverCancel)
    .expect("full trade replay");

    let mut partial = trade_ledger(513);
    let partial_request = fixture.request(
        513,
        514,
        fixture.manifests[..2].to_vec(),
        partial.state_hash(),
    );
    SerialReplayEngine::new(&fixture.archive, &mut partial, ReplayLimits::production())
        .run(&partial_request, &NeverCancel)
        .expect("partial trade replay");
    let mut resumed = CanonicalLedger::try_from_state_image(
        partial.state_image().clone(),
        CanonicalTradeReducerV1,
        LedgerLimits::production(),
    )
    .expect("restored trade state");
    let resume_request = fixture.request(
        515,
        515,
        fixture.manifests[2..].to_vec(),
        resumed.state_hash(),
    );
    SerialReplayEngine::new(&fixture.archive, &mut resumed, ReplayLimits::production())
        .run(&resume_request, &NeverCancel)
        .expect("resumed trade replay");

    assert_eq!(resumed.state_hash(), uninterrupted.state_hash());
    assert_eq!(
        resumed.state_image().canonical_bytes(),
        uninterrupted.state_image().canonical_bytes()
    );
    assert_eq!(resumed.state_image().entries().len(), 12);
    let trade_id = TradeId::new("trd-515").unwrap();
    let key = TradeReconciliationRecordV1::state_key(&trade_id).unwrap();
    let assessment = TradeReconciliationRecordV1::decode_at(
        &key,
        resumed.state_image().entries().get(&key).unwrap(),
    )
    .unwrap();
    assert!(assessment.passed());
    assert_eq!(assessment.block_height(), BlockHeight::new(515));
}

#[test]
fn manifest_plan_is_preflighted_before_any_state_mutation() {
    let fixture = ReplayFixture::empty(&[520, 521]);
    let mut ledger = ledger(520);
    let before = ledger.state_hash();
    let request = fixture.request(
        520,
        521,
        fixture.manifests.iter().rev().cloned().collect(),
        before,
    );

    let error = SerialReplayEngine::new(&fixture.archive, &mut ledger, ReplayLimits::production())
        .run(&request, &NeverCancel)
        .expect_err("reversed manifest plan");

    assert_eq!(error.reason_code(), "replay.manifest_plan");
    assert_eq!(error.progress().applied_block_count(), 0);
    assert_eq!(ledger.state_hash(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn start_state_mismatch_is_rejected_before_any_state_mutation() {
    let fixture = ReplayFixture::empty(&[525]);
    let mut ledger = ledger(525);
    let before = ledger.state_hash();
    let mut wrong_start = before;
    wrong_start[0] ^= 0xff;
    let request = fixture.request(525, 525, fixture.manifests.clone(), wrong_start);

    let error = SerialReplayEngine::new(&fixture.archive, &mut ledger, ReplayLimits::production())
        .run(&request, &NeverCancel)
        .expect_err("wrong starting state");

    assert_eq!(error.reason_code(), "replay.start_state_mismatch");
    assert_eq!(error.progress().applied_block_count(), 0);
    assert_eq!(ledger.state_hash(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn wrong_chain_manifest_is_rejected_during_preflight() {
    let fixture = ReplayFixture::with_blocks(vec![empty_block_on(
        "testnet",
        526,
        ConfirmationClass::CommittedPrimary,
    )]);
    let mut ledger = ledger(526);
    let before = ledger.state_hash();
    let request = fixture.request(526, 526, fixture.manifests.clone(), before);

    let error = SerialReplayEngine::new(&fixture.archive, &mut ledger, ReplayLimits::production())
        .run(&request, &NeverCancel)
        .expect_err("wrong-chain manifest");

    assert_eq!(error.reason_code(), "replay.manifest_plan");
    assert_eq!(
        error.source_reason_code(),
        Some("replay.manifest_chain_mismatch")
    );
    assert_eq!(error.progress().applied_block_count(), 0);
    assert_eq!(ledger.state_hash(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn unsupported_block_is_quarantined_after_only_prior_blocks_commit() {
    let fixture =
        ReplayFixture::with_blocks(vec![empty_block(530), trade_block(531), empty_block(532)]);
    let mut ledger = ledger(530);
    let request = fixture.request(530, 532, fixture.manifests.clone(), ledger.state_hash());

    let error = SerialReplayEngine::new(&fixture.archive, &mut ledger, ReplayLimits::production())
        .run(&request, &NeverCancel)
        .expect_err("unsupported action-bearing block");

    assert_eq!(error.reason_code(), "replay.block_quarantined");
    assert_eq!(error.source_reason_code(), Some("ledger.unsupported_event"));
    assert_eq!(error.quarantine_height(), Some(BlockHeight::new(531)));
    assert_eq!(error.progress().applied_block_count(), 1);
    assert_eq!(
        ledger.checkpoint().expect("height 530").block_height(),
        BlockHeight::new(530)
    );
}

#[test]
fn corrected_archive_block_is_quarantined_without_mutating_state() {
    let fixture = ReplayFixture::with_blocks(vec![corrected_block(550)]);
    let mut ledger = ledger(550);
    let before = ledger.state_image().canonical_bytes();
    let request = fixture.request(550, 550, fixture.manifests.clone(), ledger.state_hash());

    let error = SerialReplayEngine::new(&fixture.archive, &mut ledger, ReplayLimits::production())
        .run(&request, &NeverCancel)
        .expect_err("correction quarantined");

    assert_eq!(error.reason_code(), "replay.block_quarantined");
    assert_eq!(
        error.source_reason_code(),
        Some("ledger.correction_unimplemented")
    );
    assert_eq!(error.quarantine_height(), Some(BlockHeight::new(550)));
    assert_eq!(error.progress().applied_block_count(), 0);
    assert_eq!(ledger.state_image().canonical_bytes(), before);
    assert!(ledger.checkpoint().is_none());
}

#[test]
fn cancellation_is_observed_only_at_a_block_boundary() {
    let fixture = ReplayFixture::empty(&[540, 541, 542]);
    let mut ledger = ledger(540);
    let request = fixture.request(540, 542, fixture.manifests.clone(), ledger.state_hash());
    let cancellation = CancelAfterChecks {
        checks: Cell::new(0),
        cancel_at: 1,
    };

    let outcome =
        SerialReplayEngine::new(&fixture.archive, &mut ledger, ReplayLimits::production())
            .run(&request, &cancellation)
            .expect("cancelled replay");

    let ReplayOutcome::Cancelled(receipt) = outcome else {
        panic!("expected cancellation");
    };
    assert_eq!(receipt.applied_block_count(), 1);
    assert_eq!(receipt.last_applied_height(), Some(BlockHeight::new(540)));
    assert_eq!(
        ledger.checkpoint().expect("checkpoint").block_height(),
        BlockHeight::new(540)
    );
}

#[derive(Debug)]
struct ReplayFixture {
    _temporary: tempfile::TempDir,
    archive: LocalParquetArchive,
    manifests: Vec<ManifestId>,
    schema_fingerprint: [u8; 32],
}

impl ReplayFixture {
    fn empty(heights: &[u64]) -> Self {
        Self::with_blocks(heights.iter().copied().map(empty_block).collect())
    }

    fn with_blocks(blocks: Vec<BlockEnvelope>) -> Self {
        let temporary = tempfile::tempdir().expect("archive root");
        let archive = LocalParquetArchive::open(
            temporary.path(),
            ArchiveConfig::deterministic_fixture(
                "replay-test",
                KnownTime::from_unix_micros(10_000).expect("time"),
            )
            .expect("config"),
        )
        .expect("archive");
        let manifests: Vec<_> = blocks
            .iter()
            .map(|block| {
                archive
                    .append_block(block)
                    .expect("append")
                    .manifest_id()
                    .clone()
            })
            .collect();
        let verified = archive
            .verify_manifest(manifests.first().expect("manifest"))
            .expect("verified manifest");
        let schema_fingerprint = *verified
            .schema_fingerprints()
            .get("canonical_events")
            .expect("canonical schema");
        Self {
            _temporary: temporary,
            archive,
            manifests,
            schema_fingerprint,
        }
    }

    fn request(
        &self,
        start: u64,
        end: u64,
        manifests: Vec<ManifestId>,
        expected_start_state_hash: [u8; 32],
    ) -> ReplayRequest {
        ReplayRequest::try_new(
            ChainId::new("mainnet").expect("chain"),
            BlockRange::new(BlockHeight::new(start), BlockHeight::new(end)).expect("range"),
            manifests,
            expected_start_state_hash,
            "canonical_events",
            self.schema_fingerprint,
        )
        .expect("request")
    }
}

#[derive(Debug, Clone, Copy)]
struct NeverCancel;

impl ReplayCancellation for NeverCancel {
    fn is_cancelled(&self) -> bool {
        false
    }
}

#[derive(Debug)]
struct CancelAfterChecks {
    checks: Cell<usize>,
    cancel_at: usize,
}

impl ReplayCancellation for CancelAfterChecks {
    fn is_cancelled(&self) -> bool {
        let current = self.checks.get();
        self.checks.set(current + 1);
        current >= self.cancel_at
    }
}

fn ledger(first_height: u64) -> CanonicalLedger<WatermarkOnlyReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(first_height),
        WatermarkOnlyReducerV1,
        LedgerLimits::production(),
    )
    .expect("ledger")
}

fn trade_ledger(first_height: u64) -> CanonicalLedger<CanonicalTradeReducerV1> {
    CanonicalLedger::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(first_height),
        CanonicalTradeReducerV1,
        LedgerLimits::production(),
    )
    .expect("trade ledger")
}

fn empty_block(height: u64) -> BlockEnvelope {
    empty_block_on("mainnet", height, ConfirmationClass::CommittedPrimary)
}

fn corrected_block(height: u64) -> BlockEnvelope {
    empty_block_on("mainnet", height, ConfirmationClass::Corrected)
}

fn empty_block_on(chain: &str, height: u64, confirmation: ConfirmationClass) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new(chain).expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        confirmation,
        Vec::new(),
        source_hashes(height),
    )
    .expect("empty block")
}

fn trade_block(height: u64) -> BlockEnvelope {
    let time = ProtocolTime::from_unix_micros(height as i64).expect("time");
    let market_id = MarketId::new("perp:BTC").expect("market");
    let payload = EventPayload::TradeMatched(TradeMatched {
        trade_id: Some(TradeId::new(format!("trd-{height}")).expect("trade")),
        market_id: Some(market_id.clone()),
        maker_order_id: None,
        taker_order_id: None,
        price: Price::parse_at_scale("65000", 6).expect("price"),
        quantity: Quantity::parse_at_scale("0.01", 8).expect("quantity"),
        deterministic_seed: 1,
        participants: None,
    });
    let payload_hash = *blake3::hash(&payload.encode_to_vec().expect("payload")).as_bytes();
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(height),
        block_time: time,
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![market_id],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![
            SourceEvidence::try_new(
                SourceId::new("test-primary").expect("source"),
                "v1",
                height.to_string(),
                payload_hash,
            )
            .expect("evidence"),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).expect("known"),
        ingested_at: KnownTime::from_unix_micros(height as i64).expect("known"),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).expect("known"),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .expect("event");
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        time,
        ConfirmationClass::CommittedPrimary,
        vec![event],
        source_hashes(height),
    )
    .expect("trade block")
}

fn source_hashes(height: u64) -> BTreeMap<SourceId, [u8; 32]> {
    BTreeMap::from([(
        SourceId::new("test-primary").expect("source"),
        *blake3::hash(&height.to_be_bytes()).as_bytes(),
    )])
}
