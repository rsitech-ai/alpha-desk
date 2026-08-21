use std::collections::BTreeMap;

use serde_json::Value as JsonValue;

use super::wire::{self, WireValue};
use super::{
    BLOCK_HASH_CONTEXT, EvmChainId, EvmError, EvmLog, EvmReceipt, EvmTransaction, Hash32,
    SystemTransaction, Wei, address_from_wire, chain_from_tx, hash_fact, hash32_from_wire,
    optional_hash32, optional_wei, u64_from_wire, wei_from_wire,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlockPace {
    Fast,
    Slow,
    Unknown,
}

impl BlockPace {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Slow => "slow",
            Self::Unknown => "unknown",
        }
    }

    pub fn parse_wire(value: &str) -> Self {
        match value {
            "fast" | "small" => Self::Fast,
            "slow" | "big" | "large" => Self::Slow,
            _ => Self::Unknown,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvmHeader {
    hash: Hash32,
    parent_hash: Hash32,
    number: u64,
    timestamp: u64,
    miner: domain_types::Address,
    sha3_uncles: Hash32,
    state_root: Hash32,
    transactions_root: Hash32,
    receipts_root: Hash32,
    extra_data: Vec<u8>,
    gas_limit: Wei,
    gas_used: Wei,
    base_fee_per_gas: Option<Wei>,
    logs_bloom: Option<Vec<u8>>,
    mix_hash: Option<Hash32>,
    nonce: Option<Vec<u8>>,
    withdrawals_root: Option<Hash32>,
    blob_gas_used: Option<Wei>,
    excess_blob_gas: Option<Wei>,
    parent_beacon_block_root: Option<Hash32>,
    requests_hash: Option<Hash32>,
    extra: BTreeMap<String, JsonValue>,
}

impl EvmHeader {
    pub fn new(
        hash: Hash32,
        parent_hash: Hash32,
        number: u64,
        timestamp: u64,
        miner: domain_types::Address,
    ) -> Self {
        let zero = Hash32::from_bytes([0; 32]);
        Self {
            hash,
            parent_hash,
            number,
            timestamp,
            miner,
            sha3_uncles: zero,
            state_root: zero,
            transactions_root: zero,
            receipts_root: zero,
            extra_data: Vec::new(),
            gas_limit: Wei::from_u64(30_000_000),
            gas_used: Wei::ZERO,
            base_fee_per_gas: None,
            logs_bloom: None,
            mix_hash: None,
            nonce: None,
            withdrawals_root: None,
            blob_gas_used: None,
            excess_blob_gas: None,
            parent_beacon_block_root: None,
            requests_hash: None,
            extra: BTreeMap::new(),
        }
    }

    #[must_use]
    pub const fn hash(&self) -> Hash32 {
        self.hash
    }

    #[must_use]
    pub const fn parent_hash(&self) -> Hash32 {
        self.parent_hash
    }

    #[must_use]
    pub const fn number(&self) -> u64 {
        self.number
    }

    #[must_use]
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }

    #[must_use]
    pub const fn miner(&self) -> domain_types::Address {
        self.miner
    }

    #[must_use]
    pub const fn sha3_uncles(&self) -> Hash32 {
        self.sha3_uncles
    }

    #[must_use]
    pub const fn state_root(&self) -> Hash32 {
        self.state_root
    }

    #[must_use]
    pub const fn transactions_root(&self) -> Hash32 {
        self.transactions_root
    }

    #[must_use]
    pub const fn receipts_root(&self) -> Hash32 {
        self.receipts_root
    }

    #[must_use]
    pub fn extra_data(&self) -> &[u8] {
        &self.extra_data
    }

    #[must_use]
    pub const fn gas_limit(&self) -> Wei {
        self.gas_limit
    }

    #[must_use]
    pub const fn gas_used(&self) -> Wei {
        self.gas_used
    }

    #[must_use]
    pub const fn base_fee_per_gas(&self) -> Option<Wei> {
        self.base_fee_per_gas
    }

    #[must_use]
    pub fn extra(&self) -> &BTreeMap<String, JsonValue> {
        &self.extra
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvmBlock {
    chain_id: EvmChainId,
    header: EvmHeader,
    pace: BlockPace,
    fact_hash: [u8; 32],
}

impl EvmBlock {
    pub fn new(chain_id: EvmChainId, header: EvmHeader, pace: BlockPace) -> Self {
        let fact_hash = hash_block(chain_id, &header);
        Self {
            chain_id,
            header,
            pace,
            fact_hash,
        }
    }

    #[must_use]
    pub const fn chain_id(&self) -> EvmChainId {
        self.chain_id
    }

    #[must_use]
    pub const fn header(&self) -> &EvmHeader {
        &self.header
    }

    #[must_use]
    pub const fn number(&self) -> u64 {
        self.header.number
    }

    #[must_use]
    pub const fn hash(&self) -> Hash32 {
        self.header.hash
    }

    #[must_use]
    pub const fn parent_hash(&self) -> Hash32 {
        self.header.parent_hash
    }

    #[must_use]
    pub const fn pace(&self) -> BlockPace {
        self.pace
    }

    #[must_use]
    pub const fn fact_hash(&self) -> [u8; 32] {
        self.fact_hash
    }

    #[must_use]
    pub fn fact_id(&self) -> String {
        format!("evb_{}", hex::encode(self.fact_hash))
    }

    #[must_use]
    pub fn is_parent_of(&self, child: &Self) -> bool {
        child.chain_id == self.chain_id
            && child.parent_hash() == self.hash()
            && child.number() == self.number().saturating_add(1)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvmBlockAndReceipts {
    block: EvmBlock,
    transactions: Vec<EvmTransaction>,
    receipts: Vec<EvmReceipt>,
    system_transactions: Vec<SystemTransaction>,
}

impl EvmBlockAndReceipts {
    pub fn new(
        block: EvmBlock,
        transactions: Vec<EvmTransaction>,
        receipts: Vec<EvmReceipt>,
        system_transactions: Vec<SystemTransaction>,
    ) -> Result<Self, EvmError> {
        let chain_id = block.chain_id();
        let block_hash = block.hash();
        let block_number = block.number();
        for tx in &transactions {
            if tx.chain_id() != chain_id
                || tx.block_hash() != block_hash
                || tx.block_number() != block_number
            {
                return Err(EvmError::SchemaDrift(
                    "transaction coordinates do not match the enclosing block".to_owned(),
                ));
            }
        }
        for receipt in &receipts {
            if receipt.chain_id() != chain_id
                || receipt.block_hash() != block_hash
                || receipt.block_number() != block_number
            {
                return Err(EvmError::SchemaDrift(
                    "receipt coordinates do not match the enclosing block".to_owned(),
                ));
            }
        }
        Ok(Self {
            block,
            transactions,
            receipts,
            system_transactions,
        })
    }

    #[must_use]
    pub const fn block(&self) -> &EvmBlock {
        &self.block
    }

    #[must_use]
    pub fn transactions(&self) -> &[EvmTransaction] {
        &self.transactions
    }

    #[must_use]
    pub fn receipts(&self) -> &[EvmReceipt] {
        &self.receipts
    }

    #[must_use]
    pub fn system_transactions(&self) -> &[SystemTransaction] {
        &self.system_transactions
    }

    #[must_use]
    pub fn logs(&self) -> impl Iterator<Item = &EvmLog> {
        self.receipts.iter().flat_map(EvmReceipt::logs)
    }

    pub(crate) fn from_wire(
        value: &WireValue,
        fallback_chain: EvmChainId,
    ) -> Result<Self, EvmError> {
        let root = value.as_map()?;
        let block_value = wire::map_get(root, &["block"])
            .ok_or_else(|| EvmError::MalformedPayload("archive object missing block".to_owned()))?;
        let reth = match wire::tagged_enum(block_value) {
            Ok(("Reth115", inner)) => inner,
            Ok((tag, _)) => {
                return Err(EvmError::SchemaDrift(format!(
                    "unsupported HyperEVM block codec {tag}"
                )));
            }
            Err(_) => block_value,
        };
        let reth_map = reth.as_map()?;
        let sealed = wire::map_get(reth_map, &["header"])
            .ok_or_else(|| EvmError::MalformedPayload("block missing header".to_owned()))?
            .as_map()?;
        let hash = hash32_from_wire(wire::map_get(sealed, &["hash"]))?;
        let header_map = match wire::map_get(sealed, &["header"]) {
            Some(nested) => nested.as_map()?,
            None => sealed,
        };
        let extra = wire::leftover_json(
            header_map,
            &[
                "parentHash",
                "parent_hash",
                "sha3Uncles",
                "sha3_uncles",
                "ommersHash",
                "miner",
                "beneficiary",
                "stateRoot",
                "state_root",
                "transactionsRoot",
                "transactions_root",
                "receiptsRoot",
                "receipts_root",
                "number",
                "timestamp",
                "extraData",
                "extra_data",
                "gasLimit",
                "gas_limit",
                "gasUsed",
                "gas_used",
                "baseFeePerGas",
                "base_fee_per_gas",
                "logsBloom",
                "logs_bloom",
                "mixHash",
                "mix_hash",
                "nonce",
                "withdrawalsRoot",
                "withdrawals_root",
                "blobGasUsed",
                "blob_gas_used",
                "excessBlobGas",
                "excess_blob_gas",
                "parentBeaconBlockRoot",
                "parent_beacon_block_root",
                "requestsHash",
                "requests_hash",
            ],
        );
        let header = EvmHeader {
            hash,
            parent_hash: hash32_from_wire(wire::map_get(
                header_map,
                &["parentHash", "parent_hash"],
            ))?,
            number: u64_from_wire(wire::map_get(header_map, &["number"]))?,
            timestamp: u64_from_wire(wire::map_get(header_map, &["timestamp"]))?,
            miner: address_from_wire(wire::map_get(header_map, &["miner", "beneficiary"]))?,
            sha3_uncles: hash32_from_wire(wire::map_get(
                header_map,
                &["sha3Uncles", "sha3_uncles", "ommersHash"],
            ))?,
            state_root: hash32_from_wire(wire::map_get(header_map, &["stateRoot", "state_root"]))?,
            transactions_root: hash32_from_wire(wire::map_get(
                header_map,
                &["transactionsRoot", "transactions_root"],
            ))?,
            receipts_root: hash32_from_wire(wire::map_get(
                header_map,
                &["receiptsRoot", "receipts_root"],
            ))?,
            extra_data: wire::map_get(header_map, &["extraData", "extra_data"])
                .map(wire::as_bytes)
                .transpose()?
                .unwrap_or_default(),
            gas_limit: wei_from_wire(wire::map_get(header_map, &["gasLimit", "gas_limit"]))?,
            gas_used: wei_from_wire(wire::map_get(header_map, &["gasUsed", "gas_used"]))?,
            base_fee_per_gas: optional_wei(wire::map_get(
                header_map,
                &["baseFeePerGas", "base_fee_per_gas"],
            ))?,
            logs_bloom: wire::map_get(header_map, &["logsBloom", "logs_bloom"])
                .map(wire::as_bytes)
                .transpose()?,
            mix_hash: optional_hash32(wire::map_get(header_map, &["mixHash", "mix_hash"]))?,
            nonce: wire::map_get(header_map, &["nonce"])
                .map(wire::as_bytes)
                .transpose()?,
            withdrawals_root: optional_hash32(wire::map_get(
                header_map,
                &["withdrawalsRoot", "withdrawals_root"],
            ))?,
            blob_gas_used: optional_wei(wire::map_get(
                header_map,
                &["blobGasUsed", "blob_gas_used"],
            ))?,
            excess_blob_gas: optional_wei(wire::map_get(
                header_map,
                &["excessBlobGas", "excess_blob_gas"],
            ))?,
            parent_beacon_block_root: optional_hash32(wire::map_get(
                header_map,
                &["parentBeaconBlockRoot", "parent_beacon_block_root"],
            ))?,
            requests_hash: optional_hash32(wire::map_get(
                header_map,
                &["requestsHash", "requests_hash"],
            ))?,
            extra,
        };
        let pace = wire::map_get(root, &["pace", "blockType", "block_type", "usingBigBlocks"])
            .and_then(wire::as_str)
            .map(BlockPace::parse_wire)
            .unwrap_or(BlockPace::Unknown);

        let empty_body = WireValue::Map(Vec::new());
        let body = wire::map_get(reth_map, &["body"]).unwrap_or(&empty_body);
        let body_map = match body {
            WireValue::Map(entries) => entries.as_slice(),
            _ => &[],
        };
        let tx_values = match wire::map_get(body_map, &["transactions"]) {
            Some(WireValue::Array(items)) => items.as_slice(),
            None => &[],
            Some(_) => {
                return Err(EvmError::MalformedPayload(
                    "block transactions must be an array".to_owned(),
                ));
            }
        };

        let mut tx_chain = None;
        let mut transactions = Vec::with_capacity(tx_values.len());
        for (index, tx_value) in tx_values.iter().enumerate() {
            let tx_index = u32::try_from(index).map_err(|_| EvmError::InvalidIdentity)?;
            let parsed = EvmTransaction::from_wire(
                tx_value,
                fallback_chain,
                header.hash,
                header.number,
                tx_index,
            )?;
            tx_chain = Some(parsed.chain_id().get());
            transactions.push(parsed);
        }
        let chain_id = chain_from_tx(tx_chain, fallback_chain)?;
        let block = EvmBlock::new(chain_id, header, pace);

        let receipts = parse_receipts(
            wire::map_get(root, &["receipts", "Receipts"]),
            chain_id,
            block.hash(),
            block.number(),
            &transactions,
        )?;
        let system_transactions = parse_system_txs(
            wire::map_get(root, &["systemTransactions", "system_txs", "systemTxs"]),
            chain_id,
            block.hash(),
            block.number(),
        )?;

        Self::new(block, transactions, receipts, system_transactions)
    }

    pub(crate) fn to_wire(&self) -> WireValue {
        let header = self.block.header();
        let mut inner_values = vec![
            (
                WireValue::String("parentHash".to_owned()),
                wire::bin_bytes(header.parent_hash.as_bytes()),
            ),
            (
                WireValue::String("sha3Uncles".to_owned()),
                wire::bin_bytes(header.sha3_uncles.as_bytes()),
            ),
            (
                WireValue::String("miner".to_owned()),
                wire::bin_bytes(header.miner.as_bytes()),
            ),
            (
                WireValue::String("stateRoot".to_owned()),
                wire::bin_bytes(header.state_root.as_bytes()),
            ),
            (
                WireValue::String("transactionsRoot".to_owned()),
                wire::bin_bytes(header.transactions_root.as_bytes()),
            ),
            (
                WireValue::String("receiptsRoot".to_owned()),
                wire::bin_bytes(header.receipts_root.as_bytes()),
            ),
            (
                WireValue::String("number".to_owned()),
                wire::bin_bytes(&header.number.to_be_bytes()),
            ),
            (
                WireValue::String("timestamp".to_owned()),
                wire::bin_bytes(&header.timestamp.to_be_bytes()),
            ),
            (
                WireValue::String("extraData".to_owned()),
                wire::bin_bytes(&header.extra_data),
            ),
            (
                WireValue::String("gasLimit".to_owned()),
                wire::bin_bytes(header.gas_limit.as_be_bytes()),
            ),
            (
                WireValue::String("gasUsed".to_owned()),
                wire::bin_bytes(header.gas_used.as_be_bytes()),
            ),
        ];
        if let Some(base_fee) = header.base_fee_per_gas {
            inner_values.push((
                WireValue::String("baseFeePerGas".to_owned()),
                wire::bin_bytes(base_fee.as_be_bytes()),
            ));
        }
        inner_values.extend(wire::extra_to_wire(&header.extra));

        let sealed = wire::string_map(vec![
            ("hash", wire::bin_bytes(header.hash.as_bytes())),
            ("header", WireValue::Map(inner_values)),
        ]);
        let tx_values = self
            .transactions
            .iter()
            .map(EvmTransaction::to_wire)
            .collect();
        let body = wire::string_map(vec![("transactions", WireValue::Array(tx_values))]);
        let reth = wire::string_map(vec![("header", sealed), ("body", body)]);
        let block = wire::string_map(vec![("Reth115", reth)]);
        let mut root = vec![
            (WireValue::String("block".to_owned()), block),
            (
                WireValue::String("receipts".to_owned()),
                WireValue::Array(self.receipts.iter().map(EvmReceipt::to_wire).collect()),
            ),
        ];
        if !self.system_transactions.is_empty() {
            root.push((
                WireValue::String("systemTransactions".to_owned()),
                WireValue::Array(
                    self.system_transactions
                        .iter()
                        .map(|tx| tx.transaction().to_wire())
                        .collect(),
                ),
            ));
        }
        if self.block.pace != BlockPace::Unknown {
            root.push((
                WireValue::String("pace".to_owned()),
                WireValue::String(self.block.pace.as_wire_name().to_owned()),
            ));
        }
        WireValue::Map(root)
    }
}

fn parse_receipts(
    value: Option<&WireValue>,
    chain_id: EvmChainId,
    block_hash: Hash32,
    block_number: u64,
    transactions: &[EvmTransaction],
) -> Result<Vec<EvmReceipt>, EvmError> {
    let Some(WireValue::Array(items)) = value else {
        if value.is_none() {
            return Ok(Vec::new());
        }
        return Err(EvmError::MalformedPayload(
            "receipts must be an array".to_owned(),
        ));
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let tx_index = u32::try_from(index).map_err(|_| EvmError::InvalidIdentity)?;
            let tx_hash = transactions
                .get(index)
                .map(EvmTransaction::hash)
                .ok_or_else(|| {
                    EvmError::SchemaDrift("receipt index has no matching transaction".to_owned())
                })?;
            EvmReceipt::from_wire(item, chain_id, block_hash, block_number, tx_hash, tx_index)
        })
        .collect()
}

fn parse_system_txs(
    value: Option<&WireValue>,
    chain_id: EvmChainId,
    block_hash: Hash32,
    block_number: u64,
) -> Result<Vec<SystemTransaction>, EvmError> {
    let Some(WireValue::Array(items)) = value else {
        return Ok(Vec::new());
    };
    items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let tx_index = u32::try_from(index).map_err(|_| EvmError::InvalidIdentity)?;
            let tx = EvmTransaction::from_wire(item, chain_id, block_hash, block_number, tx_index)?;
            SystemTransaction::from_transaction(tx, None)
        })
        .collect()
}

fn hash_block(chain_id: EvmChainId, header: &EvmHeader) -> [u8; 32] {
    hash_fact(
        BLOCK_HASH_CONTEXT,
        &[
            &chain_id.get().to_be_bytes(),
            &header.number.to_be_bytes(),
            header.hash.as_bytes(),
            header.parent_hash.as_bytes(),
            &header.timestamp.to_be_bytes(),
            header.miner.as_bytes(),
            header.gas_used.as_be_bytes(),
            header.gas_limit.as_be_bytes(),
        ],
    )
}
