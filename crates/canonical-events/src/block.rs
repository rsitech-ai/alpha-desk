use std::collections::{BTreeMap, BTreeSet};

use domain_types::{BlockHeight, ChainId, EventId, ProtocolTime, SourceId, TransactionId};

use crate::{CanonicalEventEnvelope, ConfirmationClass};

const BLOCK_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/canonical-block/v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockEnvelope {
    chain_id: ChainId,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    confirmation_class: ConfirmationClass,
    events: Vec<CanonicalEventEnvelope>,
    source_block_hashes: BTreeMap<SourceId, [u8; 32]>,
    canonical_block_hash: [u8; 32],
}

impl BlockEnvelope {
    pub fn try_new(
        chain_id: ChainId,
        block_height: BlockHeight,
        block_time: ProtocolTime,
        confirmation_class: ConfirmationClass,
        events: Vec<CanonicalEventEnvelope>,
        source_block_hashes: BTreeMap<SourceId, [u8; 32]>,
    ) -> Result<Self, BlockError> {
        if source_block_hashes.is_empty() {
            return Err(BlockError::MissingSourceBlockHashes);
        }
        validate_events(
            &events,
            &chain_id,
            block_height,
            block_time,
            confirmation_class,
        )?;
        let canonical_block_hash = compute_block_hash(&chain_id, block_height, block_time, &events);

        Ok(Self {
            chain_id,
            block_height,
            block_time,
            confirmation_class,
            events,
            source_block_hashes,
            canonical_block_hash,
        })
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn block_time(&self) -> ProtocolTime {
        self.block_time
    }

    #[must_use]
    pub const fn confirmation_class(&self) -> ConfirmationClass {
        self.confirmation_class
    }

    #[must_use]
    pub fn events(&self) -> &[CanonicalEventEnvelope] {
        &self.events
    }

    #[must_use]
    pub const fn source_block_hashes(&self) -> &BTreeMap<SourceId, [u8; 32]> {
        &self.source_block_hashes
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum BlockError {
    #[error("canonical block requires at least one source block hash")]
    MissingSourceBlockHashes,
    #[error("canonical block contains an event from another chain")]
    MixedChain,
    #[error("canonical block contains an event from another height")]
    MixedHeight,
    #[error("canonical block contains an event with another block time")]
    MixedTime,
    #[error("canonical block contains an event with another confirmation class")]
    MixedConfirmation,
    #[error("canonical block contains duplicate event ID {0}")]
    DuplicateEventId(EventId),
    #[error("canonical event ID {actual} does not match expected identity {expected}")]
    InvalidEventId { actual: EventId, expected: EventId },
    #[error("canonical event order is invalid: {reason}")]
    InvalidEventOrder { reason: &'static str },
}

impl BlockError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::MissingSourceBlockHashes => "canonical_block.missing_source_block_hashes",
            Self::MixedChain => "canonical_block.mixed_chain",
            Self::MixedHeight => "canonical_block.mixed_height",
            Self::MixedTime => "canonical_block.mixed_time",
            Self::MixedConfirmation => "canonical_block.mixed_confirmation",
            Self::DuplicateEventId(_) => "canonical_block.duplicate_event_id",
            Self::InvalidEventId { .. } => "canonical_block.invalid_event_id",
            Self::InvalidEventOrder { .. } => "canonical_block.invalid_event_order",
        }
    }
}

fn validate_events(
    events: &[CanonicalEventEnvelope],
    chain_id: &ChainId,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    confirmation_class: ConfirmationClass,
) -> Result<(), BlockError> {
    let mut event_ids = BTreeSet::new();
    let mut previous_order: Option<(u32, u32, &TransactionId)> = None;

    for event in events {
        if event.chain_id() != chain_id {
            return Err(BlockError::MixedChain);
        }
        if event.block_height() != block_height {
            return Err(BlockError::MixedHeight);
        }
        if event.block_time() != block_time {
            return Err(BlockError::MixedTime);
        }
        if event.confirmation_class() != confirmation_class {
            return Err(BlockError::MixedConfirmation);
        }
        if !event_ids.insert(event.event_id().clone()) {
            return Err(BlockError::DuplicateEventId(event.event_id().clone()));
        }

        let expected = event.expected_event_id();
        if event.event_id() != &expected {
            return Err(BlockError::InvalidEventId {
                actual: event.event_id().clone(),
                expected,
            });
        }

        validate_order(previous_order, event)?;
        previous_order = Some((
            event.transaction_index(),
            event.canonical_event_index(),
            event.transaction_id(),
        ));
    }

    Ok(())
}

fn validate_order(
    previous: Option<(u32, u32, &TransactionId)>,
    event: &CanonicalEventEnvelope,
) -> Result<(), BlockError> {
    let transaction_index = event.transaction_index();
    let event_index = event.canonical_event_index();
    let Some((previous_transaction, previous_event, previous_identity)) = previous else {
        return if event_index == 0 {
            Ok(())
        } else {
            Err(BlockError::InvalidEventOrder {
                reason: "the first canonical event in a transaction must have index zero",
            })
        };
    };

    match transaction_index.cmp(&previous_transaction) {
        std::cmp::Ordering::Less => Err(BlockError::InvalidEventOrder {
            reason: "transaction index regressed",
        }),
        std::cmp::Ordering::Equal => {
            if event.transaction_id() != previous_identity {
                return Err(BlockError::InvalidEventOrder {
                    reason: "one transaction index contains multiple transaction identities",
                });
            }
            if previous_event.checked_add(1) == Some(event_index) {
                Ok(())
            } else {
                Err(BlockError::InvalidEventOrder {
                    reason: "canonical event indices are not contiguous within a transaction",
                })
            }
        }
        std::cmp::Ordering::Greater => {
            if event_index == 0 {
                Ok(())
            } else {
                Err(BlockError::InvalidEventOrder {
                    reason: "a new transaction must begin with canonical event index zero",
                })
            }
        }
    }
}

fn compute_block_hash(
    chain_id: &ChainId,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    events: &[CanonicalEventEnvelope],
) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(BLOCK_HASH_CONTEXT);
    hash_bytes(&mut hasher, chain_id.as_str().as_bytes());
    hasher.update(&block_height.get().to_be_bytes());
    hasher.update(&block_time.unix_micros().to_be_bytes());
    let event_count = match u64::try_from(events.len()) {
        Ok(count) => count,
        Err(_) => unreachable!("canonical event counts cannot exceed the u64 framing limit"),
    };
    hasher.update(&event_count.to_be_bytes());

    for event in events {
        hasher.update(&event.transaction_index().to_be_bytes());
        hasher.update(&event.canonical_event_index().to_be_bytes());
        hash_bytes(&mut hasher, event.event_id().as_str().as_bytes());
        hash_bytes(&mut hasher, event.event_kind().as_wire_name().as_bytes());
        hasher.update(&event.payload_hash());
    }

    *hasher.finalize().as_bytes()
}

fn hash_bytes(hasher: &mut blake3::Hasher, bytes: &[u8]) {
    let length = match u64::try_from(bytes.len()) {
        Ok(length) => length,
        Err(_) => unreachable!("canonical block fields cannot exceed the u64 framing limit"),
    };
    hasher.update(&length.to_be_bytes());
    hasher.update(bytes);
}
