use std::{collections::BTreeMap, path::PathBuf, time::Duration};

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, ManifestId, Price, ProtocolTime, Quantity, SourceId,
    TransactionId,
};
use hl_capture::bus::{
    CANONICAL_STREAM, CanonicalPublisher, CommittedPublicationBatch, JetStreamAuthentication,
    JetStreamConfig, JetStreamConfigError, JetStreamPublisher, PublicationError,
};
use storage_ports::ArchiveReceipt;

fn known(micros: i64) -> KnownTime {
    KnownTime::from_unix_micros(micros).expect("known time")
}

fn canonical_block(height: u64, seed: u64) -> BlockEnvelope {
    let block_time_micros = 1_721_779_200_000_000_i64
        .checked_add(i64::try_from(height).expect("height fits i64"))
        .expect("block time");
    let block_time =
        ProtocolTime::from_unix_micros(block_time_micros).expect("protocol block time");
    let source_id = SourceId::new("primary-node").expect("source ID");
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: ChainId::new("mainnet").expect("chain ID"),
        block_height: BlockHeight::new(height),
        block_time,
        transaction_id: TransactionId::new(format!("jetstream-tx-{height}"))
            .expect("transaction ID"),
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: Vec::new(),
        source_evidence: vec![
            SourceEvidence::try_new(
                source_id.clone(),
                "node-v1",
                format!("block-{height}:0"),
                [u8::try_from(seed).unwrap_or(0x7f); 32],
            )
            .expect("source evidence"),
        ],
        confirmation_class: ConfirmationClass::CommittedPrimary,
        observed_at: known(block_time_micros),
        ingested_at: known(block_time_micros + 1),
        canonicalized_at: known(block_time_micros + 2),
        parser_version: "canonical-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).expect("price"),
            Quantity::parse_at_scale("0.01", 8).expect("quantity"),
            seed,
        )),
    })
    .expect("canonical event");

    BlockEnvelope::try_new(
        ChainId::new("mainnet").expect("chain ID"),
        BlockHeight::new(height),
        block_time,
        ConfirmationClass::CommittedPrimary,
        vec![event],
        BTreeMap::from([(source_id, [0x55; 32])]),
    )
    .expect("canonical block")
}

fn archive_receipt(block: &BlockEnvelope) -> ArchiveReceipt {
    ArchiveReceipt::try_new(
        format!("receipt-{}", block.block_height().get()),
        ManifestId::new(format!(
            "manifest-{}",
            hex::encode(block.canonical_block_hash())
        ))
        .expect("manifest ID"),
        block.block_height(),
        block.canonical_block_hash(),
        [0x11; 32],
        [0x22; 32],
        [0x33; 32],
        known(1_721_779_300_000_000),
    )
    .expect("archive receipt")
}

#[test]
fn jetstream_configuration_rejects_inline_credentials_and_unbounded_limits() {
    let inline = JetStreamConfig::try_new(
        "nats://user:secret@127.0.0.1:4222",
        JetStreamAuthentication::Anonymous,
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
        1024,
    )
    .expect_err("inline credentials");
    assert_eq!(inline, JetStreamConfigError::UnsafeServerUrl);

    let zero_inflight = JetStreamConfig::try_new(
        "nats://127.0.0.1:4222",
        JetStreamAuthentication::Anonymous,
        Duration::from_secs(5),
        Duration::from_secs(5),
        0,
        1024,
    )
    .expect_err("zero in-flight limit");
    assert_eq!(zero_inflight, JetStreamConfigError::InvalidMaxAckInflight);

    let relative_credentials = JetStreamConfig::try_new(
        "nats://127.0.0.1:4222",
        JetStreamAuthentication::CredentialsFile(PathBuf::from("secret.creds")),
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
        1024,
    )
    .expect_err("relative credential path");
    assert_eq!(
        relative_credentials,
        JetStreamConfigError::UnsafeCredentialsPath
    );

    let invalid_username = JetStreamConfig::try_new(
        "nats://127.0.0.1:4222",
        JetStreamAuthentication::UserPasswordFile {
            username: " capture ".to_owned(),
            password_path: PathBuf::from("/run/secrets/nats-password"),
        },
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
        1024,
    )
    .expect_err("whitespace-padded username");
    assert_eq!(invalid_username, JetStreamConfigError::InvalidUsername);
}

#[tokio::test]
async fn real_jetstream_deduplicates_retry_and_rejects_divergent_id() {
    let Ok(server_url) = std::env::var("ALPHA_DESK_NATS_TEST_URL") else {
        eprintln!(
            "SKIP real JetStream test: set ALPHA_DESK_NATS_TEST_URL to select integration lane"
        );
        return;
    };
    let config = JetStreamConfig::try_new(
        server_url,
        match (
            std::env::var("ALPHA_DESK_NATS_TEST_USER"),
            std::env::var("ALPHA_DESK_NATS_TEST_PASSWORD_FILE"),
        ) {
            (Ok(username), Ok(path)) => JetStreamAuthentication::UserPasswordFile {
                username,
                password_path: PathBuf::from(path),
            },
            (Err(_), Err(_)) => JetStreamAuthentication::Anonymous,
            _ => panic!(
                "ALPHA_DESK_NATS_TEST_USER and ALPHA_DESK_NATS_TEST_PASSWORD_FILE must be set together"
            ),
        },
        Duration::from_secs(5),
        Duration::from_secs(5),
        64,
        1024,
    )
    .expect("JetStream config");
    let publisher = JetStreamPublisher::connect(config)
        .await
        .expect("connect JetStream publisher");
    let height = std::env::var("ALPHA_DESK_NATS_TEST_HEIGHT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(9_000_000_001);
    let block = canonical_block(height, 7);
    let batch = CommittedPublicationBatch::try_new(&block, &archive_receipt(&block))
        .expect("publication batch");
    let event = &batch.events()[0];

    let first = publisher.publish(event).await.expect("first publication");
    let retry = publisher.publish(event).await.expect("retry publication");
    assert_eq!(first.stream(), CANONICAL_STREAM);
    assert_eq!(retry.stream(), CANONICAL_STREAM);
    assert_eq!(first.stream_sequence(), retry.stream_sequence());
    assert!(!first.duplicate());
    assert!(retry.duplicate());
    assert_eq!(first.message_id(), event.message_id());

    let conflicting_block = canonical_block(height, 8);
    let conflicting = CommittedPublicationBatch::try_new(
        &conflicting_block,
        &archive_receipt(&conflicting_block),
    )
    .expect("conflicting publication");
    let error = publisher
        .publish(&conflicting.events()[0])
        .await
        .expect_err("same event ID with divergent content");
    assert!(matches!(
        error,
        PublicationError::DivergentMessageId { ref message_id }
            if message_id == event.message_id()
    ));
}
