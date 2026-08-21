use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, CANDLE_KNOWN_FIELDS, InfoObservationKind, InfoParseContext, InfoRegistry,
    market_id_from_coin, parse_all_mids, parse_candle_snapshot, parse_exchange_status,
    parse_l2_book, parse_recent_trades,
};
use hl_protocol::{ObservationClass, observation_qualifies_committed_source};
use serde_json::json;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t06-general"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t06-general").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[test]
fn info_spot_and_perp_coins_remap_to_canonical_market_ids() {
    assert_eq!(
        market_id_from_coin("BTC").expect("btc").as_str(),
        "perp:BTC"
    );
    assert_eq!(
        market_id_from_coin("xyz:XYZ100").expect("hip3").as_str(),
        "perp:xyz:XYZ100"
    );
    assert_eq!(
        market_id_from_coin("@107").expect("spot index").as_str(),
        "spot:@107"
    );
    assert_eq!(
        market_id_from_coin("PURR/USDC").expect("purr").as_str(),
        "spot:PURR/USDC"
    );
    assert_eq!(
        market_id_from_coin("BTC/USDC")
            .expect("slash is spot protocol coin")
            .as_str(),
        "spot:BTC/USDC"
    );

    let (_parsed, mids) =
        parse_all_mids(&read_fixture("response-all-mids-remap.json"), context()).expect("mids");
    assert_eq!(mids.kind(), InfoObservationKind::ReferenceSnapshot);
    assert_eq!(mids.mids()["BTC"].market_id().as_str(), "perp:BTC");
    assert_eq!(
        mids.mids()["xyz:XYZ100"].market_id().as_str(),
        "perp:xyz:XYZ100"
    );
    assert_eq!(mids.mids()["@107"].market_id().as_str(), "spot:@107");
    assert_eq!(
        mids.mids()["PURR/USDC"].market_id().as_str(),
        "spot:PURR/USDC"
    );
}

#[test]
fn info_candle_snapshot_declares_uppercase_t_and_does_not_use_star() {
    assert!(CANDLE_KNOWN_FIELDS.contains(&"/T"));
    assert!(CANDLE_KNOWN_FIELDS.contains(&"/t"));
    assert!(!CANDLE_KNOWN_FIELDS.contains(&"/*"));

    let (parsed, candles) =
        parse_candle_snapshot(&read_fixture("response-candle-snapshot.json"), context())
            .expect("candles");
    assert_eq!(candles.kind(), InfoObservationKind::BoundedHistory);
    assert_eq!(candles.candles()[0].close_time_millis(), 1_681_924_499_999);
    assert!(parsed.unknown_fields().is_empty());

    let mut extra =
        serde_json::from_slice::<serde_json::Value>(&read_fixture("response-candle-snapshot.json"))
            .expect("json");
    extra[0]["U"] = json!("drift");
    let raw = serde_json::to_vec(&extra).expect("encode");
    let (drifted, _) = parse_candle_snapshot(&raw, context()).expect("still parses");
    assert!(
        drifted
            .unknown_fields()
            .iter()
            .any(|path| path.as_str() == "/0/U"),
        "uppercase extras must surface in unknown_fields, not hide in /*: {:?}",
        drifted
            .unknown_fields()
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn info_l2_book_exchange_status_and_recent_trades_parse() {
    let (_parsed, book) =
        parse_l2_book(&read_fixture("response-l2-book.json"), context()).expect("book");
    assert_eq!(book.market_id().as_str(), "perp:BTC");
    assert_eq!(book.bids().len(), 2);
    assert_eq!(book.asks().len(), 1);

    let (_parsed, status) =
        parse_exchange_status(&read_fixture("response-exchange-status.json"), context())
            .expect("status");
    assert_eq!(status.kind(), InfoObservationKind::ReferenceSnapshot);
    assert_eq!(status.time_millis(), 1_754_450_974_231);

    let (_parsed, trades) =
        parse_recent_trades(&read_fixture("response-recent-trades.json"), context())
            .expect("trades");
    assert_eq!(trades.trades()[0].tid(), 118_906_512_037_719);
    assert_eq!(trades.trades()[0].market_id().unwrap().as_str(), "perp:BTC");
}

#[test]
fn info_typed_general_is_reconciliation_not_committed() {
    let endpoint = InfoRegistry::official()
        .get("official.info.candle_snapshot")
        .expect("endpoint");
    assert_eq!(endpoint.observation_class(), ObservationClass::Snapshot);
    assert!(!observation_qualifies_committed_source(
        endpoint.observation_class()
    ));
}

#[test]
fn info_typed_entry_still_rejects_json_floats() {
    let error =
        parse_l2_book(&read_fixture("response-json-float.json"), context()).expect_err("float");
    assert!(matches!(
        error,
        hl_protocol::info::InfoError::ForbiddenJsonNumber { .. }
    ));
}
