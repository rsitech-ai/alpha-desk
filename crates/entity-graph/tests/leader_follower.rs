use domain_types::{AccountId, MarketId, Price, ProtocolTime};
use entity_graph::{ActionDirection, ActionEvent, GraphError, RelationshipClass, classify_pair};

fn event(
    account: &str,
    seconds: i64,
    direction: ActionDirection,
    market_move_bps: i64,
) -> ActionEvent {
    priced_event(
        account,
        "BTC",
        seconds,
        direction,
        market_move_bps,
        1,
        None,
        None,
    )
}

fn sized_event(
    account: &str,
    market: &str,
    seconds: i64,
    direction: ActionDirection,
    market_move_bps: i64,
    size: u64,
) -> ActionEvent {
    priced_event(
        account,
        market,
        seconds,
        direction,
        market_move_bps,
        size,
        None,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
fn priced_event(
    account: &str,
    market: &str,
    seconds: i64,
    direction: ActionDirection,
    market_move_bps: i64,
    size: u64,
    entry_price: Option<Price>,
    forward_markout_bps: Option<i64>,
) -> ActionEvent {
    ActionEvent {
        account: AccountId::new(account).unwrap(),
        market: MarketId::new(market).unwrap(),
        direction,
        protocol_time: ProtocolTime::from_unix_micros(seconds * 1_000_000).unwrap(),
        size,
        market_move_bps,
        entry_price,
        forward_markout_bps,
    }
}

fn price(value: &str) -> Price {
    Price::parse_at_scale(value, 2).unwrap()
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
    assert_eq!(edge.leader_class, RelationshipClass::IndependentConfirmer);
    assert_ne!(edge.leader_class, RelationshipClass::Originator);
    assert_eq!(edge.follower_adds_independent_value, Some(false));
    assert!(edge.entry_degradation_bps.is_none());
    assert!(edge.edge_decay_bps.is_none());
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
    assert_eq!(edge.leader_class, RelationshipClass::Originator);
    assert!(edge.follower_probability.ppm() >= 600_000);
    assert!(edge.follower_adds_independent_value.is_none());
    assert_eq!(edge.market_overlap_ppm.ppm(), 1_000_000);
}

#[test]
fn empty_history_and_invalid_lags_keep_existing_errors() {
    let lead = event("lead", 1, ActionDirection::Buy, 0);
    let follow = event("copy", 2, ActionDirection::Buy, 0);
    let empty =
        classify_pair(&[], std::slice::from_ref(&follow), 2_000_000, 10_000_000).unwrap_err();
    assert!(matches!(
        empty,
        GraphError::Malformed {
            what: "leader_follower",
            reason: "empty event history"
        }
    ));
    let bounds = classify_pair(&[lead], &[follow], 10, 5).unwrap_err();
    assert!(matches!(
        bounds,
        GraphError::Malformed {
            what: "leader_follower",
            reason: "lag bounds invalid"
        }
    ));
}

#[test]
fn mixed_accounts_fail_closed() {
    let leaders = vec![
        event("lead-a", 1, ActionDirection::Buy, 0),
        event("lead-b", 2, ActionDirection::Buy, 0),
    ];
    let followers = vec![event("copy", 3, ActionDirection::Buy, 0)];
    let error = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap_err();
    assert!(matches!(
        error,
        GraphError::Malformed {
            what: "leader_follower",
            reason: "mixed leader accounts"
        }
    ));
}

#[test]
fn size_relationship_and_partial_market_overlap_are_observed() {
    let leaders = vec![
        sized_event("lead", "BTC", 0, ActionDirection::Buy, 0, 10),
        sized_event("lead", "ETH", 10, ActionDirection::Buy, 0, 10),
    ];
    let followers = vec![
        sized_event("copy", "BTC", 1, ActionDirection::Buy, 0, 4),
        sized_event("copy", "BTC", 11, ActionDirection::Buy, 0, 4),
    ];
    let edge = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap();
    let size = edge.size_relationship.expect("matched sizes");
    assert_eq!(size.median_leader_size, 10);
    assert_eq!(size.median_follower_size, 4);
    assert_eq!(edge.market_overlap_ppm.ppm(), 500_000);
}

#[test]
fn unobserved_prices_and_markouts_are_withheld() {
    let mut leaders = Vec::new();
    let mut followers = Vec::new();
    for second in 0..8 {
        leaders.push(event("lead", second * 10, ActionDirection::Buy, 0));
        followers.push(event("copy", second * 10 + 1, ActionDirection::Buy, 0));
    }
    let edge = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap();
    assert_eq!(edge.leader_class, RelationshipClass::Originator);
    assert!(edge.entry_degradation_bps.is_none());
    assert!(edge.edge_decay_bps.is_none());
    assert!(edge.follower_adds_independent_value.is_none());
}

#[test]
fn observed_buy_degradation_and_edge_decay_are_exact() {
    let leaders = vec![priced_event(
        "lead",
        "BTC",
        0,
        ActionDirection::Buy,
        0,
        1,
        Some(price("100.00")),
        Some(40),
    )];
    let followers = vec![priced_event(
        "copy",
        "BTC",
        1,
        ActionDirection::Buy,
        0,
        1,
        Some(price("101.00")),
        Some(10),
    )];
    let edge = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap();
    assert_eq!(edge.entry_degradation_bps.unwrap().raw(), 100);
    assert_eq!(edge.edge_decay_bps.unwrap().raw(), 30);
    assert_eq!(edge.follower_adds_independent_value, Some(true));
}

#[test]
fn scale_mismatch_entry_prices_fail_closed() {
    let leaders = vec![priced_event(
        "lead",
        "BTC",
        0,
        ActionDirection::Buy,
        0,
        1,
        Some(Price::parse_at_scale("100.00", 2).unwrap()),
        None,
    )];
    let followers = vec![priced_event(
        "copy",
        "BTC",
        1,
        ActionDirection::Buy,
        0,
        1,
        Some(Price::parse_at_scale("101.000", 3).unwrap()),
        None,
    )];
    let error = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap_err();
    assert!(matches!(
        error,
        GraphError::Malformed {
            what: "leader_follower",
            reason: "entry price scale mismatch"
        }
    ));
}

#[test]
fn zero_leader_entry_price_fails_closed() {
    let leaders = vec![priced_event(
        "lead",
        "BTC",
        0,
        ActionDirection::Buy,
        0,
        1,
        Some(price("0.00")),
        None,
    )];
    let followers = vec![priced_event(
        "copy",
        "BTC",
        1,
        ActionDirection::Buy,
        0,
        1,
        Some(price("101.00")),
        None,
    )];
    let error = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap_err();
    assert!(matches!(
        error,
        GraphError::Malformed {
            what: "leader_follower",
            reason: "zero leader entry price"
        }
    ));
}

#[test]
fn contrarian_responder_marks_the_leader_as_originator() {
    let mut leaders = Vec::new();
    let mut followers = Vec::new();
    for second in 0..10 {
        leaders.push(event("lead", second * 10, ActionDirection::Buy, 0));
        followers.push(event("fade", second * 10 + 1, ActionDirection::Sell, 0));
    }
    let edge = classify_pair(&leaders, &followers, 2_000_000, 10_000_000).unwrap();
    assert_eq!(edge.class, RelationshipClass::ContrarianResponder);
    assert_eq!(edge.leader_class, RelationshipClass::Originator);
    assert!(edge.entry_degradation_bps.is_none());
}
