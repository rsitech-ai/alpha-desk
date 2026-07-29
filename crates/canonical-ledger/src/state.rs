use std::collections::BTreeMap;

use domain_types::{BlockHeight, ChainId};

use crate::{LedgerError, StateKeyError};

const STATE_IMAGE_SCHEMA: &[u8] = b"hyperliquid-alpha-desk/state-image/v1";
const STATE_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/state-hash/v1";
const MAX_ABSOLUTE_KEY_BYTES: usize = 64 * 1024;
const MAX_NAMESPACE_BYTES: usize = 96;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct StateKey {
    namespace: String,
    key: Vec<u8>,
}

impl StateKey {
    pub fn try_new(namespace: impl Into<String>, key: Vec<u8>) -> Result<Self, StateKeyError> {
        let namespace = namespace.into();
        if !valid_namespace(&namespace) {
            return Err(StateKeyError::InvalidNamespace);
        }
        if key.is_empty() || key.len() > MAX_ABSOLUTE_KEY_BYTES {
            return Err(StateKeyError::InvalidKey);
        }
        Ok(Self { namespace, key })
    }

    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[must_use]
    pub fn key(&self) -> &[u8] {
        &self.key
    }

    pub(crate) fn encoded_len(&self) -> Result<usize, LedgerError> {
        self.namespace
            .len()
            .checked_add(self.key.len())
            .ok_or(LedgerError::MutationLimitExceeded)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateMutation {
    Put { key: StateKey, value: Vec<u8> },
    Delete { key: StateKey },
}

impl StateMutation {
    #[must_use]
    pub fn put(key: StateKey, value: Vec<u8>) -> Self {
        Self::Put { key, value }
    }

    #[must_use]
    pub const fn delete(key: StateKey) -> Self {
        Self::Delete { key }
    }

    #[must_use]
    pub const fn key(&self) -> &StateKey {
        match self {
            Self::Put { key, .. } | Self::Delete { key } => key,
        }
    }

    #[must_use]
    pub fn value(&self) -> Option<&[u8]> {
        match self {
            Self::Put { value, .. } => Some(value),
            Self::Delete { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMutation {
    key: StateKey,
    previous: Option<Vec<u8>>,
    current: Option<Vec<u8>>,
}

impl AppliedMutation {
    pub(crate) const fn new(
        key: StateKey,
        previous: Option<Vec<u8>>,
        current: Option<Vec<u8>>,
    ) -> Self {
        Self {
            key,
            previous,
            current,
        }
    }

    #[must_use]
    pub const fn key(&self) -> &StateKey {
        &self.key
    }

    #[must_use]
    pub fn previous(&self) -> Option<&[u8]> {
        self.previous.as_deref()
    }

    #[must_use]
    pub fn current(&self) -> Option<&[u8]> {
        self.current.as_deref()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StateWatermark {
    pub(crate) block_height: BlockHeight,
    pub(crate) canonical_block_hash: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StateImage {
    chain_id: ChainId,
    first_height: BlockHeight,
    reducer_set_version: String,
    watermark: Option<StateWatermark>,
    entries: BTreeMap<StateKey, Vec<u8>>,
}

impl StateImage {
    pub(crate) fn empty(
        chain_id: ChainId,
        first_height: BlockHeight,
        reducer_set_version: String,
    ) -> Self {
        Self {
            chain_id,
            first_height,
            reducer_set_version,
            watermark: None,
            entries: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn first_height(&self) -> BlockHeight {
        self.first_height
    }

    #[must_use]
    pub fn reducer_set_version(&self) -> &str {
        &self.reducer_set_version
    }

    #[must_use]
    pub const fn block_height(&self) -> Option<BlockHeight> {
        match self.watermark {
            Some(watermark) => Some(watermark.block_height),
            None => None,
        }
    }

    #[must_use]
    pub fn entries(&self) -> &BTreeMap<StateKey, Vec<u8>> {
        &self.entries
    }

    #[must_use]
    pub fn canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        frame(&mut bytes, STATE_IMAGE_SCHEMA);
        frame(&mut bytes, self.chain_id.as_str().as_bytes());
        bytes.extend_from_slice(&self.first_height.get().to_be_bytes());
        frame(&mut bytes, self.reducer_set_version.as_bytes());
        match self.watermark {
            Some(watermark) => {
                bytes.push(1);
                bytes.extend_from_slice(&watermark.block_height.get().to_be_bytes());
                bytes.extend_from_slice(&watermark.canonical_block_hash);
            }
            None => bytes.push(0),
        }
        extend_count(&mut bytes, self.entries.len());
        for (key, value) in &self.entries {
            frame(&mut bytes, key.namespace.as_bytes());
            frame(&mut bytes, &key.key);
            frame(&mut bytes, value);
        }
        bytes
    }

    #[must_use]
    pub fn state_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new_derive_key(STATE_HASH_CONTEXT);
        hasher.update(&self.canonical_bytes());
        *hasher.finalize().as_bytes()
    }

    pub(crate) fn candidate_entries(&self) -> BTreeMap<StateKey, Vec<u8>> {
        self.entries.clone()
    }

    pub(crate) fn commit(
        &mut self,
        watermark: StateWatermark,
        entries: BTreeMap<StateKey, Vec<u8>>,
    ) {
        self.watermark = Some(watermark);
        self.entries = entries;
    }

    pub(crate) const fn watermark(&self) -> Option<StateWatermark> {
        self.watermark
    }
}

#[derive(Debug, Clone, Copy)]
pub struct StateView<'a> {
    entries: &'a BTreeMap<StateKey, Vec<u8>>,
}

impl<'a> StateView<'a> {
    #[must_use]
    pub fn get(&self, key: &StateKey) -> Option<&'a [u8]> {
        self.entries.get(key).map(Vec::as_slice)
    }

    #[must_use]
    pub fn contains_key(&self, key: &StateKey) -> bool {
        self.entries.contains_key(key)
    }

    /// Iterates in canonical `StateKey` byte order.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = (&'a StateKey, &'a [u8])> + 'a {
        self.entries
            .iter()
            .map(|(key, value)| (key, value.as_slice()))
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

pub(crate) fn view_entries(entries: &BTreeMap<StateKey, Vec<u8>>) -> StateView<'_> {
    StateView { entries }
}

fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_NAMESPACE_BYTES
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .as_bytes()
            .last()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-' | b'_')
        })
}

fn extend_count(bytes: &mut Vec<u8>, count: usize) {
    let count = u64::try_from(count)
        .expect("state entry count cannot exceed the canonical u64 framing limit");
    bytes.extend_from_slice(&count.to_be_bytes());
}

fn frame(output: &mut Vec<u8>, value: &[u8]) {
    let length = u64::try_from(value.len())
        .expect("state value cannot exceed the canonical u64 framing limit");
    output.extend_from_slice(&length.to_be_bytes());
    output.extend_from_slice(value);
}
