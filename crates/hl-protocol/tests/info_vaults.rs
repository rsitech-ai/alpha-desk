use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoObservationKind, InfoParseContext, VaultRelationship,
    parse_user_vault_equities, parse_vault_details,
};
use serde_json::json;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t07-vaults"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t07-vaults").expect("archive ref"),
    )
}

#[test]
fn info_vault_equities_and_details_parse() {
    let equities = json!([{
        "vaultAddress": "0xdfc24b077bc1425ad1dea75bcb6f8158e10df303",
        "equity": "742500.082809"
    }]);
    let parsed =
        parse_user_vault_equities(&serde_json::to_vec(&equities).expect("json"), context())
            .expect("equities")
            .1;
    assert_eq!(parsed.kind(), InfoObservationKind::ReferenceSnapshot);
    assert_eq!(
        parsed.equities()[0].vault_address().to_api_string(),
        "0xdfc24b077bc1425ad1dea75bcb6f8158e10df303"
    );

    let details = parse_vault_details(
        &fs::read(fixture_root().join("response-vault-details.json")).expect("fixture"),
        context(),
    )
    .expect("details")
    .1;
    assert_eq!(details.kind(), InfoObservationKind::DirectLookup);
    assert_eq!(details.portfolio()[0].period(), "day");
    assert!(details.follower_state().is_none());
    match details.relationship() {
        Some(VaultRelationship::Parent { child_addresses }) => {
            assert_eq!(child_addresses.len(), 1);
        }
        other => panic!("expected parent relationship, got {other:?}"),
    }
}

#[test]
fn info_vault_details_follower_state_is_typed_not_unknown() {
    let mut payload = serde_json::from_slice::<serde_json::Value>(
        &fs::read(fixture_root().join("response-vault-details.json")).expect("fixture"),
    )
    .expect("json");
    payload["followerState"] = json!({
        "user": "0x005844b2ffb2e122cf4244be7dbcb4f84924907c",
        "vaultEquity": "10.0",
        "pnl": "1.0",
        "allTimePnl": "2.0",
        "daysFollowing": 10,
        "vaultEntryTime": 1700926145201_i64,
        "lockupUntil": 1734824439201_i64
    });
    let (parsed, details) =
        parse_vault_details(&serde_json::to_vec(&payload).expect("json"), context())
            .expect("follower");
    let follower = details.follower_state().expect("non-null followerState");
    assert_eq!(
        follower.user().to_api_string(),
        "0x005844b2ffb2e122cf4244be7dbcb4f84924907c"
    );
    assert_eq!(follower.vault_equity().to_string(), "10.0");
    assert!(
        parsed.unknown_fields().is_empty(),
        "{:?}",
        parsed
            .unknown_fields()
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
    );

    payload["followerState"]["U"] = json!("drift");
    let (drifted, details) =
        parse_vault_details(&serde_json::to_vec(&payload).expect("json"), context())
            .expect("still parses");
    assert_eq!(
        details
            .follower_state()
            .expect("still typed")
            .vault_equity()
            .to_string(),
        "10.0"
    );
    assert!(
        !drifted
            .unknown_fields()
            .iter()
            .any(|path| path.as_str() == "/followerState/vaultEquity"),
        "parsed followerState children must not be drift: {:?}",
        drifted
            .unknown_fields()
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
    );
    assert!(
        drifted
            .unknown_fields()
            .iter()
            .any(|path| path.as_str() == "/followerState/U"),
        "extra followerState field must surface in unknown_fields: {:?}",
        drifted
            .unknown_fields()
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
    );
}
