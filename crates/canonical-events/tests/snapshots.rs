use canonical_events::{
    CanonicalSnapshotEnvelope, EventKind, SNAPSHOT_ENVELOPE_SCHEMA, SnapshotClass, SnapshotFamily,
    admit_snapshot_as_ledger_transition,
};
use domain_types::{ChainId, KnownTime};

#[test]
fn snapshot_families_are_not_event_kinds_and_cannot_enter_the_ledger() {
    for family in SnapshotFamily::ALL {
        assert!(
            EventKind::try_from(family.as_wire_name()).is_err(),
            "{} must not be an EventKind",
            family.as_wire_name()
        );
    }
    assert!(EventKind::try_from("ReconciledSnapshot").is_err());
    assert!(EventKind::try_from("ReferenceSnapshot").is_err());
    assert!(!EventKind::ALL.iter().any(|kind| {
        kind.as_wire_name().contains("Snapshot") || kind.as_wire_name() == "OpenOrders"
    }));

    let snapshot = CanonicalSnapshotEnvelope::try_new(
        SNAPSHOT_ENVELOPE_SCHEMA,
        SnapshotFamily::MarketContext,
        SnapshotClass::Reference,
        ChainId::new("hyperliquid-mainnet").unwrap(),
        None,
        KnownTime::from_unix_micros(1_700_000_000_000_000).unwrap(),
        "snapshot-parser-v1",
        b"{\"markPx\":\"1\"}".to_vec(),
    )
    .unwrap();
    let error = admit_snapshot_as_ledger_transition(&snapshot).unwrap_err();
    assert!(error.to_string().contains("not ledger transitions"));

    let bytes = snapshot.encode_to_vec();
    let decoded = CanonicalSnapshotEnvelope::decode(&bytes).unwrap();
    assert_eq!(decoded, snapshot);
    assert_eq!(decoded.schema_version(), "1.0.0");
}

#[test]
fn parquet_snapshot_schemas_are_float_free_and_named() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/parquet");
    for name in [
        "reconciled-snapshots-v1.json",
        "reference-snapshots-v1.json",
        "unknown-payloads-v1.json",
        "historical-object-manifest-v1.json",
    ] {
        let text = std::fs::read_to_string(root.join(name)).unwrap_or_else(|_| panic!("{name}"));
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["format"], "hyperliquid-alpha-desk/parquet-schema/v1");
        assert_eq!(value["semantic_version"], "1.0.0");
        assert!(!text.contains("float"));
        assert!(!text.contains("double"));
        for field in value["fields"].as_array().unwrap() {
            let ty = field["type"].as_str().unwrap();
            assert_ne!(ty, "float");
            assert_ne!(ty, "double");
        }
    }
    let manifest =
        std::fs::read_to_string(root.join("historical-object-manifest-v1.json")).unwrap();
    for column in [
        "first_block",
        "last_block",
        "first_event_time_micros",
        "last_event_time_micros",
        "gap_status",
        "requester_pays_billed_bytes",
    ] {
        assert!(manifest.contains(column), "{column}");
    }
}
