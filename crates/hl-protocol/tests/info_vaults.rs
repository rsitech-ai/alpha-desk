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
    match details.relationship() {
        Some(VaultRelationship::Parent { child_addresses }) => {
            assert_eq!(child_addresses.len(), 1);
        }
        other => panic!("expected parent relationship, got {other:?}"),
    }
}
