use std::collections::{BTreeMap, BTreeSet};

use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId};

use crate::{
    AppliedMutation, ApplyContext, EventReducer, LedgerError, StateImage, StateKey, StateMutation,
    error::valid_reducer_version,
    state::{StateWatermark, view_entries},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerLimits {
    max_events_per_block: usize,
    max_mutations_per_event: usize,
    max_key_bytes: usize,
    max_value_bytes: usize,
    max_block_delta_bytes: usize,
}

impl LedgerLimits {
    pub fn try_new(
        max_events_per_block: usize,
        max_mutations_per_event: usize,
        max_key_bytes: usize,
        max_value_bytes: usize,
        max_block_delta_bytes: usize,
    ) -> Result<Self, LedgerError> {
        if [
            max_events_per_block,
            max_mutations_per_event,
            max_key_bytes,
            max_value_bytes,
            max_block_delta_bytes,
        ]
        .contains(&0)
            || max_key_bytes > max_block_delta_bytes
            || max_value_bytes > max_block_delta_bytes
        {
            return Err(LedgerError::InvalidLimits);
        }
        Ok(Self {
            max_events_per_block,
            max_mutations_per_event,
            max_key_bytes,
            max_value_bytes,
            max_block_delta_bytes,
        })
    }

    #[must_use]
    pub const fn production() -> Self {
        Self {
            max_events_per_block: 100_000,
            max_mutations_per_event: 4_096,
            max_key_bytes: 4 * 1_024,
            max_value_bytes: 16 * 1_024 * 1_024,
            max_block_delta_bytes: 256 * 1_024 * 1_024,
        }
    }
}

impl Default for LedgerLimits {
    fn default() -> Self {
        Self::production()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateCheckpoint {
    chain_id: ChainId,
    block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
    state_hash: [u8; 32],
    reducer_set_version: String,
}

impl StateCheckpoint {
    pub(crate) fn from_parts(
        chain_id: ChainId,
        block_height: BlockHeight,
        canonical_block_hash: [u8; 32],
        state_hash: [u8; 32],
        reducer_set_version: String,
    ) -> Self {
        Self {
            chain_id,
            block_height,
            canonical_block_hash,
            state_hash,
            reducer_set_version,
        }
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
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }

    #[must_use]
    pub const fn state_hash(&self) -> [u8; 32] {
        self.state_hash
    }

    #[must_use]
    pub fn reducer_set_version(&self) -> &str {
        &self.reducer_set_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateDelta {
    checkpoint: StateCheckpoint,
    before_state_hash: [u8; 32],
    mutations: Vec<AppliedMutation>,
    event_count: u64,
}

impl StateDelta {
    #[must_use]
    pub const fn checkpoint(&self) -> &StateCheckpoint {
        &self.checkpoint
    }

    #[must_use]
    pub const fn before_state_hash(&self) -> [u8; 32] {
        self.before_state_hash
    }

    #[must_use]
    pub const fn after_state_hash(&self) -> [u8; 32] {
        self.checkpoint.state_hash
    }

    #[must_use]
    pub fn mutations(&self) -> &[AppliedMutation] {
        &self.mutations
    }

    #[must_use]
    pub const fn event_count(&self) -> u64 {
        self.event_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyOutcome {
    Applied(StateDelta),
    AlreadyApplied(StateCheckpoint),
}

#[derive(Debug)]
pub struct PreparedBlock {
    next_state: StateImage,
    delta: StateDelta,
}

impl PreparedBlock {
    #[must_use]
    pub const fn state_image(&self) -> &StateImage {
        &self.next_state
    }

    #[must_use]
    pub const fn delta(&self) -> &StateDelta {
        &self.delta
    }
}

#[derive(Debug)]
pub enum PrepareOutcome {
    Ready(PreparedBlock),
    AlreadyApplied(StateCheckpoint),
}

#[derive(Debug)]
pub struct CanonicalLedger<R> {
    reducer: R,
    limits: LedgerLimits,
    state: StateImage,
    state_hash: [u8; 32],
}

impl<R: EventReducer> CanonicalLedger<R> {
    pub fn try_new(
        chain_id: ChainId,
        first_height: BlockHeight,
        reducer: R,
        limits: LedgerLimits,
    ) -> Result<Self, LedgerError> {
        let reducer_set_version = reducer.reducer_set_version();
        if !valid_reducer_version(reducer_set_version) {
            return Err(LedgerError::InvalidReducerVersion);
        }
        let state = StateImage::empty(chain_id, first_height, reducer_set_version.to_owned());
        let state_hash = state.state_hash();
        Ok(Self {
            reducer,
            limits,
            state,
            state_hash,
        })
    }

    pub fn try_from_state_image(
        state: StateImage,
        reducer: R,
        limits: LedgerLimits,
    ) -> Result<Self, LedgerError> {
        let reducer_set_version = reducer.reducer_set_version();
        if !valid_reducer_version(reducer_set_version) {
            return Err(LedgerError::InvalidReducerVersion);
        }
        if reducer_set_version != state.reducer_set_version() {
            return Err(LedgerError::ReducerVersionDrift);
        }
        let state_hash = state.state_hash();
        Ok(Self {
            reducer,
            limits,
            state,
            state_hash,
        })
    }

    #[must_use]
    pub const fn state_image(&self) -> &StateImage {
        &self.state
    }

    #[must_use]
    pub const fn state_hash(&self) -> [u8; 32] {
        self.state_hash
    }

    #[must_use]
    pub fn checkpoint(&self) -> Option<StateCheckpoint> {
        self.state
            .watermark()
            .map(|watermark| self.checkpoint_from(watermark, self.state_hash))
    }

    pub fn next_height(&self) -> Result<BlockHeight, LedgerError> {
        match self.state.watermark() {
            Some(watermark) => watermark
                .block_height
                .get()
                .checked_add(1)
                .map(BlockHeight::new)
                .ok_or(LedgerError::HeightExhausted),
            None => Ok(self.state.first_height()),
        }
    }

    pub fn apply_block(&mut self, block: &BlockEnvelope) -> Result<ApplyOutcome, LedgerError> {
        match self.prepare_block(block)? {
            PrepareOutcome::Ready(prepared) => {
                self.commit_prepared(prepared).map(ApplyOutcome::Applied)
            }
            PrepareOutcome::AlreadyApplied(checkpoint) => {
                Ok(ApplyOutcome::AlreadyApplied(checkpoint))
            }
        }
    }

    pub fn prepare_block(&self, block: &BlockEnvelope) -> Result<PrepareOutcome, LedgerError> {
        if self.reducer.reducer_set_version() != self.state.reducer_set_version() {
            return Err(LedgerError::ReducerVersionDrift);
        }
        self.validate_boundary(block)?;
        if let Some(checkpoint) = self.duplicate_checkpoint(block)? {
            return Ok(PrepareOutcome::AlreadyApplied(checkpoint));
        }
        self.validate_next_height(block.block_height())?;
        if block.events().len() > self.limits.max_events_per_block {
            return Err(LedgerError::MutationLimitExceeded);
        }
        let event_count =
            u64::try_from(block.events().len()).map_err(|_| LedgerError::MutationLimitExceeded)?;

        let before_state_hash = self.state_hash;
        let mut candidate = self.state.candidate_entries();
        let mut applied_mutations = Vec::new();
        let mut block_delta_bytes = 0_usize;
        let context = ApplyContext::new(
            block.chain_id(),
            block.block_height(),
            block.block_time(),
            block.confirmation_class(),
        );

        for event in block.events() {
            if !self.reducer.supports(event) {
                return Err(LedgerError::UnsupportedEvent {
                    kind: event.event_kind(),
                    schema_version: event.schema_version().to_owned(),
                });
            }
            let mutations = self
                .reducer
                .reduce(&view_entries(&candidate), event, &context)
                .map_err(|source| LedgerError::ReducerFailed { source })?;
            self.validate_mutations(&mutations, &mut block_delta_bytes)?;
            apply_mutations(&mut candidate, mutations, &mut applied_mutations)?;
        }

        self.reducer
            .validate_block(&view_entries(&candidate), &context)
            .map_err(|source| LedgerError::ReducerFailed { source })?;

        let watermark = StateWatermark {
            block_height: block.block_height(),
            canonical_block_hash: block.canonical_block_hash(),
        };
        let mut next_state = self.state.clone();
        next_state.commit(watermark, candidate);
        let after_state_hash = next_state.state_hash();
        let checkpoint = self.checkpoint_from(watermark, after_state_hash);

        Ok(PrepareOutcome::Ready(PreparedBlock {
            next_state,
            delta: StateDelta {
                checkpoint,
                before_state_hash,
                mutations: applied_mutations,
                event_count,
            },
        }))
    }

    pub fn commit_prepared(&mut self, prepared: PreparedBlock) -> Result<StateDelta, LedgerError> {
        if self.state_hash != prepared.delta.before_state_hash {
            return Err(LedgerError::PreparedStateDrift);
        }
        self.state = prepared.next_state;
        self.state_hash = prepared.delta.after_state_hash();
        Ok(prepared.delta)
    }

    fn validate_boundary(&self, block: &BlockEnvelope) -> Result<(), LedgerError> {
        if block.chain_id() != self.state.chain_id() {
            return Err(LedgerError::ChainMismatch);
        }
        if !matches!(
            block.confirmation_class(),
            ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent
        ) {
            return Err(LedgerError::NonCommittedBlock);
        }
        Ok(())
    }

    fn duplicate_checkpoint(
        &self,
        block: &BlockEnvelope,
    ) -> Result<Option<StateCheckpoint>, LedgerError> {
        let Some(watermark) = self.state.watermark() else {
            return Ok(None);
        };
        if block.block_height() != watermark.block_height {
            return Ok(None);
        }
        if block.canonical_block_hash() != watermark.canonical_block_hash {
            return Err(LedgerError::CanonicalDivergence);
        }
        Ok(Some(self.checkpoint_from(watermark, self.state_hash)))
    }

    fn validate_next_height(&self, actual: BlockHeight) -> Result<(), LedgerError> {
        let expected = self.next_height()?;
        if actual != expected {
            return Err(LedgerError::HeightDiscontinuity { expected, actual });
        }
        Ok(())
    }

    fn validate_mutations(
        &self,
        mutations: &[StateMutation],
        block_delta_bytes: &mut usize,
    ) -> Result<(), LedgerError> {
        if mutations.len() > self.limits.max_mutations_per_event {
            return Err(LedgerError::MutationLimitExceeded);
        }
        let mut event_keys = BTreeSet::new();
        for mutation in mutations {
            let key = mutation.key();
            if !event_keys.insert(key) {
                return Err(LedgerError::InvalidMutation {
                    reason: "one event produced multiple mutations for the same state key",
                });
            }
            let key_bytes = key.encoded_len()?;
            if key_bytes > self.limits.max_key_bytes {
                return Err(LedgerError::MutationLimitExceeded);
            }
            let value_bytes = mutation.value().map_or(0, <[u8]>::len);
            if value_bytes > self.limits.max_value_bytes {
                return Err(LedgerError::MutationLimitExceeded);
            }
            *block_delta_bytes = block_delta_bytes
                .checked_add(key_bytes)
                .and_then(|size| size.checked_add(value_bytes))
                .ok_or(LedgerError::MutationLimitExceeded)?;
            if *block_delta_bytes > self.limits.max_block_delta_bytes {
                return Err(LedgerError::MutationLimitExceeded);
            }
        }
        Ok(())
    }

    fn checkpoint_from(&self, watermark: StateWatermark, state_hash: [u8; 32]) -> StateCheckpoint {
        StateCheckpoint {
            chain_id: self.state.chain_id().clone(),
            block_height: watermark.block_height,
            canonical_block_hash: watermark.canonical_block_hash,
            state_hash,
            reducer_set_version: self.state.reducer_set_version().to_owned(),
        }
    }
}

fn apply_mutations(
    candidate: &mut BTreeMap<StateKey, Vec<u8>>,
    mutations: Vec<StateMutation>,
    applied: &mut Vec<AppliedMutation>,
) -> Result<(), LedgerError> {
    for mutation in mutations {
        match mutation {
            StateMutation::Put { key, value } => {
                let previous = candidate.insert(key.clone(), value.clone());
                applied.push(AppliedMutation::new(key, previous, Some(value)));
            }
            StateMutation::Delete { key } => {
                let previous = candidate.remove(&key).ok_or(LedgerError::InvalidMutation {
                    reason: "cannot delete a missing state key",
                })?;
                applied.push(AppliedMutation::new(key, Some(previous), None));
            }
        }
    }
    Ok(())
}
