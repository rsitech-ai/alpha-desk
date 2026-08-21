use domain_types::Address;

use super::wire::{self, WireValue};
use super::{EvmChainId, EvmError, Hash32, LOG_HASH_CONTEXT, address_from_wire, hash_fact};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EvmLogId {
    tx_hash: Hash32,
    log_index: u32,
}

impl EvmLogId {
    #[must_use]
    pub const fn new(tx_hash: Hash32, log_index: u32) -> Self {
        Self { tx_hash, log_index }
    }

    #[must_use]
    pub const fn tx_hash(&self) -> Hash32 {
        self.tx_hash
    }

    #[must_use]
    pub const fn log_index(&self) -> u32 {
        self.log_index
    }

    #[must_use]
    pub fn as_wire(&self) -> String {
        format!(
            "{}:{log_index}",
            self.tx_hash.to_api_string(),
            log_index = self.log_index
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvmLog {
    chain_id: EvmChainId,
    block_hash: Hash32,
    block_number: u64,
    tx_hash: Hash32,
    tx_index: u32,
    log_index: u32,
    address: Address,
    topics: Vec<Hash32>,
    data: Vec<u8>,
    fact_hash: [u8; 32],
}

impl EvmLog {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: EvmChainId,
        block_hash: Hash32,
        block_number: u64,
        tx_hash: Hash32,
        tx_index: u32,
        log_index: u32,
        address: Address,
        topics: Vec<Hash32>,
        data: Vec<u8>,
    ) -> Self {
        let fact_hash = hash_log(chain_id, tx_hash, log_index, address, &topics, &data);
        Self {
            chain_id,
            block_hash,
            block_number,
            tx_hash,
            tx_index,
            log_index,
            address,
            topics,
            data,
            fact_hash,
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> EvmChainId {
        self.chain_id
    }

    #[must_use]
    pub const fn block_hash(&self) -> Hash32 {
        self.block_hash
    }

    #[must_use]
    pub const fn block_number(&self) -> u64 {
        self.block_number
    }

    #[must_use]
    pub const fn tx_hash(&self) -> Hash32 {
        self.tx_hash
    }

    #[must_use]
    pub const fn tx_index(&self) -> u32 {
        self.tx_index
    }

    #[must_use]
    pub const fn log_index(&self) -> u32 {
        self.log_index
    }

    #[must_use]
    pub const fn address(&self) -> Address {
        self.address
    }

    #[must_use]
    pub fn topics(&self) -> &[Hash32] {
        &self.topics
    }

    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    #[must_use]
    pub const fn id(&self) -> EvmLogId {
        EvmLogId::new(self.tx_hash, self.log_index)
    }

    #[must_use]
    pub const fn fact_hash(&self) -> [u8; 32] {
        self.fact_hash
    }

    #[must_use]
    pub fn fact_id(&self) -> String {
        format!("evl_{}", hex::encode(self.fact_hash))
    }

    pub(crate) fn from_wire(
        value: &WireValue,
        chain_id: EvmChainId,
        block_hash: Hash32,
        block_number: u64,
        tx_hash: Hash32,
        tx_index: u32,
        log_index: u32,
    ) -> Result<Self, EvmError> {
        let map = value.as_map()?;
        let topics = match wire::map_get(map, &["topics", "Topics"]) {
            Some(WireValue::Array(items)) => items
                .iter()
                .map(|item| super::hash32_from_wire(Some(item)))
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(),
            Some(_) => {
                return Err(EvmError::MalformedPayload(
                    "log topics must be an array".to_owned(),
                ));
            }
        };
        let parsed_index = wire::map_get(map, &["index", "logIndex", "log_index"])
            .map(|value| super::u64_from_wire(Some(value)))
            .transpose()?
            .map(|value| u32::try_from(value).map_err(|_| EvmError::InvalidIdentity))
            .transpose()?
            .unwrap_or(log_index);
        Ok(Self::new(
            chain_id,
            block_hash,
            block_number,
            tx_hash,
            tx_index,
            parsed_index,
            address_from_wire(wire::map_get(map, &["address", "Address"]))?,
            topics,
            wire::map_get(map, &["data", "Data"])
                .map(wire::as_bytes)
                .transpose()?
                .unwrap_or_default(),
        ))
    }

    pub(crate) fn to_wire(&self) -> WireValue {
        WireValue::Map(vec![
            (
                WireValue::String("address".to_owned()),
                wire::bin_bytes(self.address.as_bytes()),
            ),
            (
                WireValue::String("topics".to_owned()),
                WireValue::Array(
                    self.topics
                        .iter()
                        .map(|topic| wire::bin_bytes(topic.as_bytes()))
                        .collect(),
                ),
            ),
            (
                WireValue::String("data".to_owned()),
                wire::bin_bytes(&self.data),
            ),
            (
                WireValue::String("index".to_owned()),
                WireValue::Uint(u64::from(self.log_index)),
            ),
        ])
    }
}

fn hash_log(
    chain_id: EvmChainId,
    tx_hash: Hash32,
    log_index: u32,
    address: Address,
    topics: &[Hash32],
    data: &[u8],
) -> [u8; 32] {
    let mut topic_bytes = Vec::with_capacity(topics.len() * 32);
    for topic in topics {
        topic_bytes.extend_from_slice(topic.as_bytes());
    }
    hash_fact(
        LOG_HASH_CONTEXT,
        &[
            &chain_id.get().to_be_bytes(),
            tx_hash.as_bytes(),
            &log_index.to_be_bytes(),
            address.as_bytes(),
            &topic_bytes,
            data,
        ],
    )
}
