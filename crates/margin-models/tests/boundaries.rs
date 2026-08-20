use std::collections::BTreeMap;

use domain_types::{
    Address, BlockHeight, DexId, FeeRate, MarginRatio, MarketId, PositionQuantity, Price, UsdAmount,
};
use margin_models::{
    AccountModeMetadata, CalculationConfidence, HIP3_RULES_V1, LiquidationEstimate, MarginError,
    MarginInput, PORTFOLIO_RULES_UNSUPPORTED_EXACT, PositionState, evaluate,
};

#[test]
fn standard_cross_exact_boundary_and_one_unit_neighbors() {
    let market = MarketId::new("perp:BTC").unwrap();
    let oracle = price("10.000");
    let base = input(
        AccountModeMetadata::StandardCross,
        usd("1.000"),
        vec![position(&market, "1", "0.100", "0.100")],
        BTreeMap::from([(market, oracle)]),
    );

    let at_boundary = evaluate(&base).unwrap();
    assert_eq!(at_boundary.maintenance_margin, usd("1.000"));
    assert_eq!(at_boundary.initial_margin, usd("1.000"));
    assert_eq!(at_boundary.margin_ratio, ratio("1.000"));
    assert_eq!(
        at_boundary.liquidation,
        LiquidationEstimate::Exact {
            trigger_price: oracle
        }
    );
    assert_eq!(at_boundary.confidence, CalculationConfidence::Exact);

    let mut above = base.clone();
    above.collateral_value = usd("1.001");
    let above = evaluate(&above).unwrap();
    assert_eq!(above.margin_ratio, ratio("1.001"));
    assert_eq!(
        above.liquidation,
        LiquidationEstimate::Exact {
            trigger_price: price("10.010")
        }
    );

    let mut below = base;
    below.collateral_value = usd("0.999");
    let below = evaluate(&below).unwrap();
    assert_eq!(below.margin_ratio, ratio("0.999"));
    assert_eq!(
        below.liquidation,
        LiquidationEstimate::Exact {
            trigger_price: price("9.990")
        }
    );
}

#[test]
fn isolated_exhaustion_and_missing_oracle_fail_closed() {
    let market = MarketId::new("perp:ETH").unwrap();
    let mut isolated = input(
        AccountModeMetadata::StandardIsolated {
            market_id: market.clone(),
        },
        usd("0.000"),
        vec![position(&market, "1", "0.100", "0.100")],
        BTreeMap::from([(market.clone(), price("10.000"))]),
    );
    let exhausted = evaluate(&isolated).unwrap();
    assert_eq!(exhausted.maintenance_margin, usd("1.000"));
    assert_eq!(exhausted.margin_ratio, ratio("0.000"));

    isolated.oracle_prices.clear();
    assert!(matches!(
        evaluate(&isolated),
        Err(MarginError::MissingInput(name)) if name == "oracle:perp:ETH"
    ));
}

#[test]
fn unified_nets_offsetting_maintenance_below_cross_sum() {
    let long_market = MarketId::new("perp:BTC").unwrap();
    let short_market = MarketId::new("perp:ETH").unwrap();
    let positions = vec![
        position(&long_market, "1", "0.100", "0.100"),
        position(&short_market, "-1", "0.100", "0.100"),
    ];
    let oracles = BTreeMap::from([
        (long_market.clone(), price("10.000")),
        (short_market.clone(), price("5.000")),
    ]);

    let cross = evaluate(&input(
        AccountModeMetadata::StandardCross,
        usd("1.500"),
        positions.clone(),
        oracles.clone(),
    ))
    .unwrap();
    let unified = evaluate(&input(
        AccountModeMetadata::Unified,
        usd("1.500"),
        positions,
        oracles,
    ))
    .unwrap();

    assert_eq!(cross.maintenance_margin, usd("1.500"));
    assert_eq!(unified.maintenance_margin, usd("1.000"));
    assert!(matches!(
        unified.liquidation,
        LiquidationEstimate::Range { .. }
    ));
}

#[test]
fn portfolio_never_returns_an_exact_liquidation_price() {
    let market = MarketId::new("perp:BTC").unwrap();
    let assessment = evaluate(&input(
        AccountModeMetadata::Portfolio {
            rules_version: "portfolio-unknown".to_owned(),
        },
        usd("1.000"),
        vec![position(&market, "1", "0.100", "0.100")],
        BTreeMap::from([(market, price("10.000"))]),
    ))
    .unwrap();
    assert_eq!(assessment.confidence, CalculationConfidence::Bounded);
    match assessment.liquidation {
        LiquidationEstimate::Range { reason, .. } => {
            assert_eq!(reason, PORTFOLIO_RULES_UNSUPPORTED_EXACT);
        }
        other => panic!("portfolio must not coerce exact liquidation: {other:?}"),
    }
}

#[test]
fn hip3_unknown_rules_are_unsupported_and_known_rules_are_exact() {
    let market = MarketId::new("perp:BTC").unwrap();
    let dex = DexId::new("hip3-dex").unwrap();
    let unknown = evaluate(&input(
        AccountModeMetadata::Hip3 {
            dex_id: dex.clone(),
            rules_version: "hip3-margin@0.0.1".to_owned(),
        },
        usd("1.000"),
        vec![position(&market, "1", "0.100", "0.100")],
        BTreeMap::from([(market.clone(), price("10.000"))]),
    ));
    assert_eq!(unknown, Err(MarginError::UnsupportedVersion));

    let known = evaluate(&input(
        AccountModeMetadata::Hip3 {
            dex_id: dex,
            rules_version: HIP3_RULES_V1.to_owned(),
        },
        usd("1.000"),
        vec![position(&market, "1", "0.100", "0.100")],
        BTreeMap::from([(market, price("10.000"))]),
    ))
    .unwrap();
    assert_eq!(known.confidence, CalculationConfidence::Exact);
    assert_eq!(
        known.liquidation,
        LiquidationEstimate::Exact {
            trigger_price: price("10.000")
        }
    );
}

#[test]
fn outcome_markets_do_not_emit_perpetual_liquidation_prices() {
    let market = MarketId::new("outcome:YES").unwrap();
    let assessment = evaluate(&input(
        AccountModeMetadata::Outcome {
            market_id: market.clone(),
        },
        usd("1.000"),
        vec![position(&market, "1", "0.100", "0.100")],
        BTreeMap::from([(market, price("10.000"))]),
    ))
    .unwrap();
    assert_eq!(assessment.liquidation, LiquidationEstimate::NotApplicable);
    assert_eq!(assessment.maintenance_margin, usd("1.000"));
}

fn input(
    mode: AccountModeMetadata,
    collateral: UsdAmount,
    positions: Vec<PositionState>,
    oracle_prices: BTreeMap<MarketId, Price>,
) -> MarginInput {
    MarginInput {
        account_id: Address::from_bytes([0x11; 20]),
        mode,
        collateral_value: collateral,
        positions,
        oracle_prices,
        metadata_block: BlockHeight::new(1),
    }
}

fn position(market: &MarketId, quantity: &str, im: &str, mm: &str) -> PositionState {
    PositionState {
        market_id: market.clone(),
        quantity: PositionQuantity::parse_at_scale(quantity, 0).unwrap(),
        initial_margin_rate: FeeRate::parse_at_scale(im, 3).unwrap(),
        maintenance_margin_rate: FeeRate::parse_at_scale(mm, 3).unwrap(),
    }
}

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, 3).unwrap()
}

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 3).unwrap()
}

fn ratio(value: &str) -> MarginRatio {
    MarginRatio::parse_at_scale(value, 3).unwrap()
}
