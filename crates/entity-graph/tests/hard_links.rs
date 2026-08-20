use domain_types::{
    AccountId, ClusterVersionId, EntityId, EvidenceId, KnownTime, ProbabilityPpm, ProtocolTime,
};
use entity_graph::{
    ClusterMembershipVersion, EntityGraph, GraphNodeId, LinkEvidence, LinkKind, membership_hash,
};

fn account(name: &str) -> GraphNodeId {
    GraphNodeId::Account(AccountId::new(name).unwrap())
}

fn time(block: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(block * 1_000).unwrap()
}

fn known(block: i64) -> KnownTime {
    KnownTime::from_unix_micros(block * 1_000).unwrap()
}

#[test]
fn subaccount_learned_later_is_hidden_from_earlier_known_time() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(
            LinkEvidence::try_new(
                EvidenceId::new("sub-1").unwrap(),
                account("master"),
                account("sub"),
                LinkKind::ProtocolSubaccount,
                ProbabilityPpm::ONE,
                time(1_000),
                known(1_002),
                Vec::new(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(graph.links_as_of(time(1_000), known(1_001)).is_empty());
    assert_eq!(graph.links_as_of(time(1_000), known(1_002)).len(), 1);
}

#[test]
fn annotation_appends_a_version_and_does_not_rewrite_history() {
    let mut graph = EntityGraph::new();
    let first = ClusterMembershipVersion {
        cluster_version_id: ClusterVersionId::new("cv-1").unwrap(),
        entity_id: EntityId::new("ent-1").unwrap(),
        member: AccountId::new("acct-a").unwrap(),
        weight: ProbabilityPpm::ONE,
        effective_at: time(1),
        known_at: known(1),
        superseded_at: None,
        revision: 1,
        evidence_set_hash: membership_hash(&[], "entity-link-policy-v1"),
        policy_version: "entity-link-policy-v1".to_owned(),
    };
    graph.append_membership(first.clone()).unwrap();
    let second = ClusterMembershipVersion {
        cluster_version_id: ClusterVersionId::new("cv-2").unwrap(),
        entity_id: EntityId::new("ent-1").unwrap(),
        member: AccountId::new("acct-a").unwrap(),
        weight: ProbabilityPpm::from_ppm(900_000).unwrap(),
        effective_at: time(1),
        known_at: known(5),
        superseded_at: None,
        revision: 2,
        evidence_set_hash: membership_hash(
            &[EvidenceId::new("ann-1").unwrap()],
            "entity-link-policy-v1",
        ),
        policy_version: "entity-link-policy-v1".to_owned(),
    };
    graph.append_membership(second).unwrap();
    assert_eq!(
        graph.membership_as_of(time(1), known(1)).unwrap().revision,
        1
    );
    assert_eq!(
        graph.membership_as_of(time(1), known(5)).unwrap().revision,
        2
    );
    assert_eq!(
        graph.membership_as_of(time(1), known(1)).unwrap().weight,
        ProbabilityPpm::ONE
    );
}

#[test]
fn soft_links_do_not_create_administrative_groups() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(
            LinkEvidence::try_new(
                EvidenceId::new("soft-1").unwrap(),
                account("a"),
                account("b"),
                LinkKind::FundingPath,
                ProbabilityPpm::from_ppm(700_000).unwrap(),
                time(1),
                known(1),
                Vec::new(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
    assert!(
        graph
            .known_administrative_groups(time(1), known(1))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn hard_links_form_an_administrative_group() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(
            LinkEvidence::try_new(
                EvidenceId::new("hard-1").unwrap(),
                account("master"),
                account("sub"),
                LinkKind::ProtocolSubaccount,
                ProbabilityPpm::ONE,
                time(1),
                known(1),
                Vec::new(),
                None,
            )
            .unwrap(),
        )
        .unwrap();
    let groups = graph
        .known_administrative_groups(time(1), known(1))
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 2);
}
