use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use domain_types::{BlockHeight, ChainId, KnownTime};
use hl_capture::{
    AppError, CaptureHealth, CaptureSourceHealth, CaptureStatus, CommittedSourceClass,
    FailoverReason, OwnedTask, StatusError, StatusWriter, run_owned_tasks,
};
use tempfile::tempdir;
use tokio::time::{sleep, timeout};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn cancellation_joins_every_owned_task_before_returning() {
    let cancellation = CancellationToken::new();
    let active = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let mut tasks = Vec::new();

    for name in ["source", "status", "metrics"] {
        let task_cancellation = cancellation.child_token();
        let task_active = Arc::clone(&active);
        let task_completed = Arc::clone(&completed);
        tasks.push(OwnedTask::new(name, async move {
            task_active.fetch_add(1, Ordering::SeqCst);
            task_cancellation.cancelled().await;
            sleep(Duration::from_millis(5)).await;
            task_completed.fetch_add(1, Ordering::SeqCst);
            task_active.fetch_sub(1, Ordering::SeqCst);
            Ok(())
        }));
    }

    let supervisor = tokio::spawn(run_owned_tasks(
        cancellation.clone(),
        Duration::from_secs(1),
        tasks,
    ));
    timeout(Duration::from_secs(1), async {
        while active.load(Ordering::SeqCst) != 3 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("all owned tasks started");

    cancellation.cancel();
    supervisor
        .await
        .expect("supervisor task joined")
        .expect("clean cancellation");

    assert_eq!(active.load(Ordering::SeqCst), 0);
    assert_eq!(completed.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn task_failure_cancels_and_joins_its_peers() {
    let cancellation = CancellationToken::new();
    let peer_joined = Arc::new(AtomicUsize::new(0));
    let peer_cancellation = cancellation.child_token();
    let joined = Arc::clone(&peer_joined);
    let tasks = vec![
        OwnedTask::new("failing", async {
            Err(AppError::TaskFailed {
                task: "failing",
                reason_code: "fixture.failure",
            })
        }),
        OwnedTask::new("peer", async move {
            peer_cancellation.cancelled().await;
            joined.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }),
    ];

    let error = run_owned_tasks(cancellation, Duration::from_secs(1), tasks)
        .await
        .expect_err("task failure must fail the service");

    assert_eq!(error.reason_code(), "fixture.failure");
    assert_eq!(peer_joined.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn task_panic_is_a_fatal_joined_failure() {
    let cancellation = CancellationToken::new();
    let tasks = vec![OwnedTask::new("panicking", async {
        panic!("test panic payload must not be surfaced");
        #[allow(unreachable_code)]
        Ok(())
    })];

    let error = run_owned_tasks(cancellation, Duration::from_secs(1), tasks)
        .await
        .expect_err("task panic must fail the service");

    assert_eq!(error.reason_code(), "capture_app.task_panicked");
    assert!(!error.to_string().contains("test panic payload"));
}

#[tokio::test]
async fn dropping_an_unstarted_owned_join_task_aborts_the_underlying_task() {
    struct ActiveGuard(Arc<AtomicUsize>);

    impl Drop for ActiveGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, Ordering::SeqCst);
        }
    }

    let active = Arc::new(AtomicUsize::new(0));
    let task_active = Arc::clone(&active);
    let handle = tokio::spawn(async move {
        task_active.fetch_add(1, Ordering::SeqCst);
        let _active = ActiveGuard(task_active);
        std::future::pending::<()>().await;
        #[allow(unreachable_code)]
        Ok(())
    });
    let owned = OwnedTask::from_join_handle("database-driver", handle);

    timeout(Duration::from_secs(1), async {
        while active.load(Ordering::SeqCst) != 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("underlying task started");
    drop(owned);

    timeout(Duration::from_secs(1), async {
        while active.load(Ordering::SeqCst) != 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("underlying task aborted when ownership was dropped");
}

#[test]
fn status_snapshot_is_atomic_versioned_bounded_and_secret_free() {
    let directory = tempdir().expect("temporary status directory");
    let status_path = directory.path().join("capture-status.json");
    let writer = StatusWriter::new(status_path.clone()).expect("status writer");
    let first = CaptureStatus::new(
        KnownTime::from_unix_micros(100).expect("time"),
        "build-123",
        ChainId::new("mainnet").expect("chain"),
        CaptureHealth::Yellow,
    )
    .with_readiness(false)
    .with_durable_height(Some(BlockHeight::new(41)))
    .with_pending_blocks(2)
    .with_archive_manifest_id(Some("manifest-41".to_owned()))
    .with_last_error_reason(Some("capture_bus.unavailable".to_owned()));
    writer.write(&first).expect("first status write");

    let second = CaptureStatus::new(
        KnownTime::from_unix_micros(200).expect("time"),
        "build-123",
        ChainId::new("mainnet").expect("chain"),
        CaptureHealth::Green,
    )
    .with_readiness(true)
    .with_source_state(
        CommittedSourceClass::LocallyVerifiedCommitted,
        CaptureSourceHealth::Healthy,
        None,
        None,
        None,
    )
    .with_durable_height(Some(BlockHeight::new(42)))
    .with_pending_blocks(0)
    .with_archive_manifest_id(Some("manifest-42".to_owned()));
    writer.write(&second).expect("second status write");

    let bytes = fs::read(&status_path).expect("published status");
    assert!(bytes.len() < 16 * 1024);
    let text = String::from_utf8(bytes).expect("status is UTF-8 JSON");
    assert!(!text.contains("secret"));
    assert!(!text.contains("postgresql://"));
    assert!(!text.contains("nats://"));
    let decoded: serde_json::Value = serde_json::from_str(&text).expect("status JSON");
    assert_eq!(decoded["schema_version"], "hl.capture.status.v5");
    assert_eq!(decoded["maintenance"]["enabled"], false);
    assert_eq!(decoded["maintenance"]["retention_authorized"], false);
    assert!(decoded.get("throughput_records_per_sec").is_none());
    assert!(decoded.get("throughput_blocks_per_sec").is_none());
    assert_eq!(decoded["snapshot_at_micros"], 200);
    assert_eq!(decoded["health"], "green");
    assert_eq!(decoded["ready"], true);
    assert_eq!(decoded["chain_id"], "mainnet");
    assert_eq!(decoded["durable_height"], 42);
    assert_eq!(decoded["pending_blocks"], 0);
    assert_eq!(
        decoded["active_committed_source"],
        "locally-verified-committed"
    );
    assert_eq!(decoded["primary_source_health"], "healthy");
    assert!(decoded.get("independent_source_health").is_none());
    assert!(decoded.get("failover_height").is_none());
    assert_eq!(decoded["archive_manifest_id"], "manifest-42");
    assert!(decoded.get("last_error_reason").is_none());
}

#[test]
fn status_rejects_inconsistent_backlog_and_disk_capacity() {
    let directory = tempdir().expect("temporary status directory");
    let writer =
        StatusWriter::new(directory.path().join("capture-status.json")).expect("status writer");
    let base = CaptureStatus::new(
        KnownTime::from_unix_micros(1).expect("time"),
        "build-123",
        ChainId::new("mainnet").expect("chain"),
        CaptureHealth::Green,
    )
    .with_source_state(
        CommittedSourceClass::LocallyVerifiedCommitted,
        CaptureSourceHealth::Healthy,
        None,
        None,
        None,
    );

    let missing_backlog =
        base.clone()
            .with_capture_capacity(0, Some(BlockHeight::new(42)), Some(2_000));
    assert!(matches!(
        writer.write(&missing_backlog),
        Err(StatusError::InvalidField)
    ));

    let invalid_percentage =
        base.with_capture_capacity(1, Some(BlockHeight::new(42)), Some(10_001));
    assert!(matches!(
        writer.write(&invalid_percentage),
        Err(StatusError::InvalidField)
    ));

    let green_failover = CaptureStatus::new(
        KnownTime::from_unix_micros(2).expect("time"),
        "build-123",
        ChainId::new("mainnet").expect("chain"),
        CaptureHealth::Green,
    )
    .with_readiness(true)
    .with_source_state(
        CommittedSourceClass::IndependentCommitted,
        CaptureSourceHealth::RangeUnavailable,
        Some(CaptureSourceHealth::Healthy),
        Some(BlockHeight::new(42)),
        Some(FailoverReason::PrimaryRangeUnavailable),
    );
    assert!(matches!(
        writer.write(&green_failover),
        Err(StatusError::InvalidField)
    ));
}
