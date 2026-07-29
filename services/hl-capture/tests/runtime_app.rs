use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use canonical_events::BlockEnvelope;
use domain_types::{BlockHeight, ChainId};
use hl_capture::bus::{CanonicalPublisher, PublicationAck, PublicationError, PublicationMessage};
use hl_capture::coordinator::{
    CaptureArchive, CaptureCoordinator, NoCoordinatorFaults, SystemAcknowledgementClock,
};
use hl_capture::progress::InMemoryProgressStore;
use hl_capture::{CaptureRuntime, CaptureRuntimeConfig, OwnedTask, StatusWriter, read_status};
use storage_ports::{ArchiveError, ArchiveReceipt, CaptureProgressStore};
use tempfile::tempdir;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

#[derive(Debug)]
struct EmptyArchive;

#[async_trait]
impl CaptureArchive for EmptyArchive {
    async fn append_block(&self, _block: &BlockEnvelope) -> Result<ArchiveReceipt, ArchiveError> {
        Err(ArchiveError::Io("test archive append is not expected"))
    }

    async fn load_block(
        &self,
        _chain_id: &ChainId,
        _block_height: BlockHeight,
    ) -> Result<BlockEnvelope, ArchiveError> {
        Err(ArchiveError::RangeUnavailable)
    }
}

#[derive(Debug)]
struct UnusedPublisher;

#[async_trait]
impl CanonicalPublisher for UnusedPublisher {
    async fn publish(
        &self,
        _message: &PublicationMessage,
    ) -> Result<PublicationAck, PublicationError> {
        Err(PublicationError::TransportPublish)
    }
}

#[tokio::test]
async fn runtime_becomes_ready_after_recovery_and_marks_shutdown_not_ready() {
    let directory = tempdir().expect("temporary runtime directory");
    let status_path = directory.path().join("capture-status.json");
    let progress = Arc::new(InMemoryProgressStore::new(32).expect("progress store"));
    let coordinator = Arc::new(CaptureCoordinator::new(
        Arc::new(EmptyArchive),
        Arc::clone(&progress) as Arc<dyn CaptureProgressStore>,
        Arc::new(UnusedPublisher),
        Arc::new(SystemAcknowledgementClock),
        Arc::new(NoCoordinatorFaults),
    ));
    let runtime = CaptureRuntime::new(
        CaptureRuntimeConfig::try_new(
            ChainId::new("mainnet").expect("chain"),
            BlockHeight::new(42),
            32,
            Duration::from_millis(10),
            Duration::from_secs(1),
            "build-runtime-test",
        )
        .expect("runtime config"),
        coordinator,
        progress as Arc<dyn CaptureProgressStore>,
        Arc::new(StatusWriter::new(status_path.clone()).expect("status writer")),
    );
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.child_token();
    let worker = OwnedTask::new("source", async move {
        worker_cancellation.cancelled().await;
        Ok(())
    });
    let run = tokio::spawn(runtime.run(cancellation.clone(), vec![worker]));

    timeout(Duration::from_secs(1), async {
        loop {
            if let Ok(status) = read_status(&status_path) {
                let encoded = serde_json::to_value(status).expect("status value");
                if encoded["ready"] == true {
                    assert_eq!(encoded["health"], "green");
                    assert_eq!(encoded["chain_id"], "mainnet");
                    assert_eq!(encoded["pending_blocks"], 0);
                    break;
                }
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("runtime became ready");

    cancellation.cancel();
    run.await
        .expect("runtime task joined")
        .expect("clean runtime shutdown");

    let stopped = serde_json::to_value(read_status(&status_path).expect("stopped status"))
        .expect("status value");
    assert_eq!(stopped["ready"], false);
    assert_eq!(stopped["health"], "yellow");
    assert!(stopped.get("last_error_reason").is_none());
}
