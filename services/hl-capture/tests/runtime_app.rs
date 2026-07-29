use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use canonical_events::BlockEnvelope;
use domain_types::{BlockHeight, ChainId, KnownTime};
use hl_capture::bus::{CanonicalPublisher, PublicationAck, PublicationError, PublicationMessage};
use hl_capture::coordinator::{
    CaptureArchive, CaptureCoordinator, NoCoordinatorFaults, SystemAcknowledgementClock,
};
use hl_capture::progress::InMemoryProgressStore;
use hl_capture::{
    CaptureHealth, CaptureRuntime, CaptureRuntimeConfig, CaptureSourceHealth, CaptureStatus,
    CommittedSourceClass, OwnedTask, StatusWriter, read_status,
};
use storage_ports::{
    ArchiveError, ArchiveReceipt, ArchivedBlockPlan, CaptureCursor, CaptureProgressStore,
    ProgressError, ProgressRecordDisposition, PublicationAcknowledgement,
};
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

#[derive(Debug)]
struct InitiallyUnavailableProgress {
    available: AtomicBool,
    inner: InMemoryProgressStore,
}

impl InitiallyUnavailableProgress {
    fn new() -> Self {
        Self {
            available: AtomicBool::new(false),
            inner: InMemoryProgressStore::new(32).expect("progress store"),
        }
    }

    fn make_available(&self) {
        self.available.store(true, Ordering::Release);
    }

    fn require_available(&self) -> Result<(), ProgressError> {
        if self.available.load(Ordering::Acquire) {
            Ok(())
        } else {
            Err(ProgressError::Storage("test dependency unavailable"))
        }
    }
}

#[async_trait]
impl CaptureProgressStore for InitiallyUnavailableProgress {
    async fn initialize_chain(
        &self,
        chain_id: &ChainId,
        first_block_height: BlockHeight,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        self.require_available()?;
        self.inner
            .initialize_chain(chain_id, first_block_height)
            .await
    }

    async fn record_archived(
        &self,
        plan: &ArchivedBlockPlan,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        self.require_available()?;
        self.inner.record_archived(plan).await
    }

    async fn record_acknowledgement(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
        acknowledgement: &PublicationAcknowledgement,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        self.require_available()?;
        self.inner
            .record_acknowledgement(chain_id, block_height, acknowledgement)
            .await
    }

    async fn advance_cursor(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<CaptureCursor, ProgressError> {
        self.require_available()?;
        self.inner.advance_cursor(chain_id, block_height).await
    }

    async fn load_cursor(
        &self,
        chain_id: &ChainId,
    ) -> Result<Option<CaptureCursor>, ProgressError> {
        self.require_available()?;
        self.inner.load_cursor(chain_id).await
    }

    async fn next_expected_height(&self, chain_id: &ChainId) -> Result<BlockHeight, ProgressError> {
        self.require_available()?;
        self.inner.next_expected_height(chain_id).await
    }

    async fn load_archived_block(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Option<ArchivedBlockPlan>, ProgressError> {
        self.require_available()?;
        self.inner.load_archived_block(chain_id, block_height).await
    }

    async fn load_acknowledgements(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Vec<PublicationAcknowledgement>, ProgressError> {
        self.require_available()?;
        self.inner
            .load_acknowledgements(chain_id, block_height)
            .await
    }

    async fn pending_blocks(
        &self,
        chain_id: &ChainId,
        limit: usize,
    ) -> Result<Vec<ArchivedBlockPlan>, ProgressError> {
        self.require_available()?;
        self.inner.pending_blocks(chain_id, limit).await
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

#[tokio::test]
async fn unavailable_progress_does_not_prevent_owned_work_and_recovers_readiness() {
    let directory = tempdir().expect("temporary runtime directory");
    let status_path = directory.path().join("capture-status.json");
    StatusWriter::new(status_path.clone())
        .expect("status writer")
        .write(
            &CaptureStatus::new(
                KnownTime::from_unix_micros(1).expect("known time"),
                "stale-build",
                ChainId::new("stale-chain").expect("chain"),
                CaptureHealth::Green,
            )
            .with_source_state(
                CommittedSourceClass::LocallyVerifiedCommitted,
                CaptureSourceHealth::Healthy,
                None,
                None,
                None,
            ),
        )
        .expect("stale status fixture");
    let progress = Arc::new(InitiallyUnavailableProgress::new());
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
            "build-runtime-outage-test",
        )
        .expect("runtime config"),
        coordinator,
        Arc::clone(&progress) as Arc<dyn CaptureProgressStore>,
        Arc::new(StatusWriter::new(status_path.clone()).expect("status writer")),
    );
    let cancellation = CancellationToken::new();
    let worker_cancellation = cancellation.child_token();
    let worker_started = Arc::new(AtomicBool::new(false));
    let task_started = Arc::clone(&worker_started);
    let worker = OwnedTask::new("source", async move {
        task_started.store(true, Ordering::Release);
        worker_cancellation.cancelled().await;
        Ok(())
    });
    let run = tokio::spawn(runtime.run(cancellation.clone(), vec![worker]));

    timeout(Duration::from_secs(1), async {
        loop {
            if worker_started.load(Ordering::Acquire)
                && let Ok(status) = read_status(&status_path)
            {
                let encoded = serde_json::to_value(status).expect("status value");
                if encoded["ready"] == false {
                    assert_ne!(encoded["health"], "green");
                    assert_eq!(encoded["chain_id"], "mainnet");
                    assert_eq!(encoded["build_id"], "build-runtime-outage-test");
                    break;
                }
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("owned work started while progress was unavailable");

    progress.make_available();
    timeout(Duration::from_secs(2), async {
        loop {
            if let Ok(status) = read_status(&status_path) {
                let encoded = serde_json::to_value(status).expect("status value");
                if encoded["ready"] == true {
                    assert_eq!(encoded["health"], "green");
                    break;
                }
            }
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("runtime recovered readiness");

    cancellation.cancel();
    run.await
        .expect("runtime task joined")
        .expect("clean runtime shutdown");
}
