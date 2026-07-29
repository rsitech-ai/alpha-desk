use std::collections::BTreeMap;

use domain_types::{BlockHeight, ChainId};

use crate::{LedgerError, StateImageError, StateKeyError, error::valid_reducer_version};

const STATE_IMAGE_SCHEMA: &[u8] = b"hyperliquid-alpha-desk/state-image/v1";
const STATE_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/state-hash/v1";
const MAX_ABSOLUTE_KEY_BYTES: usize = 64 * 1024;
const MAX_NAMESPACE_BYTES: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StateImageLimits {
    max_state_bytes: usize,
    max_entries: usize,
    max_key_bytes: usize,
    max_value_bytes: usize,
}

impl StateImageLimits {
    pub const fn try_new(
        max_state_bytes: usize,
        max_entries: usize,
        max_key_bytes: usize,
        max_value_bytes: usize,
    ) -> Result<Self, StateImageError> {
        if max_state_bytes == 0
            || max_entries == 0
            || max_key_bytes == 0
            || max_value_bytes == 0
            || max_key_bytes > MAX_ABSOLUTE_KEY_BYTES
        {
            return Err(StateImageError::InvalidLimits);
        }
        Ok(Self {
            max_state_bytes,
            max_entries,
            max_key_bytes,
            max_value_bytes,
        })
    }

    #[must_use]
    pub const fn production() -> Self {
        Self {
            max_state_bytes: 4 * 1_024 * 1_024 * 1_024,
            max_entries: 50_000_000,
            max_key_bytes: 4 * 1_024,
            max_value_bytes: 16 * 1_024 * 1_024,
        }
    }
}

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

    pub fn decode_canonical(
        bytes: &[u8],
        limits: StateImageLimits,
    ) -> Result<Self, StateImageError> {
        if bytes.len() > limits.max_state_bytes {
            return Err(StateImageError::LimitExceeded);
        }
        let mut cursor = StateCursor::new(bytes);
        if cursor.frame(128)? != STATE_IMAGE_SCHEMA {
            return Err(StateImageError::InvalidSchema);
        }
        let chain_id = decode_text(cursor.frame(256)?, "chain_id").and_then(|value| {
            ChainId::new(value).map_err(|_| StateImageError::InvalidField("chain_id"))
        })?;
        let first_height = BlockHeight::new(cursor.u64()?);
        let reducer_set_version = decode_text(cursor.frame(128)?, "reducer_set_version")?;
        if !valid_reducer_version(&reducer_set_version) {
            return Err(StateImageError::InvalidField("reducer_set_version"));
        }
        let watermark = match cursor.byte()? {
            0 => None,
            1 => {
                let block_height = BlockHeight::new(cursor.u64()?);
                if block_height < first_height {
                    return Err(StateImageError::InvalidField("block_height"));
                }
                Some(StateWatermark {
                    block_height,
                    canonical_block_hash: cursor.hash()?,
                })
            }
            _ => return Err(StateImageError::InvalidField("watermark_present")),
        };
        let entry_count = cursor.count(limits.max_entries)?;
        if watermark.is_none() && entry_count != 0 {
            return Err(StateImageError::InvalidField(
                "uncheckpointed state entries",
            ));
        }
        let mut entries = BTreeMap::new();
        let mut previous: Option<StateKey> = None;
        for _ in 0..entry_count {
            let namespace = decode_text(cursor.frame(MAX_NAMESPACE_BYTES)?, "state namespace")?;
            let key = StateKey::try_new(namespace, cursor.frame(limits.max_key_bytes)?.to_vec())
                .map_err(|_| StateImageError::InvalidField("state key"))?;
            if previous.as_ref().is_some_and(|prior| prior >= &key) {
                return Err(StateImageError::NonCanonicalOrder);
            }
            let value = cursor.frame(limits.max_value_bytes)?.to_vec();
            previous = Some(key.clone());
            entries.insert(key, value);
        }
        if !cursor.is_finished() {
            return Err(StateImageError::TrailingBytes);
        }
        Ok(Self {
            chain_id,
            first_height,
            reducer_set_version,
            watermark,
            entries,
        })
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

struct StateCursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> StateCursor<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], StateImageError> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or(StateImageError::LimitExceeded)?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or(StateImageError::Truncated)?;
        self.offset = end;
        Ok(value)
    }

    fn byte(&mut self) -> Result<u8, StateImageError> {
        Ok(self.take(1)?[0])
    }

    fn u64(&mut self) -> Result<u64, StateImageError> {
        let mut encoded = [0_u8; 8];
        encoded.copy_from_slice(self.take(8)?);
        Ok(u64::from_be_bytes(encoded))
    }

    fn hash(&mut self) -> Result<[u8; 32], StateImageError> {
        let mut hash = [0_u8; 32];
        hash.copy_from_slice(self.take(32)?);
        Ok(hash)
    }

    fn frame(&mut self, max_bytes: usize) -> Result<&'a [u8], StateImageError> {
        let length = usize::try_from(self.u64()?).map_err(|_| StateImageError::LimitExceeded)?;
        if length > max_bytes {
            return Err(StateImageError::LimitExceeded);
        }
        self.take(length)
    }

    fn count(&mut self, max_count: usize) -> Result<usize, StateImageError> {
        let count = usize::try_from(self.u64()?).map_err(|_| StateImageError::LimitExceeded)?;
        if count > max_count {
            return Err(StateImageError::LimitExceeded);
        }
        Ok(count)
    }

    const fn is_finished(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

fn decode_text(bytes: &[u8], field: &'static str) -> Result<String, StateImageError> {
    std::str::from_utf8(bytes)
        .map(str::to_owned)
        .map_err(|_| StateImageError::InvalidField(field))
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
