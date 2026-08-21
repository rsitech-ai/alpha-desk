use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use canonical_events::BlockEnvelope;
use domain_types::{BlockHeight, ChainId, SourceId};
use hl_capture::bus::Subject;
use hl_capture::{
    CanonicalBlockCommitter, CommittedFact, CommittedNodePipeline, CommittedNodePipelineConfig,
    FindingStatus, InboundClass, LaneDecision, OfficialWsLimits, PlannerConfig, PlannerInput,
    ProcessIpBudget, ProvisionalWsLane, SubscriptionDemand, WsLaneObservation, open_plan_sessions,
    plan_subscriptions, snapshot_subject_for_family,
};
use hl_protocol::{
    ObservationClass, ReceiveTimestamps, SourceAdmission, SourceCursor, SourceObservation,
    SourceTrust,
};

#[derive(Debug, Default)]
struct RecordingCommitter {
    committed: Mutex<Vec<BlockEnvelope>>,
}

#[async_trait]
impl CanonicalBlockCommitter for RecordingCommitter {
    async fn commit(&self, block: &BlockEnvelope) -> Result<(), &'static str> {
        self.committed.lock().unwrap().push(block.clone());
        Ok(())
    }
}

fn pipeline_config() -> CommittedNodePipelineConfig {
    CommittedNodePipelineConfig::try_new(
        ChainId::new("mainnet").unwrap(),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        SourceAdmission::new(
            SourceTrust::LocallyVerifiedCommitted,
            ObservationClass::CommittedBlock,
        )
        .unwrap(),
        BlockHeight::new(992_814_678),
        32,
        32,
    )
    .unwrap()
}

fn node_observation(height: u64) -> SourceObservation {
    let payload = serde_json::to_vec(&serde_json::json!({
        "abci_block": {
            "time": "2026-07-28T12:00:00.000000000",
            "round": height,
            "parent_round": height - 1,
            "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487"
        },
        "signed_action_bundles": []
    }))
    .unwrap();
    SourceObservation::new(
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new("node-directory-epoch", height).unwrap(),
        ReceiveTimestamps::new(1_785_240_000_000_100, 100).unwrap(),
        "node-v1",
        Bytes::from(payload),
        Vec::new(),
        1024 * 1024,
    )
    .unwrap()
}

fn ws_obs(
    key: &str,
    family: &str,
    hash: u8,
    class: InboundClass,
    received_at_millis: u64,
) -> WsLaneObservation {
    WsLaneObservation::try_new(key, family, [hash; 32], class, received_at_millis).unwrap()
}

#[tokio::test]
async fn provisional_ws_never_advances_committed_watermark() {
    let committer = RecordingCommitter::default();
    let mut pipeline = CommittedNodePipeline::new(pipeline_config(), &committer);
    let mut lane = ProvisionalWsLane::official();

    assert!(!lane.advances_committed_watermark());
    assert_eq!(pipeline.committed_watermark(), None);

    let opened = lane.observe_ws(&ws_obs(
        "fill:0xaa:1",
        "userFills",
        7,
        InboundClass::IncrementalEvent,
        0,
    ));
    assert!(matches!(opened, LaneDecision::ProvisionalOpen { .. }));
    assert_eq!(pipeline.committed_watermark(), None);

    pipeline
        .process_spooled(&node_observation(992_814_678))
        .await
        .unwrap();
    let committed = pipeline.committed_watermark();
    assert_eq!(committed, Some(BlockHeight::new(992_814_678)));

    lane.observe_ws(&ws_obs(
        "fill:0xaa:2",
        "userFills",
        8,
        InboundClass::IncrementalEvent,
        1,
    ));
    assert_eq!(pipeline.committed_watermark(), committed);
    assert!(!lane.advances_committed_watermark());
}

#[test]
fn committed_counterpart_confirms_unmatched_provisional() {
    let mut lane = ProvisionalWsLane::official();
    lane.observe_ws(&ws_obs(
        "fill:0xaa:1",
        "userFills",
        7,
        InboundClass::IncrementalEvent,
        0,
    ));
    assert_eq!(lane.unmatched_count(), 1);

    let fact = CommittedFact::try_new("fill:0xaa:1", [7; 32]).unwrap();
    let committer = RecordingCommitter::default();
    let pipeline = CommittedNodePipeline::new(pipeline_config(), &committer);
    let decisions = pipeline.reconcile_provisional(&mut lane, &[fact], 10);

    assert_eq!(
        decisions,
        [LaneDecision::Confirmed {
            key: "fill:0xaa:1".to_owned()
        }]
    );
    assert_eq!(lane.unmatched_count(), 0);
}

#[test]
fn unmatched_provisional_expires() {
    let mut lane = ProvisionalWsLane::new(10);
    lane.observe_ws(&ws_obs(
        "fill:0xaa:1",
        "userFills",
        7,
        InboundClass::IncrementalEvent,
        0,
    ));
    assert!(lane.expire(9).is_empty());
    assert_eq!(lane.unmatched_count(), 1);
    assert_eq!(
        lane.expire(10),
        [LaneDecision::Expired {
            key: "fill:0xaa:1".to_owned()
        }]
    );
    assert_eq!(lane.unmatched_count(), 0);
}

#[test]
fn reconnect_snapshot_replaces_state_without_double_count() {
    let mut lane = ProvisionalWsLane::official();
    let first = lane.observe_ws(&ws_obs(
        "acct:0xaa",
        "clearinghouseState",
        3,
        InboundClass::SnapshotReplace,
        0,
    ));
    assert!(matches!(
        first,
        LaneDecision::SnapshotReplace {
            subject: Subject::SnapshotAccount,
            ..
        }
    ));
    assert_eq!(lane.snapshot_count(), 1);
    assert_eq!(lane.unmatched_count(), 0);

    let duplicate = lane.observe_ws(&ws_obs(
        "acct:0xaa",
        "clearinghouseState",
        3,
        InboundClass::SnapshotReplace,
        1,
    ));
    assert_eq!(
        duplicate,
        LaneDecision::DuplicateSnapshot {
            key: "acct:0xaa".to_owned()
        }
    );
    assert_eq!(lane.snapshot_count(), 1);

    let mut restarted = ProvisionalWsLane::official();
    let replaced = restarted.observe_ws(&ws_obs(
        "acct:0xaa",
        "clearinghouseState",
        3,
        InboundClass::SnapshotReplace,
        0,
    ));
    assert!(matches!(replaced, LaneDecision::SnapshotReplace { .. }));
    assert_eq!(restarted.snapshot_count(), 1);
    assert_eq!(restarted.unmatched_count(), 0);
}

#[test]
fn conflicting_committed_fact_wins_and_produces_finding() {
    let mut lane = ProvisionalWsLane::official();
    lane.observe_ws(&ws_obs(
        "fill:0xaa:1",
        "userFills",
        1,
        InboundClass::IncrementalEvent,
        0,
    ));
    let fact = CommittedFact::try_new("fill:0xaa:1", [2; 32]).unwrap();
    let decision = lane.observe_committed(&fact, 10);
    match decision {
        LaneDecision::Conflict { finding } => {
            assert_eq!(finding.expected_hash(), [1; 32]);
            assert_eq!(finding.observed_hash(), [2; 32]);
            assert_eq!(finding.status(), FindingStatus::Open);
            assert_eq!(finding.subject(), "hl.v1.snapshot.account");
            assert!(!finding.finding_id().is_empty());
        }
        other => panic!("expected conflict, got {other:?}"),
    }
    assert_eq!(lane.unmatched_count(), 0);
    assert_eq!(lane.findings().len(), 1);
}

#[test]
fn red_provisional_source_suppresses_provisional_only_features() {
    let mut lane = ProvisionalWsLane::official();
    lane.set_source_red(true);
    let decision = lane.observe_ws(&ws_obs(
        "fill:0xaa:1",
        "userFills",
        7,
        InboundClass::IncrementalEvent,
        0,
    ));
    assert_eq!(
        decision,
        LaneDecision::Suppressed {
            key: "fill:0xaa:1".to_owned()
        }
    );
    assert_eq!(lane.unmatched_count(), 0);
    assert!(!lane.provisional_features_admitted());
    assert!(lane.source_health().suppress_provisional_features());
    assert_eq!(lane.source_health().suppresses(), &["provisional"]);
    assert_eq!(lane.source_health().reason_code(), "capture_ws.source_red");
}

#[test]
fn existing_subject_strings_remain_stable() {
    let frozen = [
        (Subject::BlockCommitted, "hl.v1.block.committed"),
        (Subject::BlockProvisional, "hl.v1.block.provisional"),
        (Subject::EventFill, "hl.v1.event.fill"),
        (Subject::EventOrder, "hl.v1.event.order"),
        (Subject::EventLedger, "hl.v1.event.ledger"),
        (Subject::EventMarketMeta, "hl.v1.event.market_meta"),
        (Subject::EventOracle, "hl.v1.event.oracle"),
        (Subject::StateAccountDelta, "hl.v1.state.account_delta"),
        (Subject::StateBookDelta, "hl.v1.state.book_delta"),
        (Subject::FeatureWallet, "hl.v1.feature.wallet"),
        (Subject::FeatureEntity, "hl.v1.feature.entity"),
        (Subject::FeatureMarket, "hl.v1.feature.market"),
        (Subject::SignalCandidate, "hl.v1.signal.candidate"),
        (Subject::SignalLive, "hl.v1.signal.live"),
        (Subject::SignalResolved, "hl.v1.signal.resolved"),
        (Subject::HealthData, "hl.v1.health.data"),
        (Subject::HealthModel, "hl.v1.health.model"),
    ];
    for (subject, wire) in frozen {
        assert_eq!(subject.as_str(), wire);
    }
    assert_eq!(Subject::SnapshotAccount.as_str(), "hl.v1.snapshot.account");
    assert_eq!(Subject::SnapshotMarket.as_str(), "hl.v1.snapshot.market");
    assert_eq!(
        Subject::SnapshotEcosystem.as_str(),
        "hl.v1.snapshot.ecosystem"
    );
    assert_eq!(Subject::HealthSource.as_str(), "hl.v1.health.source");
    assert_eq!(Subject::ALL.len(), 21);
    assert_eq!(
        snapshot_subject_for_family("userFills"),
        Subject::SnapshotAccount
    );
    assert_eq!(
        snapshot_subject_for_family("trades"),
        Subject::SnapshotMarket
    );
    assert_eq!(
        snapshot_subject_for_family("allMids"),
        Subject::SnapshotEcosystem
    );
}

#[test]
fn one_process_budget_is_handed_to_every_spawned_session() {
    let plan = plan_subscriptions(
        PlannerConfig::official(),
        PlannerInput::new(vec![
            SubscriptionDemand::new("userFills")
                .with_user("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
            SubscriptionDemand::new("userFills")
                .with_user("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"),
        ]),
    );
    let tight = ProcessIpBudget::new(1, OfficialWsLimits::official().max_outgoing_per_minute());
    let error = open_plan_sessions(&plan, tight, 0).expect_err("shared cap");
    assert_eq!(error.reason_code(), "capture_ws.connect_rate");

    let shared = ProcessIpBudget::official();
    let sessions = open_plan_sessions(&plan, shared, 0).expect("official cap");
    assert!(sessions.len() > 1);
}
