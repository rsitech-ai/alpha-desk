mod info_scheduler {
    use std::collections::BTreeMap;

    use domain_types::KnownTime;
    use hl_capture::{
        BudgetError, InfoHttpResponse, InfoJob, InfoScheduler, RequestBudget, SchedulePriority,
        SchedulerError, ScriptedInfoTransport, TimePageCrawlRequest, TimePageStopReason,
        crawl_time_pages, fetch_info, parse_request_cost,
    };
    use hl_protocol::info::{InfoRegistry, TimePageCursor};
    use serde_json::{Value, json};

    fn now() -> KnownTime {
        KnownTime::from_unix_micros(1).expect("time")
    }

    fn page(records: &[(&str, i64)]) -> InfoHttpResponse {
        let items: Vec<Value> = records
            .iter()
            .map(|(id, time)| json!({ "id": id, "time": time }))
            .collect();
        InfoHttpResponse::new(200, serde_json::to_vec(&items).expect("json"))
    }

    fn job(id: &str, priority: SchedulePriority, deadline: u64, risk: u32, cost: u32) -> InfoJob {
        InfoJob::try_new(
            id,
            priority,
            deadline,
            risk,
            0,
            cost,
            "official.info.all_mids",
        )
        .expect("job")
    }

    fn posted_start_time(body: &[u8]) -> i64 {
        let value: Value = serde_json::from_slice(body).expect("posted json");
        value
            .get("startTime")
            .and_then(Value::as_i64)
            .expect("startTime")
    }

    #[test]
    fn p0_is_not_starved_by_backfill() {
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let mut scheduler = InfoScheduler::new();
        scheduler
            .enqueue(job("backfill-a", SchedulePriority::P4, 10, 0, 200))
            .unwrap();
        scheduler
            .enqueue(job("backfill-b", SchedulePriority::P4, 11, 0, 200))
            .unwrap();
        scheduler
            .enqueue(job("backfill-c", SchedulePriority::P4, 12, 0, 200))
            .unwrap();
        let first = scheduler.dispatch(&mut budget, 0).unwrap().expect("p4");
        assert_eq!(first.job().id(), "backfill-a");
        let second = scheduler.dispatch(&mut budget, 0).unwrap().expect("p4");
        assert_eq!(second.job().id(), "backfill-b");
        assert!(scheduler.dispatch(&mut budget, 0).unwrap().is_none());
        scheduler
            .enqueue(job("health", SchedulePriority::P0, 1, 9, 10))
            .unwrap();
        let protected = scheduler.dispatch(&mut budget, 0).unwrap().expect("p0");
        assert_eq!(protected.job().id(), "health");
        assert_eq!(protected.job().priority(), SchedulePriority::P0);
    }

    #[test]
    fn scheduling_is_deterministic_under_a_fixed_clock() {
        fn order(insert: &[&str]) -> Vec<String> {
            let mut budget =
                RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 42).expect("budget");
            let mut scheduler = InfoScheduler::new();
            for id in insert {
                let priority = match *id {
                    "p0" => SchedulePriority::P0,
                    "p2" => SchedulePriority::P2,
                    _ => SchedulePriority::P4,
                };
                scheduler.enqueue(job(id, priority, 5, 1, 10)).unwrap();
            }
            let mut seen = Vec::new();
            while let Some(inflight) = scheduler.dispatch(&mut budget, 0).unwrap() {
                seen.push(inflight.job().id().to_owned());
                scheduler.complete(&mut budget, inflight, 10, 0).unwrap();
            }
            seen
        }
        assert_eq!(order(&["p4", "p2", "p0"]), vec!["p0", "p2", "p4"]);
        assert_eq!(order(&["p0", "p4", "p2"]), vec!["p0", "p2", "p4"]);
    }

    #[test]
    fn shutdown_returns_leases() {
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let before = budget.snapshot(0).available_total();
        let mut scheduler = InfoScheduler::new();
        scheduler
            .enqueue(job("live", SchedulePriority::P1, 1, 0, 25))
            .unwrap();
        let inflight = scheduler.dispatch(&mut budget, 0).unwrap().expect("lease");
        assert!(budget.snapshot(0).available_total() < before);
        scheduler.shutdown(&mut budget, vec![inflight], 0).unwrap();
        assert_eq!(budget.snapshot(0).available_total(), before);
        assert!(scheduler.is_empty());
    }

    #[test]
    fn rate_limit_does_not_retry_in_the_same_window() {
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 9).expect("budget");
        let mut scheduler = InfoScheduler::new();
        scheduler
            .enqueue(job("one", SchedulePriority::P2, 1, 0, 20))
            .unwrap();
        scheduler
            .enqueue(job("two", SchedulePriority::P2, 2, 0, 20))
            .unwrap();
        let first = scheduler.dispatch(&mut budget, 0).unwrap().expect("first");
        scheduler.on_429(&mut budget, first, 0).unwrap();
        let error = scheduler.dispatch(&mut budget, 0).expect_err("storm");
        assert!(matches!(error, SchedulerError::CircuitOpen));
        assert_eq!(error.reason_code(), "capture_info.circuit_open");
    }

    #[test]
    fn official_info_does_not_advance_committed_watermark() {
        let mut transport =
            ScriptedInfoTransport::new([InfoHttpResponse::new(200, b"[]".to_vec())]);
        let fetched = fetch_info(
            &mut transport,
            InfoRegistry::official(),
            "official.info.user_fills_by_time",
            &BTreeMap::new(),
            now(),
            "test-archive",
        )
        .expect("fetch");
        assert!(!fetched.admission().can_advance_committed_watermark());
    }

    #[test]
    fn empty_venue_page_stops() {
        let mut transport = ScriptedInfoTransport::new([page(&[])]);
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let extra = BTreeMap::new();
        let cost = parse_request_cost("base:20 variable:window").expect("cost");
        let crawl = crawl_time_pages(
            &mut transport,
            &mut budget,
            InfoRegistry::official(),
            TimePageCrawlRequest::new(
                "official.info.user_fills_by_time",
                &extra,
                TimePageCursor::new(0, 5).expect("cursor"),
                2,
                "fills",
                SchedulePriority::P3,
                0,
                now(),
                "test-archive",
                cost,
            ),
        )
        .expect("crawl");
        assert_eq!(crawl.stop(), TimePageStopReason::EmptyVenue);
        assert!(crawl.records().is_empty());
    }

    #[test]
    fn exhausted_page_with_records_stops() {
        let mut transport = ScriptedInfoTransport::new([page(&[("a", 100)])]);
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let extra = BTreeMap::new();
        let cost = parse_request_cost("base:20 variable:window").expect("cost");
        let crawl = crawl_time_pages(
            &mut transport,
            &mut budget,
            InfoRegistry::official(),
            TimePageCrawlRequest::new(
                "official.info.user_fills_by_time",
                &extra,
                TimePageCursor::new(0, 5).expect("cursor"),
                2,
                "fills",
                SchedulePriority::P3,
                0,
                now(),
                "test-archive",
                cost,
            ),
        )
        .expect("crawl");
        assert_eq!(crawl.stop(), TimePageStopReason::Exhausted);
        assert_eq!(crawl.records().len(), 1);
        assert_eq!(crawl.records()[0].stable_id(), Some("a"));
    }

    #[test]
    fn overlap_query_never_uses_last_timestamp_plus_one() {
        let mut transport =
            ScriptedInfoTransport::new([page(&[("a", 100), ("b", 200)]), page(&[("c", 250)])]);
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let extra = BTreeMap::new();
        let cost = parse_request_cost("base:20 variable:window").expect("cost");
        let crawl = crawl_time_pages(
            &mut transport,
            &mut budget,
            InfoRegistry::official(),
            TimePageCrawlRequest::new(
                "official.info.user_fills_by_time",
                &extra,
                TimePageCursor::new(0, 5).expect("cursor"),
                2,
                "fills",
                SchedulePriority::P3,
                0,
                now(),
                "test-archive",
                cost,
            ),
        )
        .expect("crawl");
        assert_eq!(crawl.stop(), TimePageStopReason::Exhausted);
        let posted = transport.posted();
        assert_eq!(posted.len(), 2);
        assert_eq!(posted_start_time(&posted[0]), 0);
        assert_eq!(posted_start_time(&posted[1]), 195);
        assert_ne!(posted_start_time(&posted[1]), 201);
    }

    #[test]
    fn no_progress_on_a_short_page_is_end_of_stream() {
        let mut transport =
            ScriptedInfoTransport::new([page(&[("a", 100), ("b", 100)]), page(&[("a", 100)])]);
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let extra = BTreeMap::new();
        let cost = parse_request_cost("base:20 variable:window").expect("cost");
        let crawl = crawl_time_pages(
            &mut transport,
            &mut budget,
            InfoRegistry::official(),
            TimePageCrawlRequest::new(
                "official.info.user_fills_by_time",
                &extra,
                TimePageCursor::new(0, 5).expect("cursor"),
                2,
                "fills",
                SchedulePriority::P3,
                0,
                now(),
                "test-archive",
                cost,
            ),
        )
        .expect("crawl");
        assert_eq!(crawl.stop(), TimePageStopReason::EndOfStream);
        assert_eq!(crawl.records().len(), 2);
        assert!(crawl.coverage().known_gaps().is_empty());
        assert!(!crawl.coverage().truncated());
    }

    #[test]
    fn no_progress_on_a_full_page_records_known_gaps() {
        let mut transport = ScriptedInfoTransport::new([
            page(&[("a", 100), ("b", 100)]),
            page(&[("a", 100), ("b", 100)]),
        ]);
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let extra = BTreeMap::new();
        let cost = parse_request_cost("base:20 variable:window").expect("cost");
        let crawl = crawl_time_pages(
            &mut transport,
            &mut budget,
            InfoRegistry::official(),
            TimePageCrawlRequest::new(
                "official.info.user_fills_by_time",
                &extra,
                TimePageCursor::new(0, 5).expect("cursor"),
                2,
                "fills",
                SchedulePriority::P3,
                0,
                now(),
                "test-archive",
                cost,
            ),
        )
        .expect("crawl");
        assert_eq!(crawl.stop(), TimePageStopReason::SameMillisecondBurst);
        assert_eq!(crawl.records().len(), 2);
        assert!(crawl.coverage().truncated());
        assert_eq!(crawl.coverage().known_gaps().len(), 1);
        assert_eq!(crawl.coverage().known_gaps()[0].start_millis(), 100);
        assert_eq!(crawl.coverage().known_gaps()[0].end_millis(), 100);
    }

    #[test]
    fn cancel_returns_in_flight_lease() {
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let before = budget.snapshot(0).available_total();
        let mut scheduler = InfoScheduler::new();
        scheduler
            .enqueue(job("snap", SchedulePriority::P1, 1, 0, 15))
            .unwrap();
        let inflight = scheduler
            .dispatch(&mut budget, 0)
            .unwrap()
            .expect("inflight");
        scheduler.cancel(&mut budget, inflight, 0).unwrap();
        assert_eq!(budget.snapshot(0).available_total(), before);
    }

    #[test]
    fn unknown_budget_error_still_has_stable_reason() {
        assert_eq!(
            BudgetError::Insufficient.reason_code(),
            "capture_info.insufficient_budget"
        );
    }
}
