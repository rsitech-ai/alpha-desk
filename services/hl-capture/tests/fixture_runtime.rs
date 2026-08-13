use canonical_events::{ConfirmationClass, EventKind};
use domain_types::{BlockHeight, ChainId, KnownTime, SourceId};
use hl_capture::{
    BlockCandidate, CanonicalSequencer, CaptureHealth, CaptureSourceHealth, CaptureStatus,
    CommittedSourceClass, FailoverDecision, FailoverError, FailoverReason,
    FailoverRecordDisposition, FailoverStore, SequencerConfig, SequencerDecision, SequencerHealth,
    StatusError, StatusWriter, synthetic_fixture_block, synthetic_independent_fixture_block,
};
use hl_protocol::{ObservationClass, SourceAdmission, SourceTrust};
use tempfile::tempdir;

#[test]
fn synthetic_fixture_blocks_are_deterministic_contiguous_and_explicitly_committed() {
    let chain = ChainId::new("fixture-mainnet").expect("chain");
    let first = synthetic_fixture_block(&chain, BlockHeight::new(10)).expect("first fixture block");
    let repeated =
        synthetic_fixture_block(&chain, BlockHeight::new(10)).expect("repeated fixture block");
    let next = synthetic_fixture_block(&chain, BlockHeight::new(11)).expect("next fixture block");

    assert_eq!(first, repeated);
    assert_ne!(first.canonical_block_hash(), next.canonical_block_hash());
    assert_eq!(first.chain_id(), &chain);
    assert_eq!(first.block_height(), BlockHeight::new(10));
    assert_eq!(
        first.confirmation_class(),
        ConfirmationClass::CommittedPrimary
    );
    assert_eq!(first.events().len(), 1);
    assert_eq!(first.events()[0].payload().kind(), EventKind::TradeMatched);
    assert_eq!(
        first.events()[0].source_evidence()[0].source_id().as_str(),
        "synthetic-fixture"
    );
}

#[test]
fn synthetic_independent_fixture_matches_primary_content_without_reconciliation_class() {
    let chain = ChainId::new("fixture-mainnet").expect("chain");
    let primary =
        synthetic_fixture_block(&chain, BlockHeight::new(11)).expect("primary fixture block");
    let independent = synthetic_independent_fixture_block(&chain, BlockHeight::new(11))
        .expect("independent fixture block");

    assert_eq!(
        independent.confirmation_class(),
        ConfirmationClass::CommittedIndependent
    );
    assert_ne!(
        independent.confirmation_class(),
        ConfirmationClass::ReconciledSnapshot
    );
    assert_eq!(
        primary.canonical_block_hash(),
        independent.canonical_block_hash()
    );
    assert_eq!(
        independent.events()[0].source_evidence()[0]
            .source_id()
            .as_str(),
        "synthetic-independent-fixture"
    );
}

#[test]
fn independent_failover_status_is_yellow_ready_at_most_and_never_green() {
    let directory = tempdir().expect("temporary status directory");
    let writer =
        StatusWriter::new(directory.path().join("capture-status.json")).expect("status writer");
    let yellow_ready = independent_failover_status(CaptureHealth::Yellow, true);

    writer
        .write(&yellow_ready)
        .expect("yellow-ready independent status is publishable");
    let encoded = serde_json::to_value(&yellow_ready).expect("status JSON");
    assert_independent_path_never_green(&encoded);
    assert_eq!(encoded["ready"], true);
    assert_eq!(encoded["failover_reason"], "primary-range-unavailable");
    assert!(encoded.get("qualification").is_none());
    assert!(encoded.get("stage_1_qualified").is_none());
    assert!(encoded.get("stage_2_qualified").is_none());
    assert!(encoded.get("live_source_qualified").is_none());
    assert!(encoded.get("independent_source_qualified").is_none());
    assert!(encoded.get("deployed_source_qualified").is_none());

    let green_independent = independent_failover_status(CaptureHealth::Green, true);
    assert!(matches!(
        writer.write(&green_independent),
        Err(StatusError::InvalidField)
    ));
}

#[test]
fn overlapping_independent_height_is_duplicate_not_reconciliation_truth() {
    let mut sequencer = fixture_sequencer();
    let primary_ten = fixture_candidate(10, false);
    let primary_twelve = fixture_candidate(12, false);
    let independent_eleven = fixture_candidate(11, true);
    let independent_ten = fixture_candidate(10, true);
    let independent_twelve = fixture_candidate(12, true);

    assert_eq!(
        independent_eleven.block().confirmation_class(),
        ConfirmationClass::CommittedIndependent
    );
    assert_ne!(
        independent_eleven.block().confirmation_class(),
        ConfirmationClass::ReconciledSnapshot
    );

    assert_eq!(
        committed_heights(&sequencer.observe(primary_ten).expect("primary height 10")),
        vec![10]
    );
    let gap = sequencer
        .observe(primary_twelve)
        .expect("primary gap at height 11");
    assert!(matches!(
        gap.as_slice(),
        [SequencerDecision::RequestGap {
            start,
            end_inclusive,
            ..
        }] if *start == BlockHeight::new(11) && *end_inclusive == BlockHeight::new(11)
    ));

    let recovered = sequencer
        .observe(independent_eleven)
        .expect("independent gap fill");
    assert_eq!(committed_heights(&recovered), vec![11, 12]);
    assert_eq!(sequencer.committed_watermark(), Some(BlockHeight::new(12)));
    assert_eq!(sequencer.outstanding_gap(), None);

    let overlap = sequencer
        .observe(independent_ten)
        .expect("independent overlap at committed height");
    assert!(matches!(
        overlap.as_slice(),
        [SequencerDecision::RecordDuplicate {
            block_height,
            source_id
        }] if *block_height == BlockHeight::new(10)
            && source_id.as_str() == "synthetic-independent-fixture"
    ));
    let later_overlap = sequencer
        .observe(independent_twelve)
        .expect("independent overlap after recovery");
    assert!(matches!(
        later_overlap.as_slice(),
        [SequencerDecision::RecordDuplicate {
            block_height,
            source_id
        }] if *block_height == BlockHeight::new(12)
            && source_id.as_str() == "synthetic-independent-fixture"
    ));
    assert!(
        sequencer.quarantines().is_empty(),
        "matching overlap must not invent a reconciliation incident"
    );
    assert_eq!(sequencer.health(), SequencerHealth::Green);
    assert_eq!(sequencer.committed_watermark(), Some(BlockHeight::new(12)));
}

#[test]
fn pending_independent_overlap_keeps_primary_confirmation_not_reconciled_snapshot() {
    let mut sequencer = fixture_sequencer();
    sequencer
        .observe(fixture_candidate(11, false))
        .expect("future primary block");
    let overlap = sequencer
        .observe(fixture_candidate(11, true))
        .expect("matching independent pending block");
    assert!(matches!(
        overlap.as_slice(),
        [SequencerDecision::RecordDuplicate {
            block_height,
            source_id
        }] if *block_height == BlockHeight::new(11)
            && source_id.as_str() == "synthetic-independent-fixture"
    ));

    let recovered = sequencer
        .observe(fixture_candidate(10, false))
        .expect("gap fill from primary");
    let committed_eleven = recovered
        .iter()
        .find_map(|decision| match decision {
            SequencerDecision::Commit(block) if block.block_height() == BlockHeight::new(11) => {
                Some(block)
            }
            _ => None,
        })
        .expect("committed height 11");
    let source_ids = committed_eleven.events()[0]
        .source_evidence()
        .iter()
        .map(|evidence| evidence.source_id().as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        committed_eleven.confirmation_class(),
        ConfirmationClass::CommittedPrimary
    );
    assert_ne!(
        committed_eleven.confirmation_class(),
        ConfirmationClass::ReconciledSnapshot
    );
    assert_eq!(
        source_ids,
        vec!["synthetic-fixture", "synthetic-independent-fixture"]
    );
}

#[test]
fn overlapping_primary_repair_cannot_rewrite_failover_decision_as_truth() {
    let root = tempdir().expect("temporary failover directory");
    let path = root
        .path()
        .canonicalize()
        .expect("canonical temp root")
        .join("committed-source-failover.json");
    let store = FailoverStore::new(path).expect("failover store");
    let recorded = failover_decision(11);
    assert_eq!(
        store.record(&recorded).expect("create-once decision"),
        FailoverRecordDisposition::Recorded
    );
    assert_eq!(
        store
            .record(&failover_decision(12))
            .expect_err("overlap must not rewrite failover truth"),
        FailoverError::ConflictingDecision
    );
    assert_eq!(store.load().expect("durable decision"), Some(recorded));
}

fn independent_failover_status(health: CaptureHealth, ready: bool) -> CaptureStatus {
    CaptureStatus::new(
        KnownTime::from_unix_micros(1_700_000_000_000_000).expect("time"),
        "synthetic-fixture-build",
        ChainId::new("fixture-mainnet").expect("chain"),
        health,
    )
    .with_readiness(ready)
    .with_source_state(
        CommittedSourceClass::IndependentCommitted,
        CaptureSourceHealth::Healthy,
        Some(CaptureSourceHealth::Healthy),
        Some(BlockHeight::new(11)),
        Some(FailoverReason::PrimaryRangeUnavailable),
    )
}

fn assert_independent_path_never_green(value: &serde_json::Value) {
    assert_ne!(value["health"], "green");
    assert_eq!(value["health"], "yellow");
    assert_eq!(value["active_committed_source"], "independent-committed");
    assert_eq!(value["independent_source_health"], "healthy");
    assert_eq!(value["primary_source_health"], "healthy");
}

fn fixture_sequencer() -> CanonicalSequencer {
    CanonicalSequencer::new(
        SequencerConfig::try_new(
            ChainId::new("fixture-mainnet").expect("chain"),
            BlockHeight::new(10),
            8,
            8,
        )
        .expect("sequencer config"),
    )
}

fn fixture_candidate(height: u64, independent: bool) -> BlockCandidate {
    let chain = ChainId::new("fixture-mainnet").expect("chain");
    let block = if independent {
        synthetic_independent_fixture_block(&chain, BlockHeight::new(height))
            .expect("independent fixture")
    } else {
        synthetic_fixture_block(&chain, BlockHeight::new(height)).expect("primary fixture")
    };
    let source_id = block
        .source_block_hashes()
        .keys()
        .next()
        .cloned()
        .expect("fixture source hash");
    let trust = if independent {
        SourceTrust::IndependentCommitted
    } else {
        SourceTrust::LocallyVerifiedCommitted
    };
    BlockCandidate::try_new(
        source_id,
        SourceAdmission::new(trust, ObservationClass::CommittedBlock).expect("admission"),
        block,
    )
    .expect("fixture candidate")
}

fn committed_heights(decisions: &[SequencerDecision]) -> Vec<u64> {
    decisions
        .iter()
        .filter_map(|decision| match decision {
            SequencerDecision::Commit(block) => Some(block.block_height().get()),
            _ => None,
        })
        .collect()
}

fn failover_decision(height: u64) -> FailoverDecision {
    FailoverDecision::try_new(
        ChainId::new("fixture-mainnet").expect("chain"),
        SourceId::new("synthetic-fixture").expect("primary"),
        SourceId::new("synthetic-independent-fixture").expect("independent"),
        BlockHeight::new(height),
        FailoverReason::PrimaryRangeUnavailable,
    )
    .expect("failover decision")
}
