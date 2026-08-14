use std::{collections::BTreeMap, fs, os::unix::fs::PermissionsExt, time::Duration};

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use canonical_ledger::StateImageLimits;
use canonical_state_store::SyncedWriteBatchStore;
use domain_types::{
    Address, BlockHeight, ChainId, KnownTime, ManifestId, MarketId, Price, ProtocolTime, Quantity,
    SourceId, TransactionId,
};
use hl_core::{
    committed_block_delivery, committed_event_delivery, CoreConfig, CoreRuntime, CoreStatusHandle,
    InMemoryCanonicalSource, DEAD_LETTER_SCHEMA_V1,
};
use storage_ports::{ArchiveReceipt, AtomicStateStore};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn missing_store_parent_fails_closed_without_opening_nats() {
    let root = private_root();
    let missing = root.path().join("absent").join("state");
    let config = CoreConfig::from_toml(&valid_toml(&missing, 200, None)).expect("config");
    let error = match CoreRuntime::open(config) {
        Ok(_) => panic!("missing store must fail closed"),
        Err(error) => error,
    };
    assert_eq!(error.reason_code(), "core_runtime.store");
}

#[tokio::test]
async fn action_bearing_source_still_fails_closed_without_ack_or_state_advance() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, None)).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let status = runtime.status().clone();
    let block = trade_block(200);
    let receipt = archive_receipt(&block);
    let marker = committed_block_delivery(&block, &receipt).expect("marker");
    let event = committed_event_delivery(&block.events()[0], &block, &receipt).expect("event");
    let event_id = event.message_id.clone();
    let marker_id = marker.message_id.clone();
    let mut source = InMemoryCanonicalSource::new([event, marker]);
    let cancellation = CancellationToken::new();

    let error = runtime
        .run_source(&mut source, cancellation)
        .await
        .expect_err("action-bearing mapping/reducer still rejects");
    assert_eq!(error.reason_code(), "ledger.unsupported_event");
    assert!(!source.acked().contains(&event_id));
    assert!(!source.acked().contains(&marker_id));
    let snapshot = status.snapshot();
    assert!(!snapshot.ready());
    assert_eq!(
        snapshot.fail_closed_reason(),
        Some("ledger.unsupported_event")
    );
    assert!(snapshot.last_applied_watermark().is_none());
    assert!(!snapshot.live_qualified());
    assert!(!snapshot.stage_2_qualified());

    let store =
        SyncedWriteBatchStore::open(&store_path, StateImageLimits::production()).expect("reopen");
    assert!(store
        .load_latest(StateImageLimits::production())
        .expect("load")
        .is_none());
    let encoded = fs::read_to_string(root.path().join("dead-letter.jsonl")).expect("dlq");
    let record: serde_json::Value = serde_json::from_str(encoded.trim()).expect("dlq json");
    assert_eq!(record["reason_code"], "ledger.unsupported_event");
    assert!(record.get("stream_sequence").is_none());
    assert!(record.get("live_qualified").is_none());
    assert!(record.get("stage_2_qualified").is_none());
}

#[tokio::test]
async fn empty_source_shuts_down_cleanly_without_qualification_claims() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, None)).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut source = InMemoryCanonicalSource::new([]);
    let report = runtime
        .run_source(&mut source, cancellation)
        .await
        .expect("clean shutdown");
    assert_eq!(report.applied, 0);
    assert!(report.last_height.is_none());
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
}

#[tokio::test]
async fn idle_loop_observes_cancellation_without_connecting_to_nats() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, None)).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let cancellation = CancellationToken::new();
    let mut source = InMemoryCanonicalSource::new([]);
    let run = runtime.run_source(&mut source, cancellation.clone());
    let shutdown = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancellation.cancel();
    };
    let (report, ()) = tokio::join!(run, shutdown);
    let report = report.expect("cancelled idle loop");
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_runtime_serves_ready_loopback_status_without_qualification() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config =
        CoreConfig::from_toml(&valid_toml(&store_path, 200, Some("127.0.0.1:0"))).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let status = runtime.status().clone();
    let cancellation = CancellationToken::new();
    let mut source = InMemoryCanonicalSource::new([]);
    let run = runtime.run_source(&mut source, cancellation.clone());
    let probe = async {
        let addr = wait_for_listen_addr(&status).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if status.snapshot().ready() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("ready");
        let (health_code, health_body) = http_get(addr, "/healthz").await;
        assert_eq!(health_code, 200);
        let health = json_from_http(&health_body);
        assert_eq!(health["ready"], true);
        assert_eq!(health["live_qualified"], false);
        assert_eq!(health["stage_2_qualified"], false);
        let (status_code, status_body) = http_get(addr, "/status").await;
        assert_eq!(status_code, 200);
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], true);
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        assert!(value.get("fail_closed_reason").is_none());
        cancellation.cancel();
    };
    let (report, ()) = tokio::join!(run, probe);
    let report = report.expect("cancelled ready loop");
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn applied_empty_block_publishes_watermark_and_stays_unqualified() {
    let root = private_root();
    let store_path = root.path().join("state");
    let config =
        CoreConfig::from_toml(&valid_toml(&store_path, 200, Some("127.0.0.1:0"))).expect("config");
    let runtime = CoreRuntime::open(config).expect("runtime");
    let status = runtime.status().clone();
    let block = empty_block(200);
    let delivery = committed_block_delivery(&block, &archive_receipt(&block)).expect("delivery");
    let mut source = InMemoryCanonicalSource::new([delivery]);
    let cancellation = CancellationToken::new();
    let run = runtime.run_source(&mut source, cancellation.clone());
    let probe = async {
        let addr = wait_for_listen_addr(&status).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if status.snapshot().last_applied_watermark() == Some(200) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("watermark");
        let (_, status_body) = http_get(addr, "/status").await;
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], true);
        assert_eq!(value["last_applied_watermark"], 200);
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        cancellation.cancel();
    };
    let (report, ()) = tokio::join!(run, probe);
    let report = report.expect("cancelled after apply");
    assert_eq!(report.last_height, Some(BlockHeight::new(200)));
    assert!(!report.live_qualified);
    assert!(!report.stage_2_qualified);
}

#[tokio::test]
async fn connect_transport_failure_dlqs_then_latches_fail_closed_status() {
    let root = private_root();
    let store_path = root.path().join("state");
    let missing_password = root.path().join("missing-nats-password");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, None).replace(
        "password_path = \"/run/secrets/alpha-desk-nats-core-password\"",
        &format!("password_path = \"{}\"", missing_password.display()),
    ))
    .expect("config");
    let runtime = CoreRuntime::from_config(config);
    let status = runtime.status().clone();

    let error = runtime
        .run_jetstream(CancellationToken::new())
        .await
        .expect_err("connect transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert!(
        !store_path.exists(),
        "connect failure must not open the file-store"
    );

    let snapshot = status.snapshot();
    assert!(!snapshot.ready());
    assert_eq!(
        snapshot.fail_closed_reason(),
        Some("core.jetstream_transport")
    );
    assert!(snapshot.last_applied_watermark().is_none());
    assert!(!snapshot.live_qualified());
    assert!(!snapshot.stage_2_qualified());

    let encoded = fs::read_to_string(root.path().join("dead-letter.jsonl")).expect("dlq");
    let record: serde_json::Value = serde_json::from_str(encoded.trim()).expect("dlq json");
    assert_eq!(record["schema_version"], DEAD_LETTER_SCHEMA_V1);
    assert_eq!(record["reason_code"], "core.jetstream_transport");
    assert_eq!(record["subject"], "hl.v1.connect.transport");
    assert_eq!(record["message_id"], "connect");
    assert!(record.get("live_qualified").is_none());
    assert!(record.get("stage_2_qualified").is_none());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_transport_failure_is_visible_on_loopback_status() {
    let root = private_root();
    let store_path = root.path().join("state");
    let missing_password = root.path().join("missing-nats-password");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, Some("127.0.0.1:0")).replace(
        "password_path = \"/run/secrets/alpha-desk-nats-core-password\"",
        &format!("password_path = \"{}\"", missing_password.display()),
    ))
    .expect("config");
    let runtime = CoreRuntime::from_config(config);
    let status = runtime.status().clone();
    let cancellation = CancellationToken::new();
    let run = runtime.run_jetstream(cancellation.clone());
    let probe = async {
        let addr = wait_for_listen_addr(&status).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if status.snapshot().fail_closed_reason() == Some("core.jetstream_transport") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fail_closed");
        assert!(
            !store_path.exists(),
            "connect failure must not open the file-store"
        );
        let (health_code, health_body) = http_get(addr, "/healthz").await;
        assert_eq!(health_code, 503);
        let health = json_from_http(&health_body);
        assert_eq!(health["ok"], false);
        assert_eq!(health["ready"], false);
        assert_eq!(health["reason_code"], "core.jetstream_transport");
        assert_eq!(health["live_qualified"], false);
        assert_eq!(health["stage_2_qualified"], false);
        let (status_code, status_body) = http_get(addr, "/status").await;
        assert_eq!(status_code, 200);
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], false);
        assert_eq!(value["fail_closed_reason"], "core.jetstream_transport");
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(run, probe);
    let error = result.expect_err("connect transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    let snapshot = status.snapshot();
    assert!(!snapshot.ready());
    assert!(!snapshot.live_qualified());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn missing_store_parent_is_visible_on_loopback_status() {
    let root = private_root();
    let missing = root.path().join("absent").join("state");
    let config =
        CoreConfig::from_toml(&valid_toml(&missing, 200, Some("127.0.0.1:0"))).expect("config");
    let runtime = CoreRuntime::from_config(config);
    let status = runtime.status().clone();
    let cancellation = CancellationToken::new();
    let mut source = InMemoryCanonicalSource::new([]);
    let run = runtime.run_source(&mut source, cancellation.clone());
    let probe = async {
        let addr = wait_for_listen_addr(&status).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if status.snapshot().fail_closed_reason() == Some("core_runtime.store") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fail_closed");
        let (health_code, health_body) = http_get(addr, "/healthz").await;
        assert_eq!(health_code, 503);
        let health = json_from_http(&health_body);
        assert_eq!(health["ok"], false);
        assert_eq!(health["ready"], false);
        assert_eq!(health["reason_code"], "core_runtime.store");
        assert_eq!(health["live_qualified"], false);
        assert_eq!(health["stage_2_qualified"], false);
        let (status_code, status_body) = http_get(addr, "/status").await;
        assert_eq!(status_code, 200);
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], false);
        assert_eq!(value["fail_closed_reason"], "core_runtime.store");
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(run, probe);
    let error = result.expect_err("missing store");
    assert_eq!(error.reason_code(), "core_runtime.store");
    let snapshot = status.snapshot();
    assert!(!snapshot.ready());
    assert!(!snapshot.live_qualified());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_connect_serves_starting_loopback_status_without_opening_store() {
    let root = private_root();
    let store_path = root.path().join("state");
    let password_path = write_protected_secret(root.path());
    let nats_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("hung NATS listener");
    let nats_addr = nats_listener.local_addr().expect("nats addr");
    let _hold = tokio::spawn(hold_tcp_without_nats(nats_listener));
    let config = CoreConfig::from_toml(&hung_connect_toml(
        &store_path,
        &password_path,
        nats_addr,
        2_500,
        Some("127.0.0.1:0"),
    ))
    .expect("config");
    let runtime = CoreRuntime::from_config(config);
    let status = runtime.status().clone();
    let cancellation = CancellationToken::new();
    let run = runtime.run_jetstream(cancellation.clone());
    let probe = async {
        let addr = wait_for_listen_addr(&status).await;
        let (health_code, health_body) = http_get(addr, "/healthz").await;
        assert_eq!(health_code, 503);
        let health = json_from_http(&health_body);
        assert_eq!(health["ok"], false);
        assert_eq!(health["ready"], false);
        assert_eq!(health["reason_code"], "core_status.not_ready");
        assert_eq!(health["live_qualified"], false);
        assert_eq!(health["stage_2_qualified"], false);
        let (status_code, status_body) = http_get(addr, "/status").await;
        assert_eq!(status_code, 200);
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], false);
        assert!(value.get("fail_closed_reason").is_none());
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        assert!(
            !store_path.exists(),
            "hung connect must not open the file-store"
        );
        assert!(
            !root.path().join("dead-letter.jsonl").exists(),
            "hung connect wait must not create dead-letter.jsonl before timeout"
        );

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if status.snapshot().fail_closed_reason() == Some("core.jetstream_transport") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fail_closed after hung connect");

        let (health_code, health_body) = http_get(addr, "/healthz").await;
        assert_eq!(health_code, 503);
        let health = json_from_http(&health_body);
        assert_eq!(health["reason_code"], "core.jetstream_transport");
        let (status_code, status_body) = http_get(addr, "/status").await;
        assert_eq!(status_code, 200);
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], false);
        assert_eq!(value["fail_closed_reason"], "core.jetstream_transport");
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        assert!(
            !store_path.exists(),
            "connect failure must not open the file-store"
        );
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(run, probe);
    let error = result.expect_err("hung connect transport");
    assert_eq!(error.reason_code(), "core.jetstream_transport");
    assert!(!store_path.exists());
    assert_connect_transport_sentinel(&root.path().join("dead-letter.jsonl"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hung_connect_abort_does_not_leave_empty_dlq_without_sentinel() {
    let root = private_root();
    let store_path = root.path().join("state");
    let dead_letter_path = root.path().join("dead-letter.jsonl");
    let password_path = write_protected_secret(root.path());
    let nats_listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("hung NATS listener");
    let nats_addr = nats_listener.local_addr().expect("nats addr");
    let _hold = tokio::spawn(hold_tcp_without_nats(nats_listener));
    let config = CoreConfig::from_toml(&hung_connect_toml(
        &store_path,
        &password_path,
        nats_addr,
        30_000,
        Some("127.0.0.1:0"),
    ))
    .expect("config");
    let runtime = CoreRuntime::from_config(config);
    let status = runtime.status().clone();
    let run = tokio::spawn(runtime.run_jetstream(CancellationToken::new()));
    let addr = wait_for_listen_addr(&status).await;
    let (health_code, health_body) = http_get(addr, "/healthz").await;
    assert_eq!(health_code, 503);
    let health = json_from_http(&health_body);
    assert_eq!(health["ready"], false);
    assert_eq!(health["reason_code"], "core_status.not_ready");
    assert!(
        !store_path.exists(),
        "hung connect must not open the file-store"
    );
    assert!(
        !dead_letter_path.exists(),
        "hung connect wait must not create dead-letter.jsonl before abort"
    );
    run.abort();
    let _ = run.await;
    assert!(!store_path.exists());
    assert!(
        !dead_letter_path.exists(),
        "abort during hung connect wait must not leave dead-letter.jsonl"
    );
}

#[tokio::test]
async fn dead_letter_open_before_store_fails_closed_with_typed_reason() {
    // `run_jetstream` opens the file DLQ before connect/store. Occupying that
    // path fails closed without live NATS and without creating the file-store.
    let root = private_root();
    let store_path = root.path().join("state");
    let dead_letter_path = root.path().join("dead-letter.jsonl");
    fs::create_dir(&dead_letter_path).expect("dead-letter path occupied by a directory");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, None)).expect("config");
    let runtime = CoreRuntime::from_config(config);

    let error = runtime
        .run_jetstream(CancellationToken::new())
        .await
        .expect_err("dead-letter open");
    assert_eq!(error.reason_code(), "core.deadletter_unsafe_path");
    assert!(
        !store_path.exists(),
        "dead-letter open must not create the file-store"
    );
    assert!(
        dead_letter_path.is_dir(),
        "occupied dead-letter path must remain a directory"
    );
}

#[tokio::test]
async fn dead_letter_corrupt_open_before_store_fails_closed_with_typed_reason() {
    let root = private_root();
    let store_path = root.path().join("state");
    let dead_letter_path = root.path().join("dead-letter.jsonl");
    let leftover = b"{\"schema_version\":\"hl.core.deadletter.v1\"";
    fs::write(&dead_letter_path, leftover).expect("truncated leftover");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, None)).expect("config");
    let runtime = CoreRuntime::from_config(config);

    let error = runtime
        .run_jetstream(CancellationToken::new())
        .await
        .expect_err("dead-letter open");
    assert_eq!(error.reason_code(), "core.deadletter_corrupt");
    assert!(
        !store_path.exists(),
        "dead-letter open must not create the file-store"
    );
    assert_eq!(
        fs::read(&dead_letter_path).expect("corrupt leftover left in place"),
        leftover
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_letter_open_before_store_is_visible_on_loopback_status() {
    // Production order: bind HTTP → open file DLQ → connect wait → store only
    // on success. Occupying the DLQ path proves HTTP scrape before connect.
    let root = private_root();
    let store_path = root.path().join("state");
    let dead_letter_path = root.path().join("dead-letter.jsonl");
    fs::create_dir(&dead_letter_path).expect("dead-letter path occupied by a directory");
    let config =
        CoreConfig::from_toml(&valid_toml(&store_path, 200, Some("127.0.0.1:0"))).expect("config");
    let runtime = CoreRuntime::from_config(config);
    let status = runtime.status().clone();
    assert!(
        !store_path.exists(),
        "file-store must stay closed until after successful connect"
    );
    let cancellation = CancellationToken::new();
    let run = runtime.run_jetstream(cancellation.clone());
    let probe = async {
        let addr = wait_for_listen_addr(&status).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if status.snapshot().fail_closed_reason() == Some("core.deadletter_unsafe_path") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fail_closed");
        assert!(
            !store_path.exists(),
            "dead-letter open must not create the file-store"
        );
        let (health_code, health_body) = http_get(addr, "/healthz").await;
        assert_eq!(health_code, 503);
        let health = json_from_http(&health_body);
        assert_eq!(health["ok"], false);
        assert_eq!(health["ready"], false);
        assert_eq!(health["reason_code"], "core.deadletter_unsafe_path");
        assert_eq!(health["live_qualified"], false);
        assert_eq!(health["stage_2_qualified"], false);
        let (status_code, status_body) = http_get(addr, "/status").await;
        assert_eq!(status_code, 200);
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], false);
        assert_eq!(value["fail_closed_reason"], "core.deadletter_unsafe_path");
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        let (metrics_code, metrics_body) = http_get(addr, "/metrics").await;
        assert_eq!(metrics_code, 200);
        assert!(metrics_body.contains("hl_core_ready 0"));
        assert!(metrics_body.contains("hl_core_live_qualified 0"));
        assert!(metrics_body.contains("hl_core_stage_2_qualified 0"));
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(run, probe);
    let error = result.expect_err("dead-letter open");
    assert_eq!(error.reason_code(), "core.deadletter_unsafe_path");
    let snapshot = status.snapshot();
    assert!(!snapshot.ready());
    assert!(!snapshot.live_qualified());
    assert!(!snapshot.stage_2_qualified());
    assert!(!store_path.exists());
}

#[tokio::test]
async fn dead_letter_open_after_store_fails_closed_with_typed_reason() {
    let root = private_root();
    let store_path = root.path().join("state");
    let dead_letter_path = root.path().join("dead-letter.jsonl");
    fs::create_dir(&dead_letter_path).expect("dead-letter path occupied by a directory");
    let config = CoreConfig::from_toml(&valid_toml(&store_path, 200, None)).expect("config");
    let runtime = CoreRuntime::open(config).expect("store open");
    assert!(
        store_path.exists(),
        "file-store must already be in play before dead-letter open"
    );

    let error = runtime
        .run_source(
            &mut InMemoryCanonicalSource::new([]),
            CancellationToken::new(),
        )
        .await
        .expect_err("dead-letter open");
    assert_eq!(error.reason_code(), "core.deadletter_unsafe_path");
    assert!(
        store_path.exists(),
        "dead-letter open must not unwind the store"
    );
    assert!(
        dead_letter_path.is_dir(),
        "occupied dead-letter path must remain a directory"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn dead_letter_open_after_store_is_visible_on_loopback_status() {
    // Store-then-DLQ-open is the `run_source` path. `run_jetstream` store-open
    // after successful connect still needs live NATS; live Term / HL_DEADLETTER
    // remains unproven. DLQ-open-before-store HTTP is covered separately.
    let root = private_root();
    let store_path = root.path().join("state");
    let dead_letter_path = root.path().join("dead-letter.jsonl");
    fs::create_dir(&dead_letter_path).expect("dead-letter path occupied by a directory");
    let config =
        CoreConfig::from_toml(&valid_toml(&store_path, 200, Some("127.0.0.1:0"))).expect("config");
    let runtime = CoreRuntime::open(config).expect("store open");
    let status = runtime.status().clone();
    assert!(
        store_path.exists(),
        "file-store must already be in play before dead-letter open"
    );
    let cancellation = CancellationToken::new();
    let mut source = InMemoryCanonicalSource::new([]);
    let run = runtime.run_source(&mut source, cancellation.clone());
    let probe = async {
        let addr = wait_for_listen_addr(&status).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if status.snapshot().fail_closed_reason() == Some("core.deadletter_unsafe_path") {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("fail_closed");
        assert!(
            store_path.exists(),
            "file-store must stay in play after dead-letter open fails"
        );
        let (health_code, health_body) = http_get(addr, "/healthz").await;
        assert_eq!(health_code, 503);
        let health = json_from_http(&health_body);
        assert_eq!(health["ok"], false);
        assert_eq!(health["ready"], false);
        assert_eq!(health["reason_code"], "core.deadletter_unsafe_path");
        assert_eq!(health["live_qualified"], false);
        assert_eq!(health["stage_2_qualified"], false);
        let (status_code, status_body) = http_get(addr, "/status").await;
        assert_eq!(status_code, 200);
        let value = json_from_http(&status_body);
        assert_eq!(value["ready"], false);
        assert_eq!(value["fail_closed_reason"], "core.deadletter_unsafe_path");
        assert_eq!(value["live_qualified"], false);
        assert_eq!(value["stage_2_qualified"], false);
        let (metrics_code, metrics_body) = http_get(addr, "/metrics").await;
        assert_eq!(metrics_code, 200);
        assert!(metrics_body.contains("hl_core_ready 0"));
        assert!(metrics_body.contains("hl_core_live_qualified 0"));
        assert!(metrics_body.contains("hl_core_stage_2_qualified 0"));
        cancellation.cancel();
    };
    let (result, ()) = tokio::join!(run, probe);
    let error = result.expect_err("dead-letter open");
    assert_eq!(error.reason_code(), "core.deadletter_unsafe_path");
    let snapshot = status.snapshot();
    assert!(!snapshot.ready());
    assert!(!snapshot.live_qualified());
    assert!(!snapshot.stage_2_qualified());
}

fn valid_toml(store_path: &std::path::Path, first_height: u64, listen: Option<&str>) -> String {
    let status = listen
        .map(|listen| format!("\n[status]\nlisten = \"{listen}\"\n"))
        .unwrap_or_default();
    format!(
        r#"
chain_id = "mainnet"
first_height = {first_height}
shutdown_grace_millis = 15000
idle_poll_millis = 50

[store]
path = "{path}"
{status}
[nats]
server_url = "nats://127.0.0.1:4222"
stream = "HL_CANONICAL"
username = "core"
password_path = "/run/secrets/alpha-desk-nats-core-password"
connect_timeout_millis = 5000
acknowledgement_timeout_millis = 5000
max_ack_inflight = 64
durable_name = "hl-core-file-replay"
fetch_batch = 64
"#,
        path = store_path.display()
    )
}

fn hung_connect_toml(
    store_path: &std::path::Path,
    password_path: &std::path::Path,
    nats_addr: std::net::SocketAddr,
    connect_timeout_millis: u64,
    listen: Option<&str>,
) -> String {
    valid_toml(store_path, 200, listen)
        .replace(
            "server_url = \"nats://127.0.0.1:4222\"",
            &format!("server_url = \"nats://{nats_addr}\""),
        )
        .replace(
            "password_path = \"/run/secrets/alpha-desk-nats-core-password\"",
            &format!("password_path = \"{}\"", password_path.display()),
        )
        .replace(
            "connect_timeout_millis = 5000",
            &format!("connect_timeout_millis = {connect_timeout_millis}"),
        )
}

fn write_protected_secret(root: &std::path::Path) -> std::path::PathBuf {
    let path = root.join("nats-password");
    fs::write(&path, "hung-connect-test\n").expect("write secret");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).expect("secret mode");
    path
}

async fn hold_tcp_without_nats(listener: TcpListener) {
    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                tokio::spawn(async move {
                    let _stream = stream;
                    std::future::pending::<()>().await;
                });
            }
            Err(_) => return,
        }
    }
}

fn private_root() -> tempfile::TempDir {
    let temporary = tempfile::tempdir().expect("temporary root");
    fs::set_permissions(temporary.path(), fs::Permissions::from_mode(0o700))
        .expect("private parent");
    temporary
}

fn assert_connect_transport_sentinel(path: &std::path::Path) {
    let encoded = fs::read_to_string(path).expect("dlq");
    let found = encoded.lines().filter(|line| !line.is_empty()).any(|line| {
        let record: serde_json::Value = serde_json::from_str(line).expect("jsonl");
        record["schema_version"] == DEAD_LETTER_SCHEMA_V1
            && record["reason_code"] == "core.jetstream_transport"
            && record["subject"] == "hl.v1.connect.transport"
            && record["message_id"] == "connect"
    });
    assert!(found, "connect transport sentinel missing from {path:?}");
}

fn trade_block(height: u64) -> BlockEnvelope {
    let event = trade_event(height);
    BlockEnvelope::try_new(
        event.chain_id().clone(),
        event.block_height(),
        event.block_time(),
        ConfirmationClass::CommittedPrimary,
        vec![event],
        BTreeMap::from([(
            SourceId::new("jetstream-replay").expect("source"),
            [0x44; 32],
        )]),
    )
    .expect("action-bearing block")
}

fn trade_event(height: u64) -> CanonicalEventEnvelope {
    let block_time = ProtocolTime::from_unix_micros(height as i64).expect("time");
    let payload = EventPayload::TradeMatched(TradeMatched::without_identities(
        Price::parse_at_scale("65000", 6).expect("price"),
        Quantity::parse_at_scale("0.01", 8).expect("quantity"),
        1,
    ));
    let payload_hash = *blake3::hash(&payload.encode_to_vec().expect("payload bytes")).as_bytes();
    CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("tx-{height}")).expect("transaction"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: vec![MarketId::new("perp:BTC").expect("market")],
        account_ids: vec![
            Address::from_bytes([0x11; 20]),
            Address::from_bytes([0x22; 20]),
        ],
        source_evidence: vec![SourceEvidence::try_new_indexed(
            SourceId::new("jetstream-replay").expect("source"),
            "v1",
            height.to_string(),
            payload_hash,
            0,
        )
        .expect("evidence")],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        ingested_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        canonicalized_at: KnownTime::from_unix_micros(height as i64).expect("known time"),
        parser_version: "test-parser-v1".to_owned(),
        payload,
    })
    .expect("event")
}

fn empty_block(height: u64) -> BlockEnvelope {
    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain"),
        BlockHeight::new(height),
        ProtocolTime::from_unix_micros(height as i64).expect("time"),
        ConfirmationClass::CommittedPrimary,
        Vec::new(),
        BTreeMap::from([(SourceId::new("jetstream-replay").expect("source"), [1; 32])]),
    )
    .expect("empty block")
}

async fn wait_for_listen_addr(status: &CoreStatusHandle) -> std::net::SocketAddr {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(addr) = status.listen_addr() {
                return addr;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("listen addr")
}

async fn http_get(addr: std::net::SocketAddr, path: &str) -> (u16, String) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match TcpStream::connect(addr).await {
                Ok(mut stream) => {
                    stream
                        .write_all(
                            format!(
                                "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
                            )
                            .as_bytes(),
                        )
                        .await
                        .expect("write request");
                    stream.flush().await.expect("flush request");
                    let mut body = Vec::new();
                    stream.read_to_end(&mut body).await.expect("read response");
                    let body = String::from_utf8(body).expect("UTF-8 response");
                    let status = body
                        .split_whitespace()
                        .nth(1)
                        .and_then(|value| value.parse().ok())
                        .expect("HTTP status");
                    return (status, body);
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(5)).await,
            }
        }
    })
    .await
    .expect("http response")
}

fn json_from_http(body: &str) -> serde_json::Value {
    let json_start = body.find("\r\n\r\n").expect("header terminator") + 4;
    serde_json::from_str(&body[json_start..]).expect("JSON body")
}

fn archive_receipt(block: &BlockEnvelope) -> ArchiveReceipt {
    ArchiveReceipt::try_new(
        format!("receipt-{}", block.block_height().get()),
        ManifestId::new(format!(
            "manifest-{}",
            hex::encode(block.canonical_block_hash())
        ))
        .expect("manifest"),
        block.block_height(),
        block.canonical_block_hash(),
        [0x11; 32],
        [0x22; 32],
        [0x33; 32],
        KnownTime::from_unix_micros(1_721_779_300_000_000).expect("durable at"),
    )
    .expect("archive receipt")
}
