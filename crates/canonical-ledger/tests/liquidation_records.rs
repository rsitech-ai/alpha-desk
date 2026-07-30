use canonical_ledger::{
    BackstopLiquidationFactRecordV1, CanonicalLiquidationReducerV1, LiquidationCurrentRecordV1,
    LiquidationFillFactRecordV1, LiquidationMarketFlowCurrentRecordV1, LiquidationObservedStatusV1,
    LiquidationSourceValueResolutionV1, LiquidationStartFactRecordV1,
    PositionSettlementFactRecordV1, PositionStateError,
};
use domain_types::{Address, EventId, LiquidationId, MarketId};

const ACCOUNT: &str = "0x1111111111111111111111111111111111111111";
const BACKSTOP: &str = "0x2222222222222222222222222222222222222222";
const LIQUIDATION: &str = "liq-001";
const MARKET: &str = "perp:BTC";
const START_EVENT: &str = "evt-start";
const FILL_EVENT: &str = "evt-fill";
const BACKSTOP_EVENT: &str = "evt-backstop";
const SETTLEMENT_EVENT: &str = "evt-settlement";
const RULE_VERSION: &str = "hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0";
const PAYLOAD_HASH: &str = "1111111111111111111111111111111111111111111111111111111111111111";

fn account() -> Address {
    Address::parse_api(ACCOUNT).unwrap()
}

fn backstop() -> Address {
    Address::parse_api(BACKSTOP).unwrap()
}

fn liquidation() -> LiquidationId {
    LiquidationId::new(LIQUIDATION).unwrap()
}

fn market() -> MarketId {
    MarketId::new(MARKET).unwrap()
}

fn event(value: &str) -> EventId {
    EventId::new(value).unwrap()
}

fn started_current_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/liquidation-current/v1","#,
            r#""liquidation_id":"{LIQUIDATION}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""start_margin_value":"9.00","#,
            r#""start_maintenance_requirement":"10.00","#,
            r#""observed_status":"started","#,
            r#""start_event_id":"{START_EVENT}","#,
            r#""start_block_height":100,"#,
            r#""start_transaction_index":1,"#,
            r#""start_canonical_event_index":2,"#,
            r#""first_backstop_event_id":null,"#,
            r#""first_backstop_block_height":null,"#,
            r#""first_backstop_transaction_index":null,"#,
            r#""first_backstop_canonical_event_index":null,"#,
            r#""last_observation_event_id":"{START_EVENT}","#,
            r#""last_observation_block_height":100,"#,
            r#""last_observation_transaction_index":1,"#,
            r#""last_observation_canonical_event_index":2,"#,
            r#""rule_version":"{RULE_VERSION}"}}"#
        ),
        LIQUIDATION = LIQUIDATION,
        ACCOUNT = ACCOUNT,
        START_EVENT = START_EVENT,
        RULE_VERSION = RULE_VERSION,
    )
    .into_bytes()
}

fn backstop_current_bytes() -> Vec<u8> {
    String::from_utf8(started_current_bytes())
        .unwrap()
        .replace(
            r#""observed_status":"started""#,
            r#""observed_status":"backstop_observed""#,
        )
        .replace(
            r#""first_backstop_event_id":null"#,
            &format!(r#""first_backstop_event_id":"{BACKSTOP_EVENT}""#),
        )
        .replace(
            r#""first_backstop_block_height":null"#,
            r#""first_backstop_block_height":100"#,
        )
        .replace(
            r#""first_backstop_transaction_index":null"#,
            r#""first_backstop_transaction_index":1"#,
        )
        .replace(
            r#""first_backstop_canonical_event_index":null"#,
            r#""first_backstop_canonical_event_index":3"#,
        )
        .replace(
            &format!(r#""last_observation_event_id":"{START_EVENT}""#),
            &format!(r#""last_observation_event_id":"{BACKSTOP_EVENT}""#),
        )
        .replace(
            r#""last_observation_canonical_event_index":2"#,
            r#""last_observation_canonical_event_index":3"#,
        )
        .into_bytes()
}

fn start_fact_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/liquidation-start-fact/v1","#,
            r#""liquidation_id":"{LIQUIDATION}","#,
            r#""event_id":"{START_EVENT}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""margin_value":"9.00","#,
            r#""maintenance_requirement":"10.00","#,
            r#""block_height":100,"#,
            r#""transaction_index":1,"#,
            r#""canonical_event_index":2,"#,
            r#""payload_blake3":"{PAYLOAD_HASH}","#,
            r#""rule_version":"{RULE_VERSION}"}}"#
        ),
        LIQUIDATION = LIQUIDATION,
        START_EVENT = START_EVENT,
        ACCOUNT = ACCOUNT,
        PAYLOAD_HASH = PAYLOAD_HASH,
        RULE_VERSION = RULE_VERSION,
    )
    .into_bytes()
}

fn fill_fact_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/liquidation-fill-fact/v1","#,
            r#""liquidation_id":"{LIQUIDATION}","#,
            r#""event_id":"{FILL_EVENT}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""market_id":"{MARKET}","#,
            r#""price":"100.50","#,
            r#""quantity":"0.250","#,
            r#""block_height":100,"#,
            r#""transaction_index":1,"#,
            r#""canonical_event_index":3,"#,
            r#""payload_blake3":"{PAYLOAD_HASH}","#,
            r#""rule_version":"{RULE_VERSION}"}}"#
        ),
        LIQUIDATION = LIQUIDATION,
        FILL_EVENT = FILL_EVENT,
        ACCOUNT = ACCOUNT,
        MARKET = MARKET,
        PAYLOAD_HASH = PAYLOAD_HASH,
        RULE_VERSION = RULE_VERSION,
    )
    .into_bytes()
}

fn flow_current_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/liquidation-market-flow-current/v1","#,
            r#""liquidation_id":"{LIQUIDATION}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""market_id":"{MARKET}","#,
            r#""observed_filled_quantity":"0.250","#,
            r#""first_fill_event_id":"{FILL_EVENT}","#,
            r#""first_fill_block_height":100,"#,
            r#""first_fill_transaction_index":1,"#,
            r#""first_fill_canonical_event_index":3,"#,
            r#""last_fill_event_id":"{FILL_EVENT}","#,
            r#""last_fill_block_height":100,"#,
            r#""last_fill_transaction_index":1,"#,
            r#""last_fill_canonical_event_index":3,"#,
            r#""rule_version":"{RULE_VERSION}"}}"#
        ),
        LIQUIDATION = LIQUIDATION,
        ACCOUNT = ACCOUNT,
        MARKET = MARKET,
        FILL_EVENT = FILL_EVENT,
        RULE_VERSION = RULE_VERSION,
    )
    .into_bytes()
}

fn backstop_fact_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/backstop-liquidation-fact/v1","#,
            r#""liquidation_id":"{LIQUIDATION}","#,
            r#""event_id":"{BACKSTOP_EVENT}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""backstop_account_id":"{BACKSTOP}","#,
            r#""market_id":"{MARKET}","#,
            r#""quantity":"0.125","#,
            r#""transfer_price_resolution":"unavailable_from_source","#,
            r#""entry_price_resolution":"unavailable_from_source","#,
            r#""block_height":100,"#,
            r#""transaction_index":1,"#,
            r#""canonical_event_index":4,"#,
            r#""payload_blake3":"{PAYLOAD_HASH}","#,
            r#""rule_version":"{RULE_VERSION}"}}"#
        ),
        LIQUIDATION = LIQUIDATION,
        BACKSTOP_EVENT = BACKSTOP_EVENT,
        ACCOUNT = ACCOUNT,
        BACKSTOP = BACKSTOP,
        MARKET = MARKET,
        PAYLOAD_HASH = PAYLOAD_HASH,
        RULE_VERSION = RULE_VERSION,
    )
    .into_bytes()
}

fn settlement_fact_bytes() -> Vec<u8> {
    format!(
        concat!(
            r#"{{"schema":"hyperliquid-alpha-desk/position-settlement-fact/v1","#,
            r#""event_id":"{SETTLEMENT_EVENT}","#,
            r#""account_id":"{ACCOUNT}","#,
            r#""market_id":"{MARKET}","#,
            r#""settlement_price":"0.00","#,
            r#""settled_quantity":"1.000","#,
            r#""realized_pnl":"-12.50","#,
            r#""block_height":101,"#,
            r#""transaction_index":0,"#,
            r#""canonical_event_index":0,"#,
            r#""payload_blake3":"{PAYLOAD_HASH}","#,
            r#""rule_version":"{RULE_VERSION}"}}"#
        ),
        SETTLEMENT_EVENT = SETTLEMENT_EVENT,
        ACCOUNT = ACCOUNT,
        MARKET = MARKET,
        PAYLOAD_HASH = PAYLOAD_HASH,
        RULE_VERSION = RULE_VERSION,
    )
    .into_bytes()
}

fn replace(bytes: &[u8], from: &str, to: &str) -> Vec<u8> {
    String::from_utf8(bytes.to_vec())
        .unwrap()
        .replace(from, to)
        .into_bytes()
}

fn assert_value_bound(
    base: &[u8],
    identity: &str,
    decode: impl Fn(&[u8]) -> Result<(), PositionStateError>,
) {
    let exact_identity_len = 16 * 1024 - (base.len() - identity.len());
    let exact = replace(base, identity, &"v".repeat(exact_identity_len));
    assert_eq!(exact.len(), 16 * 1024);
    decode(&exact).unwrap();
    let oversized = replace(base, identity, &"v".repeat(exact_identity_len + 1));
    assert_eq!(decode(&oversized), Err(PositionStateError::LimitExceeded));
}

#[test]
fn frozen_record_family_decodes_with_exact_versions_status_and_source_absence() {
    assert_eq!(CanonicalLiquidationReducerV1::VERSION, RULE_VERSION);

    let started = LiquidationCurrentRecordV1::decode(&started_current_bytes()).unwrap();
    assert_eq!(
        started.observed_status(),
        LiquidationObservedStatusV1::Started
    );
    assert_eq!(started.start_margin_value().to_string(), "9.00");
    assert_eq!(started.start_maintenance_requirement().to_string(), "10.00");

    let backstop_current = LiquidationCurrentRecordV1::decode(&backstop_current_bytes()).unwrap();
    assert_eq!(
        backstop_current.observed_status(),
        LiquidationObservedStatusV1::BackstopObserved
    );
    assert_eq!(
        backstop_current.first_backstop_event_id().unwrap(),
        &event(BACKSTOP_EVENT)
    );

    let start = LiquidationStartFactRecordV1::decode(&start_fact_bytes()).unwrap();
    assert_eq!(start.payload_blake3(), &[0x11; 32]);
    let fill = LiquidationFillFactRecordV1::decode(&fill_fact_bytes()).unwrap();
    assert_eq!(fill.price().to_string(), "100.50");
    assert_eq!(fill.quantity().to_string(), "0.250");
    let flow = LiquidationMarketFlowCurrentRecordV1::decode(&flow_current_bytes()).unwrap();
    assert_eq!(flow.observed_filled_quantity().to_string(), "0.250");
    let backstop_fact = BackstopLiquidationFactRecordV1::decode(&backstop_fact_bytes()).unwrap();
    assert_eq!(
        backstop_fact.transfer_price_resolution(),
        LiquidationSourceValueResolutionV1::UnavailableFromSource
    );
    assert_eq!(
        backstop_fact.entry_price_resolution(),
        LiquidationSourceValueResolutionV1::UnavailableFromSource
    );
    let settlement = PositionSettlementFactRecordV1::decode(&settlement_fact_bytes()).unwrap();
    assert_eq!(settlement.settlement_price().to_string(), "0.00");
    assert_eq!(settlement.realized_pnl().to_string(), "-12.50");
}

#[test]
fn keys_freeze_text_frames_raw_accounts_and_per_market_separation() {
    let current_key = LiquidationCurrentRecordV1::state_key(&liquidation()).unwrap();
    assert_eq!(
        current_key.key(),
        b"\x00\x00\x00\x00\x00\x00\x00\x07liq-001"
    );
    let start_key =
        LiquidationStartFactRecordV1::state_key(&liquidation(), &event(START_EVENT)).unwrap();
    assert_eq!(
        start_key.key(),
        b"\x00\x00\x00\x00\x00\x00\x00\x07liq-001\
          \x00\x00\x00\x00\x00\x00\x00\x09evt-start"
    );
    let flow_key =
        LiquidationMarketFlowCurrentRecordV1::state_key(&liquidation(), &account(), &market())
            .unwrap();
    let account_start = 8 + LIQUIDATION.len() + 8;
    assert_eq!(
        &flow_key.key()[account_start..account_start + 20],
        account().as_bytes()
    );
    let eth = MarketId::new("perp:ETH").unwrap();
    assert_ne!(
        flow_key,
        LiquidationMarketFlowCurrentRecordV1::state_key(&liquidation(), &account(), &eth).unwrap()
    );
    let settlement_key =
        PositionSettlementFactRecordV1::state_key(&event(SETTLEMENT_EVENT), &account(), &market())
            .unwrap();
    assert_eq!(
        &settlement_key.key()[8 + SETTLEMENT_EVENT.len() + 8..][..20],
        account().as_bytes()
    );
}

#[test]
fn every_record_is_key_bound_and_wrong_identity_is_rejected() {
    let other_liquidation = LiquidationId::new("liq-other").unwrap();
    let other_event = event("evt-other");
    let other_account = backstop();
    let other_market = MarketId::new("perp:ETH").unwrap();

    assert_eq!(
        LiquidationCurrentRecordV1::decode_at(
            &LiquidationCurrentRecordV1::state_key(&other_liquidation).unwrap(),
            &started_current_bytes(),
        ),
        Err(PositionStateError::KeyMismatch)
    );
    assert_eq!(
        LiquidationStartFactRecordV1::decode_at(
            &LiquidationStartFactRecordV1::state_key(&liquidation(), &other_event).unwrap(),
            &start_fact_bytes(),
        ),
        Err(PositionStateError::KeyMismatch)
    );
    assert_eq!(
        LiquidationFillFactRecordV1::decode_at(
            &LiquidationFillFactRecordV1::state_key(&liquidation(), &other_event).unwrap(),
            &fill_fact_bytes(),
        ),
        Err(PositionStateError::KeyMismatch)
    );
    assert_eq!(
        LiquidationMarketFlowCurrentRecordV1::decode_at(
            &LiquidationMarketFlowCurrentRecordV1::state_key(
                &liquidation(),
                &other_account,
                &other_market,
            )
            .unwrap(),
            &flow_current_bytes(),
        ),
        Err(PositionStateError::KeyMismatch)
    );
    assert_eq!(
        BackstopLiquidationFactRecordV1::decode_at(
            &BackstopLiquidationFactRecordV1::state_key(&liquidation(), &other_event).unwrap(),
            &backstop_fact_bytes(),
        ),
        Err(PositionStateError::KeyMismatch)
    );
    assert_eq!(
        PositionSettlementFactRecordV1::decode_at(
            &PositionSettlementFactRecordV1::state_key(
                &other_event,
                &other_account,
                &other_market,
            )
            .unwrap(),
            &settlement_fact_bytes(),
        ),
        Err(PositionStateError::KeyMismatch)
    );
}

#[test]
fn current_and_flow_provenance_require_event_tuple_equivalence_and_strict_order() {
    let equal_position_different_id = replace(
        &started_current_bytes(),
        &format!(r#""last_observation_event_id":"{START_EVENT}""#),
        r#""last_observation_event_id":"evt-other""#,
    );
    assert_eq!(
        LiquidationCurrentRecordV1::decode(&equal_position_different_id),
        Err(PositionStateError::InvalidRecord)
    );
    let same_id_different_position = replace(
        &started_current_bytes(),
        r#""last_observation_canonical_event_index":2"#,
        r#""last_observation_canonical_event_index":3"#,
    );
    assert_eq!(
        LiquidationCurrentRecordV1::decode(&same_id_different_position),
        Err(PositionStateError::InvalidRecord)
    );
    let same_tuple_backstop = replace(
        &backstop_current_bytes(),
        r#""first_backstop_canonical_event_index":3"#,
        r#""first_backstop_canonical_event_index":2"#,
    );
    assert_eq!(
        LiquidationCurrentRecordV1::decode(&same_tuple_backstop),
        Err(PositionStateError::InvalidRecord)
    );
    LiquidationCurrentRecordV1::decode(&backstop_current_bytes()).unwrap();

    let equal_flow_position_different_id = replace(
        &flow_current_bytes(),
        &format!(r#""last_fill_event_id":"{FILL_EVENT}""#),
        r#""last_fill_event_id":"evt-fill-2""#,
    );
    assert_eq!(
        LiquidationMarketFlowCurrentRecordV1::decode(&equal_flow_position_different_id),
        Err(PositionStateError::InvalidRecord)
    );
    let later_same_block = replace(
        &equal_flow_position_different_id,
        r#""last_fill_canonical_event_index":3"#,
        r#""last_fill_canonical_event_index":4"#,
    );
    LiquidationMarketFlowCurrentRecordV1::decode(&later_same_block).unwrap();
}

#[test]
fn financial_and_status_matrices_fail_closed_without_invented_values() {
    let started_after_fill = replace(
        &replace(
            &started_current_bytes(),
            &format!(r#""last_observation_event_id":"{START_EVENT}""#),
            r#""last_observation_event_id":"evt-fill-later""#,
        ),
        r#""last_observation_canonical_event_index":2"#,
        r#""last_observation_canonical_event_index":3"#,
    );
    LiquidationCurrentRecordV1::decode(&started_after_fill).unwrap();
    let backstop_after_fill = replace(
        &replace(
            &backstop_current_bytes(),
            &format!(r#""last_observation_event_id":"{BACKSTOP_EVENT}""#),
            r#""last_observation_event_id":"evt-fill-after-backstop""#,
        ),
        r#""last_observation_canonical_event_index":3"#,
        r#""last_observation_canonical_event_index":4"#,
    );
    LiquidationCurrentRecordV1::decode(&backstop_after_fill).unwrap();
    let repeated_backstop = replace(
        &replace(
            &backstop_current_bytes(),
            &format!(r#""last_observation_event_id":"{BACKSTOP_EVENT}""#),
            r#""last_observation_event_id":"evt-backstop-2""#,
        ),
        r#""last_observation_canonical_event_index":3"#,
        r#""last_observation_canonical_event_index":4"#,
    );
    LiquidationCurrentRecordV1::decode(&repeated_backstop).unwrap();

    for invalid in [
        replace(
            &started_current_bytes(),
            r#""start_margin_value":"9.00""#,
            r#""start_margin_value":"-1.00""#,
        ),
        replace(
            &started_current_bytes(),
            r#""start_maintenance_requirement":"10.00""#,
            r#""start_maintenance_requirement":"9.000""#,
        ),
        replace(
            &started_current_bytes(),
            r#""start_margin_value":"9.00""#,
            r#""start_margin_value":"10.00""#,
        ),
        replace(
            &started_current_bytes(),
            r#""observed_status":"started""#,
            r#""observed_status":"completed""#,
        ),
        replace(
            &started_current_bytes(),
            r#""first_backstop_event_id":null"#,
            r#""first_backstop_event_id":"evt-backstop""#,
        ),
    ] {
        assert_eq!(
            LiquidationCurrentRecordV1::decode(&invalid),
            Err(PositionStateError::InvalidRecord)
        );
    }

    for invalid in [
        replace(
            &start_fact_bytes(),
            r#""margin_value":"9.00""#,
            r#""margin_value":"-1.00""#,
        ),
        replace(
            &start_fact_bytes(),
            r#""maintenance_requirement":"10.00""#,
            r#""maintenance_requirement":"10.000""#,
        ),
        replace(
            &start_fact_bytes(),
            r#""margin_value":"9.00""#,
            r#""margin_value":"10.00""#,
        ),
    ] {
        assert_eq!(
            LiquidationStartFactRecordV1::decode(&invalid),
            Err(PositionStateError::InvalidRecord)
        );
    }

    for invalid in [
        replace(
            &fill_fact_bytes(),
            r#""price":"100.50""#,
            r#""price":"0.00""#,
        ),
        replace(
            &fill_fact_bytes(),
            r#""quantity":"0.250""#,
            r#""quantity":"0.000""#,
        ),
    ] {
        assert_eq!(
            LiquidationFillFactRecordV1::decode(&invalid),
            Err(PositionStateError::InvalidRecord)
        );
    }
    assert_eq!(
        LiquidationMarketFlowCurrentRecordV1::decode(&replace(
            &flow_current_bytes(),
            r#""observed_filled_quantity":"0.250""#,
            r#""observed_filled_quantity":"0.000""#,
        )),
        Err(PositionStateError::InvalidRecord)
    );
    assert_eq!(
        LiquidationMarketFlowCurrentRecordV1::decode(&replace(
            &flow_current_bytes(),
            r#""observed_filled_quantity":"0.250""#,
            r#""observed_filled_quantity":"-0.250""#,
        )),
        Err(PositionStateError::InvalidRecord)
    );
    assert_eq!(
        BackstopLiquidationFactRecordV1::decode(&replace(
            &backstop_fact_bytes(),
            &format!(r#""backstop_account_id":"{BACKSTOP}""#),
            &format!(r#""backstop_account_id":"{ACCOUNT}""#),
        )),
        Err(PositionStateError::InvalidRecord)
    );
    for invalid_quantity in ["0.000", "-0.125"] {
        assert_eq!(
            BackstopLiquidationFactRecordV1::decode(&replace(
                &backstop_fact_bytes(),
                r#""quantity":"0.125""#,
                &format!(r#""quantity":"{invalid_quantity}""#),
            )),
            Err(PositionStateError::InvalidRecord)
        );
    }
    assert_eq!(
        BackstopLiquidationFactRecordV1::decode(&replace(
            &backstop_fact_bytes(),
            r#""transfer_price_resolution":"unavailable_from_source""#,
            r#""transfer_price_resolution":"known""#,
        )),
        Err(PositionStateError::InvalidRecord)
    );
    assert_eq!(
        BackstopLiquidationFactRecordV1::decode(&replace(
            &backstop_fact_bytes(),
            r#""entry_price_resolution":"unavailable_from_source""#,
            r#""entry_price_resolution":"known""#,
        )),
        Err(PositionStateError::InvalidRecord)
    );
    assert_eq!(
        PositionSettlementFactRecordV1::decode(&replace(
            &settlement_fact_bytes(),
            r#""settled_quantity":"1.000""#,
            r#""settled_quantity":"0.000""#,
        )),
        Err(PositionStateError::InvalidRecord)
    );
    assert_eq!(
        PositionSettlementFactRecordV1::decode(&replace(
            &settlement_fact_bytes(),
            r#""settlement_price":"0.00""#,
            r#""settlement_price":"-0.01""#,
        )),
        Err(PositionStateError::InvalidRecord)
    );
    for pnl in ["0.00", "12.50", "-12.50"] {
        PositionSettlementFactRecordV1::decode(&replace(
            &settlement_fact_bytes(),
            r#""realized_pnl":"-12.50""#,
            &format!(r#""realized_pnl":"{pnl}""#),
        ))
        .unwrap();
    }
}

#[test]
fn canonical_json_rule_version_and_payload_hash_are_strict() {
    for invalid in [
        replace(&start_fact_bytes(), r#""event_id":"evt-start","#, ""),
        replace(
            &start_fact_bytes(),
            r#""event_id":"evt-start","#,
            r#""event_id":"evt-start","event_id":"evt-start","#,
        ),
        replace(
            &start_fact_bytes(),
            r#""rule_version":"hyperliquid-alpha-desk-canonical-position-liquidation@1.0.0""#,
            r#""rule_version":"other@1.0.0""#,
        ),
        replace(
            &start_fact_bytes(),
            PAYLOAD_HASH,
            &PAYLOAD_HASH.to_ascii_uppercase().replace('1', "A"),
        ),
        replace(&start_fact_bytes(), PAYLOAD_HASH, "11"),
        replace(
            &start_fact_bytes(),
            PAYLOAD_HASH,
            "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz",
        ),
    ] {
        assert!(LiquidationStartFactRecordV1::decode(&invalid).is_err());
    }
    let reordered = replace(
        &start_fact_bytes(),
        r#""liquidation_id":"liq-001","event_id":"evt-start""#,
        r#""event_id":"evt-start","liquidation_id":"liq-001""#,
    );
    assert_eq!(
        LiquidationStartFactRecordV1::decode(&reordered),
        Err(PositionStateError::NonCanonical)
    );
    let unknown = replace(
        &start_fact_bytes(),
        r#""rule_version""#,
        r#""unknown":0,"rule_version""#,
    );
    assert!(LiquidationStartFactRecordV1::decode(&unknown).is_err());
}

#[test]
fn codecs_are_state_independent_and_preserve_source_decimal_scales() {
    let fill = LiquidationFillFactRecordV1::decode(&fill_fact_bytes()).unwrap();
    assert_eq!(fill.quantity().scale(), 3);
    assert_eq!(fill.price().scale(), 2);
    let settlement = PositionSettlementFactRecordV1::decode(&settlement_fact_bytes()).unwrap();
    assert_eq!(settlement.settled_quantity().scale(), 3);
    assert_eq!(settlement.settlement_price().scale(), 2);
    assert_eq!(settlement.realized_pnl().scale(), 2);
    BackstopLiquidationFactRecordV1::decode(&backstop_fact_bytes()).unwrap();
}

#[test]
fn key_and_value_bounds_are_inclusive_and_fail_closed_above_limits() {
    let exact_key_id = LiquidationId::new("k".repeat(64 * 1024 - 8)).unwrap();
    assert_eq!(
        LiquidationCurrentRecordV1::state_key(&exact_key_id)
            .unwrap()
            .key()
            .len(),
        64 * 1024
    );
    let oversized_key_id = LiquidationId::new("k".repeat(64 * 1024 - 7)).unwrap();
    assert_eq!(
        LiquidationCurrentRecordV1::state_key(&oversized_key_id),
        Err(PositionStateError::InvalidKey)
    );

    let two_text_id_len = 64 * 1024 - 16 - START_EVENT.len();
    let two_text_id = LiquidationId::new("k".repeat(two_text_id_len)).unwrap();
    assert_eq!(
        LiquidationStartFactRecordV1::state_key(&two_text_id, &event(START_EVENT))
            .unwrap()
            .key()
            .len(),
        64 * 1024
    );
    let oversized_two_text_id = LiquidationId::new("k".repeat(two_text_id_len + 1)).unwrap();
    assert_eq!(
        LiquidationStartFactRecordV1::state_key(&oversized_two_text_id, &event(START_EVENT)),
        Err(PositionStateError::InvalidKey)
    );

    let liq_account_market_id_len = 64 * 1024 - 24 - 20 - MARKET.len();
    let liq_account_market_id = LiquidationId::new("k".repeat(liq_account_market_id_len)).unwrap();
    assert_eq!(
        LiquidationMarketFlowCurrentRecordV1::state_key(
            &liq_account_market_id,
            &account(),
            &market(),
        )
        .unwrap()
        .key()
        .len(),
        64 * 1024
    );
    let oversized_liq_account_market_id =
        LiquidationId::new("k".repeat(liq_account_market_id_len + 1)).unwrap();
    assert_eq!(
        LiquidationMarketFlowCurrentRecordV1::state_key(
            &oversized_liq_account_market_id,
            &account(),
            &market(),
        ),
        Err(PositionStateError::InvalidKey)
    );

    let event_account_market_id_len = 64 * 1024 - 24 - 20 - MARKET.len();
    let event_account_market_id = EventId::new("k".repeat(event_account_market_id_len)).unwrap();
    assert_eq!(
        PositionSettlementFactRecordV1::state_key(&event_account_market_id, &account(), &market(),)
            .unwrap()
            .key()
            .len(),
        64 * 1024
    );
    let oversized_event_account_market_id =
        EventId::new("k".repeat(event_account_market_id_len + 1)).unwrap();
    assert_eq!(
        PositionSettlementFactRecordV1::state_key(
            &oversized_event_account_market_id,
            &account(),
            &market(),
        ),
        Err(PositionStateError::InvalidKey)
    );

    assert_value_bound(&started_current_bytes(), LIQUIDATION, |bytes| {
        LiquidationCurrentRecordV1::decode(bytes).map(|_| ())
    });
    assert_value_bound(&start_fact_bytes(), LIQUIDATION, |bytes| {
        LiquidationStartFactRecordV1::decode(bytes).map(|_| ())
    });
    assert_value_bound(&fill_fact_bytes(), LIQUIDATION, |bytes| {
        LiquidationFillFactRecordV1::decode(bytes).map(|_| ())
    });
    assert_value_bound(&flow_current_bytes(), LIQUIDATION, |bytes| {
        LiquidationMarketFlowCurrentRecordV1::decode(bytes).map(|_| ())
    });
    assert_value_bound(&backstop_fact_bytes(), LIQUIDATION, |bytes| {
        BackstopLiquidationFactRecordV1::decode(bytes).map(|_| ())
    });
    assert_value_bound(&settlement_fact_bytes(), SETTLEMENT_EVENT, |bytes| {
        PositionSettlementFactRecordV1::decode(bytes).map(|_| ())
    });
}
