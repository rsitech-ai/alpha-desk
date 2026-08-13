use margin_models::{assert_margin_fixture, parse_margin_fixture};

const CROSS_LONG: &str = include_str!("../../../fixtures/golden/margin/cross-long.json");
const ISOLATED_SHORT: &str = include_str!("../../../fixtures/golden/margin/isolated-short.json");
const PORTFOLIO: &str = include_str!("../../../fixtures/golden/margin/portfolio-estimated.json");
const BOUNDARY: &str = include_str!("../../../fixtures/golden/margin/liquidation-boundary.json");

#[test]
fn golden_margin_fixtures_match_versioned_models() {
    for (json, id) in [
        (CROSS_LONG, "cross-long"),
        (ISOLATED_SHORT, "isolated-short"),
        (PORTFOLIO, "portfolio-estimated"),
        (BOUNDARY, "liquidation-boundary"),
    ] {
        let fixture = parse_margin_fixture(json).unwrap();
        assert_eq!(fixture.id, id);
        assert!(!fixture.stage_1_qualified);
        assert!(!fixture.stage_2_qualified);
        assert_margin_fixture(&fixture).unwrap();
    }
}

#[test]
fn golden_margin_fixture_refuses_qualification_claims() {
    let mut fixture = parse_margin_fixture(CROSS_LONG).unwrap();
    fixture.stage_1_qualified = true;
    assert!(assert_margin_fixture(&fixture).is_err());
}
