use api_contracts::FILE_DESCRIPTOR_SET;
use prost::Message;
use prost_types::{
    DescriptorProto, FieldDescriptorProto, FileDescriptorProto, FileDescriptorSet,
    field_descriptor_proto::Type,
};
use std::collections::BTreeSet;

const PAYLOAD_MESSAGES: &[&str] = &[
    "OrderAccepted",
    "OrderRested",
    "OrderModified",
    "OrderPartiallyFilled",
    "OrderFilled",
    "OrderCancelled",
    "OrderRejected",
    "TriggerOrderActivated",
    "TwapStarted",
    "TwapSliceFilled",
    "TwapCompleted",
    "TradeMatched",
    "DepositCredited",
    "WithdrawalDebited",
    "SpotTransfer",
    "PerpTransfer",
    "SubaccountTransfer",
    "VaultDeposit",
    "VaultWithdrawal",
    "FeeCharged",
    "BuilderFeeCharged",
    "FundingPaid",
    "FundingReceived",
    "ReferralReward",
    "AccountModeChanged",
    "MarginModeChanged",
    "LeverageChanged",
    "LiquidationStarted",
    "LiquidationFill",
    "BackstopLiquidation",
    "PositionSettled",
    "MarketHalted",
    "MarketResumed",
    "OpenInterestCapChanged",
    "MarginTableChanged",
    "MarketCreated",
    "MarketMetadataChanged",
    "OracleUpdated",
    "FundingRateUpdated",
    "AssetContextUpdated",
    "DexCreated",
    "OutcomeCreated",
    "OutcomeResolved",
    "NonUserOrderCancelled",
    "InternalTransfer",
    "AccountClassTransfer",
    "VaultCreated",
    "VaultDistribution",
    "VaultLeaderCommissionPaid",
    "RewardClaimed",
    "SpotGenesisApplied",
    "StakingDeposit",
    "StakingDelegated",
    "StakingUndelegated",
    "StakingWithdrawalQueued",
    "StakingWithdrawalCompleted",
    "ValidatorRewardPaid",
];

fn descriptor_set() -> FileDescriptorSet {
    FileDescriptorSet::decode(FILE_DESCRIPTOR_SET)
        .expect("the build-generated descriptor set must decode")
}

fn file<'a>(set: &'a FileDescriptorSet, name: &str) -> &'a FileDescriptorProto {
    set.file
        .iter()
        .find(|file| file.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing descriptor for {name}"))
}

fn message<'a>(file: &'a FileDescriptorProto, name: &str) -> &'a DescriptorProto {
    file.message_type
        .iter()
        .find(|message| message.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing message {name}"))
}

fn field_signature(field: &FieldDescriptorProto) -> (&str, i32) {
    (
        field.name.as_deref().expect("field names are required"),
        field.number.expect("field numbers are required"),
    )
}

#[test]
fn canonical_envelope_keeps_the_exact_v1_field_numbers() {
    let set = descriptor_set();
    let envelope = message(
        file(&set, "canonical/v1/events.proto"),
        "CanonicalEventEnvelope",
    );
    let actual = envelope
        .field
        .iter()
        .map(field_signature)
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            ("schema_version", 1),
            ("chain_id", 2),
            ("block_height", 3),
            ("block_time_micros", 4),
            ("transaction_id", 5),
            ("transaction_index", 6),
            ("event_index", 7),
            ("event_id", 8),
            ("event_kind", 9),
            ("market_ids", 10),
            ("account_ids", 11),
            ("source_evidence", 12),
            ("confirmation_class", 13),
            ("observed_at_micros", 14),
            ("ingested_at_micros", 15),
            ("canonicalized_at_micros", 16),
            ("payload_hash", 17),
            ("parser_version", 18),
            ("payload", 19),
            ("superseded_event_id", 20),
        ]
    );
}

#[test]
fn every_v1_event_family_has_a_distinct_payload_message() {
    let set = descriptor_set();
    let canonical = file(&set, "canonical/v1/events.proto");
    let actual = canonical
        .message_type
        .iter()
        .filter_map(|message| message.name.as_deref())
        .filter(|name| PAYLOAD_MESSAGES.contains(name))
        .collect::<BTreeSet<_>>();
    let expected = PAYLOAD_MESSAGES.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
    assert_eq!(actual.len(), 57);
    for name in PAYLOAD_MESSAGES {
        assert!(
            !message(canonical, name).field.is_empty(),
            "{name} must be a real payload contract"
        );
    }
}

#[test]
fn contract_descriptors_never_use_floating_point_wire_fields() {
    let set = descriptor_set();
    for file in &set.file {
        for message in &file.message_type {
            for field in &message.field {
                assert_ne!(field.r#type(), Type::Float);
                assert_ne!(field.r#type(), Type::Double);
            }
        }
    }
}

#[test]
fn common_and_stream_contracts_are_versioned_and_nonempty() {
    let set = descriptor_set();
    let common = file(&set, "common/v1/types.proto");
    let stream = file(&set, "stream/v1/envelope.proto");
    assert_eq!(common.package.as_deref(), Some("hl.common.v1"));
    assert_eq!(stream.package.as_deref(), Some("hl.stream.v1"));
    assert!(
        common
            .message_type
            .iter()
            .any(|message| !message.field.is_empty()),
        "common/v1/types.proto must define real types"
    );
    assert!(
        stream
            .message_type
            .iter()
            .any(|message| !message.field.is_empty()),
        "stream/v1/envelope.proto must define a real stream contract"
    );
}

#[test]
fn health_contract_is_versioned_and_nonempty() {
    let set = descriptor_set();
    let health = file(&set, "health/v1/health.proto");
    assert_eq!(health.package.as_deref(), Some("hl.health.v1"));
    assert!(
        health
            .message_type
            .iter()
            .any(|message| !message.field.is_empty()),
        "health/v1/health.proto must define a real health contract"
    );
    assert!(
        health
            .enum_type
            .iter()
            .any(|enumeration| !enumeration.value.is_empty()),
        "health/v1/health.proto must define a real health state enum"
    );
}

#[test]
fn snapshot_envelope_keeps_v1_field_numbers_and_is_not_an_event_payload() {
    let set = descriptor_set();
    let snapshots = file(&set, "canonical/v1/snapshots.proto");
    assert_eq!(snapshots.package.as_deref(), Some("hl.canonical.v1"));
    let envelope = message(snapshots, "CanonicalSnapshotEnvelope");
    let actual = envelope
        .field
        .iter()
        .map(field_signature)
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            ("schema_version", 1),
            ("family", 2),
            ("class", 3),
            ("chain_id", 4),
            ("as_of_block", 5),
            ("observed_at_micros", 6),
            ("payload_hash", 7),
            ("parser_version", 8),
            ("payload", 9),
        ]
    );
    assert!(!PAYLOAD_MESSAGES.contains(&"CanonicalSnapshotEnvelope"));
}

#[test]
fn health_assessment_keeps_v1_fields_and_source_health_is_additive() {
    let set = descriptor_set();
    let health = file(&set, "health/v1/health.proto");
    let assessment = message(health, "HealthAssessment");
    let actual = assessment
        .field
        .iter()
        .map(field_signature)
        .collect::<Vec<_>>();
    assert_eq!(
        actual,
        vec![
            ("scope", 1),
            ("state", 2),
            ("reason_code", 3),
            ("observed_at_micros", 4),
            ("suppresses", 5),
        ]
    );
    let source = message(health, "SourceHealth");
    let source_fields = source.field.iter().map(field_signature).collect::<Vec<_>>();
    assert_eq!(
        source_fields,
        vec![
            ("source_id", 1),
            ("state", 2),
            ("reason_code", 3),
            ("observed_at_micros", 4),
            ("suppresses", 5),
            ("suppress_provisional_features", 6),
        ]
    );
}

#[test]
fn generated_artifact_export_is_complete_and_refuses_overwrite() {
    let temporary = tempfile::tempdir().expect("temporary directory must be available");
    let descriptor = temporary.path().join("current.pb");
    let rust_output = temporary.path().join("generated");
    api_contracts::export_contract_artifacts(&descriptor, &rust_output)
        .expect("fresh generated output must be exported");

    let mut names = std::fs::read_dir(&rust_output)
        .expect("generated output must be readable")
        .map(|entry| {
            entry
                .expect("generated entry must be readable")
                .file_name()
                .into_string()
                .expect("generated names must be UTF-8")
        })
        .collect::<Vec<_>>();
    names.sort();
    assert_eq!(
        names,
        [
            "hl.canonical.v1.rs",
            "hl.common.v1.rs",
            "hl.health.v1.rs",
            "hl.stream.v1.rs",
        ]
    );
    assert_eq!(
        std::fs::read(&descriptor).expect("exported descriptor must be readable"),
        FILE_DESCRIPTOR_SET
    );

    let second_descriptor = temporary.path().join("second.pb");
    let error = api_contracts::export_contract_artifacts(&second_descriptor, &rust_output)
        .expect_err("an existing output directory must not be overwritten");
    assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    assert!(!second_descriptor.exists());
}
