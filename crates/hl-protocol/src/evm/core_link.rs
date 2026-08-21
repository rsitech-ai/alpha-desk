use domain_types::{Address, BlockHeight};

use super::{EvmChainId, EvmError, Hash32};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreEvmBlockLink {
    evm_chain_id: EvmChainId,
    evm_block_hash: Hash32,
    evm_block_number: u64,
    core_height: Option<BlockHeight>,
}

impl CoreEvmBlockLink {
    pub fn new(
        evm_chain_id: EvmChainId,
        evm_block_hash: Hash32,
        evm_block_number: u64,
        core_height: Option<BlockHeight>,
    ) -> Self {
        Self {
            evm_chain_id,
            evm_block_hash,
            evm_block_number,
            core_height,
        }
    }

    #[must_use]
    pub const fn evm_chain_id(&self) -> EvmChainId {
        self.evm_chain_id
    }

    #[must_use]
    pub const fn evm_block_hash(&self) -> Hash32 {
        self.evm_block_hash
    }

    #[must_use]
    pub const fn evm_block_number(&self) -> u64 {
        self.evm_block_number
    }

    #[must_use]
    pub const fn core_height(&self) -> Option<BlockHeight> {
        self.core_height
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenContractLink {
    token_id: u64,
    contract: Address,
    chain_id: EvmChainId,
}

impl TokenContractLink {
    pub fn new(token_id: u64, contract: Address, chain_id: EvmChainId) -> Self {
        Self {
            token_id,
            contract,
            chain_id,
        }
    }

    #[must_use]
    pub const fn token_id(&self) -> u64 {
        self.token_id
    }

    #[must_use]
    pub const fn contract(&self) -> Address {
        self.contract
    }

    #[must_use]
    pub const fn chain_id(&self) -> EvmChainId {
        self.chain_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreWriterAction {
    version: u8,
    action_id: u32,
    payload: Vec<u8>,
}

impl CoreWriterAction {
    pub fn parse(input: &[u8]) -> Result<Self, EvmError> {
        if input.len() < 4 {
            return Err(EvmError::MalformedPayload(
                "CoreWriter action shorter than version+id header".to_owned(),
            ));
        }
        Ok(Self {
            version: input[0],
            action_id: u32::from_be_bytes([0, input[1], input[2], input[3]]),
            payload: input[4..].to_vec(),
        })
    }

    #[must_use]
    pub const fn version(&self) -> u8 {
        self.version
    }

    #[must_use]
    pub const fn action_id(&self) -> u32 {
        self.action_id
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}
