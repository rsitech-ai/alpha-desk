use domain_types::{AccountId, EvidenceId, KnownTime, ProbabilityPpm, ProtocolTime};
use entity_graph::{EvidenceFamily, GraphError, GraphNodeId, LinkEvidence, LinkKind, LinkPolicy};

#[test]
fn malformed_hard_link_probability_fails_closed() {
    let error = LinkEvidence::try_new(
        EvidenceId::new("bad").unwrap(),
        GraphNodeId::Account(AccountId::new("a").unwrap()),
        GraphNodeId::Account(AccountId::new("b").unwrap()),
        LinkKind::ProtocolSubaccount,
        ProbabilityPpm::from_ppm(500_000).unwrap(),
        ProtocolTime::from_unix_micros(1).unwrap(),
        KnownTime::from_unix_micros(1).unwrap(),
        Vec::new(),
        None,
    )
    .unwrap_err();
    assert!(matches!(error, GraphError::Malformed { .. }));
}

#[test]
fn self_links_and_policy_minimums_fail_closed() {
    assert!(
        LinkEvidence::try_new(
            EvidenceId::new("self").unwrap(),
            GraphNodeId::Account(AccountId::new("a").unwrap()),
            GraphNodeId::Account(AccountId::new("a").unwrap()),
            LinkKind::FundingPath,
            ProbabilityPpm::from_ppm(500_000).unwrap(),
            ProtocolTime::from_unix_micros(1).unwrap(),
            KnownTime::from_unix_micros(1).unwrap(),
            Vec::new(),
            None,
        )
        .is_err()
    );
    let policy = LinkPolicy {
        version: "entity-link-policy-v1".to_owned(),
        min_distinct_families: 2,
        posterior_threshold_ppm: 800_000,
        stability_duration_micros: 1,
    };
    assert!(!policy.allows_soft_merge(&[EvidenceFamily::FundingPath], 999_000));
}
