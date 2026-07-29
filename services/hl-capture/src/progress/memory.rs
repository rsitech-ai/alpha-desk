use std::{
    collections::{BTreeMap, btree_map::Entry},
    sync::Mutex,
};

use async_trait::async_trait;
use domain_types::{BlockHeight, ChainId};
use storage_ports::{
    ArchivedBlockPlan, CaptureCursor, CaptureProgressStore, PlannedPublication, ProgressError,
    ProgressRecordDisposition, PublicationAcknowledgement,
};

#[derive(Debug)]
pub struct InMemoryProgressStore {
    capacity: usize,
    state: Mutex<StoreState>,
}

#[derive(Debug, Default)]
struct StoreState {
    chains: BTreeMap<ChainId, ChainState>,
    block_count: usize,
}

#[derive(Debug)]
struct ChainState {
    first_block_height: BlockHeight,
    cursor: Option<CaptureCursor>,
    blocks: BTreeMap<BlockHeight, BlockState>,
}

#[derive(Debug)]
struct BlockState {
    plan: ArchivedBlockPlan,
    acknowledgements: BTreeMap<u32, PublicationAcknowledgement>,
}

impl InMemoryProgressStore {
    pub fn new(capacity: usize) -> Result<Self, ProgressError> {
        if capacity == 0 {
            return Err(ProgressError::InvalidInput("zero progress capacity"));
        }
        Ok(Self {
            capacity,
            state: Mutex::new(StoreState::default()),
        })
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, StoreState>, ProgressError> {
        self.state
            .lock()
            .map_err(|_| ProgressError::Storage("in-memory lock poisoned"))
    }
}

#[async_trait]
impl CaptureProgressStore for InMemoryProgressStore {
    async fn initialize_chain(
        &self,
        chain_id: &ChainId,
        first_block_height: BlockHeight,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut state = self.lock()?;
        match state.chains.entry(chain_id.clone()) {
            Entry::Vacant(entry) => {
                entry.insert(ChainState {
                    first_block_height,
                    cursor: None,
                    blocks: BTreeMap::new(),
                });
                Ok(ProgressRecordDisposition::New)
            }
            Entry::Occupied(entry) if entry.get().first_block_height == first_block_height => {
                Ok(ProgressRecordDisposition::IdenticalDuplicate)
            }
            Entry::Occupied(_) => Err(ProgressError::ConflictingInitialization),
        }
    }

    async fn record_archived(
        &self,
        plan: &ArchivedBlockPlan,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut state = self.lock()?;
        let existing = state
            .chains
            .get(plan.chain_id())
            .ok_or(ProgressError::ChainNotInitialized)?;
        if plan.block_height() < existing.first_block_height {
            return Err(ProgressError::BelowFirstHeight);
        }
        if let Some(block) = existing.blocks.get(&plan.block_height()) {
            return if block.plan == *plan {
                Ok(ProgressRecordDisposition::IdenticalDuplicate)
            } else {
                Err(ProgressError::ConflictingBlock)
            };
        }
        if state.block_count >= self.capacity {
            return Err(ProgressError::CapacityExceeded {
                limit: self.capacity,
            });
        }
        let chain = state
            .chains
            .get_mut(plan.chain_id())
            .ok_or(ProgressError::ChainNotInitialized)?;
        chain.blocks.insert(
            plan.block_height(),
            BlockState {
                plan: plan.clone(),
                acknowledgements: BTreeMap::new(),
            },
        );
        state.block_count =
            state
                .block_count
                .checked_add(1)
                .ok_or(ProgressError::CapacityExceeded {
                    limit: self.capacity,
                })?;
        Ok(ProgressRecordDisposition::New)
    }

    async fn record_acknowledgement(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
        acknowledgement: &PublicationAcknowledgement,
    ) -> Result<ProgressRecordDisposition, ProgressError> {
        let mut state = self.lock()?;
        let block = state
            .chains
            .get_mut(chain_id)
            .ok_or(ProgressError::ChainNotInitialized)?
            .blocks
            .get_mut(&block_height)
            .ok_or(ProgressError::UnknownBlock)?;
        let publication = block
            .plan
            .publications()
            .get(
                usize::try_from(acknowledgement.ordinal())
                    .map_err(|_| ProgressError::AcknowledgementMismatch)?,
            )
            .ok_or(ProgressError::AcknowledgementMismatch)?;
        if !ack_matches_publication(acknowledgement, publication) {
            return Err(ProgressError::AcknowledgementMismatch);
        }
        match block.acknowledgements.entry(acknowledgement.ordinal()) {
            Entry::Vacant(entry) => {
                entry.insert(acknowledgement.clone());
                Ok(ProgressRecordDisposition::New)
            }
            Entry::Occupied(entry) if entry.get() == acknowledgement => {
                Ok(ProgressRecordDisposition::IdenticalDuplicate)
            }
            Entry::Occupied(_) => Err(ProgressError::ConflictingAcknowledgement),
        }
    }

    async fn advance_cursor(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<CaptureCursor, ProgressError> {
        let mut state = self.lock()?;
        let chain = state
            .chains
            .get_mut(chain_id)
            .ok_or(ProgressError::ChainNotInitialized)?;
        if let Some(cursor) = &chain.cursor
            && cursor.committed_block_height() == block_height
        {
            return Ok(cursor.clone());
        }
        let expected = match &chain.cursor {
            Some(cursor) => cursor
                .committed_block_height()
                .get()
                .checked_add(1)
                .map(BlockHeight::new)
                .ok_or(ProgressError::CursorOverflow)?,
            None => chain.first_block_height,
        };
        if block_height != expected {
            return Err(ProgressError::NonContiguousAdvance {
                expected,
                actual: block_height,
            });
        }
        let block = chain
            .blocks
            .get(&block_height)
            .ok_or(ProgressError::UnknownBlock)?;
        if block.acknowledgements.len() != block.plan.publications().len() {
            return Err(ProgressError::PublicationIncomplete);
        }
        let cursor_version = match &chain.cursor {
            Some(cursor) => cursor
                .cursor_version()
                .checked_add(1)
                .ok_or(ProgressError::CursorOverflow)?,
            None => 1,
        };
        let updated_at = block
            .acknowledgements
            .values()
            .map(PublicationAcknowledgement::acknowledged_at)
            .max()
            .ok_or(ProgressError::PublicationIncomplete)?;
        let cursor = CaptureCursor::try_new(
            chain_id.clone(),
            block_height,
            block.plan.canonical_block_hash(),
            block.plan.archive_receipt_id(),
            block.plan.archive_manifest_sha256(),
            cursor_version,
            updated_at,
        )?;
        chain.cursor = Some(cursor.clone());
        Ok(cursor)
    }

    async fn load_cursor(
        &self,
        chain_id: &ChainId,
    ) -> Result<Option<CaptureCursor>, ProgressError> {
        let state = self.lock()?;
        Ok(state
            .chains
            .get(chain_id)
            .ok_or(ProgressError::ChainNotInitialized)?
            .cursor
            .clone())
    }

    async fn next_expected_height(&self, chain_id: &ChainId) -> Result<BlockHeight, ProgressError> {
        let state = self.lock()?;
        let chain = state
            .chains
            .get(chain_id)
            .ok_or(ProgressError::ChainNotInitialized)?;
        match &chain.cursor {
            Some(cursor) => cursor
                .committed_block_height()
                .get()
                .checked_add(1)
                .map(BlockHeight::new)
                .ok_or(ProgressError::CursorOverflow),
            None => Ok(chain.first_block_height),
        }
    }

    async fn load_archived_block(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Option<ArchivedBlockPlan>, ProgressError> {
        let state = self.lock()?;
        Ok(state
            .chains
            .get(chain_id)
            .ok_or(ProgressError::ChainNotInitialized)?
            .blocks
            .get(&block_height)
            .map(|block| block.plan.clone()))
    }

    async fn load_acknowledgements(
        &self,
        chain_id: &ChainId,
        block_height: BlockHeight,
    ) -> Result<Vec<PublicationAcknowledgement>, ProgressError> {
        let state = self.lock()?;
        Ok(state
            .chains
            .get(chain_id)
            .ok_or(ProgressError::ChainNotInitialized)?
            .blocks
            .get(&block_height)
            .ok_or(ProgressError::UnknownBlock)?
            .acknowledgements
            .values()
            .cloned()
            .collect())
    }

    async fn pending_blocks(
        &self,
        chain_id: &ChainId,
        limit: usize,
    ) -> Result<Vec<ArchivedBlockPlan>, ProgressError> {
        if limit == 0 {
            return Err(ProgressError::InvalidLimit);
        }
        let state = self.lock()?;
        let chain = state
            .chains
            .get(chain_id)
            .ok_or(ProgressError::ChainNotInitialized)?;
        let committed = chain
            .cursor
            .as_ref()
            .map(CaptureCursor::committed_block_height);
        Ok(chain
            .blocks
            .iter()
            .filter(|(height, _)| committed.is_none_or(|value| **height > value))
            .take(limit)
            .map(|(_, block)| block.plan.clone())
            .collect())
    }
}

fn ack_matches_publication(
    acknowledgement: &PublicationAcknowledgement,
    publication: &PlannedPublication,
) -> bool {
    acknowledgement.ordinal() == publication.ordinal()
        && acknowledgement.message_id() == publication.message_id()
        && acknowledgement.subject() == publication.subject()
        && acknowledgement.publication_sha256() == publication.publication_sha256()
}
