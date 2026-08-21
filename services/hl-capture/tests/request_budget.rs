mod request_budget {
    use hl_capture::{
        BudgetError, CaptureConfig, ConfigError, RequestBudget, SchedulePriority,
        encode_info_budget_status, forbids_exchange_request, parse_request_cost,
        request_cost_for_identifier, spec_12_1_base_weight,
    };
    use hl_protocol::info::InfoRegistry;

    fn example_toml() -> String {
        include_str!("../../../config/capture.example.toml").to_owned()
    }

    fn with_egress(block: &str) -> String {
        format!("{}\n\n{block}", example_toml())
    }

    #[test]
    fn rest_info_request_cost_matches_spec_12_1() {
        for endpoint in InfoRegistry::official().endpoints() {
            let cost = request_cost_for_identifier(endpoint.identifier(), endpoint.request_cost())
                .unwrap_or_else(|error| {
                    panic!(
                        "{} {} {}: {}",
                        endpoint.capability_id(),
                        endpoint.identifier(),
                        endpoint.request_cost(),
                        error.reason_code()
                    )
                });
            assert_eq!(
                cost.base(),
                spec_12_1_base_weight(endpoint.identifier()),
                "{}",
                endpoint.identifier()
            );
            if endpoint.request_cost().contains("variable:window") {
                assert_eq!(cost.estimated_weight(10), cost.base() + 10);
                assert_eq!(cost.actual_weight(3), cost.base() + 3);
            } else {
                assert_eq!(cost.estimated_weight(99), cost.base());
            }
        }
    }

    #[test]
    fn safety_envelope_is_seventy_to_eighty_percent() {
        let mut budget = RequestBudget::official("official-info", 75, 0, 1).expect("budget");
        let snapshot = budget.snapshot(0);
        assert_eq!(snapshot.ceiling_weight_per_minute(), 1_200);
        assert_eq!(snapshot.envelope_weight_per_minute(), 900);
        assert_eq!(snapshot.available_total(), 900);
        assert!(RequestBudget::official("official-info", 69, 0, 1).is_err());
        assert!(RequestBudget::official("official-info", 81, 0, 1).is_err());
        RequestBudget::official("official-info", 70, 0, 1).expect("70");
        RequestBudget::official("official-info", 80, 0, 1).expect("80");
    }

    #[test]
    fn variable_window_reserves_conservatively_and_reconciles() {
        let cost = parse_request_cost("base:20 variable:window").expect("cost");
        let reserved = cost.estimated_weight(100);
        assert_eq!(reserved, 120);
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let before = budget.snapshot(0).available_general();
        let lease = budget
            .reserve(0, "candles", SchedulePriority::P4, reserved)
            .expect("reserve");
        assert_eq!(lease.reserved(), 120);
        budget
            .commit(0, lease, cost.actual_weight(3))
            .expect("commit");
        assert_eq!(budget.snapshot(0).available_general(), before - 23);
    }

    #[test]
    fn independent_egress_ids_have_independent_buckets() {
        let mut left = RequestBudget::try_new("egress-a", 1_000, 80, 40, 0, 1).expect("left");
        let mut right = RequestBudget::try_new("egress-b", 1_000, 80, 40, 0, 1).expect("right");
        let right_before = right.snapshot(0).available_total();
        let _lease = left
            .reserve(0, "job", SchedulePriority::P4, 40)
            .expect("reserve");
        assert_eq!(right.snapshot(0).available_total(), right_before);
        assert!(left.snapshot(0).available_total() < right_before);
    }

    #[test]
    fn http_429_reduces_budget_and_opens_circuit() {
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 7).expect("budget");
        let lease = budget
            .reserve(0, "job-a", SchedulePriority::P2, 20)
            .expect("reserve");
        let until = budget.on_429(0, lease).expect("429");
        assert!(until > 0);
        assert_eq!(budget.snapshot(0).http_429_count(), 1);
        assert_eq!(budget.snapshot(0).available_total(), 0);
        let error = budget
            .reserve(0, "job-b", SchedulePriority::P0, 2)
            .expect_err("circuit");
        assert!(matches!(error, BudgetError::CircuitOpen));
        assert_eq!(error.reason_code(), "capture_info.circuit_open");
        budget
            .reserve(until, "job-b", SchedulePriority::P0, 2)
            .expect("after backoff");
    }

    #[test]
    fn cancellation_returns_the_lease() {
        let mut budget =
            RequestBudget::try_new("official-info", 1_000, 80, 40, 0, 1).expect("budget");
        let before = budget.snapshot(0).available_total();
        let lease = budget
            .reserve(0, "job", SchedulePriority::P1, 30)
            .expect("reserve");
        assert!(budget.snapshot(0).available_total() < before);
        budget.release(0, lease).expect("release");
        assert_eq!(budget.snapshot(0).available_total(), before);
    }

    #[test]
    fn anonymous_proxy_configuration_is_rejected() {
        let source = with_egress(
            r#"[[egress]]
id = "official-info"
kind = "official-info"
base_url = "https://api.hyperliquid.xyz"
weight_per_minute = 1200
safety_envelope_percent = 75
proxy = { mode = "anonymous-rotate", rotate = true, urls = ["http://a.example", "http://b.example"] }
"#,
        );
        let error = CaptureConfig::from_toml(&source).expect_err("proxy");
        assert!(matches!(error, ConfigError::AnonymousProxyRejected));
        assert_eq!(error.reason_code(), "capture_config.anonymous_proxy");
    }

    #[test]
    fn exchange_egress_url_is_rejected() {
        let source = with_egress(
            r#"[[egress]]
id = "official-info"
kind = "official-info"
base_url = "https://api.hyperliquid.xyz/exchange"
weight_per_minute = 1200
safety_envelope_percent = 75
"#,
        );
        let error = CaptureConfig::from_toml(&source).expect_err("exchange");
        assert!(matches!(error, ConfigError::ExchangeEndpointForbidden));
        assert_eq!(error.reason_code(), "capture_config.exchange_forbidden");
    }

    #[test]
    fn official_info_egress_parses_and_keeps_example_without_egress() {
        let source = with_egress(
            r#"[[egress]]
id = "official-info"
kind = "official-info"
base_url = "https://api.hyperliquid.xyz"
weight_per_minute = 1200
safety_envelope_percent = 75
"#,
        );
        let config = CaptureConfig::from_toml(&source).expect("egress");
        assert_eq!(config.egress().len(), 1);
        assert_eq!(config.egress()[0].id(), "official-info");
        assert_eq!(config.egress()[0].safety_envelope_percent(), 75);
        let example = CaptureConfig::from_toml(&example_toml()).expect("example");
        assert!(example.egress().is_empty());
    }

    #[test]
    fn operator_encodes_info_budget_snapshot() {
        let mut budget = RequestBudget::official("official-info", 75, 0, 3).expect("budget");
        let body = encode_info_budget_status(&budget.snapshot(0)).expect("json");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("parse");
        assert_eq!(value["schema_version"], "hl.capture.info-budget.v1");
        assert_eq!(value["egress_id"], "official-info");
        assert_eq!(value["ceiling_weight_per_minute"], 1_200);
        assert_eq!(value["envelope_weight_per_minute"], 900);
    }

    #[test]
    fn fine_grained_refill_matches_one_minute_step() {
        fn drained() -> RequestBudget {
            let mut budget = RequestBudget::official("official-info", 75, 0, 1).expect("budget");
            let general = budget.snapshot(0).available_general();
            let lease = budget
                .reserve(0, "drain", SchedulePriority::P4, general)
                .expect("drain");
            budget.commit(0, lease, general).expect("spent");
            budget
        }
        let mut dripped = drained();
        for step in 1..=600 {
            let _ = dripped.snapshot(step * 100);
        }
        let dripped_general = dripped.snapshot(60_000).available_general();
        let mut stepped = drained();
        let stepped_general = stepped.snapshot(60_000).available_general();
        assert_eq!(stepped_general, 540);
        let delta = i64::from(dripped_general) - i64::from(stepped_general);
        assert!(
            delta.abs() <= 1,
            "drip={dripped_general} step={stepped_general}"
        );
    }

    #[test]
    fn http_429_refills_after_backoff_under_a_ticking_clock() {
        let mut budget = RequestBudget::official("official-info", 75, 0, 7).expect("budget");
        let lease = budget
            .reserve(0, "job-a", SchedulePriority::P4, 20)
            .expect("reserve");
        let until = budget.on_429(0, lease).expect("429");
        assert!(until > 0);
        let mut now = 0_u64;
        while now < until {
            now = now.saturating_add(100);
            let result = budget.reserve(now, "job-b", SchedulePriority::P4, 2);
            if now < until {
                assert!(
                    matches!(result, Err(BudgetError::CircuitOpen)),
                    "now={now} until={until} {result:?}"
                );
            }
        }
        budget
            .reserve(now, "job-b", SchedulePriority::P4, 2)
            .expect("after backoff");
    }

    #[test]
    fn order_type_and_exchange_path_stay_forbidden() {
        assert!(forbids_exchange_request(
            "order",
            br#"{"type":"order"}"#,
            "",
        ));
        assert!(forbids_exchange_request(
            "allMids",
            b"{}",
            "https://api.hyperliquid.xyz/exchange",
        ));
        assert!(!forbids_exchange_request(
            "exchangeStatus",
            br#"{"type":"exchangeStatus"}"#,
            "https://api.hyperliquid.xyz/info",
        ));
    }
}
