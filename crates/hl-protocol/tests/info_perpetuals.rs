use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::info::{
    ArchiveRef, InfoObservationKind, InfoParseContext, InfoRegistry, parse_active_asset_data,
    parse_all_perp_metas, parse_clearinghouse_state, parse_funding_history, parse_meta,
    parse_meta_and_asset_ctxs, parse_non_user_funding_updates, parse_perp_annotation,
    parse_perp_categories, parse_perp_concise_annotations, parse_perp_deploy_auction_status,
    parse_perp_dex_limits, parse_perp_dex_status, parse_perp_dexs,
    parse_perps_at_open_interest_cap, parse_predicted_fundings, parse_sub_accounts,
    parse_user_funding,
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
        blake3::hash(b"t07-perps"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t07-perps").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

#[test]
fn info_builder_deployed_perp_asset_ids() {
    let dexs = parse_perp_dexs(&read_fixture("response-perp-dexs.json"), context())
        .expect("dexs")
        .1;
    assert!(dexs.dexs()[0].is_none());
    let hip3 = dexs.dexs()[1].as_ref().expect("builder dex");
    assert_eq!(hip3.name().as_str(), "xyz");

    let meta = parse_meta(&read_fixture("response-meta.json"), context())
        .expect("meta")
        .1;
    assert_eq!(meta.universe()[0].market_id().as_str(), "perp:BTC");
    assert_eq!(meta.universe()[1].market_id().as_str(), "perp:xyz:XYZ100");
    assert_eq!(meta.collateral_token(), Some(0));
    assert_eq!(meta.universe()[1].sz_decimals(), 4);
}

#[test]
fn info_dex_specific_clearinghouse_state() {
    let state = parse_clearinghouse_state(
        &read_fixture("response-clearinghouse-state.json"),
        context(),
    )
    .expect("ch")
    .1;
    assert_eq!(state.kind(), InfoObservationKind::ReconciledSnapshot);
    assert_eq!(
        state.asset_positions()[0].market_id().as_str(),
        "perp:xyz:XYZ100"
    );
    assert!(state.asset_positions()[0].liquidation_px().is_none());
}

#[test]
fn info_decimals_and_quote_collateral_mapping() {
    let meta = parse_meta(&read_fixture("response-meta.json"), context())
        .expect("meta")
        .1;
    assert_eq!(meta.universe()[0].sz_decimals(), 5);
    assert_eq!(meta.collateral_token(), Some(0));

    let ctxs = parse_meta_and_asset_ctxs(
        &read_fixture("response-meta-and-asset-ctxs.json"),
        context(),
    )
    .expect("ctxs")
    .1;
    assert!(ctxs.ctxs()[0].premium().is_none());
    assert!(ctxs.ctxs()[0].impact_pxs().is_none());
}

#[test]
fn info_subaccounts_reuse_clearinghouse_and_spot_types() {
    let accounts = parse_sub_accounts(&read_fixture("response-sub-accounts.json"), context())
        .expect("subs")
        .1;
    let row = &accounts.accounts()[0];
    assert_eq!(
        row.clearinghouse_state().kind(),
        InfoObservationKind::ReconciledSnapshot
    );
    assert!(row.clearinghouse_state().asset_positions().is_empty());
    assert!(row.spot_state().balances().is_empty());
}

#[test]
fn info_typed_t07_is_reconciliation_not_committed() {
    for id in [
        "official.info.meta",
        "official.info.clearinghouse_state",
        "official.info.outcome_meta",
    ] {
        let endpoint = InfoRegistry::official().get(id).expect("endpoint");
        assert_eq!(endpoint.observation_class(), ObservationClass::Snapshot);
        assert!(!observation_qualifies_committed_source(
            endpoint.observation_class()
        ));
    }
}

#[test]
fn info_typed_t07_rejects_json_floats() {
    let error = parse_clearinghouse_state(&read_fixture("response-json-float.json"), context())
        .expect_err("float");
    assert!(matches!(
        error,
        hl_protocol::info::InfoError::ForbiddenJsonNumber { .. }
    ));
}

#[test]
fn info_remaining_perp_family_parsers() {
    let all = json!([[
        {
            "universe": [{"name": "BTC", "szDecimals": 5, "maxLeverage": 50}],
            "marginTables": [],
            "collateralToken": 0
        },
        [{
            "funding": "0.0",
            "openInterest": "0.0",
            "prevDayPx": "1.0",
            "dayNtlVlm": "0.0",
            "premium": "0.0",
            "oraclePx": "1.0",
            "markPx": "1.0",
            "midPx": "1.0",
            "impactPxs": ["1.0", "1.1"],
            "dayBaseVlm": "0.0"
        }]
    ]]);
    let metas = parse_all_perp_metas(&serde_json::to_vec(&all).expect("json"), context())
        .expect("all")
        .1;
    assert_eq!(metas.dexs().len(), 1);

    let funding = json!([{
        "time": 1681222254710_i64,
        "hash": "0xaa",
        "delta": {
            "type": "funding",
            "coin": "xyz:XYZ100",
            "usdc": "2.37",
            "szi": "-15.0",
            "fundingRate": "0.00000625",
            "nSamples": null
        }
    }]);
    let raw = serde_json::to_vec(&funding).expect("json");
    assert_eq!(
        parse_user_funding(&raw, context())
            .expect("user funding")
            .1
            .updates()[0]
            .market_id()
            .as_str(),
        "perp:xyz:XYZ100"
    );
    assert_eq!(
        parse_non_user_funding_updates(&raw, context())
            .expect("non-user")
            .1
            .kind(),
        InfoObservationKind::BoundedHistory
    );

    let history = json!([{
        "coin": "ETH",
        "fundingRate": "-0.00022196",
        "premium": "-0.00052196",
        "time": 1683849600076_i64
    }]);
    assert_eq!(
        parse_funding_history(&serde_json::to_vec(&history).expect("json"), context())
            .expect("hist")
            .1
            .samples()[0]
            .coin(),
        "ETH"
    );

    let predicted = json!([[
        "AVAX",
        [["HlPerp", {"fundingRate": "0.0000125", "nextFundingTime": 1733958000000_i64}]]
    ]]);
    assert_eq!(
        parse_predicted_fundings(&serde_json::to_vec(&predicted).expect("json"), context())
            .expect("pred")
            .1
            .coins()[0]
            .venues()[0]
            .venue(),
        "HlPerp"
    );

    let cap = parse_perps_at_open_interest_cap(br#"["BADGER","xyz:XYZ100"]"#, context())
        .expect("cap")
        .1;
    assert_eq!(cap.coins()[1].1.as_str(), "perp:xyz:XYZ100");

    let auction = json!({
        "startTimeSeconds": 1747656000,
        "durationSeconds": 111600,
        "startGas": "500.0",
        "currentGas": "500.0",
        "endGas": null
    });
    assert!(
        parse_perp_deploy_auction_status(&serde_json::to_vec(&auction).expect("json"), context())
            .expect("auction")
            .1
            .auction()
            .end_gas()
            .is_none()
    );

    let active = json!({
        "user": "0xb65822a30bbaaa68942d6f4c43d78704faeabbbb",
        "coin": "xyz:XYZ100",
        "leverage": {"type": "isolated", "value": 20, "rawUsd": "0.0"},
        "maxTradeSzs": ["0.0", "0.0"],
        "availableToTrade": ["0.0", "0.0"],
        "markPx": "25451.0"
    });
    assert_eq!(
        parse_active_asset_data(&serde_json::to_vec(&active).expect("json"), context())
            .expect("active")
            .1
            .market_id()
            .as_str(),
        "perp:xyz:XYZ100"
    );

    let limits = json!({
        "totalOiCap": "10000000.0",
        "oiSzCapPerPerp": "10000000000.0",
        "maxTransferNtl": "100000000.0",
        "coinToOiCap": [["COIN1", "100000.0"]]
    });
    assert_eq!(
        parse_perp_dex_limits(&serde_json::to_vec(&limits).expect("json"), context())
            .expect("limits")
            .1
            .coin_to_oi_cap()
            .len(),
        1
    );

    let status = json!({"totalNetDeposit": "4103492112.4478230476"});
    assert_eq!(
        parse_perp_dex_status(&serde_json::to_vec(&status).expect("json"), context())
            .expect("status")
            .1
            .kind(),
        InfoObservationKind::ReferenceSnapshot
    );

    let annotation = json!({"category": "other", "description": "other perps"});
    assert_eq!(
        parse_perp_annotation(&serde_json::to_vec(&annotation).expect("json"), context())
            .expect("ann")
            .1
            .category(),
        "other"
    );

    let categories = json!([["birb:PENGU", "test_cat"]]);
    assert_eq!(
        parse_perp_categories(&serde_json::to_vec(&categories).expect("json"), context())
            .expect("cats")
            .1
            .coins()[0]
            .0
            .as_str(),
        "perp:birb:PENGU"
    );

    let concise = json!([["dex:CATS", {"category": "indices", "keywords": ["meow"]}]]);
    assert_eq!(
        parse_perp_concise_annotations(&serde_json::to_vec(&concise).expect("json"), context())
            .expect("concise")
            .1
            .annotations()[0]
            .keywords()[0],
        "meow"
    );
}
