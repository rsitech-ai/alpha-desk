use std::fs;
use std::path::{Path, PathBuf};

use bytes::Bytes;
use hl_protocol::ws::{
    SnapshotRelation, UserEventKind, WsAckMethod, WsObservation, families, parse_ws_message,
    relate_snapshots,
};
use hl_protocol::{ObservationClass, SourceError};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct Manifest {
    schema_version: u32,
    corpus_kind: String,
    production_recording: bool,
    fixture: Vec<Fixture>,
}

#[derive(Debug, Deserialize)]
struct Fixture {
    file: String,
    kind: Option<String>,
    identifier: Option<String>,
    quarantine: Option<String>,
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct CapabilityManifest {
    capability: Vec<Capability>,
}

#[derive(Debug, Deserialize)]
struct Capability {
    id: String,
    transport: String,
    identifier: String,
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-ws")
}

fn capabilities_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("config/hyperliquid/capabilities.toml")
}

fn load_manifest() -> Manifest {
    toml::from_str(&fs::read_to_string(fixture_root().join("manifest.toml")).expect("manifest"))
        .expect("valid manifest")
}

fn parse_file(name: &str) -> Result<WsObservation, SourceError> {
    let payload = fs::read(fixture_root().join(name)).expect("fixture");
    parse_ws_message(Bytes::from(payload))
}

#[test]
fn ws_registry_covers_every_t02_websocket_identifier() {
    let manifest: CapabilityManifest =
        toml::from_str(&fs::read_to_string(capabilities_path()).expect("capabilities"))
            .expect("valid capabilities");
    let mut expected = manifest
        .capability
        .into_iter()
        .filter(|row| row.transport == "websocket")
        .map(|row| (row.identifier, row.id))
        .collect::<Vec<_>>();
    expected.sort();
    let mut registered = families()
        .iter()
        .map(|family| {
            (
                family.identifier.to_owned(),
                family.capability_id.to_owned(),
            )
        })
        .collect::<Vec<_>>();
    registered.sort();
    assert_eq!(registered, expected);
}

#[test]
fn ws_registry_matches_appendix_b_subscription_names() {
    let expected = [
        "allMids",
        "notification",
        "webData3",
        "twapStates",
        "clearinghouseState",
        "openOrders",
        "candle",
        "l2Book",
        "trades",
        "orderUpdates",
        "userEvents",
        "userFills",
        "userFundings",
        "userNonFundingLedgerUpdates",
        "activeAssetCtx",
        "activeAssetData",
        "userTwapSliceFills",
        "userTwapHistory",
        "bbo",
        "spotState",
        "allDexsClearinghouseState",
        "allDexsAssetCtxs",
    ];
    let registered = families()
        .iter()
        .map(|family| family.identifier)
        .collect::<Vec<_>>();
    assert_eq!(registered, expected);
}

#[test]
fn official_ws_corpus_is_hashed_and_classifies_as_documented() {
    let root = fixture_root();
    let manifest = load_manifest();
    assert_eq!(manifest.schema_version, 1);
    assert_eq!(
        manifest.corpus_kind,
        "normalized-official-documentation-examples"
    );
    assert!(!manifest.production_recording);

    let mut parsed_identifiers = std::collections::BTreeSet::new();
    for fixture in &manifest.fixture {
        let payload = fs::read(root.join(&fixture.file)).expect("fixture payload");
        assert_eq!(hex::encode(Sha256::digest(&payload)), fixture.sha256);
        match (fixture.kind.as_deref(), fixture.quarantine.as_deref()) {
            (Some(kind), None) => {
                let parsed = parse_ws_message(Bytes::from(payload.clone())).expect("known record");
                assert_eq!(parsed.payload().as_ref(), payload.as_slice());
                assert_eq!(parsed.content_hash(), blake3::hash(&payload));
                assert_ne!(parsed.observation_class(), ObservationClass::CommittedBlock);
                match (kind, parsed) {
                    ("ack", WsObservation::Ack(ack)) => {
                        assert_eq!(ack.method(), WsAckMethod::Subscribe);
                        assert_eq!(
                            Some(ack.subscription().identifier()),
                            fixture.identifier.as_deref()
                        );
                    }
                    ("snapshot", WsObservation::Snapshot(snapshot)) => {
                        assert_eq!(Some(snapshot.identifier()), fixture.identifier.as_deref());
                        parsed_identifiers.insert(snapshot.identifier());
                    }
                    ("incremental", WsObservation::Incremental(incremental)) => {
                        assert_eq!(
                            Some(incremental.identifier()),
                            fixture.identifier.as_deref()
                        );
                        parsed_identifiers.insert(incremental.identifier());
                    }
                    ("heartbeat", WsObservation::Heartbeat(_)) => {}
                    ("unknown", WsObservation::Unknown(unknown)) => {
                        assert!(!unknown.channel().is_empty());
                    }
                    (other, observation) => {
                        panic!(
                            "fixture {} classified as {observation:?}, expected {other}",
                            fixture.file
                        );
                    }
                }
            }
            (None, Some("schema_drift")) => {
                let error = parse_ws_message(Bytes::from(payload)).expect_err("quarantine");
                assert!(matches!(error, SourceError::SchemaDrift(_)));
                assert_eq!(error.reason_code(), "source.schema_drift");
            }
            other => panic!(
                "fixture {} has invalid kind/quarantine {other:?}",
                fixture.file
            ),
        }
    }

    for family in families() {
        assert!(
            parsed_identifiers.contains(family.identifier),
            "missing parse fixture for {}",
            family.identifier
        );
    }
}

#[test]
fn ws_subscription_ack_binds_stable_identity() {
    let observation = parse_file("ack-all-mids.json").expect("ack");
    let WsObservation::Ack(ack) = observation else {
        panic!("expected ack");
    };
    let subscription = ack.subscription();
    assert_eq!(subscription.identifier(), "allMids");
    let again = parse_file("ack-all-mids.json").expect("ack again");
    let WsObservation::Ack(again) = again else {
        panic!("expected ack");
    };
    assert_eq!(subscription.identity(), again.subscription().identity());
}

#[test]
fn ws_initial_snapshot_is_not_an_incremental_event() {
    let observation = parse_file("user-fills-snapshot.json").expect("snapshot");
    let WsObservation::Snapshot(snapshot) = &observation else {
        panic!("expected snapshot");
    };
    assert!(snapshot.flagged_is_snapshot());
    assert_eq!(snapshot.identifier(), "userFills");
    assert_eq!(observation.observation_class(), ObservationClass::Snapshot);
}

#[test]
fn ws_duplicate_snapshot_is_detected_by_content_hash() {
    let first = parse_file("user-fills-snapshot.json").expect("first");
    let second = parse_file("user-fills-snapshot.json").expect("second");
    assert_eq!(
        relate_snapshots(None, &first),
        Some(SnapshotRelation::Initial)
    );
    assert_eq!(
        relate_snapshots(Some(first.content_hash()), &second),
        Some(SnapshotRelation::Duplicate)
    );
    let replaced = parse_file("all-mids.json").expect("other snapshot");
    assert_eq!(
        relate_snapshots(Some(first.content_hash()), &replaced),
        Some(SnapshotRelation::Replaced)
    );
}

#[test]
fn ws_incremental_update_follows_snapshot_tag() {
    let observation = parse_file("user-fills-incremental.json").expect("incremental");
    let WsObservation::Incremental(incremental) = &observation else {
        panic!("expected incremental");
    };
    assert!(!incremental.flagged_is_snapshot());
    assert_eq!(incremental.identifier(), "userFills");
    assert_eq!(
        observation.observation_class(),
        ObservationClass::ProvisionalFeed
    );
    assert!(relate_snapshots(None, &observation).is_none());
}

#[test]
fn ws_unknown_channel_is_preserved_without_commit() {
    let observation = parse_file("unknown-channel.json").expect("unknown");
    let WsObservation::Unknown(unknown) = &observation else {
        panic!("expected unknown");
    };
    assert_eq!(unknown.channel(), "explorerBlock");
    assert_eq!(
        observation.observation_class(),
        ObservationClass::ProvisionalFeed
    );
}

#[test]
fn ws_undocumented_live_channels_are_unknown_not_registry_rows() {
    let observation = parse_file("outcome-meta-updates.json").expect("unknown live channel");
    assert!(matches!(observation, WsObservation::Unknown(_)));
    assert!(
        families()
            .iter()
            .all(|family| family.identifier != "outcomeMetaUpdates")
    );
    assert!(
        families()
            .iter()
            .all(|family| family.identifier != "fastAssetCtxs")
    );
}

#[test]
fn ws_unknown_user_event_variant_is_quarantined() {
    let error = parse_file("user-events-unknown-variant.json").expect_err("unknown variant");
    assert!(matches!(error, SourceError::SchemaDrift(_)));
}

#[test]
fn ws_liquidation_and_non_user_cancel_parse_as_user_events() {
    let liquidation = parse_file("user-events-liquidation.json").expect("liquidation");
    let WsObservation::Incremental(liquidation) = liquidation else {
        panic!("expected incremental");
    };
    assert_eq!(liquidation.user_event(), Some(UserEventKind::Liquidation));
    assert_eq!(liquidation.channel(), "user");
    assert_eq!(liquidation.identifier(), "userEvents");

    let cancel = parse_file("user-events-non-user-cancel.json").expect("cancel");
    let WsObservation::Incremental(cancel) = cancel else {
        panic!("expected incremental");
    };
    assert_eq!(cancel.user_event(), Some(UserEventKind::NonUserCancel));
}

#[test]
fn ws_parsing_is_deterministic() {
    let payload = fs::read(fixture_root().join("trades.json")).expect("trades");
    let first = parse_ws_message(Bytes::from(payload.clone())).expect("first");
    let second = parse_ws_message(Bytes::from(payload.clone())).expect("second");
    assert_eq!(first, second);
    assert_eq!(first.content_hash(), blake3::hash(&payload));
}

#[test]
fn ws_state_affecting_unknown_channel_is_quarantined() {
    let error =
        parse_file("unknown-channel-state-affecting.json").expect_err("state-affecting unknown");
    assert!(matches!(error, SourceError::SchemaDrift(_)));
}

#[test]
fn ws_unknown_ledger_delta_is_quarantined() {
    let error =
        parse_file("user-non-funding-ledger-unknown-delta.json").expect_err("unknown delta");
    assert!(matches!(error, SourceError::SchemaDrift(_)));
}

#[test]
fn ws_observations_are_never_committed_truth() {
    for fixture in load_manifest().fixture {
        if fixture.quarantine.is_some() {
            continue;
        }
        let observation = parse_file(&fixture.file).expect("parse");
        assert_ne!(
            observation.observation_class(),
            ObservationClass::CommittedBlock
        );
        assert_ne!(
            observation.observation_class(),
            ObservationClass::AuxiliaryLedger
        );
    }
}
