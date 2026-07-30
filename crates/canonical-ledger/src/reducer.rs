use canonical_events::{CanonicalEventEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, ProtocolTime};

use crate::{ReducerError, StateKey, StateMutation, StateView};

#[derive(Debug, Clone, Copy)]
pub struct ApplyContext<'a> {
    chain_id: &'a ChainId,
    block_height: BlockHeight,
    block_time: ProtocolTime,
    confirmation_class: ConfirmationClass,
}

impl<'a> ApplyContext<'a> {
    pub(crate) const fn new(
        chain_id: &'a ChainId,
        block_height: BlockHeight,
        block_time: ProtocolTime,
        confirmation_class: ConfirmationClass,
    ) -> Self {
        Self {
            chain_id,
            block_height,
            block_time,
            confirmation_class,
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        self.chain_id
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
}

#[derive(Debug, Clone, Copy)]
pub struct BlockDeltaEntry<'a> {
    key: &'a StateKey,
    block_start_value: Option<&'a [u8]>,
    block_final_value: Option<&'a [u8]>,
    write_count: u32,
}

impl<'a> BlockDeltaEntry<'a> {
    pub(crate) const fn new(
        key: &'a StateKey,
        block_start_value: Option<&'a [u8]>,
        block_final_value: Option<&'a [u8]>,
        write_count: u32,
    ) -> Self {
        Self {
            key,
            block_start_value,
            block_final_value,
            write_count,
        }
    }

    #[must_use]
    pub const fn key(&self) -> &StateKey {
        self.key
    }

    #[must_use]
    pub const fn block_start_value(&self) -> Option<&[u8]> {
        self.block_start_value
    }

    #[must_use]
    pub const fn block_final_value(&self) -> Option<&[u8]> {
        self.block_final_value
    }

    #[must_use]
    pub const fn write_count(&self) -> u32 {
        self.write_count
    }
}

#[derive(Debug, Clone, Copy)]
pub struct BlockDeltaView<'a> {
    entries: &'a [BlockDeltaEntry<'a>],
}

impl<'a> BlockDeltaView<'a> {
    pub(crate) const fn new(entries: &'a [BlockDeltaEntry<'a>]) -> Self {
        Self { entries }
    }

    #[must_use]
    pub const fn entries(&self) -> &[BlockDeltaEntry<'a>] {
        self.entries
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, BlockDeltaEntry<'a>> {
        self.entries.iter()
    }
}

impl<'view, 'entry> IntoIterator for &'view BlockDeltaView<'entry> {
    type Item = &'view BlockDeltaEntry<'entry>;
    type IntoIter = std::slice::Iter<'view, BlockDeltaEntry<'entry>>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

pub trait EventReducer {
    /// Immutable identifier for the exact reducer collection and semantics.
    fn reducer_set_version(&self) -> &str;

    /// Returns true only when this reducer owns the event kind and exact schema.
    fn supports(&self, event: &CanonicalEventEnvelope) -> bool;

    /// Prepares deterministic mutations against the state produced by all
    /// preceding events in the same block.
    fn reduce(
        &self,
        state: &StateView<'_>,
        event: &CanonicalEventEnvelope,
        context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError>;

    /// Runs block-wide invariants against the complete candidate state.
    fn validate_block(
        &self,
        _state: &StateView<'_>,
        _context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        Ok(())
    }

    /// Runs block-wide invariants with a bounded view of every touched key.
    fn validate_block_delta(
        &self,
        final_state: &StateView<'_>,
        _delta: &BlockDeltaView<'_>,
        context: &ApplyContext<'_>,
    ) -> Result<(), ReducerError> {
        self.validate_block(final_state, context)
    }
}
