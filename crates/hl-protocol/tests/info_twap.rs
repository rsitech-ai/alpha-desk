use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoObservationKind, InfoParseContext, TwapStatusKind, parse_twap_history,
    parse_user_twap_slice_fills, parse_user_twap_slice_fills_by_time,
};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t06-twap"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t06-twap").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[test]
fn info_twap_slice_fills_and_history_parse() {
    let slices = parse_user_twap_slice_fills(
        &read_fixture("response-user-twap-slice-fills.json"),
        context(),
    )
    .expect("slices")
    .1;
    assert_eq!(slices.kind(), InfoObservationKind::BoundedHistory);
    assert!(!slices.by_time());
    assert_eq!(slices.fills()[0].twap_id(), 3156);
    assert_eq!(slices.fills()[0].fill().tid(), 118_906_512_037_719);

    let by_time = parse_user_twap_slice_fills_by_time(
        &read_fixture("response-user-twap-slice-fills.json"),
        context(),
    )
    .expect("by time")
    .1;
    assert!(by_time.by_time());

    let history = parse_twap_history(&read_fixture("response-twap-history.json"), context())
        .expect("history")
        .1;
    assert_eq!(history.kind(), InfoObservationKind::ReferenceSnapshot);
    assert_eq!(history.records()[0].status(), TwapStatusKind::Finished);
    assert_eq!(
        history.records()[0].state().user().to_api_string(),
        "0x1111111111111111111111111111111111111111"
    );
}
