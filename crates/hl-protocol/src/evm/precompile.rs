use domain_types::Address;

use super::{CoreWriterAction, EvmError, Hash32};

pub const CORE_WRITER_ADDRESS: Address = Address::from_bytes([0x33; 20]);
pub const READ_PRECOMPILE_BASE: u16 = 0x0800;
pub const READ_PRECOMPILE_LAST: u16 = 0x08ff;

#[must_use]
pub fn is_core_writer(address: Address) -> bool {
    address == CORE_WRITER_ADDRESS
}

#[must_use]
pub fn is_read_precompile(address: Address) -> bool {
    let bytes = address.as_bytes();
    if bytes[..18].iter().any(|byte| *byte != 0) {
        return false;
    }
    let suffix = u16::from_be_bytes([bytes[18], bytes[19]]);
    (READ_PRECOMPILE_BASE..=READ_PRECOMPILE_LAST).contains(&suffix)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreWriterCall {
    block_hash: Hash32,
    tx_hash: Hash32,
    action: CoreWriterAction,
}

impl CoreWriterCall {
    pub fn new(block_hash: Hash32, tx_hash: Hash32, input: &[u8]) -> Result<Self, EvmError> {
        Ok(Self {
            block_hash,
            tx_hash,
            action: CoreWriterAction::parse(input)?,
        })
    }

    #[must_use]
    pub const fn block_hash(&self) -> Hash32 {
        self.block_hash
    }

    #[must_use]
    pub const fn tx_hash(&self) -> Hash32 {
        self.tx_hash
    }

    #[must_use]
    pub const fn action(&self) -> &CoreWriterAction {
        &self.action
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompileObservation {
    block_hash: Hash32,
    address: Address,
    input: Vec<u8>,
    output: Vec<u8>,
}

impl PrecompileObservation {
    pub fn new(
        block_hash: Hash32,
        address: Address,
        input: Vec<u8>,
        output: Vec<u8>,
    ) -> Result<Self, EvmError> {
        if !is_read_precompile(address) {
            return Err(EvmError::InvalidAddress);
        }
        Ok(Self {
            block_hash,
            address,
            input,
            output,
        })
    }

    #[must_use]
    pub const fn block_hash(&self) -> Hash32 {
        self.block_hash
    }

    #[must_use]
    pub const fn address(&self) -> Address {
        self.address
    }

    #[must_use]
    pub fn input(&self) -> &[u8] {
        &self.input
    }

    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    #[must_use]
    pub fn precompile_offset(&self) -> u16 {
        let bytes = self.address.as_bytes();
        u16::from_be_bytes([bytes[18], bytes[19]])
    }
}
