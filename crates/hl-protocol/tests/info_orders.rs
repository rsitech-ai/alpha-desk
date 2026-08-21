use std::fs;
use std::path::{Path, PathBuf};

use domain_types::KnownTime;
use hl_protocol::ErrorDisposition;
use hl_protocol::info::{
    ArchiveRef, InfoError, InfoObservationKind, InfoParseContext, ORDER_STATUS_NAMES,
    OrderLookupStatus, OrderStatus, USER_FILLS_BY_TIME_AVAILABLE_CAP, USER_FILLS_PAGE_LIMIT,
    history_coverage, parse_frontend_open_orders, parse_historical_orders, parse_open_orders,
    parse_order_status, parse_user_fills, parse_user_fills_by_time,
};
use serde_json::{Value, json};

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("fixtures/hyperliquid/official-info")
}

fn context() -> InfoParseContext {
    InfoParseContext::new(
        blake3::hash(b"t06-orders"),
        KnownTime::from_unix_micros(1_721_000_000_000_000).expect("time"),
        ArchiveRef::new("fixture:t06-orders").expect("archive ref"),
    )
}

fn read_fixture(name: &str) -> Vec<u8> {
    fs::read(fixture_root().join(name)).expect("fixture")
}

fn frontend_order() -> Value {
    json!({
        "coin": "ETH",
        "side": "A",
        "limitPx": "2412.7",
        "sz": "0.0",
        "oid": 1,
        "timestamp": 1724361546645_i64,
        "triggerCondition": "N/A",
        "isTrigger": false,
        "triggerPx": "0.0",
        "children": [],
        "isPositionTpsl": false,
        "reduceOnly": true,
        "orderType": "Market",
        "origSz": "0.0076",
        "tif": "FrontendMarket",
        "cloid": null
    })
}

#[test]
fn info_all_documented_order_statuses_parse_including_cancel_and_reject() {
    assert_eq!(ORDER_STATUS_NAMES.len(), 29);
    for name in ORDER_STATUS_NAMES {
        let status = OrderStatus::from_wire("/status", name).expect(name);
        assert_eq!(status.as_str(), *name);
        let raw = serde_json::to_vec(&json!([{
            "order": frontend_order(),
            "status": name,
            "statusTimestamp": 1724361546645_i64
        }]))
        .expect("encode");
        let (_parsed, orders) = parse_historical_orders(&raw, context()).expect(name);
        assert_eq!(orders.orders()[0].status().as_str(), *name);
        if name.ends_with("Canceled") || *name == "canceled" || *name == "scheduledCancel" {
            assert!(status.is_cancel(), "{name}");
        }
        if name.ends_with("Rejected") || *name == "rejected" {
            assert!(status.is_reject(), "{name}");
        }
    }
}

#[test]
fn info_unknown_order_status_quarantines() {
    let raw = serde_json::to_vec(&json!([{
        "order": frontend_order(),
        "status": "brandNewCancelReason",
        "statusTimestamp": 1724361546645_i64
    }]))
    .expect("encode");
    let error = parse_historical_orders(&raw, context()).expect_err("unknown");
    assert_eq!(
        error,
        InfoError::UnknownStateAffectingVariant {
            path: "/0/status".to_owned(),
            value: "brandNewCancelReason".to_owned(),
        }
    );
    assert_eq!(error.disposition(), ErrorDisposition::Quarantine);
}

#[test]
fn info_fill_ids_are_stable_and_partials_do_not_collapse() {
    let (_parsed, fills) =
        parse_user_fills(&read_fixture("response-user-fills.json"), context()).expect("fills");
    assert_eq!(fills.kind(), InfoObservationKind::BoundedHistory);
    assert!(!fills.by_time());
    let same_oid: Vec<_> = fills
        .fills()
        .iter()
        .filter(|fill| fill.oid() == 90_542_681)
        .collect();
    assert_eq!(same_oid.len(), 2);
    assert_ne!(same_oid[0].tid(), same_oid[1].tid());
    assert_eq!(same_oid[0].fill_id(), same_oid[0].tid().to_string());
    assert_eq!(fills.fills()[0].market_id().as_str(), "perp:AVAX");
    assert_eq!(fills.fills()[1].market_id().as_str(), "perp:xyz:XYZ100");
    assert_eq!(fills.fills()[2].market_id().as_str(), "spot:@107");

    let by_time = parse_user_fills_by_time(&read_fixture("response-user-fills.json"), context())
        .expect("by time")
        .1;
    assert!(by_time.by_time());
    assert_eq!(
        by_time.history().available_cap(),
        USER_FILLS_BY_TIME_AVAILABLE_CAP
    );
}

#[test]
fn info_user_history_cap_metadata_is_emitted() {
    let fills = parse_user_fills(&read_fixture("response-user-fills.json"), context())
        .expect("fills")
        .1;
    assert_eq!(fills.history().page_limit(), USER_FILLS_PAGE_LIMIT);
    assert!(!fills.history().coverage().truncated());

    let capped = history_coverage(2000, 2000, 2000, Some(1_681_222_254_710)).expect("cap");
    assert!(capped.coverage().truncated());
    assert_eq!(
        capped.coverage().earliest_reliable_millis(),
        Some(1_681_222_254_710)
    );
    assert_eq!(capped.received(), 2000);

    let mut page = Vec::with_capacity(USER_FILLS_PAGE_LIMIT);
    let sample: Value =
        serde_json::from_slice(&read_fixture("response-user-fills.json")).expect("sample");
    let template = sample[0].clone();
    for index in 0..USER_FILLS_PAGE_LIMIT {
        let mut fill = template.clone();
        fill["tid"] = json!(index as u64);
        fill["time"] = json!(1_681_222_254_710_i64 + index as i64);
        page.push(fill);
    }
    let raw = serde_json::to_vec(&page).expect("page");
    let capped_parse = parse_user_fills(&raw, context()).expect("2000 fills").1;
    assert!(capped_parse.history().coverage().truncated());
    assert_eq!(capped_parse.fills().len(), USER_FILLS_PAGE_LIMIT);
}

#[test]
fn info_open_orders_and_order_status_lookup() {
    let orders = parse_open_orders(&read_fixture("response-open-orders.json"), context())
        .expect("open")
        .1;
    assert_eq!(orders.kind(), InfoObservationKind::ReconciledSnapshot);
    assert_eq!(orders.orders()[0].oid(), 91_490_942);

    let frontend = parse_frontend_open_orders(
        &read_fixture("response-frontend-open-orders.json"),
        context(),
    )
    .expect("frontend")
    .1;
    assert_eq!(frontend.orders()[0].order_type(), "Limit");

    let historical =
        parse_historical_orders(&read_fixture("response-historical-orders.json"), context())
            .expect("history")
            .1;
    assert!(historical.orders()[1].status().is_cancel());
    assert!(historical.orders()[2].status().is_reject());

    let found = parse_order_status(&read_fixture("response-order-status.json"), context())
        .expect("found")
        .1;
    assert_eq!(found.kind(), InfoObservationKind::DirectLookup);
    assert_eq!(found.lookup(), OrderLookupStatus::Order);
    assert_eq!(found.order().expect("order").status(), OrderStatus::Filled);

    let missing = parse_order_status(
        &read_fixture("response-order-status-unknown.json"),
        context(),
    )
    .expect("missing")
    .1;
    assert_eq!(missing.lookup(), OrderLookupStatus::UnknownOid);
    assert!(missing.order().is_none());
}
