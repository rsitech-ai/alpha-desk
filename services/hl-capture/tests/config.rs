use std::path::{Path, PathBuf};

use hl_capture::{CaptureConfig, ConfigError, SourceAdapterConfig};
use hl_protocol::{ObservationClass, SourceTrust};

fn valid_config() -> String {
    include_str!("../../../config/capture.example.toml").to_owned()
}

fn replace_once(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture token missing: {from}");
    source.replacen(from, to, 1)
}

#[test]
fn example_configuration_is_strict_valid_and_complete() {
    let example = include_str!("../../../config/capture.example.toml");
    let config = CaptureConfig::from_toml(example).expect("checked-in example must parse");

    assert_eq!(config.parser_version(), "parser-v1");
    assert_eq!(config.spool().path(), Path::new("state/capture-spool"));
    assert_eq!(config.runtime().chain_id().as_str(), "mainnet");
    assert_eq!(config.runtime().first_height().get(), 1);
    assert_eq!(
        config.runtime().archive_path(),
        Path::new("state/canonical-archive")
    );
    assert_eq!(
        config.runtime().status_path(),
        Path::new("state/capture-status.json")
    );
    assert_eq!(
        config.runtime().postgres_url_path(),
        Path::new("/run/secrets/alpha-desk-postgres-url")
    );
    assert_eq!(config.runtime().nats_server_url(), "nats://127.0.0.1:4222");
    assert_eq!(config.runtime().nats_stream(), "HL_CANONICAL");
    assert_eq!(
        config.runtime().nats_password_path(),
        Path::new("/run/secrets/alpha-desk-nats-capture-password")
    );
    assert_eq!(config.runtime().max_pending_blocks(), 4_096);
    assert_eq!(config.runtime().nats_max_ack_inflight(), 4_096);
    assert_eq!(config.runtime().shutdown_grace_millis(), 15_000);
    assert_eq!(config.sources().len(), 2);
    assert_eq!(
        config
            .source("primary-node")
            .expect("primary source")
            .source_version(),
        "hyperliquid-node-v1"
    );
    assert_eq!(
        config
            .source("primary-node")
            .expect("primary source")
            .observation_class(),
        ObservationClass::CommittedBlock
    );
    assert_eq!(
        config
            .source("primary-node")
            .expect("primary source")
            .trust(),
        SourceTrust::LocallyVerifiedCommitted
    );
    assert!(
        config
            .source("primary-node")
            .expect("primary source")
            .admission()
            .expect("validated admission")
            .can_advance_committed_watermark()
    );
    assert!(
        !config
            .source("public-market")
            .expect("public source")
            .admission()
            .expect("validated admission")
            .can_advance_committed_watermark()
    );
    assert!(matches!(
        config
            .source("primary-node")
            .expect("primary source")
            .adapter(),
        Some(SourceAdapterConfig::NodeBlockDirectory { .. })
    ));
    assert_eq!(
        config
            .payload_limit("public-market")
            .expect("public source limit"),
        1_048_576
    );
}

#[test]
fn unknown_keys_fail_startup_at_every_configuration_level() {
    for (from, to) in [
        (
            "parser_version = \"parser-v1\"",
            "parser_version = \"parser-v1\"\nunknown_root = true",
        ),
        (
            "segment_target_bytes = 67108864",
            "segment_target_bytes = 67108864\nunknown_spool = true",
        ),
        (
            "queue_capacity = 4096",
            "queue_capacity = 4096\nunknown_source = true",
        ),
        (
            "mode = \"batched\"",
            "mode = \"batched\"\nunknown_durability = true",
        ),
        (
            "chain_id = \"mainnet\"",
            "chain_id = \"mainnet\"\nunknown_runtime = true",
        ),
    ] {
        let error = CaptureConfig::from_toml(&replace_once(&valid_config(), from, to))
            .expect_err("unknown key must fail");
        assert_eq!(error.reason_code(), "capture_config.invalid_toml");
    }
}

#[test]
fn queue_payload_segment_rotation_and_durability_limits_fail_closed() {
    let cases = [
        (
            "queue_capacity = 4096",
            "queue_capacity = 0",
            "capture_config.invalid_queue_capacity",
        ),
        (
            "max_payload_bytes = 8388608",
            "max_payload_bytes = 0",
            "capture_config.invalid_payload_limit",
        ),
        (
            "segment_target_bytes = 67108864",
            "segment_target_bytes = 1024",
            "capture_config.invalid_segment_target",
        ),
        (
            "segment_target_bytes = 67108864",
            "segment_target_bytes = 536870913",
            "capture_config.invalid_segment_target",
        ),
        (
            "rotation_interval_seconds = 300",
            "rotation_interval_seconds = 0",
            "capture_config.invalid_rotation_interval",
        ),
        (
            "max_records = 128",
            "max_records = 0",
            "capture_config.invalid_durability_policy",
        ),
        (
            "max_delay_millis = 100",
            "max_delay_millis = 0",
            "capture_config.invalid_durability_policy",
        ),
    ];

    for (from, to, reason_code) in cases {
        let error = CaptureConfig::from_toml(&replace_once(&valid_config(), from, to))
            .expect_err("invalid bound must fail");
        assert_eq!(error.reason_code(), reason_code);
    }
}

#[test]
fn runtime_boundaries_reject_inline_credentials_unsafe_paths_and_unbounded_limits() {
    let cases = [
        (
            "nats_server_url = \"nats://127.0.0.1:4222\"",
            "nats_server_url = \"nats://capture:secret@127.0.0.1:4222\"",
            "capture_config.invalid_nats_server",
        ),
        (
            "nats_server_url = \"nats://127.0.0.1:4222\"",
            "nats_server_url = \"nats://127.0.0.1.evil.example:4222\"",
            "capture_config.invalid_nats_server",
        ),
        (
            "archive_path = \"state/canonical-archive\"",
            "archive_path = \"../canonical-archive\"",
            "capture_config.invalid_runtime_path",
        ),
        (
            "postgres_url_path = \"/run/secrets/alpha-desk-postgres-url\"",
            "postgres_url_path = \"postgresql://alpha:secret@localhost/alpha\"",
            "capture_config.invalid_credential_path",
        ),
        (
            "postgres_operation_timeout_millis = 5000",
            "postgres_operation_timeout_millis = 0",
            "capture_config.invalid_runtime_limit",
        ),
        (
            "publish_timeout_millis = 5000",
            "publish_timeout_millis = 0",
            "capture_config.invalid_runtime_limit",
        ),
        (
            "nats_max_ack_inflight = 4096",
            "nats_max_ack_inflight = 0",
            "capture_config.invalid_runtime_limit",
        ),
        (
            "nats_stream = \"HL_CANONICAL\"",
            "nats_stream = \"HL_DATA\"",
            "capture_config.invalid_nats_stream",
        ),
        (
            "disk_reserve_bytes = 10737418240",
            "disk_reserve_bytes = 0",
            "capture_config.invalid_runtime_limit",
        ),
    ];

    for (from, to, reason_code) in cases {
        let error = CaptureConfig::from_toml(&replace_once(&valid_config(), from, to))
            .expect_err("invalid runtime boundary must fail");
        assert_eq!(error.reason_code(), reason_code);
    }
}

#[test]
fn committed_evidence_requires_per_record_durability() {
    let source = replace_once(
        &valid_config(),
        "[spool.committed_durability]\nmode = \"fsync-every-record\"",
        "[spool.committed_durability]\nmode = \"batched\"\nmax_records = 8\nmax_delay_millis = 10",
    );
    let error = CaptureConfig::from_toml(&source).expect_err("committed batching is forbidden");
    assert_eq!(
        error.reason_code(),
        "capture_config.invalid_durability_policy"
    );
}

#[test]
fn parser_source_and_spool_identity_are_canonical() {
    let cases = [
        (
            "parser_version = \"parser-v1\"",
            "parser_version = \" parser-v1\"",
            "capture_config.invalid_parser_version",
        ),
        (
            "id = \"primary-node\"",
            "id = \" primary-node\"",
            "capture_config.invalid_source_id",
        ),
        (
            "source_version = \"hyperliquid-node-v1\"",
            "source_version = \" hyperliquid-node-v1\"",
            "capture_config.invalid_source_version",
        ),
        (
            "path = \"state/capture-spool\"",
            "path = \"../capture-spool\"",
            "capture_config.invalid_spool_path",
        ),
        (
            "credential_path = \"/run/secrets/public-market-token\"",
            "credential_path = \"secrets/token\"",
            "capture_config.invalid_credential_path",
        ),
    ];
    for (from, to, reason_code) in cases {
        let error = CaptureConfig::from_toml(&replace_once(&valid_config(), from, to))
            .expect_err("noncanonical identity must fail");
        assert_eq!(error.reason_code(), reason_code);
    }
}

#[test]
fn duplicate_source_ids_and_unknown_classes_fail() {
    let duplicate = valid_config().replace("id = \"public-market\"", "id = \"primary-node\"");
    let error = CaptureConfig::from_toml(&duplicate).expect_err("duplicate source");
    assert_eq!(error.reason_code(), "capture_config.duplicate_source");

    let invalid_class = replace_once(
        &valid_config(),
        "class = \"committed-block\"",
        "class = \"unknown\"",
    );
    let error = CaptureConfig::from_toml(&invalid_class).expect_err("unknown class");
    assert_eq!(error.reason_code(), "capture_config.invalid_toml");
}

#[test]
fn source_trust_is_required_and_must_match_the_observation_class() {
    let missing = valid_config().replace("trust = \"locally-verified-committed\"\n", "");
    let error = CaptureConfig::from_toml(&missing).expect_err("source trust is required");
    assert_eq!(error.reason_code(), "capture_config.invalid_toml");

    let unknown = replace_once(
        &valid_config(),
        "trust = \"locally-verified-committed\"",
        "trust = \"complete-because-i-said-so\"",
    );
    let error = CaptureConfig::from_toml(&unknown).expect_err("unknown trust");
    assert_eq!(error.reason_code(), "capture_config.invalid_toml");

    let incompatible = replace_once(
        &valid_config(),
        "trust = \"locally-verified-committed\"",
        "trust = \"third-party-provisional\"",
    );
    let error = CaptureConfig::from_toml(&incompatible).expect_err("incompatible trust and class");
    assert_eq!(error.reason_code(), "capture_config.invalid_source_trust");
}

#[test]
fn credentials_are_path_references_and_serialization_cannot_embed_values() {
    let config = CaptureConfig::from_toml(&valid_config()).expect("valid config");
    let serialized = config.to_toml().expect("serialize validated config");

    assert!(serialized.contains("/run/secrets/public-market-token"));
    assert!(!serialized.contains("secret_value"));
    let expected = PathBuf::from("/run/secrets/public-market-token");
    assert_eq!(
        config
            .source("public-market")
            .expect("source")
            .credential_path(),
        Some(expected.as_path())
    );

    let embedded = replace_once(
        &valid_config(),
        "credential_path = \"/run/secrets/public-market-token\"",
        "credential_path = \"/run/secrets/public-market-token\"\nsecret_value = \"do-not-accept\"",
    );
    let error = CaptureConfig::from_toml(&embedded).expect_err("embedded secret field");
    assert_eq!(error.reason_code(), "capture_config.invalid_toml");
}

#[test]
fn error_type_is_stable_and_does_not_echo_configuration_text() {
    let secret = "do-not-leak-this-value";
    let source = format!("{secret} = true");
    let error = CaptureConfig::from_toml(&source).expect_err("invalid config");

    assert!(matches!(error, ConfigError::InvalidToml));
    assert_eq!(error.reason_code(), "capture_config.invalid_toml");
    assert!(!error.to_string().contains(secret));
}

#[test]
fn node_adapter_path_poll_interval_and_class_are_validated() {
    for (from, to) in [
        (
            "path = \"/var/lib/hyperliquid/hl/data/replica_cmds\"",
            "path = \"relative/replica_cmds\"",
        ),
        ("poll_interval_millis = 25", "poll_interval_millis = 0"),
        (
            "class = \"committed-block\"",
            "class = \"auxiliary-ledger\"",
        ),
    ] {
        let error = CaptureConfig::from_toml(&replace_once(&valid_config(), from, to))
            .expect_err("invalid adapter configuration");
        assert_eq!(error.reason_code(), "capture_config.invalid_source_adapter");
    }
}

#[test]
fn unknown_node_adapter_keys_and_streams_fail_strict_deserialization() {
    for (from, to) in [
        (
            "poll_interval_millis = 25",
            "poll_interval_millis = 25, unknown_adapter = true",
        ),
        (
            "kind = \"node-block-directory\"",
            "kind = \"unknown-node-source\"",
        ),
    ] {
        let error = CaptureConfig::from_toml(&replace_once(&valid_config(), from, to))
            .expect_err("unknown adapter field");
        assert_eq!(error.reason_code(), "capture_config.invalid_toml");
    }
}

#[test]
fn node_line_stream_class_must_match_the_configured_output() {
    let source = replace_once(
        &replace_once(
            &valid_config(),
            "class = \"committed-block\"",
            "class = \"auxiliary-ledger\"",
        ),
        "adapter = { kind = \"node-block-directory\", path = \"/var/lib/hyperliquid/hl/data/replica_cmds\", stream_name = \"replica-cmds\", start_height = 1, poll_interval_millis = 25 }",
        "adapter = { kind = \"node-line\", path = \"/var/lib/hyperliquid/hl/data/node_fills/hourly/20260728/12\", stream_name = \"node-fills\", stream = \"fills\", poll_interval_millis = 25 }",
    );
    let config = CaptureConfig::from_toml(&source).expect("valid node line source");

    assert!(matches!(
        config
            .source("primary-node")
            .expect("node line source")
            .adapter(),
        Some(SourceAdapterConfig::NodeLine {
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            ..
        })
    ));
}
