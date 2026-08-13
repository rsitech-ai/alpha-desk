use std::path::Path;

use hl_core::{CANONICAL_STREAM, CoreConfig, CoreConfigError};

#[test]
fn example_config_is_valid_and_does_not_claim_qualification() {
    let config = CoreConfig::from_toml(include_str!("../../../config/core.example.toml"))
        .expect("example config");
    assert_eq!(config.chain_id().as_str(), "mainnet");
    assert_eq!(config.first_height().get(), 1);
    assert_eq!(config.jetstream_config().expect("nats").fetch_batch(), 64);
    assert_eq!(CANONICAL_STREAM, "HL_CANONICAL");
}

#[test]
fn missing_store_section_fails_closed() {
    let error = CoreConfig::from_toml(&valid_toml(Path::new("state/core-file-store")).replacen(
        "[store]\npath = \"state/core-file-store\"\n\n",
        "",
        1,
    ))
    .expect_err("missing store");
    assert_eq!(error, CoreConfigError::MissingStore);
    assert_eq!(error.reason_code(), "core_config.missing_store");
}

#[test]
fn missing_nats_section_fails_closed() {
    let error = CoreConfig::from_toml(
        r#"
chain_id = "mainnet"
first_height = 1
shutdown_grace_millis = 15000
idle_poll_millis = 250

[store]
path = "state/core-file-store"
"#,
    )
    .expect_err("missing nats");
    assert_eq!(error, CoreConfigError::MissingNats);
    assert_eq!(error.reason_code(), "core_config.missing_nats");
}

#[test]
fn inline_nats_credentials_fail_closed() {
    let error = CoreConfig::from_toml(
        &valid_toml(Path::new("state/core-file-store"))
            .replace("nats://127.0.0.1:4222", "nats://core:secret@127.0.0.1:4222"),
    )
    .expect_err("inline credentials");
    assert_eq!(error, CoreConfigError::InvalidNatsServer);
    assert_eq!(error.reason_code(), "core_config.invalid_nats_server");
}

#[test]
fn qualification_claims_are_rejected_as_unknown_fields() {
    let error = CoreConfig::from_toml(&format!(
        "{}\nlive_qualified = true\nstage_2_qualified = true\n",
        valid_toml(Path::new("state/core-file-store"))
    ))
    .expect_err("qualification claims");
    assert_eq!(error, CoreConfigError::InvalidToml);
    assert_eq!(error.reason_code(), "core_config.invalid_toml");
}

fn valid_toml(store_path: &Path) -> String {
    format!(
        r#"
chain_id = "mainnet"
first_height = 1
shutdown_grace_millis = 15000
idle_poll_millis = 250

[store]
path = "{path}"

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
