use domain_types::{AccountId, MarketId, ProtocolTime};
use entity_graph::{ActionDirection, ActionEvent, RelationshipClass, classify_pair};

fn event(
    account: &str,
    seconds: i64,
    direction: ActionDirection,
    market_move_bps: i64,
) -> ActionEvent {
    ActionEvent {
        account: AccountId::new(account).unwrap(),
        market: MarketId::new("BTC").unwrap(),
        direction,
        protocol_time: ProtocolTime::from_unix_micros(seconds * 1_000_000).unwrap(),
        size: 1,
        market_move_bps,
    }
}

#[test]
fn independent_market_jump_is_not_labeled_copying() {
    let mut leaders = Vec::new();
    let mut followers = Vec::new();
    for second in 0..20 {
        leaders.push(event("lead", second, ActionDirection::Buy, 80));
        followers.push(event("react", second + 1, ActionDirection::Buy, 80));
    }
    let edge = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap();
    assert_eq!(edge.class, RelationshipClass::IndependentConfirmer);
}

#[test]
fn stable_lag_without_market_move_is_a_follower() {
    let mut leaders = Vec::new();
    let mut followers = Vec::new();
    for second in 0..20 {
        leaders.push(event("lead", second * 10, ActionDirection::Buy, 0));
        followers.push(event("copy", second * 10 + 1, ActionDirection::Buy, 0));
    }
    let edge = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap();
    assert!(matches!(
        edge.class,
        RelationshipClass::FastFollower | RelationshipClass::CopyBot
    ));
    assert!(edge.follower_probability.ppm() >= 600_000);
}
