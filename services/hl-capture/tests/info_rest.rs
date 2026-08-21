mod info_rest {
    use std::collections::BTreeMap;
    use std::sync::Mutex;

    use domain_types::KnownTime;
    use hl_capture::{
        CaptureClock, EgressError, FakeCaptureClock, InfoCaptureCoordinator, InfoCaptureError,
        InfoCaptureOutcome, InfoFaultInjector, InfoFaultPoint, InfoHttpResponse, InfoJobCheckpoint,
        InfoTransport, MemoryInfoArchive, MemoryInfoPublisher, NoInfoFaults, RequestBudget,
        SchedulePriority, ScriptedInfoTransport, TimePageStopReason, capture_time_pages,
        default_info_request_url, fetch_info, forbids_exchange_request, parse_request_cost,
    };
    use hl_protocol::info::{InfoRegistry, TimePageCursor};
    use serde_json::json;

    fn now() -> KnownTime {
        KnownTime::from_unix_micros(1).expect("time")
    }

    struct OneShotInfoFault {
        point: Mutex<Option<InfoFaultPoint>>,
    }

    impl OneShotInfoFault {
        fn new(point: InfoFaultPoint) -> Self {
            Self {
                point: Mutex::new(Some(point)),
            }
        }
    }

    impl InfoFaultInjector for OneShotInfoFault {
        fn check(&self, point: InfoFaultPoint) -> Result<(), InfoCaptureError> {
            let mut selected = self.point.lock().expect("fault lock");
            if selected.as_ref() == Some(&point) {
                selected.take();
                Err(InfoCaptureError::InjectedFault(point))
            } else {
                Ok(())
            }
        }
    }

    struct TimeoutTransport;

    impl InfoTransport for TimeoutTransport {
        fn post_info(
            &mut self,
            url: &str,
            _request: &hl_protocol::info::EncodedInfoRequest,
        ) -> Result<InfoHttpResponse, EgressError> {
            let _ = hl_capture::guard_info_request_url(url)?;
            Err(EgressError::Timeout)
        }
    }

    #[test]
    fn non_allowlisted_host_fails_at_request_time() {
        let mut transport =
            ScriptedInfoTransport::new([InfoHttpResponse::new(200, b"{}".to_vec())]);
        let error = fetch_info(
            &mut transport,
            InfoRegistry::official(),
            "official.info.all_mids",
            &BTreeMap::new(),
            now(),
            "test-archive",
            "https://evil.example/info",
        )
        .expect_err("host");
        assert!(matches!(error, EgressError::HostNotAllowlisted));
        assert!(transport.posted().is_empty());
    }

    #[test]
    fn http_url_fails_tls_required() {
        let mut transport =
            ScriptedInfoTransport::new([InfoHttpResponse::new(200, b"{}".to_vec())]);
        let error = fetch_info(
            &mut transport,
            InfoRegistry::official(),
            "official.info.all_mids",
            &BTreeMap::new(),
            now(),
            "test-archive",
            "http://api.hyperliquid.xyz/info",
        )
        .expect_err("tls");
        assert!(matches!(error, EgressError::TlsRequired));
        assert!(transport.posted().is_empty());
    }

    #[test]
    fn exchange_status_still_fetches() {
        let mut transport = ScriptedInfoTransport::new([InfoHttpResponse::new(
            200,
            serde_json::to_vec(&json!({ "time": 1 })).expect("json"),
        )]);
        fetch_info(
            &mut transport,
            InfoRegistry::official(),
            "official.info.exchange_status",
            &BTreeMap::new(),
            now(),
            "test-archive",
            default_info_request_url(),
        )
        .expect("exchangeStatus");
        assert_eq!(transport.posted().len(), 1);
    }

    #[test]
    fn exchange_path_and_action_stay_forbidden() {
        assert!(forbids_exchange_request(
            "allMids",
            b"{}",
            "https://api.hyperliquid.xyz/exchange",
        ));
        assert!(forbids_exchange_request(
            "allMids",
            br#"{"action":{"type":"order"},"nonce":1}"#,
            default_info_request_url(),
        ));
        let mut transport =
            ScriptedInfoTransport::new([InfoHttpResponse::new(200, b"{}".to_vec())]);
        let error = fetch_info(
            &mut transport,
            InfoRegistry::official(),
            "official.info.all_mids",
            &BTreeMap::new(),
            now(),
            "test-archive",
            "https://api.hyperliquid.xyz/exchange",
        )
        .expect_err("exchange url");
        assert!(matches!(error, EgressError::ExchangeForbidden));
        assert!(transport.posted().is_empty());
    }

    #[test]
    fn crash_after_archive_replays_exactly_once() {
        let directory = tempfile::tempdir().expect("dir");
        let mut transport = ScriptedInfoTransport::new([InfoHttpResponse::new(
            200,
            serde_json::to_vec(&json!({ "mids": { "BTC": "1" } })).expect("json"),
        )]);
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint = InfoJobCheckpoint::new("mids", "official.info.all_mids", 1, 5);
        let faults = OneShotInfoFault::new(InfoFaultPoint::AfterArchive);
        let error = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &faults, 1_048_576);
            coordinator
                .capture_response(
                    &mut transport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    Some(directory.path()),
                )
                .expect_err("crash after archive")
        };
        assert!(matches!(
            error,
            InfoCaptureError::InjectedFault(InfoFaultPoint::AfterArchive)
        ));
        assert_eq!(archive.len(), 1);
        assert!(publisher.publications().is_empty());
        assert!(checkpoint.pending_publish_ref().is_some());

        let loaded = InfoJobCheckpoint::load_from(directory.path(), "mids")
            .expect("load")
            .expect("checkpoint");
        checkpoint = loaded;
        let mut publisher = MemoryInfoPublisher::new();
        let outcome = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .replay_pending(
                    &mut checkpoint,
                    InfoRegistry::official(),
                    now(),
                    Some(directory.path()),
                )
                .expect("replay")
        };
        assert_eq!(outcome, InfoCaptureOutcome::Published);
        assert_eq!(publisher.publications().len(), 1);

        let outcome = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .replay_pending(
                    &mut checkpoint,
                    InfoRegistry::official(),
                    now(),
                    Some(directory.path()),
                )
                .expect("second")
        };
        assert_eq!(outcome, InfoCaptureOutcome::Duplicate);
        assert_eq!(publisher.publications().len(), 1);
    }

    #[test]
    fn http_timeout_leaves_resumable_checkpoint() {
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint = InfoJobCheckpoint::new("mids", "official.info.all_mids", 50, 5);
        let error = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .capture_response(
                    &mut TimeoutTransport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    None,
                )
                .expect_err("timeout")
        };
        assert!(matches!(
            error,
            InfoCaptureError::Egress(EgressError::Timeout)
        ));
        assert_eq!(checkpoint.next_start_millis(), 50);
        assert!(checkpoint.pending_publish_ref().is_none());
        assert!(archive.is_empty());
    }

    #[test]
    fn parser_quarantine_keeps_raw_bytes() {
        let mut transport = ScriptedInfoTransport::new([InfoHttpResponse::new(200, b"{".to_vec())]);
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint = InfoJobCheckpoint::new("mids", "official.info.all_mids", 1, 5);
        let outcome = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .capture_response(
                    &mut transport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    None,
                )
                .expect("quarantine")
        };
        assert_eq!(outcome, InfoCaptureOutcome::Quarantined);
        assert_eq!(archive.len(), 1);
        assert!(publisher.publications().is_empty());
        assert!(checkpoint.quarantine_reason().is_some());
        assert!(checkpoint.request_hash().is_some());
    }

    #[test]
    fn content_identical_duplicates_do_not_republish() {
        let body = serde_json::to_vec(&json!({ "mids": { "BTC": "1" } })).expect("json");
        let mut transport = ScriptedInfoTransport::new([
            InfoHttpResponse::new(200, body.clone()),
            InfoHttpResponse::new(200, body),
        ]);
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint = InfoJobCheckpoint::new("mids", "official.info.all_mids", 1, 5);
        let first = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .capture_response(
                    &mut transport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    None,
                )
                .expect("first")
        };
        assert_eq!(first, InfoCaptureOutcome::Published);
        let dup = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .capture_response(
                    &mut transport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    None,
                )
                .expect("dup")
        };
        assert_eq!(dup, InfoCaptureOutcome::Duplicate);
        assert_eq!(archive.len(), 1);
        assert_eq!(publisher.publications().len(), 1);
    }

    #[test]
    fn bounded_body_rejected() {
        let mut transport =
            ScriptedInfoTransport::new([InfoHttpResponse::new(200, vec![b'x'; 32])]);
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint = InfoJobCheckpoint::new("mids", "official.info.all_mids", 1, 5);
        let error = {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 8);
            coordinator
                .capture_response(
                    &mut transport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    None,
                )
                .expect_err("too large")
        };
        assert!(matches!(
            error,
            InfoCaptureError::Egress(EgressError::BodyTooLarge)
        ));
        assert!(archive.is_empty());
    }

    #[test]
    fn crawl_archives_pages_before_budget_can_fail() {
        let page = InfoHttpResponse::new(
            200,
            serde_json::to_vec(&json!([{ "id": "a", "time": 100 }, { "id": "b", "time": 900 }]))
                .expect("json"),
        );
        let mut transport = ScriptedInfoTransport::new([
            page.clone(),
            InfoHttpResponse::new(500, b"nope".to_vec()),
        ]);
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint =
            InfoJobCheckpoint::new("fills", "official.info.user_fills_by_time", 50, 5);
        let extra = BTreeMap::new();
        let cost = parse_request_cost("base:20 variable:window").expect("cost");
        let clock = FakeCaptureClock::at(0);
        let crawl = capture_time_pages(
            &mut transport,
            &mut budget,
            &mut archive,
            &mut publisher,
            &mut checkpoint,
            InfoRegistry::official(),
            &extra,
            TimePageCursor::new(50, 5).expect("cursor"),
            2,
            SchedulePriority::P3,
            clock.now_millis(),
            now(),
            cost,
            default_info_request_url(),
            None,
            1_048_576,
        )
        .expect("partial");
        assert_eq!(crawl.stop(), TimePageStopReason::Incomplete);
        assert_eq!(archive.len(), 1);
        assert!(checkpoint.last_archive_ref().is_some());
        assert_eq!(
            checkpoint.next_start_millis(),
            crawl.cursor().next_query_start_millis()
        );
    }

    #[test]
    fn hashes_are_present_on_successful_capture() {
        let mut transport = ScriptedInfoTransport::new([InfoHttpResponse::new(
            200,
            serde_json::to_vec(&json!({ "mids": { "BTC": "1" } })).expect("json"),
        )]);
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint = InfoJobCheckpoint::new("mids", "official.info.all_mids", 1, 5);
        {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .capture_response(
                    &mut transport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    None,
                )
                .expect("ok");
        }
        assert!(checkpoint.request_hash().is_some());
        assert!(checkpoint.last_archive_ref().unwrap().starts_with("info-"));
        let raw = archive
            .get(checkpoint.last_archive_ref().expect("ref"))
            .expect("body");
        assert!(!raw.is_empty());
    }

    #[test]
    #[ignore = "live official /info; run with HL_INFO_CAPTURE_E2E=1"]
    fn live_official_all_mids() {
        let mut transport =
            hl_capture::HttpsInfoTransport::try_new(std::time::Duration::from_secs(10), 1_048_576)
                .expect("tls");
        let mut archive = MemoryInfoArchive::new();
        let mut publisher = MemoryInfoPublisher::new();
        let mut checkpoint = InfoJobCheckpoint::new("mids", "official.info.all_mids", 1, 5);
        {
            let mut coordinator =
                InfoCaptureCoordinator::new(&mut archive, &mut publisher, &NoInfoFaults, 1_048_576);
            coordinator
                .capture_response(
                    &mut transport,
                    &mut checkpoint,
                    InfoRegistry::official(),
                    &BTreeMap::new(),
                    now(),
                    default_info_request_url(),
                    None,
                )
                .expect("live allMids");
        }
        assert_eq!(archive.len(), 1);
        assert_eq!(publisher.publications().len(), 1);
    }
}
