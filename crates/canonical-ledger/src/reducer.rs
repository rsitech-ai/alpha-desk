use canonical_events::{CanonicalEventEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, ProtocolTime};

use crate::{ReducerError, StateMutation, StateView};

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
}
