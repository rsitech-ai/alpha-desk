use std::process::Command;

use prometheus::Registry;
use serde_json::Value;
use telemetry::{
    BuildProvenance, FoundationMetrics, HealthState, TelemetryConfig, TelemetryError,
    encode_registry, init_telemetry,
};

static INIT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn build() -> BuildProvenance {
    BuildProvenance::try_new(
        "0123456789abcdef0123456789abcdef01234567",
        false,
        "rustc 1.97.1 (stable)",
        "aarch64-apple-darwin",
        Some(1_784_894_400),
        "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789",
        "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef",
    )
    .expect("literal provenance must be valid")
}

#[test]
fn foundation_metrics_are_private_bounded_and_duplicate_safe() {
    let registry = Registry::new();
    let metrics =
        FoundationMetrics::register(&registry, &build(), false).expect("first registration works");
    metrics.observe_health(HealthState::Green);
    metrics.observe_health(HealthState::Red);

    let text = encode_registry(&registry).expect("registry encoding works");
    assert!(text.contains("alpha_desk_health_assessments_total"));
    assert!(text.contains("state=\"green\""));
    assert!(text.contains("state=\"red\""));
    assert!(text.contains("alpha_desk_build_info"));
    assert!(!text.contains("book:"));

    assert!(matches!(
        FoundationMetrics::register(&registry, &build(), false),
        Err(TelemetryError::MetricRegistration)
    ));
}

#[test]
fn telemetry_config_rejects_empty_control_bearing_and_invalid_otlp_endpoints() {
    assert!(matches!(
        TelemetryConfig::try_new("", "0.1.0", None),
        Err(TelemetryError::InvalidConfig("service_name"))
    ));
    assert!(matches!(
        TelemetryConfig::try_new("hl-core", "0.1\n.0", None),
        Err(TelemetryError::InvalidConfig("service_version"))
    ));
    for endpoint in [
        "",
        "collector:4317",
        "ftp://collector.invalid",
        "http://user@collector.invalid:4317",
        "http://collector.invalid:4317/v1/traces",
        "http://collector.invalid:4317?token=secret",
        "http://collector.invalid:4317#fragment",
        "http://collector.invalid:\n4317",
    ] {
        assert!(matches!(
            TelemetryConfig::try_new("hl-core", "0.1.0", Some(endpoint)),
            Err(TelemetryError::InvalidOtlpEndpoint)
        ));
    }
}

#[test]
fn missing_tokio_runtime_rolls_back_initialization_reservation() {
    let _init_test_guard = INIT_TEST_LOCK
        .lock()
        .expect("initialization tests must serialize");
    let config =
        TelemetryConfig::try_new("hl-core", "0.1.0", Some("http://collector.invalid:4317"))
            .expect("literal config must be valid");

    assert!(matches!(
        init_telemetry(&config, &build()),
        Err(TelemetryError::RuntimeUnavailable)
    ));
    assert!(matches!(
        init_telemetry(&config, &build()),
        Err(TelemetryError::RuntimeUnavailable)
    ));
}

#[test]
fn current_thread_runtime_is_rejected_and_reservation_rolls_back() {
    let _init_test_guard = INIT_TEST_LOCK
        .lock()
        .expect("initialization tests must serialize");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("current-thread runtime must build");
    let config =
        TelemetryConfig::try_new("hl-core", "0.1.0", Some("http://collector.invalid:4317"))
            .expect("literal config must be valid");

    runtime.block_on(async {
        assert!(matches!(
            init_telemetry(&config, &build()),
            Err(TelemetryError::UnsupportedRuntime)
        ));
        assert!(matches!(
            init_telemetry(&config, &build()),
            Err(TelemetryError::UnsupportedRuntime)
        ));
    });
}

#[test]
fn subprocess_initializes_once_and_emits_correlated_json() {
    if std::env::var_os("ALPHA_DESK_TELEMETRY_CHILD").is_some() {
        let config = TelemetryConfig::try_new("hl-api", "0.1.0", None)
            .expect("literal config must be valid");
        let guard = init_telemetry(&config, &build()).expect("child initialization must work");
        let request = tracing::info_span!("request", request_id = "request-7");
        tracing::info!(parent: &request, operation = "probe", "captured event");
        assert!(matches!(
            init_telemetry(&config, &build()),
            Err(TelemetryError::AlreadyInitialized)
        ));
        guard.shutdown().expect("provider shutdown must work");
        return;
    }

    let output = Command::new(std::env::current_exe().expect("test executable path must exist"))
        .args([
            "--exact",
            "subprocess_initializes_once_and_emits_correlated_json",
            "--nocapture",
        ])
        .env("ALPHA_DESK_TELEMETRY_CHILD", "1")
        .output()
        .expect("isolated telemetry test must run");

    assert!(
        output.status.success(),
        "child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).expect("captured logs must be UTF-8");
    let event: Value = stderr
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .find(|value: &Value| value["fields"]["operation"] == "probe")
        .expect("actual probe JSON must be captured");

    assert_eq!(event["service"]["name"], "hl-api");
    assert_eq!(event["service"]["version"], "0.1.0");
    assert_eq!(
        event["build"]["git_sha"],
        "0123456789abcdef0123456789abcdef01234567"
    );
    let trace_id = event["trace_id"].as_str().expect("trace_id must be text");
    let span_id = event["span_id"].as_str().expect("span_id must be text");
    assert_eq!(trace_id.len(), 32);
    assert_eq!(span_id.len(), 16);
    assert!(trace_id.bytes().any(|byte| byte != b'0'));
    assert!(span_id.bytes().any(|byte| byte != b'0'));
}
