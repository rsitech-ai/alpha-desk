use domain_types::{AccountId, ProbabilityPpm};
use entity_graph::{
    EvidenceFamily, IndependenceInput, LinkKind, LinkPolicy, effective_votes, independence_weight,
};

#[test]
fn twenty_followers_plus_five_independents_count_as_about_six_votes() {
    let share = ProbabilityPpm::from_ppm(50_000).unwrap();
    let follower_inputs = IndependenceInput {
        hard_cluster_share: share,
        follower_probability: ProbabilityPpm::ZERO,
        coordinated_action_probability: ProbabilityPpm::ZERO,
        evidence_quality: ProbabilityPpm::ONE,
    };
    let mut weights = Vec::new();
    for _ in 0..20 {
        weights.push(independence_weight(&follower_inputs).unwrap());
    }
    let independent = IndependenceInput {
        hard_cluster_share: ProbabilityPpm::ONE,
        follower_probability: ProbabilityPpm::ZERO,
        coordinated_action_probability: ProbabilityPpm::ZERO,
        evidence_quality: ProbabilityPpm::ONE,
    };
    for _ in 0..5 {
        weights.push(independence_weight(&independent).unwrap());
    }
    assert_eq!(effective_votes(&weights).unwrap(), 6);
}

#[test]
fn coincidental_timing_alone_does_not_merge() {
    let policy = LinkPolicy::from_toml(include_str!(
        "../../../config/models/entity-link-policy-v1.toml"
    ))
    .unwrap();
    assert!(!policy.allows_soft_merge(&[EvidenceFamily::Synchronization], 900_000));
    assert!(policy.allows_soft_merge(
        &[
            EvidenceFamily::Synchronization,
            EvidenceFamily::Fingerprint,
            EvidenceFamily::FundingPath,
        ],
        900_000
    ));
    assert!(!LinkKind::CoordinatedExecution.is_hard());
    let _ = AccountId::new("hft-a").unwrap();
}
