use std::collections::BTreeMap;

use domain_types::{
    AccountId, ClusterVersionId, EntityId, KnownTime, ProbabilityPpm, ProtocolTime,
};
use feature_core::{Bitemporal, asof};
use serde::{Deserialize, Serialize};

use crate::{GraphError, GraphNodeId, LinkEvidence, LinkKind};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClusterMembershipVersion {
    pub cluster_version_id: ClusterVersionId,
    pub entity_id: EntityId,
    pub member: AccountId,
    pub weight: ProbabilityPpm,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub superseded_at: Option<KnownTime>,
    pub revision: u32,
    pub evidence_set_hash: [u8; 32],
    pub policy_version: String,
}

impl Bitemporal for ClusterMembershipVersion {
    fn effective_at(&self) -> ProtocolTime {
        self.effective_at
    }

    fn known_at(&self) -> KnownTime {
        self.known_at
    }

    fn superseded_at(&self) -> Option<KnownTime> {
        self.superseded_at
    }

    fn revision(&self) -> u32 {
        self.revision
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityGraph {
    links: Vec<LinkEvidence>,
    memberships: Vec<ClusterMembershipVersion>,
}

impl EntityGraph {
    #[must_use]
    pub fn new() -> Self {
        Self {
            links: Vec::new(),
            memberships: Vec::new(),
        }
    }

    pub fn insert_link(&mut self, link: LinkEvidence) -> Result<(), GraphError> {
        if self
            .links
            .iter()
            .any(|existing| existing.evidence_id == link.evidence_id)
        {
            return Err(GraphError::Malformed {
                what: "entity_graph",
                reason: "duplicate evidence_id",
            });
        }
        self.links.push(link);
        Ok(())
    }

    pub fn append_membership(
        &mut self,
        membership: ClusterMembershipVersion,
    ) -> Result<(), GraphError> {
        if membership.cluster_version_id.as_str().is_empty() {
            return Err(GraphError::EmptyIdentifier {
                field: "cluster_version_id",
            });
        }
        if let Some(previous) = self.memberships.iter_mut().rev().find(|existing| {
            existing.member == membership.member && existing.superseded_at.is_none()
        }) {
            if previous.entity_id != membership.entity_id {
                return Err(GraphError::ConflictingLink {
                    reason: "account already belongs to another entity",
                });
            }
            if membership.known_at <= previous.known_at {
                return Err(GraphError::Malformed {
                    what: "cluster_membership",
                    reason: "new version must not rewrite prior known_at",
                });
            }
            previous.superseded_at = Some(membership.known_at);
        }
        self.memberships.push(membership);
        Ok(())
    }

    #[must_use]
    pub fn links_as_of(
        &self,
        effective_at: ProtocolTime,
        known_at: KnownTime,
    ) -> Vec<&LinkEvidence> {
        self.links
            .iter()
            .filter(|link| {
                link.effective_at <= effective_at
                    && link.known_at <= known_at
                    && link
                        .superseded_at
                        .is_none_or(|superseded_at| superseded_at > known_at)
            })
            .collect()
    }

    #[must_use]
    pub fn membership_as_of(
        &self,
        effective_at: ProtocolTime,
        known_at: KnownTime,
    ) -> Option<&ClusterMembershipVersion> {
        asof(&self.memberships, effective_at, known_at)
    }

    pub fn known_administrative_groups(
        &self,
        effective_at: ProtocolTime,
        known_at: KnownTime,
    ) -> Result<Vec<Vec<GraphNodeId>>, GraphError> {
        let links = self.links_as_of(effective_at, known_at);
        reject_conflicting_identity_links(&links)?;
        let mut parent: BTreeMap<GraphNodeId, GraphNodeId> = BTreeMap::new();
        for link in links {
            if !link.kind.forms_administrative_group() {
                continue;
            }
            union(&mut parent, link.left.clone(), link.right.clone());
        }
        let mut groups: BTreeMap<GraphNodeId, Vec<GraphNodeId>> = BTreeMap::new();
        let nodes: Vec<GraphNodeId> = parent.keys().cloned().collect();
        for node in nodes {
            let root = find(&mut parent, node.clone());
            groups.entry(root).or_default().push(node);
        }
        Ok(groups
            .into_values()
            .filter(|group| group.len() > 1)
            .collect())
    }

    pub fn hard_link_kinds() -> [LinkKind; 4] {
        [
            LinkKind::ProtocolSubaccount,
            LinkKind::ProtocolVaultMembership,
            LinkKind::ProtocolVaultManager,
            LinkKind::ApprovedOperatorAnnotation,
        ]
    }
}

impl Default for EntityGraph {
    fn default() -> Self {
        Self::new()
    }
}

fn reject_conflicting_identity_links(links: &[&LinkEvidence]) -> Result<(), GraphError> {
    let mut sub_masters: BTreeMap<&GraphNodeId, &GraphNodeId> = BTreeMap::new();
    let mut vault_managers: BTreeMap<&GraphNodeId, &GraphNodeId> = BTreeMap::new();
    for link in links {
        match link.kind {
            LinkKind::ProtocolSubaccount => {
                if let Some(existing) = sub_masters.get(&link.right)
                    && *existing != &link.left
                {
                    return Err(GraphError::ConflictingLink {
                        reason: "subaccount bound to multiple masters",
                    });
                }
                sub_masters.insert(&link.right, &link.left);
            }
            LinkKind::ProtocolVaultManager => {
                if let Some(existing) = vault_managers.get(&link.right)
                    && *existing != &link.left
                {
                    return Err(GraphError::ConflictingLink {
                        reason: "vault bound to multiple managers",
                    });
                }
                vault_managers.insert(&link.right, &link.left);
            }
            LinkKind::ProtocolVaultMembership
            | LinkKind::ApprovedOperatorAnnotation
            | LinkKind::FundingPath
            | LinkKind::CoordinatedExecution
            | LinkKind::SizePriceFingerprint
            | LinkKind::LeaderFollower
            | LinkKind::CounterpartyInventoryHandoff
            | LinkKind::StrategyMigration => {}
        }
    }
    for master in sub_masters.values() {
        if sub_masters.contains_key(*master) {
            return Err(GraphError::ConflictingLink {
                reason: "ambiguous subaccount hierarchy",
            });
        }
    }
    Ok(())
}

fn find(parent: &mut BTreeMap<GraphNodeId, GraphNodeId>, node: GraphNodeId) -> GraphNodeId {
    if !parent.contains_key(&node) {
        parent.insert(node.clone(), node.clone());
        return node;
    }
    let mut current = node;
    loop {
        let next = parent
            .get(&current)
            .cloned()
            .unwrap_or_else(|| current.clone());
        if next == current {
            return current;
        }
        current = next;
    }
}

fn union(parent: &mut BTreeMap<GraphNodeId, GraphNodeId>, left: GraphNodeId, right: GraphNodeId) {
    let left_root = find(parent, left);
    let right_root = find(parent, right);
    if left_root != right_root {
        parent.insert(right_root, left_root);
    }
}

pub fn membership_hash(
    evidence_ids: &[domain_types::EvidenceId],
    policy_version: &str,
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(policy_version.as_bytes());
    for evidence_id in evidence_ids {
        hasher.update(&[0]);
        hasher.update(evidence_id.as_str().as_bytes());
    }
    *hasher.finalize().as_bytes()
}
