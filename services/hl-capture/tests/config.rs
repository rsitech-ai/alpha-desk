use std::path::{Path, PathBuf};

use hl_capture::{
    CaptureConfig, ConfigError, DurabilityPolicy, NodeReplicaCmdsStyle, SourceAdapterConfig,
};
use hl_protocol::{ObservationClass, SourceTrust};

fn valid_config() -> String {
    include_str!("../../../config/capture.example.toml").to_owned()
}

fn replace_once(source: &str, from: &str, to: &str) -> String {
    assert!(source.contains(from), "fixture token missing: {from}");
    source.replacen(from, to, 1)
}

fn committed_node_source(id: &str, trust: &str, path: &str) -> String {
    format!(
        r#"

[[sources]]
id = "{id}"
source_version = "hyperliquid-node-v1"
trust = "{trust}"
class = "committed-block"
queue_capacity = 4096
max_payload_bytes = 8388608
adapter = {{ kind = "node-block-directory", path = "{path}", stream_name = "{id}-replica-cmds", start_height = 1, poll_interval_millis = 25, replica_cmds_style = "actions-and-responses" }}
"#
    )
}

fn source_trust_toml(trust: SourceTrust) -> &'static str {
    match trust {
        SourceTrust::LocallyVerifiedCommitted => "locally-verified-committed",
        SourceTrust::IndependentCommitted => "independent-committed",
        SourceTrust::ReconciledSnapshot => "reconciled-snapshot",
        SourceTrust::RecoveryOnly => "recovery-only",
        SourceTrust::ThirdPartyProvisional => "third-party-provisional",
        SourceTrust::MempoolProvisional => "mempool-provisional",
    }
}

fn replica_cmds_style_toml(style: NodeReplicaCmdsStyle) -> &'static str {
    match style {
        NodeReplicaCmdsStyle::Actions => "actions",
        NodeReplicaCmdsStyle::ActionsAndResponses => "actions-and-responses",
        NodeReplicaCmdsStyle::RecentActions => "recent-actions",
    }
}

fn committed_durability_toml(policy: DurabilityPolicy) -> String {
    match policy {
        DurabilityPolicy::FsyncEveryRecord => {
            "[spool.committed_durability]\nmode = \"fsync-every-record\"".to_owned()
        }
        DurabilityPolicy::Batched {
            max_records,
            max_delay_millis,
        } => format!(
            "[spool.committed_durability]\nmode = \"batched\"\nmax_records = {max_records}\nmax_delay_millis = {max_delay_millis}"
        ),
    }
}

fn observation_class_toml(class: ObservationClass) -> &'static str {
    match class {
        ObservationClass::CommittedBlock => "committed-block",
        ObservationClass::AuxiliaryOrderStatus => "auxiliary-order-status",
        ObservationClass::AuxiliaryBookDiff => "auxiliary-book-diff",
        ObservationClass::AuxiliaryLedger => "auxiliary-ledger",
        ObservationClass::Snapshot => "snapshot",
        ObservationClass::HistoricalBlock => "historical-block",
        ObservationClass::PublicMarketData => "public-market-data",
        ObservationClass::ProvisionalFeed => "provisional-feed",
        ObservationClass::ProvisionalMempool => "provisional-mempool",
    }
}

fn extra_source(id: &str, trust: SourceTrust, class: ObservationClass) -> String {
    format!(
        r#"

[[sources]]
id = "{id}"
source_version = "probe-v1"
trust = "{}"
class = "{}"
queue_capacity = 1024
max_payload_bytes = 1048576
"#,
        source_trust_toml(trust),
        observation_class_toml(class)
    )
}

fn independent_committed_source(id: &str) -> String {
    committed_node_source(
        id,
        "independent-committed",
        &format!("/var/lib/hyperliquid-{id}/hl/data/replica_cmds"),
    )
}

fn assert_does_not_occupy_committed_slots(trust: SourceTrust, class: ObservationClass) {
    let probe = extra_source("probe-source", trust, class);
    let parsed =
        CaptureConfig::from_toml(&format!("{}{probe}", valid_config())).unwrap_or_else(|error| {
            panic!("{trust:?}/{class:?} must not occupy the primary slot: {error:?}")
        });
    assert_eq!(
        parsed.source("probe-source").expect("probe source").trust(),
        trust
    );
    assert_eq!(
        parsed
            .source("probe-source")
            .expect("probe source")
            .observation_class(),
        class
    );

    CaptureConfig::from_toml(&format!(
        "{}{probe}{}",
        valid_config(),
        independent_committed_source("independent-node")
    ))
    .unwrap_or_else(|error| {
        panic!("{trust:?}/{class:?} must not occupy the independent slot: {error:?}")
    });
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
        config.runtime().failover_state_path(),
        Path::new("state/committed-source-failover.json")
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
    assert_eq!(
        config
            .runtime()
            .status_listen()
            .map(|addr| addr.to_string()),
        Some("127.0.0.1:8741".to_owned())
    );
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
        Some(SourceAdapterConfig::NodeBlockDirectory {
            replica_cmds_style: NodeReplicaCmdsStyle::ActionsAndResponses,
            ..
        })
    ));
    assert_eq!(
        config
            .payload_limit("public-market")
            .expect("public source limit"),
        1_048_576
    );
}

#[test]
fn topology_accepts_one_primary_and_at_most_one_independent_committed_source() {
    let source = format!(
        "{}{}",
        valid_config(),
        committed_node_source(
            "independent-node",
            "independent-committed",
            "/var/lib/hyperliquid-independent/hl/data/replica_cmds"
        )
    );
    let config = CaptureConfig::from_toml(&source).expect("valid dual committed topology");

    assert_eq!(config.sources().len(), 3);
    assert_eq!(
        config
            .source("independent-node")
            .expect("independent source")
            .trust(),
        SourceTrust::IndependentCommitted
    );
}

#[test]
fn ambiguous_committed_source_topologies_fail_before_opening_files() {
    let no_primary = replace_once(
        &valid_config(),
        "trust = \"locally-verified-committed\"",
        "trust = \"independent-committed\"",
    );
    assert_eq!(
        CaptureConfig::from_toml(&no_primary)
            .expect_err("primary is required")
            .reason_code(),
        "capture_config.missing_primary_committed_source"
    );

    let duplicate_primary = format!(
        "{}{}",
        valid_config(),
        committed_node_source(
            "primary-node-two",
            "locally-verified-committed",
            "/var/lib/hyperliquid-two/hl/data/replica_cmds"
        )
    );
    assert_eq!(
        CaptureConfig::from_toml(&duplicate_primary)
            .expect_err("primary role must be unique")
            .reason_code(),
        "capture_config.duplicate_primary_committed_source"
    );

    let duplicate_independent = format!(
        "{}{}{}",
        valid_config(),
        committed_node_source(
            "independent-node-one",
            "independent-committed",
            "/var/lib/hyperliquid-independent-one/hl/data/replica_cmds"
        ),
        committed_node_source(
            "independent-node-two",
            "independent-committed",
            "/var/lib/hyperliquid-independent-two/hl/data/replica_cmds"
        )
    );
    assert_eq!(
        CaptureConfig::from_toml(&duplicate_independent)
            .expect_err("independent role must be unique")
            .reason_code(),
        "capture_config.duplicate_independent_committed_source"
    );

    let missing_adapter = replace_once(
        &valid_config(),
        "adapter = { kind = \"node-block-directory\", path = \"/var/lib/hyperliquid/hl/data/replica_cmds\", stream_name = \"replica-cmds\", start_height = 1, poll_interval_millis = 25, replica_cmds_style = \"actions-and-responses\" }\n",
        "",
    );
    assert_eq!(
        CaptureConfig::from_toml(&missing_adapter)
            .expect_err("committed source adapter is required")
            .reason_code(),
        "capture_config.invalid_committed_source_adapter"
    );
}

#[test]
fn committed_source_adapter_covers_every_constructible_kind() {
    let admitted = CaptureConfig::from_toml(&valid_config())
        .expect("node-block-directory remains the admitted committed adapter");
    match admitted
        .source("primary-node")
        .expect("primary source")
        .adapter()
    {
        Some(SourceAdapterConfig::NodeBlockDirectory { .. }) => {}
        Some(SourceAdapterConfig::NodeLine { .. }) | None => {
            panic!("example committed adapter must remain node-block-directory")
        }
    }

    let node_line = replace_once(
        &valid_config(),
        "adapter = { kind = \"node-block-directory\", path = \"/var/lib/hyperliquid/hl/data/replica_cmds\", stream_name = \"replica-cmds\", start_height = 1, poll_interval_millis = 25, replica_cmds_style = \"actions-and-responses\" }",
        "adapter = { kind = \"node-line\", path = \"/var/lib/hyperliquid/hl/data/replica_cmds\", stream_name = \"replica-cmds\", stream = \"transaction-blocks\", poll_interval_millis = 25 }",
    );
    assert_eq!(
        CaptureConfig::from_toml(&node_line)
            .expect_err("node-line cannot admit a committed source")
            .reason_code(),
        "capture_config.invalid_committed_source_adapter"
    );

    let missing_adapter = replace_once(
        &valid_config(),
        "adapter = { kind = \"node-block-directory\", path = \"/var/lib/hyperliquid/hl/data/replica_cmds\", stream_name = \"replica-cmds\", start_height = 1, poll_interval_millis = 25, replica_cmds_style = \"actions-and-responses\" }\n",
        "",
    );
    assert_eq!(
        CaptureConfig::from_toml(&missing_adapter)
            .expect_err("committed source adapter is required")
            .reason_code(),
        "capture_config.invalid_committed_source_adapter"
    );
}

#[test]
fn replica_cmds_style_covers_every_constructible_style() {
    for style in [
        NodeReplicaCmdsStyle::Actions,
        NodeReplicaCmdsStyle::ActionsAndResponses,
        NodeReplicaCmdsStyle::RecentActions,
    ] {
        let source = replace_once(
            &valid_config(),
            "replica_cmds_style = \"actions-and-responses\"",
            &format!(
                "replica_cmds_style = \"{}\"",
                replica_cmds_style_toml(style)
            ),
        );
        match style {
            NodeReplicaCmdsStyle::ActionsAndResponses => {
                CaptureConfig::from_toml(&source)
                    .expect("actions-and-responses remains the admitted replica_cmds style");
            }
            NodeReplicaCmdsStyle::Actions | NodeReplicaCmdsStyle::RecentActions => {
                assert_eq!(
                    CaptureConfig::from_toml(&source)
                        .expect_err("non-admitted replica_cmds style")
                        .reason_code(),
                    "capture_config.invalid_source_adapter"
                );
            }
        }
    }
}

#[test]
fn topology_counter_pins_every_source_trust_count_effect() {
    for trust in SourceTrust::ALL {
        match trust {
            SourceTrust::LocallyVerifiedCommitted => {
                let duplicate_primary = format!(
                    "{}{}",
                    valid_config(),
                    committed_node_source(
                        "primary-node-two",
                        "locally-verified-committed",
                        "/var/lib/hyperliquid-two/hl/data/replica_cmds"
                    )
                );
                assert_eq!(
                    CaptureConfig::from_toml(&duplicate_primary)
                        .expect_err("committed-block primary still occupies the primary slot")
                        .reason_code(),
                    "capture_config.duplicate_primary_committed_source"
                );
                assert_does_not_occupy_committed_slots(trust, ObservationClass::AuxiliaryLedger);
            }
            SourceTrust::IndependentCommitted => {
                CaptureConfig::from_toml(&format!(
                    "{}{}",
                    valid_config(),
                    independent_committed_source("independent-node")
                ))
                .expect("one committed-block independent occupies the independent slot once");

                let duplicate_independent = format!(
                    "{}{}{}",
                    valid_config(),
                    independent_committed_source("independent-node-one"),
                    independent_committed_source("independent-node-two")
                );
                assert_eq!(
                    CaptureConfig::from_toml(&duplicate_independent)
                        .expect_err(
                            "committed-block independent still occupies the independent slot"
                        )
                        .reason_code(),
                    "capture_config.duplicate_independent_committed_source"
                );
                assert_does_not_occupy_committed_slots(trust, ObservationClass::AuxiliaryLedger);
            }
            SourceTrust::ReconciledSnapshot => {
                assert_does_not_occupy_committed_slots(trust, ObservationClass::Snapshot);
            }
            SourceTrust::RecoveryOnly => {
                assert_does_not_occupy_committed_slots(trust, ObservationClass::HistoricalBlock);
            }
            SourceTrust::ThirdPartyProvisional => {
                assert_does_not_occupy_committed_slots(trust, ObservationClass::ProvisionalFeed);
            }
            SourceTrust::MempoolProvisional => {
                assert_does_not_occupy_committed_slots(trust, ObservationClass::ProvisionalMempool);
            }
        }
    }
}

#[test]
fn committed_slot_admission_covers_every_constructible_observation_class() {
    for class in ObservationClass::ALL {
        match class {
            ObservationClass::CommittedBlock => {
                let duplicate_primary = format!(
                    "{}{}",
                    valid_config(),
                    committed_node_source(
                        "primary-node-two",
                        "locally-verified-committed",
                        "/var/lib/hyperliquid-two/hl/data/replica_cmds"
                    )
                );
                assert_eq!(
                    CaptureConfig::from_toml(&duplicate_primary)
                        .expect_err("committed-block still occupies the primary slot")
                        .reason_code(),
                    "capture_config.duplicate_primary_committed_source"
                );
                assert_eq!(
                    CaptureConfig::from_toml(&format!(
                        "{}{}",
                        valid_config(),
                        extra_source("probe-source", SourceTrust::LocallyVerifiedCommitted, class)
                    ))
                    .expect_err("committed-block still requires the committed adapter")
                    .reason_code(),
                    "capture_config.invalid_committed_source_adapter"
                );
                CaptureConfig::from_toml(&format!(
                    "{}{}",
                    valid_config(),
                    independent_committed_source("independent-node")
                ))
                .expect("one committed-block independent still occupies the independent slot once");
                assert_eq!(
                    CaptureConfig::from_toml(&format!(
                        "{}{}",
                        valid_config(),
                        extra_source("probe-source", SourceTrust::IndependentCommitted, class)
                    ))
                    .expect_err("committed-block independent still requires the committed adapter")
                    .reason_code(),
                    "capture_config.invalid_committed_source_adapter"
                );
            }
            ObservationClass::AuxiliaryOrderStatus
            | ObservationClass::AuxiliaryBookDiff
            | ObservationClass::AuxiliaryLedger => {
                assert_does_not_occupy_committed_slots(
                    SourceTrust::LocallyVerifiedCommitted,
                    class,
                );
                assert_does_not_occupy_committed_slots(SourceTrust::IndependentCommitted, class);
            }
            ObservationClass::Snapshot
            | ObservationClass::HistoricalBlock
            | ObservationClass::PublicMarketData
            | ObservationClass::ProvisionalFeed
            | ObservationClass::ProvisionalMempool => {
                for trust in [
                    SourceTrust::LocallyVerifiedCommitted,
                    SourceTrust::IndependentCommitted,
                ] {
                    let probe = extra_source("probe-source", trust, class);
                    assert_eq!(
                        CaptureConfig::from_toml(&format!("{}{probe}", valid_config()))
                            .expect_err(
                                "incompatible pairing still fails before the committed-slot count"
                            )
                            .reason_code(),
                        "capture_config.invalid_source_trust",
                        "{trust:?}/{class:?} still fails closed as invalid source trust"
                    );
                }
            }
        }
    }
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
        (
            "status_listen = \"127.0.0.1:8741\"",
            "status_listen = \"8.8.8.8:8741\"",
            "capture_config.invalid_status_listen",
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
            "failover_state_path = \"state/committed-source-failover.json\"",
            "failover_state_path = \"../committed-source-failover.json\"",
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
fn committed_durability_covers_every_constructible_policy() {
    for policy in [
        DurabilityPolicy::FsyncEveryRecord,
        DurabilityPolicy::Batched {
            max_records: 8,
            max_delay_millis: 10,
        },
    ] {
        let source = replace_once(
            &valid_config(),
            "[spool.committed_durability]\nmode = \"fsync-every-record\"",
            &committed_durability_toml(policy),
        );
        match policy {
            DurabilityPolicy::FsyncEveryRecord => {
                CaptureConfig::from_toml(&source)
                    .expect("fsync-every-record remains the admitted committed durability");
            }
            DurabilityPolicy::Batched {
                max_records: _,
                max_delay_millis: _,
            } => {
                assert_eq!(
                    CaptureConfig::from_toml(&source)
                        .expect_err("non-admitted committed durability")
                        .reason_code(),
                    "capture_config.invalid_durability_policy"
                );
            }
        }
    }
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
fn parser_identity_cannot_impersonate_the_reserved_quarantine_disposition() {
    let source = replace_once(
        &valid_config(),
        "parser_version = \"parser-v1\"",
        "parser_version = \"quarantine-v1:source.schema_drift\"",
    );
    let error = CaptureConfig::from_toml(&source).expect_err("reserved namespace must fail");
    assert_eq!(error.reason_code(), "capture_config.invalid_parser_version");
}

#[test]
fn source_ids_are_single_ascii_spool_path_components() {
    for invalid in [
        "/tmp/escaped",
        "../escaped",
        "nested/source",
        "nested\\source",
        ".",
        "..",
        "%2e%2e",
        "node%2fsource",
        "nøde-source",
        ".hidden",
    ] {
        let source = replace_once(
            &valid_config(),
            "id = \"primary-node\"",
            &format!("id = {invalid:?}"),
        );
        let error = CaptureConfig::from_toml(&source).expect_err("unsafe source ID must fail");
        assert_eq!(
            error.reason_code(),
            "capture_config.invalid_source_id",
            "unexpected result for {invalid:?}"
        );
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
    let source = format!(
        r#"{}

[[sources]]
id = "node-fills"
source_version = "hyperliquid-node-v1"
trust = "locally-verified-committed"
class = "auxiliary-ledger"
queue_capacity = 4096
max_payload_bytes = 8388608
adapter = {{ kind = "node-line", path = "/var/lib/hyperliquid/hl/data/node_fills/hourly/20260728/12", stream_name = "node-fills", stream = "fills", poll_interval_millis = 25 }}
"#,
        valid_config()
    );
    let config = CaptureConfig::from_toml(&source).expect("valid node line source");

    assert!(matches!(
        config
            .source("node-fills")
            .expect("node line source")
            .adapter(),
        Some(SourceAdapterConfig::NodeLine {
            stream: hl_protocol::node::v1::NodeStreamKind::Fills,
            ..
        })
    ));
}
