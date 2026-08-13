use domain_types::{Direction, EntityId, MarketId};
use signal_core::{
    AlertPolicy, DedupKey, IndependenceClass, MaterialChange, SignalLifecycleState, SignalType,
    dedup_key,
};

#[test]
fn repeated_originators_share_one_thread() {
    let left = dedup_key(
        MarketId::new("BTC").unwrap(),
        &SignalType::IndependentSmartFlowAcceleration,
        &[EntityId::new("b").unwrap(), EntityId::new("a").unwrap()],
        Direction::Long,
        IndependenceClass::Independent,
    );
    let right = dedup_key(
        MarketId::new("BTC").unwrap(),
        &SignalType::IndependentSmartFlowAcceleration,
        &[
            EntityId::new("a").unwrap(),
            EntityId::new("b").unwrap(),
            EntityId::new("a").unwrap(),
        ],
        Direction::Long,
        IndependenceClass::Independent,
    );
    assert_eq!(left, right);
}

#[test]
fn independent_evidence_creates_a_separate_thread() {
    let first = DedupKey {
        market_id: MarketId::new("BTC").unwrap(),
        family: SignalType::IndependentSmartFlowAcceleration
            .as_wire_name()
            .to_owned(),
        originator_hash: [1_u8; 32],
        direction: Direction::Long,
        independence_class: IndependenceClass::Independent,
    };
    let second = DedupKey {
        originator_hash: [2_u8; 32],
        ..first.clone()
    };
    assert_ne!(first, second);
}

#[test]
fn cooldown_never_suppresses_invalidation_or_risk() {
    let policy =
        AlertPolicy::from_toml(include_str!("../../../config/signals/v1/alert-policy.toml"))
            .unwrap();
    let thresholds = MaterialChange::from_toml(include_str!(
        "../../../config/signals/v1/material-change.toml"
    ))
    .unwrap();
    let quiet = MaterialChange {
        net_edge_delta_bps: 1,
        confidence_delta_ppm: 1,
        crowding_delta_ppm: 1,
    };
    assert!(!quiet.is_material(&thresholds));
    assert_eq!(
        policy
            .decide(SignalLifecycleState::Live, false, 1, 0, false,)
            .as_wire_name(),
        "cooldown_suppressed"
    );
    assert_eq!(
        policy
            .decide(SignalLifecycleState::Invalidated, false, 1, 99, false,)
            .as_wire_name(),
        "always_emit_risk"
    );
    assert_eq!(
        policy
            .decide(SignalLifecycleState::Live, false, 1, 0, true)
            .as_wire_name(),
        "always_emit_risk"
    );
}
