use domain_types::{Horizon, KnownTime, MarketId, ProtocolTime};
use wallet_intelligence::{
    IntelligenceSubject, SkillObservation, SkillPrior, effective_sample_size_milli, estimate_skill,
};

fn prior() -> SkillPrior {
    SkillPrior::from_toml(include_str!("../../../config/models/wallet-skill-v1.toml")).unwrap()
}

fn obs(bps: i64, seconds: i64) -> SkillObservation {
    SkillObservation {
        markout_bps: bps,
        observed_at: ProtocolTime::from_unix_micros(seconds * 1_000_000).unwrap(),
        market_id: MarketId::new("BTC").unwrap(),
        horizon: Horizon::MS_250,
        regime_id: None,
        segment_id: 1,
    }
}

#[test]
fn small_samples_shrink_more_than_large_samples() {
    let prior = prior();
    let small: Vec<_> = (0..3).map(|index| obs(100, index + 1)).collect();
    let large: Vec<_> = (0..300).map(|index| obs(100, index + 1)).collect();
    let known = KnownTime::from_unix_micros(400_000_000).unwrap();
    let small_vec = estimate_skill(
        &IntelligenceSubject::Account(domain_types::AccountId::new("s").unwrap()),
        &small,
        &prior,
        known,
        None,
        None,
    )
    .unwrap();
    let large_vec = estimate_skill(
        &IntelligenceSubject::Account(domain_types::AccountId::new("l").unwrap()),
        &large,
        &prior,
        known,
        None,
        None,
    )
    .unwrap();
    assert!(
        small_vec.directional.posterior_mean_bps.raw()
            < large_vec.directional.posterior_mean_bps.raw()
    );
    assert!(small_vec.directional.posterior_mean_bps.raw() < 100);
    assert!(large_vec.directional.posterior_mean_bps.raw() > 80);
}

#[test]
fn autocorrelated_series_has_lower_effective_sample_size() {
    let independent: Vec<i64> = (0..100).map(|index| i64::from(index % 10)).collect();
    let mut autocorrelated = Vec::new();
    for block in 0..10 {
        autocorrelated.extend(std::iter::repeat_n(i64::from(block), 10));
    }
    let independent_ess = effective_sample_size_milli(&independent).unwrap();
    let auto_ess = effective_sample_size_milli(&autocorrelated).unwrap();
    assert!(auto_ess < independent_ess);
}

#[test]
fn stale_evidence_reduces_freshness_without_rewriting_historical_mean() {
    let prior = prior();
    let observations = vec![obs(80, 1), obs(90, 2), obs(100, 3)];
    let historical = estimate_skill(
        &IntelligenceSubject::Account(domain_types::AccountId::new("a").unwrap()),
        &observations,
        &prior,
        KnownTime::from_unix_micros(3_000_000).unwrap(),
        None,
        None,
    )
    .unwrap();
    let stale = estimate_skill(
        &IntelligenceSubject::Account(domain_types::AccountId::new("a").unwrap()),
        &observations,
        &prior,
        KnownTime::from_unix_micros(prior.half_life_micros.try_into().unwrap()).unwrap(),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        historical.directional.posterior_mean_bps,
        stale.directional.posterior_mean_bps
    );
    assert!(stale.directional.freshness.ppm() < historical.directional.freshness.ppm());
}
