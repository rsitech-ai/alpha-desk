use std::collections::BTreeSet;

use domain_types::{BlockRange, ChainId, ManifestId};

use crate::ReplayRequestError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReplayLimits {
    max_blocks: u64,
    max_manifests: usize,
}

impl ReplayLimits {
    pub const fn try_new(
        max_blocks: u64,
        max_manifests: usize,
    ) -> Result<Self, ReplayRequestError> {
        if max_blocks == 0 || max_manifests == 0 {
            return Err(ReplayRequestError::InvalidLimits);
        }
        Ok(Self {
            max_blocks,
            max_manifests,
        })
    }

    #[must_use]
    pub const fn production() -> Self {
        Self {
            max_blocks: 10_000_000,
            max_manifests: 100_000,
        }
    }

    pub(crate) const fn max_blocks(self) -> u64 {
        self.max_blocks
    }

    pub(crate) const fn max_manifests(self) -> usize {
        self.max_manifests
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayRequest {
    chain_id: ChainId,
    range: BlockRange,
    manifests: Vec<ManifestId>,
    expected_start_state_hash: [u8; 32],
    schema_dataset: String,
    expected_schema_fingerprint: [u8; 32],
}

impl ReplayRequest {
    pub fn try_new(
        chain_id: ChainId,
        range: BlockRange,
        manifests: Vec<ManifestId>,
        expected_start_state_hash: [u8; 32],
        schema_dataset: impl Into<String>,
        expected_schema_fingerprint: [u8; 32],
    ) -> Result<Self, ReplayRequestError> {
        let schema_dataset = schema_dataset.into();
        if manifests.is_empty()
            || expected_start_state_hash == [0_u8; 32]
            || expected_schema_fingerprint == [0_u8; 32]
            || !valid_dataset(&schema_dataset)
        {
            return Err(ReplayRequestError::InvalidRequest);
        }
        let mut unique = BTreeSet::new();
        if manifests.iter().any(|manifest| !unique.insert(manifest)) {
            return Err(ReplayRequestError::DuplicateManifest);
        }
        Ok(Self {
            chain_id,
            range,
            manifests,
            expected_start_state_hash,
            schema_dataset,
            expected_schema_fingerprint,
        })
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn range(&self) -> BlockRange {
        self.range
    }

    #[must_use]
    pub fn manifests(&self) -> &[ManifestId] {
        &self.manifests
    }

    #[must_use]
    pub const fn expected_start_state_hash(&self) -> [u8; 32] {
        self.expected_start_state_hash
    }

    pub(crate) fn schema_dataset(&self) -> &str {
        &self.schema_dataset
    }

    pub(crate) const fn expected_schema_fingerprint(&self) -> [u8; 32] {
        self.expected_schema_fingerprint
    }

    pub(crate) fn block_count(&self) -> Result<u64, ReplayRequestError> {
        self.range
            .end_inclusive
            .get()
            .checked_sub(self.range.start_inclusive.get())
            .and_then(|span| span.checked_add(1))
            .ok_or(ReplayRequestError::InvalidRequest)
    }
}

fn valid_dataset(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}
