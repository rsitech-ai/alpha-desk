use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoObservationKind, InfoParseContext, parse_aligned_quote_token_info,
    parse_all_borrow_lend_reserve_states, parse_borrow_lend_reserve_state,
    parse_borrow_lend_user_state,
};
use serde_json::json;

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t07-borrow"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t07-borrow").expect("archive ref"),
    )
}

#[test]
fn info_reserve_values_and_optional_fields() {
    let user = parse_borrow_lend_user_state(
        &fs::read(fixture_root().join("response-borrow-lend-user.json")).expect("fixture"),
        context(),
    )
    .expect("user")
    .1;
    assert_eq!(user.kind(), InfoObservationKind::ReconciledSnapshot);
    assert_eq!(user.health(), "healthy");
    assert!(user.health_factor().is_none());
    assert_eq!(user.token_to_state()[0].token(), 0);

    let reserve = json!({
        "borrowYearlyRate": "0.05",
        "supplyYearlyRate": "0.0008245002",
        "balance": "3245939.4732256099",
        "utilization": "0.018322226",
        "oraclePx": "1.0",
        "ltv": "0.0",
        "totalSupplied": "3306509.7335290499",
        "totalBorrowed": "60582.61869494"
    });
    let parsed =
        parse_borrow_lend_reserve_state(&serde_json::to_vec(&reserve).expect("json"), context())
            .expect("reserve")
            .1;
    assert!(parsed.token().is_none());

    let all = json!([[0, {
        "borrowYearlyRate": "0.05",
        "supplyYearlyRate": "0.0008244951",
        "balance": "3245960.0596176102",
        "utilization": "0.0183221137",
        "oraclePx": "1.0",
        "ltv": "0.0",
        "totalSupplied": "3306530.3251102199",
        "totalBorrowed": "60582.62446067"
    }]]);
    assert_eq!(
        parse_all_borrow_lend_reserve_states(&serde_json::to_vec(&all).expect("json"), context())
            .expect("all")
            .1
            .reserves()[0]
            .token(),
        Some(0)
    );
}

#[test]
fn info_aligned_quote_token_info_allows_null() {
    assert!(
        parse_aligned_quote_token_info(b"null", context())
            .expect("null")
            .1
            .is_none()
    );

    let aligned = json!({
        "isAligned": true,
        "firstAlignedTime": 1759914226913_i64,
        "evmMintedSupply": "0.0",
        "dailyAmountOwed": [["2025-10-08", "0.0"]],
        "predictedRate": "0.03154807"
    });
    let parsed =
        parse_aligned_quote_token_info(&serde_json::to_vec(&aligned).expect("json"), context())
            .expect("aligned")
            .1
            .expect("present");
    assert!(parsed.is_aligned());
    assert_eq!(parsed.daily_amount_owed()[0].0, "2025-10-08");
}
