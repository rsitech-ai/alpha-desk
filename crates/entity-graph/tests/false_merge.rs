use domain_types::{
    AccountId, ClusterVersionId, EntityId, EvidenceId, KnownTime, ProbabilityPpm, ProtocolTime,
    VaultId,
};
use entity_graph::{
    ClusterMembershipVersion, EntityGraph, EvidenceFamily, GraphError, GraphNodeId,
    IndependenceInput, LinkEvidence, LinkKind, LinkPolicy, effective_votes, independence_weight,
    membership_hash,
};

fn account(name: &str) -> GraphNodeId {
    GraphNodeId::Account(AccountId::new(name).unwrap())
}

fn vault(name: &str) -> GraphNodeId {
    GraphNodeId::Vault(VaultId::new(name).unwrap())
}

fn time(block: i64) -> ProtocolTime {
    ProtocolTime::from_unix_micros(block * 1_000).unwrap()
}

fn known(block: i64) -> KnownTime {
    KnownTime::from_unix_micros(block * 1_000).unwrap()
}

fn link(
    evidence_id: &str,
    left: GraphNodeId,
    right: GraphNodeId,
    kind: LinkKind,
    probability: ProbabilityPpm,
) -> LinkEvidence {
    LinkEvidence::try_new(
        EvidenceId::new(evidence_id).unwrap(),
        left,
        right,
        kind,
        probability,
        time(1),
        known(1),
        Vec::new(),
        None,
    )
    .unwrap()
}

fn membership(entity: &str, member: &str, version: &str) -> ClusterMembershipVersion {
    ClusterMembershipVersion {
        cluster_version_id: ClusterVersionId::new(version).unwrap(),
        entity_id: EntityId::new(entity).unwrap(),
        member: AccountId::new(member).unwrap(),
        weight: ProbabilityPpm::ONE,
        effective_at: time(1),
        known_at: known(1),
        superseded_at: None,
        revision: 1,
        evidence_set_hash: membership_hash(&[], "entity-link-policy-v1"),
        policy_version: "entity-link-policy-v1".to_owned(),
    }
}

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
    assert!(!LinkKind::CoordinatedExecution.forms_administrative_group());
    let _ = AccountId::new("hft-a").unwrap();
}

#[test]
fn distinct_addresses_without_protocol_identity_link_stay_unmerged() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(link(
            "funding-1",
            account("wallet-a"),
            account("wallet-b"),
            LinkKind::FundingPath,
            ProbabilityPpm::from_ppm(990_000).unwrap(),
        ))
        .unwrap();
    graph
        .insert_link(link(
            "sync-1",
            account("wallet-a"),
            account("wallet-b"),
            LinkKind::CoordinatedExecution,
            ProbabilityPpm::from_ppm(990_000).unwrap(),
        ))
        .unwrap();
    assert!(
        graph
            .known_administrative_groups(time(1), known(1))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn shared_vault_membership_does_not_collapse_distinct_depositors() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(link(
            "vault-a",
            account("depositor-a"),
            vault("shared-vault"),
            LinkKind::ProtocolVaultMembership,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    graph
        .insert_link(link(
            "vault-b",
            account("depositor-b"),
            vault("shared-vault"),
            LinkKind::ProtocolVaultMembership,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    assert!(LinkKind::ProtocolVaultMembership.is_hard());
    assert!(!LinkKind::ProtocolVaultMembership.forms_administrative_group());
    assert!(
        graph
            .known_administrative_groups(time(1), known(1))
            .unwrap()
            .is_empty()
    );
}

#[test]
fn explicit_protocol_subaccount_still_forms_one_administrative_group() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(link(
            "sub-1",
            account("master"),
            account("sub"),
            LinkKind::ProtocolSubaccount,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    let groups = graph
        .known_administrative_groups(time(1), known(1))
        .unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].len(), 2);
}

#[test]
fn conflicting_subaccount_masters_fail_closed_instead_of_merging() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(link(
            "sub-master-a",
            account("master-a"),
            account("sub"),
            LinkKind::ProtocolSubaccount,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    graph
        .insert_link(link(
            "sub-master-b",
            account("master-b"),
            account("sub"),
            LinkKind::ProtocolSubaccount,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    let error = graph
        .known_administrative_groups(time(1), known(1))
        .unwrap_err();
    assert!(matches!(
        error,
        GraphError::ConflictingLink {
            reason: "subaccount bound to multiple masters"
        }
    ));
}

#[test]
fn ambiguous_subaccount_hierarchy_fails_closed() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(link(
            "a-owns-b",
            account("master-a"),
            account("mid-b"),
            LinkKind::ProtocolSubaccount,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    graph
        .insert_link(link(
            "b-owns-c",
            account("mid-b"),
            account("leaf-c"),
            LinkKind::ProtocolSubaccount,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    let error = graph
        .known_administrative_groups(time(1), known(1))
        .unwrap_err();
    assert!(matches!(
        error,
        GraphError::ConflictingLink {
            reason: "ambiguous subaccount hierarchy"
        }
    ));
}

#[test]
fn conflicting_vault_managers_fail_closed() {
    let mut graph = EntityGraph::new();
    graph
        .insert_link(link(
            "mgr-a",
            account("manager-a"),
            vault("vault"),
            LinkKind::ProtocolVaultManager,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    graph
        .insert_link(link(
            "mgr-b",
            account("manager-b"),
            vault("vault"),
            LinkKind::ProtocolVaultManager,
            ProbabilityPpm::ONE,
        ))
        .unwrap();
    let error = graph
        .known_administrative_groups(time(1), known(1))
        .unwrap_err();
    assert!(matches!(
        error,
        GraphError::ConflictingLink {
            reason: "vault bound to multiple managers"
        }
    ));
}

#[test]
fn dual_entity_membership_fails_closed() {
    let mut graph = EntityGraph::new();
    graph
        .append_membership(membership("entity-a", "acct-1", "cv-1"))
        .unwrap();
    let error = graph
        .append_membership(membership("entity-b", "acct-1", "cv-2"))
        .unwrap_err();
    assert!(matches!(
        error,
        GraphError::ConflictingLink {
            reason: "account already belongs to another entity"
        }
    ));
}
