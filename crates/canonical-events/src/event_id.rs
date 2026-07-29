use domain_types::{BlockHeight, ChainId, EventId, TransactionId};

use crate::EventKind;

const EVENT_ID_CONTEXT: &str = "hyperliquid-alpha-desk/event-id/v1";

#[derive(Debug, Clone, Copy)]
pub struct EventIdentityInput<'a> {
    pub chain_id: &'a ChainId,
    pub block_height: BlockHeight,
    pub transaction_identity: &'a TransactionId,
    pub canonical_event_index: u32,
    pub event_kind: EventKind,
    pub schema_major: u64,
}

#[must_use]
pub fn compute_event_id(input: &EventIdentityInput<'_>) -> EventId {
    let mut hasher = blake3::Hasher::new_derive_key(EVENT_ID_CONTEXT);
    hash_bytes(&mut hasher, input.chain_id.as_str().as_bytes());
    hasher.update(&input.block_height.get().to_be_bytes());
    hash_bytes(&mut hasher, input.transaction_identity.as_str().as_bytes());
    hasher.update(&input.canonical_event_index.to_be_bytes());
    hash_bytes(&mut hasher, input.event_kind.as_wire_name().as_bytes());
    hasher.update(&input.schema_major.to_be_bytes());

    let value = format!("evt_{}", hasher.finalize().to_hex());
    match EventId::new(value) {
        Ok(event_id) => event_id,
        Err(_) => unreachable!("the versioned event-id encoding is always a valid identifier"),
    }
}

fn hash_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    let length = match u64::try_from(bytes.len()) {
        Ok(length) => length,
        Err(_) => unreachable!("identity fields cannot exceed the u64 framing limit"),
    };
    hasher.update(&length.to_be_bytes());
    hasher.update(bytes);
}
