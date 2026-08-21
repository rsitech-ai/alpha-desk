use std::collections::BTreeMap;

use domain_types::Address;
use serde_json::Value as JsonValue;

use super::wire::{self, WireValue};
use super::{
    EvmChainId, EvmError, Hash32, TX_HASH_CONTEXT, Wei, hash_fact, hash32_from_wire,
    optional_address, optional_wei, u64_from_wire, wei_from_wire,
};

const TAKEN_TX_FIELDS: &[&str] = &[
    "chainId",
    "chain_id",
    "nonce",
    "gas",
    "gasLimit",
    "gas_limit",
    "gasPrice",
    "gas_price",
    "to",
    "value",
    "input",
    "data",
    "maxFeePerGas",
    "max_fee_per_gas",
    "maxPriorityFeePerGas",
    "max_priority_fee_per_gas",
    "accessList",
    "access_list",
    "blobVersionedHashes",
    "maxFeePerBlobGas",
    "authorizationList",
    "hash",
    "from",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TxKind {
    Legacy,
    Eip2930,
    Eip1559,
    Eip4844,
    Eip7702,
}

impl TxKind {
    pub fn parse_name(name: &str) -> Option<Self> {
        match name {
            "Legacy" | "legacy" | "0x0" | "0" => Some(Self::Legacy),
            "Eip2930" | "EIP2930" | "0x1" | "1" => Some(Self::Eip2930),
            "Eip1559" | "EIP1559" | "0x2" | "2" => Some(Self::Eip1559),
            "Eip4844" | "EIP4844" | "0x3" | "3" => Some(Self::Eip4844),
            "Eip7702" | "EIP7702" | "0x4" | "4" => Some(Self::Eip7702),
            _ => None,
        }
    }

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Legacy => "Legacy",
            Self::Eip2930 => "Eip2930",
            Self::Eip1559 => "Eip1559",
            Self::Eip4844 => "Eip4844",
            Self::Eip7702 => "Eip7702",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct EvmTransaction {
    chain_id: EvmChainId,
    block_hash: Hash32,
    block_number: u64,
    tx_index: u32,
    hash: Hash32,
    type_name: String,
    kind: Option<TxKind>,
    from: Option<Address>,
    to: Option<Address>,
    nonce: u64,
    gas: Wei,
    gas_price: Option<Wei>,
    max_fee_per_gas: Option<Wei>,
    max_priority_fee_per_gas: Option<Wei>,
    value: Wei,
    input: Vec<u8>,
    signature: Vec<Vec<u8>>,
    extra: BTreeMap<String, JsonValue>,
    fact_hash: [u8; 32],
}

impl EvmTransaction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain_id: EvmChainId,
        block_hash: Hash32,
        block_number: u64,
        tx_index: u32,
        hash: Hash32,
        type_name: impl Into<String>,
        from: Option<Address>,
        to: Option<Address>,
        nonce: u64,
        gas: Wei,
        gas_price: Option<Wei>,
        max_fee_per_gas: Option<Wei>,
        max_priority_fee_per_gas: Option<Wei>,
        value: Wei,
        input: Vec<u8>,
        signature: Vec<Vec<u8>>,
        extra: BTreeMap<String, JsonValue>,
    ) -> Result<Self, EvmError> {
        let type_name = type_name.into();
        if type_name.is_empty() {
            return Err(EvmError::InvalidIdentity);
        }
        let kind = TxKind::parse_name(&type_name);
        let fact_hash = hash_tx(
            chain_id,
            block_hash,
            block_number,
            tx_index,
            hash,
            &type_name,
            &input,
            &extra,
        );
        Ok(Self {
            chain_id,
            block_hash,
            block_number,
            tx_index,
            hash,
            type_name,
            kind,
            from,
            to,
            nonce,
            gas,
            gas_price,
            max_fee_per_gas,
            max_priority_fee_per_gas,
            value,
            input,
            signature,
            extra,
            fact_hash,
        })
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
    pub const fn tx_index(&self) -> u32 {
        self.tx_index
    }

    #[must_use]
    pub const fn hash(&self) -> Hash32 {
        self.hash
    }

    #[must_use]
    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    #[must_use]
    pub const fn kind(&self) -> Option<TxKind> {
        self.kind
    }

    #[must_use]
    pub const fn from(&self) -> Option<Address> {
        self.from
    }

    #[must_use]
    pub const fn to(&self) -> Option<Address> {
        self.to
    }

    #[must_use]
    pub const fn nonce(&self) -> u64 {
        self.nonce
    }

    #[must_use]
    pub const fn gas(&self) -> Wei {
        self.gas
    }

    #[must_use]
    pub const fn value(&self) -> Wei {
        self.value
    }

    #[must_use]
    pub fn input(&self) -> &[u8] {
        &self.input
    }

    #[must_use]
    pub fn extra(&self) -> &BTreeMap<String, JsonValue> {
        &self.extra
    }

    #[must_use]
    pub fn signature(&self) -> &[Vec<u8>] {
        &self.signature
    }

    #[must_use]
    pub const fn fact_hash(&self) -> [u8; 32] {
        self.fact_hash
    }

    #[must_use]
    pub fn fact_id(&self) -> String {
        format!("evx_{}", hex::encode(self.fact_hash))
    }

    #[must_use]
    pub fn tx_id(&self) -> String {
        self.hash.to_api_string()
    }

    #[must_use]
    pub fn unsigned_system_candidate(&self) -> bool {
        self.signature.iter().all(Vec::is_empty) || self.signature.is_empty()
    }

    pub(crate) fn from_wire(
        value: &WireValue,
        fallback_chain: EvmChainId,
        block_hash: Hash32,
        block_number: u64,
        tx_index: u32,
    ) -> Result<Self, EvmError> {
        let root = value.as_map()?;
        let (type_name, content) = match wire::map_get(root, &["transaction"]) {
            Some(tx) => wire::tagged_enum(tx)?,
            None => {
                if let Ok((tag, inner)) = wire::tagged_enum(value) {
                    (tag, inner)
                } else {
                    ("Legacy", value)
                }
            }
        };
        let content_map = content.as_map()?;
        let chain = u64_from_wire(wire::map_get(content_map, &["chainId", "chain_id"])).ok();
        let chain_id = super::chain_from_tx(chain, fallback_chain)?;
        let hash = match wire::map_get(root, &["hash"]).or(wire::map_get(content_map, &["hash"])) {
            Some(value) => hash32_from_wire(Some(value))?,
            None => derived_tx_hash(chain_id, block_hash, tx_index, content_map)?,
        };
        let signature = match wire::map_get(root, &["signature"]) {
            Some(WireValue::Array(items)) => items
                .iter()
                .map(wire::as_bytes)
                .collect::<Result<Vec<_>, _>>()?,
            Some(other) => vec![wire::as_bytes(other)?],
            None => Vec::new(),
        };
        Self::new(
            chain_id,
            block_hash,
            block_number,
            tx_index,
            hash,
            type_name,
            optional_address(
                wire::map_get(root, &["from"]).or(wire::map_get(content_map, &["from"])),
            )?,
            optional_address(wire::map_get(content_map, &["to"]))?,
            u64_from_wire(wire::map_get(content_map, &["nonce"]))?,
            wei_from_wire(wire::map_get(
                content_map,
                &["gas", "gasLimit", "gas_limit"],
            ))?,
            optional_wei(wire::map_get(content_map, &["gasPrice", "gas_price"]))?,
            optional_wei(wire::map_get(
                content_map,
                &["maxFeePerGas", "max_fee_per_gas"],
            ))?,
            optional_wei(wire::map_get(
                content_map,
                &["maxPriorityFeePerGas", "max_priority_fee_per_gas"],
            ))?,
            wei_from_wire(wire::map_get(content_map, &["value"]))?,
            wire::map_get(content_map, &["input", "data"])
                .map(wire::as_bytes)
                .transpose()?
                .unwrap_or_default(),
            signature,
            wire::leftover_json(content_map, TAKEN_TX_FIELDS),
        )
    }

    pub(crate) fn to_wire(&self) -> WireValue {
        let mut content = vec![
            (
                WireValue::String("chainId".to_owned()),
                wire::bin_bytes(&self.chain_id.get().to_be_bytes()),
            ),
            (
                WireValue::String("nonce".to_owned()),
                wire::bin_bytes(&self.nonce.to_be_bytes()),
            ),
            (
                WireValue::String("gas".to_owned()),
                wire::bin_bytes(self.gas.as_be_bytes()),
            ),
            (
                WireValue::String("value".to_owned()),
                wire::bin_bytes(self.value.as_be_bytes()),
            ),
            (
                WireValue::String("input".to_owned()),
                wire::bin_bytes(&self.input),
            ),
        ];
        match self.to {
            Some(to) => content.push((
                WireValue::String("to".to_owned()),
                wire::bin_bytes(to.as_bytes()),
            )),
            None => content.push((WireValue::String("to".to_owned()), WireValue::Nil)),
        }
        if let Some(price) = self.gas_price {
            content.push((
                WireValue::String("gasPrice".to_owned()),
                wire::bin_bytes(price.as_be_bytes()),
            ));
        }
        if let Some(fee) = self.max_fee_per_gas {
            content.push((
                WireValue::String("maxFeePerGas".to_owned()),
                wire::bin_bytes(fee.as_be_bytes()),
            ));
        }
        if let Some(tip) = self.max_priority_fee_per_gas {
            content.push((
                WireValue::String("maxPriorityFeePerGas".to_owned()),
                wire::bin_bytes(tip.as_be_bytes()),
            ));
        }
        content.extend(wire::extra_to_wire(&self.extra));
        let tagged = WireValue::Map(vec![(
            WireValue::String(self.type_name.clone()),
            WireValue::Map(content),
        )]);
        let signature = WireValue::Array(
            self.signature
                .iter()
                .map(|bytes| wire::bin_bytes(bytes))
                .collect(),
        );
        wire::string_map(vec![
            ("transaction", tagged),
            ("signature", signature),
            ("hash", wire::bin_bytes(self.hash.as_bytes())),
        ])
    }
}

fn derived_tx_hash(
    chain_id: EvmChainId,
    block_hash: Hash32,
    tx_index: u32,
    content: &[(WireValue, WireValue)],
) -> Result<Hash32, EvmError> {
    let packed = wire::encode_msgpack(&WireValue::Map(content.to_vec()))?;
    Ok(Hash32::from_bytes(hash_fact(
        TX_HASH_CONTEXT,
        &[
            b"derived-source-hash",
            &chain_id.get().to_be_bytes(),
            block_hash.as_bytes(),
            &tx_index.to_be_bytes(),
            &packed,
        ],
    )))
}

#[allow(clippy::too_many_arguments)]
fn hash_tx(
    chain_id: EvmChainId,
    block_hash: Hash32,
    block_number: u64,
    tx_index: u32,
    hash: Hash32,
    type_name: &str,
    input: &[u8],
    extra: &BTreeMap<String, JsonValue>,
) -> [u8; 32] {
    let extra_bytes = serde_json::to_vec(extra).unwrap_or_else(|_| b"{}".to_vec());
    hash_fact(
        TX_HASH_CONTEXT,
        &[
            &chain_id.get().to_be_bytes(),
            block_hash.as_bytes(),
            &block_number.to_be_bytes(),
            &tx_index.to_be_bytes(),
            hash.as_bytes(),
            type_name.as_bytes(),
            input,
            &extra_bytes,
        ],
    )
}
