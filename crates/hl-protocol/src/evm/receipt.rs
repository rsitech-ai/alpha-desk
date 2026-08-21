use domain_types::Address;

use super::wire::{self, WireValue};
use super::{
    EvmChainId, EvmError, EvmLog, Hash32, RECEIPT_HASH_CONTEXT, Wei, hash_fact, optional_address,
    optional_wei, wei_from_wire,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReceiptStatus {
    Success,
    Failure,
}

impl ReceiptStatus {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::Failure => "failure",
        }
    }

    pub(crate) fn from_wire(value: Option<&WireValue>) -> Result<Self, EvmError> {
        match value {
            Some(WireValue::Bool(true) | WireValue::Uint(1) | WireValue::Int(1)) => {
                Ok(Self::Success)
            }
            Some(WireValue::Bool(false) | WireValue::Uint(0) | WireValue::Int(0)) => {
                Ok(Self::Failure)
            }
            Some(WireValue::String(text)) => match text.as_str() {
                "success" | "ok" | "0x1" | "1" => Ok(Self::Success),
                "failure" | "failed" | "0x0" | "0" => Ok(Self::Failure),
                other => Err(EvmError::SchemaDrift(format!(
                    "unknown receipt status {other}"
                ))),
            },
            Some(other) => match wire::as_bytes(other) {
                Ok(bytes) if bytes.iter().all(|byte| *byte == 0) => Ok(Self::Failure),
                Ok(bytes)
                    if bytes.last() == Some(&1) && bytes.iter().rev().skip(1).all(|b| *b == 0) =>
                {
                    Ok(Self::Success)
                }
                _ => Err(EvmError::SchemaDrift(
                    "receipt status is not 0 or 1".to_owned(),
                )),
            },
            None => Err(EvmError::SchemaDrift(
                "receipt is missing status".to_owned(),
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvmReceipt {
    chain_id: EvmChainId,
    block_hash: Hash32,
    block_number: u64,
    tx_hash: Hash32,
    tx_index: u32,
    status: ReceiptStatus,
    gas_used: Option<Wei>,
    cumulative_gas_used: Wei,
    contract_address: Option<Address>,
    logs: Vec<EvmLog>,
    fact_hash: [u8; 32],
}

impl EvmReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: EvmChainId,
        block_hash: Hash32,
        block_number: u64,
        tx_hash: Hash32,
        tx_index: u32,
        status: ReceiptStatus,
        gas_used: Option<Wei>,
        cumulative_gas_used: Wei,
        contract_address: Option<Address>,
        logs: Vec<EvmLog>,
    ) -> Self {
        let fact_hash = hash_receipt(chain_id, block_hash, tx_hash, tx_index, status, &logs);
        Self {
            chain_id,
            block_hash,
            block_number,
            tx_hash,
            tx_index,
            status,
            gas_used,
            cumulative_gas_used,
            contract_address,
            logs,
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
    pub const fn status(&self) -> ReceiptStatus {
        self.status
    }

    #[must_use]
    pub const fn gas_used(&self) -> Option<Wei> {
        self.gas_used
    }

    #[must_use]
    pub const fn cumulative_gas_used(&self) -> Wei {
        self.cumulative_gas_used
    }

    #[must_use]
    pub const fn contract_address(&self) -> Option<Address> {
        self.contract_address
    }

    #[must_use]
    pub fn logs(&self) -> &[EvmLog] {
        &self.logs
    }

    #[must_use]
    pub const fn fact_hash(&self) -> [u8; 32] {
        self.fact_hash
    }

    #[must_use]
    pub fn fact_id(&self) -> String {
        format!("evr_{}", hex::encode(self.fact_hash))
    }

    pub(crate) fn from_wire(
        value: &WireValue,
        chain_id: EvmChainId,
        block_hash: Hash32,
        block_number: u64,
        tx_hash: Hash32,
        tx_index: u32,
    ) -> Result<Self, EvmError> {
        let inner = match wire::tagged_enum(value) {
            Ok((_, nested)) => nested,
            Err(_) => value,
        };
        let map = inner.as_map()?;
        let status =
            ReceiptStatus::from_wire(wire::map_get(map, &["status", "success", "Status"]))?;
        let log_values = match wire::map_get(map, &["logs", "Logs"]) {
            Some(WireValue::Array(items)) => items.as_slice(),
            None => &[],
            Some(_) => {
                return Err(EvmError::MalformedPayload(
                    "receipt logs must be an array".to_owned(),
                ));
            }
        };
        let logs = log_values
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let log_index = u32::try_from(index).map_err(|_| EvmError::InvalidIdentity)?;
                EvmLog::from_wire(
                    item,
                    chain_id,
                    block_hash,
                    block_number,
                    tx_hash,
                    tx_index,
                    log_index,
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::new(
            chain_id,
            block_hash,
            block_number,
            tx_hash,
            tx_index,
            status,
            optional_wei(wire::map_get(map, &["gasUsed", "gas_used"]))?,
            wei_from_wire(wire::map_get(
                map,
                &["cumulativeGasUsed", "cumulative_gas_used"],
            ))?,
            optional_address(wire::map_get(map, &["contractAddress", "contract_address"]))?,
            logs,
        ))
    }

    pub(crate) fn to_wire(&self) -> WireValue {
        let status_value = match self.status {
            ReceiptStatus::Success => WireValue::Uint(1),
            ReceiptStatus::Failure => WireValue::Uint(0),
        };
        let mut fields = vec![
            (WireValue::String("status".to_owned()), status_value),
            (
                WireValue::String("cumulativeGasUsed".to_owned()),
                wire::bin_bytes(self.cumulative_gas_used.as_be_bytes()),
            ),
            (
                WireValue::String("logs".to_owned()),
                WireValue::Array(self.logs.iter().map(EvmLog::to_wire).collect()),
            ),
        ];
        if let Some(gas_used) = self.gas_used {
            fields.push((
                WireValue::String("gasUsed".to_owned()),
                wire::bin_bytes(gas_used.as_be_bytes()),
            ));
        }
        if let Some(address) = self.contract_address {
            fields.push((
                WireValue::String("contractAddress".to_owned()),
                wire::bin_bytes(address.as_bytes()),
            ));
        }
        WireValue::Map(fields)
    }
}

fn hash_receipt(
    chain_id: EvmChainId,
    block_hash: Hash32,
    tx_hash: Hash32,
    tx_index: u32,
    status: ReceiptStatus,
    logs: &[EvmLog],
) -> [u8; 32] {
    let status_byte = [u8::from(status == ReceiptStatus::Success)];
    let log_hashes: Vec<[u8; 32]> = logs.iter().map(EvmLog::fact_hash).collect();
    let mut log_bytes = Vec::with_capacity(log_hashes.len() * 32);
    for hash in &log_hashes {
        log_bytes.extend_from_slice(hash);
    }
    hash_fact(
        RECEIPT_HASH_CONTEXT,
        &[
            &chain_id.get().to_be_bytes(),
            block_hash.as_bytes(),
            tx_hash.as_bytes(),
            &tx_index.to_be_bytes(),
            &status_byte,
            &log_bytes,
        ],
    )
}
