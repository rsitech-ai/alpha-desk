use std::collections::BTreeMap;

use canonical_events::BlockEnvelope;
use domain_types::BlockHeight;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RetainedBlock {
    pub block: BlockEnvelope,
}

#[derive(Debug)]
pub(super) struct Watermark {
    current: Option<BlockHeight>,
    retained: BTreeMap<BlockHeight, RetainedBlock>,
    retained_limit: usize,
}

impl Watermark {
    pub(super) fn new(retained_limit: usize) -> Self {
        Self {
            current: None,
            retained: BTreeMap::new(),
            retained_limit,
        }
    }

    pub(super) const fn current(&self) -> Option<BlockHeight> {
        self.current
    }

    pub(super) fn retained_mut(&mut self, height: BlockHeight) -> Option<&mut RetainedBlock> {
        self.retained.get_mut(&height)
    }

    pub(super) fn advance(&mut self, block: BlockEnvelope) {
        let height = block.block_height();
        self.current = Some(height);
        self.retained.insert(height, RetainedBlock { block });
        while self.retained.len() > self.retained_limit {
            let Some(oldest) = self.retained.keys().next().copied() else {
                break;
            };
            self.retained.remove(&oldest);
        }
    }
}
