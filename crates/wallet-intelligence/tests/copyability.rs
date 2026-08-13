use domain_types::{
    AccountId, EventId, FeeScheduleId, LatencyDistribution, ProbabilityPpm, UsdAmount,
};
use feature_core::{HealthAssessment, HealthState};
use wallet_intelligence::{
    CopyabilityClass, CopyabilityInputs, CopyabilityRequest, IntelligenceError,
    IntelligenceSubject, MarkoutHorizon, PortfolioContextSummary, estimate_copyability,
};

fn usd(value: &str) -> UsdAmount {
    UsdAmount::parse_at_scale(value, 8).unwrap()
}

fn request(latency_micros: u64, bankroll: &str) -> CopyabilityRequest {
    CopyabilityRequest {
        subject: IntelligenceSubject::Account(AccountId::new("acct-a").unwrap()),
        action_id: EventId::new("action-1").unwrap(),
        detection_latency: LatencyDistribution::new(
            latency_micros,
            latency_micros,
            latency_micros,
            latency_micros.saturating_add(1),
        )
        .unwrap(),
        bankroll: usd(bankroll),
        max_participation: ProbabilityPpm::from_ppm(200_000).unwrap(),
        fee_schedule_id: FeeScheduleId::new("fees-v1").unwrap(),
        portfolio_context: PortfolioContextSummary {
            gross_exposure: usd("0"),
            net_exposure: usd("0"),
            same_market_exposure: usd("0"),
            same_entity_exposure: usd("0"),
            correlated_exposure: usd("0"),
            snapshot_hash: [3_u8; 32],
        },
    }
}

fn base_inputs(latency_micros: u64, bankroll: &str) -> CopyabilityInputs {
    CopyabilityInputs {
        request: request(latency_micros, bankroll),
        markouts: vec![
            MarkoutHorizon {
                latency_micros: 250_000,
                net_return_bps: 40,
            },
            MarkoutHorizon {
                latency_micros: 4_000_000,
                net_return_bps: -20,
            },
        ],
        half_life_micros: 500_000,
        book_health: HealthAssessment::try_new("book:BTC", HealthState::Green, "healthy").unwrap(),
        executable_depth: usd("1000"),
        fee_bps: 1,
        cost_threshold_bps: 15,
        impact_bps_per_participation_ppm: 80,
    }
}

#[test]
fn latency_turns_positive_markout_into_not_copyable() {
    let fast = estimate_copyability(&base_inputs(250_000, "10")).unwrap();
    let slow = estimate_copyability(&base_inputs(4_000_000, "10")).unwrap();
    assert_eq!(fast.0.class, CopyabilityClass::LatencySensitive);
    assert_eq!(slow.0.class, CopyabilityClass::NotCopyable);
}

#[test]
fn larger_bankroll_becomes_capacity_limited_with_lower_max_notional() {
    let small = estimate_copyability(&base_inputs(250_000, "10")).unwrap();
    let mut large_inputs = base_inputs(250_000, "900");
    large_inputs.cost_threshold_bps = 5;
    let large = estimate_copyability(&large_inputs).unwrap();
    assert_eq!(large.0.class, CopyabilityClass::CapacityLimited);
    assert!(large.1.maximum_notional < large_inputs.request.bankroll);
    assert!(small.1.maximum_notional > large.1.maximum_notional || small.0.class != large.0.class);
}

#[test]
fn red_book_health_fails_closed() {
    let mut inputs = base_inputs(250_000, "10");
    inputs.book_health = HealthAssessment::try_new("book:BTC", HealthState::Red, "gap").unwrap();
    assert!(matches!(
        estimate_copyability(&inputs),
        Err(IntelligenceError::RedDataHealth { .. })
    ));
}

#[test]
fn sparse_half_life_is_research_only() {
    let mut inputs = base_inputs(250_000, "10");
    inputs.markouts = vec![MarkoutHorizon {
        latency_micros: 250_000,
        net_return_bps: 10,
    }];
    let (summary, _) = estimate_copyability(&inputs).unwrap();
    assert_eq!(summary.class, CopyabilityClass::ResearchOnly);
}
