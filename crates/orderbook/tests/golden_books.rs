use orderbook::{parse_book_fixture, replay_book_fixture};

const SNAPSHOT_DIFFS: &str = include_str!("../../../fixtures/golden/books/snapshot-diffs.json");
const GAP: &str = include_str!("../../../fixtures/golden/books/gap.json");
const DUPLICATE: &str = include_str!("../../../fixtures/golden/books/duplicate-order.json");
const CROSSED: &str = include_str!("../../../fixtures/golden/books/crossed-book.json");

#[test]
fn golden_book_fixtures_replay_to_expected_l4_and_l2() {
    for (json, id, healthy) in [
        (SNAPSHOT_DIFFS, "snapshot-diffs", true),
        (GAP, "gap", false),
        (DUPLICATE, "duplicate-order", false),
        (CROSSED, "crossed-book", false),
    ] {
        let fixture = parse_book_fixture(json).unwrap();
        assert_eq!(fixture.id, id);
        assert!(!fixture.stage_1_qualified);
        assert!(!fixture.stage_2_qualified);
        let report = replay_book_fixture(&fixture).unwrap();
        assert_eq!(report.id, id);
        assert_eq!(
            matches!(report.health, orderbook::BookHealth::Healthy),
            healthy
        );
    }
}

#[test]
fn golden_book_fixture_refuses_qualification_claims() {
    let mut fixture = parse_book_fixture(SNAPSHOT_DIFFS).unwrap();
    fixture.stage_2_qualified = true;
    assert!(replay_book_fixture(&fixture).is_err());
}
