use hl_capture::{
    PlannerConfig, PlannerInput, RejectReason, SubscriptionDemand, plan_subscriptions,
};

#[test]
fn subscription_plan_reserves_failover_capacity() {
    let plan = plan_subscriptions(
        PlannerConfig::official(),
        PlannerInput::new(vec![SubscriptionDemand::new("allMids")]),
    );
    assert_eq!(plan.reserved_connections(), 1);
    assert_eq!(plan.connections().len(), 2);
    assert!(plan.connections().iter().any(|connection| {
        matches!(
            connection.kind(),
            hl_capture::PlannedConnectionKind::FailoverReserve
        ) && connection.subscriptions().is_empty()
    }));
}

#[test]
fn subscription_plan_user_events_uses_user_family() {
    let plan = plan_subscriptions(
        PlannerConfig::official(),
        PlannerInput::new(vec![
            SubscriptionDemand::new("userEvents")
                .with_user("0x0000000000000000000000000000000000000001"),
        ]),
    );
    let canonical = plan.connections()[0].subscriptions()[0].canonical_json();
    assert!(canonical.contains("\"type\":\"userEvents\""));
    assert!(canonical.contains("0x0000000000000000000000000000000000000001"));
}

#[test]
fn subscription_plan_red_source_is_not_allocated() {
    let plan = plan_subscriptions(
        PlannerConfig::official(),
        PlannerInput::new(vec![SubscriptionDemand::new("allMids")])
            .with_health("allMids", hl_capture::SourceHealthHint::Red),
    );
    assert_eq!(plan.subscription_count(), 0);
    assert_eq!(plan.rejected()[0].reason(), RejectReason::SourceRed);
}
