use blake3::Hasher;
use domain_types::{BlockHeight, ChainId, EventId, SourceId};

use crate::{QuarantineReason, QuarantineRecord};

const INCIDENT_CONTEXT: &str = "hyperliquid-alpha-desk/source-divergence/v1";

pub(super) fn canonical_block_divergence(
    chain_id: &ChainId,
    block_height: BlockHeight,
    mut existing_sources: Vec<SourceId>,
    conflicting_source: SourceId,
    existing_hash: [u8; 32],
    conflicting_hash: [u8; 32],
) -> QuarantineRecord {
    existing_sources.sort();
    existing_sources.dedup();
    let (first_hash, second_hash) = if existing_hash <= conflicting_hash {
        (existing_hash, conflicting_hash)
    } else {
        (conflicting_hash, existing_hash)
    };
    let incident_id = incident_id(IncidentKey {
        chain_id,
        block_height,
        existing_sources: &existing_sources,
        conflicting_source: &conflicting_source,
        reason_code: "sequencer.conflicting_canonical_block",
        detail_identity: None,
        first_hash,
        second_hash,
    });
    QuarantineRecord::new(
        incident_id,
        chain_id.clone(),
        block_height,
        existing_sources,
        conflicting_source,
        QuarantineReason::ConflictingCanonicalBlock {
            existing_hash,
            conflicting_hash,
        },
    )
}

pub(super) fn source_block_hash_divergence(
    chain_id: &ChainId,
    block_height: BlockHeight,
    source_id: SourceId,
    existing_hash: [u8; 32],
    conflicting_hash: [u8; 32],
) -> QuarantineRecord {
    let (first_hash, second_hash) = if existing_hash <= conflicting_hash {
        (existing_hash, conflicting_hash)
    } else {
        (conflicting_hash, existing_hash)
    };
    let incident_id = incident_id(IncidentKey {
        chain_id,
        block_height,
        existing_sources: std::slice::from_ref(&source_id),
        conflicting_source: &source_id,
        reason_code: "sequencer.conflicting_source_block_hash",
        detail_identity: None,
        first_hash,
        second_hash,
    });
    QuarantineRecord::new(
        incident_id,
        chain_id.clone(),
        block_height,
        vec![source_id.clone()],
        source_id.clone(),
        QuarantineReason::ConflictingSourceBlockHash {
            source_id,
            existing_hash,
            conflicting_hash,
        },
    )
}

pub(super) fn event_source_evidence_divergence(
    chain_id: &ChainId,
    block_height: BlockHeight,
    event_id: EventId,
    source_id: SourceId,
    existing_hash: [u8; 32],
    conflicting_hash: [u8; 32],
) -> QuarantineRecord {
    let (first_hash, second_hash) = if existing_hash <= conflicting_hash {
        (existing_hash, conflicting_hash)
    } else {
        (conflicting_hash, existing_hash)
    };
    let incident_id = incident_id(IncidentKey {
        chain_id,
        block_height,
        existing_sources: std::slice::from_ref(&source_id),
        conflicting_source: &source_id,
        reason_code: "sequencer.conflicting_event_source_evidence",
        detail_identity: Some(event_id.as_str()),
        first_hash,
        second_hash,
    });
    QuarantineRecord::new(
        incident_id,
        chain_id.clone(),
        block_height,
        vec![source_id.clone()],
        source_id.clone(),
        QuarantineReason::ConflictingEventSourceEvidence {
            event_id,
            source_id,
            existing_hash,
            conflicting_hash,
        },
    )
}

struct IncidentKey<'a> {
    chain_id: &'a ChainId,
    block_height: BlockHeight,
    existing_sources: &'a [SourceId],
    conflicting_source: &'a SourceId,
    reason_code: &'a str,
    detail_identity: Option<&'a str>,
    first_hash: [u8; 32],
    second_hash: [u8; 32],
}

fn incident_id(key: IncidentKey<'_>) -> String {
    let mut all_sources = key.existing_sources.to_vec();
    all_sources.push(key.conflicting_source.clone());
    all_sources.sort();
    all_sources.dedup();

    let mut hasher = Hasher::new_derive_key(INCIDENT_CONTEXT);
    hash_bytes(&mut hasher, key.chain_id.as_str().as_bytes());
    hasher.update(&key.block_height.get().to_be_bytes());
    hash_bytes(&mut hasher, key.reason_code.as_bytes());
    if let Some(detail_identity) = key.detail_identity {
        hash_bytes(&mut hasher, detail_identity.as_bytes());
    } else {
        hash_bytes(&mut hasher, &[]);
    }
    for source_id in all_sources {
        hash_bytes(&mut hasher, source_id.as_str().as_bytes());
    }
    hasher.update(&key.first_hash);
    hasher.update(&key.second_hash);
    format!("inc_{}", hasher.finalize().to_hex())
}

fn hash_bytes(hasher: &mut Hasher, bytes: &[u8]) {
    let length = match u64::try_from(bytes.len()) {
        Ok(length) => length,
        Err(_) => unreachable!("source identifiers cannot exceed u64 framing"),
    };
    hasher.update(&length.to_be_bytes());
    hasher.update(bytes);
}
