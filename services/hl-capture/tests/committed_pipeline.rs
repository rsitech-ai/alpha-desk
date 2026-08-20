use std::sync::Mutex;

use async_trait::async_trait;
use bytes::Bytes;
use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, SourceId};
use hl_capture::{
    CanonicalBlockCommitter, CommittedNodePipeline, CommittedNodePipelineConfig, PipelineError,
    PipelineOutcome,
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

fn pipeline_config(
    admission: SourceAdmission,
) -> Result<CommittedNodePipelineConfig, PipelineError> {
    CommittedNodePipelineConfig::try_new(
        ChainId::new("mainnet").unwrap(),
        SourceId::new("primary-node").unwrap(),
        "hyperliquid-node-v1",
        admission,
        BlockHeight::new(992_814_678),
        32,
        32,
    )
}

fn config_for(trust: SourceTrust) -> CommittedNodePipelineConfig {
    pipeline_config(SourceAdmission::new(trust, ObservationClass::CommittedBlock).unwrap()).unwrap()
}

fn config() -> CommittedNodePipelineConfig {
    config_for(SourceTrust::LocallyVerifiedCommitted)
}

fn observation(height: u64, bundles: serde_json::Value) -> SourceObservation {
    let payload = serde_json::to_vec(&serde_json::json!({
        "abci_block": {
            "time": "2026-07-28T12:00:00.000000000",
            "round": height,
            "parent_round": height - 1,
            "proposer": "0x5ac99df645f3414876c816caa18b2d234024b487"
        },
        "signed_action_bundles": bundles
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

#[tokio::test]
async fn qualified_empty_committed_observation_reaches_the_committer_once() {
    let committer = RecordingCommitter::default();
    let mut pipeline = CommittedNodePipeline::new(config(), &committer);

    let outcome = pipeline
        .process_spooled(&observation(992_814_678, serde_json::json!([])))
        .await
        .unwrap();

    assert_eq!(
        outcome,
        PipelineOutcome::Committed {
            block_height: BlockHeight::new(992_814_678)
        }
    );
    let committed = committer.committed.lock().unwrap();
    assert_eq!(committed.len(), 1);
    assert_eq!(committed[0].block_height(), BlockHeight::new(992_814_678));
    assert_eq!(
        committed[0].confirmation_class(),
        ConfirmationClass::CommittedPrimary
    );
    assert!(committed[0].events().is_empty());
}

#[tokio::test]
async fn independent_committed_observation_still_reaches_the_committer_once() {
    let committer = RecordingCommitter::default();
    let mut pipeline =
        CommittedNodePipeline::new(config_for(SourceTrust::IndependentCommitted), &committer);

    let outcome = pipeline
        .process_spooled(&observation(992_814_678, serde_json::json!([])))
        .await
        .unwrap();

    assert_eq!(
        outcome,
        PipelineOutcome::Committed {
            block_height: BlockHeight::new(992_814_678)
        }
    );
    let committed = committer.committed.lock().unwrap();
    assert_eq!(committed.len(), 1);
    assert_eq!(
        committed[0].confirmation_class(),
        ConfirmationClass::CommittedIndependent
    );
}

#[tokio::test]
async fn action_bearing_observation_fails_closed_before_commit() {
    let committer = RecordingCommitter::default();
    let mut pipeline = CommittedNodePipeline::new(config(), &committer);

    let error = pipeline
        .process_spooled(&observation(
            992_814_678,
            serde_json::json!([["0xbundle", {"signed_actions": []}]]),
        ))
        .await
        .unwrap_err();

    assert!(matches!(error, PipelineError::Mapping(_)));
    assert_eq!(
        error.reason_code(),
        "canonical_mapping.unsupported_committed_actions"
    );
    assert!(committer.committed.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_gap_is_reported_without_publishing_the_later_block() {
    let committer = RecordingCommitter::default();
    let mut pipeline = CommittedNodePipeline::new(config(), &committer);
    pipeline
        .process_spooled(&observation(992_814_678, serde_json::json!([])))
        .await
        .unwrap();

    let outcome = pipeline
        .process_spooled(&observation(992_814_680, serde_json::json!([])))
        .await
        .unwrap();

    assert!(matches!(
        outcome,
        PipelineOutcome::Gap {
            start,
            end_inclusive,
            ..
        } if start == BlockHeight::new(992_814_679)
            && end_inclusive == BlockHeight::new(992_814_679)
    ));
    assert_eq!(committer.committed.lock().unwrap().len(), 1);
}

#[test]
fn committed_pipeline_fail_closes_every_constructible_non_admitted_trust() {
    for trust in SourceTrust::ALL {
        for class in ObservationClass::ALL {
            let Ok(admission) = SourceAdmission::new(trust, class) else {
                continue;
            };
            let result = pipeline_config(admission);
            match trust {
                SourceTrust::LocallyVerifiedCommitted | SourceTrust::IndependentCommitted => {
                    if class == ObservationClass::CommittedBlock {
                        result.expect("admitted committed trusts still construct");
                    } else {
                        assert_invalid_config(result, trust, class);
                    }
                }
                SourceTrust::ReconciledSnapshot
                | SourceTrust::RecoveryOnly
                | SourceTrust::ThirdPartyProvisional
                | SourceTrust::MempoolProvisional => {
                    assert_invalid_config(result, trust, class);
                }
            }
        }
    }
}

fn assert_invalid_config(
    result: Result<CommittedNodePipelineConfig, PipelineError>,
    trust: SourceTrust,
    class: ObservationClass,
) {
    let error = result.expect_err("non-admitted constructible pairs fail closed");
    assert!(
        matches!(error, PipelineError::InvalidConfig),
        "{trust:?}/{class:?} must reuse InvalidConfig"
    );
    assert_eq!(
        error.reason_code(),
        "capture_pipeline.invalid_config",
        "{trust:?}/{class:?} must reuse the existing invalid-config reason"
    );
}
