//! Canonical HyperEVM chain facts.
//!
//! These types are not HyperCore `canonical_event` payloads. They map to
//! capability `state_target = evm_fact`. Unknown contract ABI events stay logs.

mod asset;
mod block;
mod core_link;
mod log;
mod precompile;
mod receipt;
mod system_transaction;
mod transaction;
mod wire;

pub use asset::{
    APPROVAL_FOR_ALL_TOPIC, APPROVAL_TOPIC, AssetStandard, Erc20Approval, Erc20Transfer,
    Erc721Transfer, Erc1155BatchTransfer, NativeHypeTransfer, TRANSFER_BATCH_TOPIC,
    TRANSFER_SINGLE_TOPIC, TRANSFER_TOPIC, WellKnownLog,
};
pub use block::{BlockPace, EvmBlock, EvmBlockAndReceipts, EvmHeader};
pub use core_link::{CoreEvmBlockLink, CoreWriterAction, TokenContractLink};
pub use log::{EvmLog, EvmLogId};
pub use precompile::{
    CORE_WRITER_ADDRESS, CoreWriterCall, PrecompileObservation, READ_PRECOMPILE_BASE,
    is_core_writer, is_read_precompile,
};
pub use receipt::{EvmReceipt, ReceiptStatus};
pub use system_transaction::{CoreOrigin, SystemTransaction};
pub use transaction::{EvmTransaction, HashProvenance, TxKind};

use crate::{ErrorDisposition, SourceError};
use domain_types::{Address, Decimal, RoundingMode, ValueError};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

const MAX_ARCHIVE_BYTES: usize = 256 * 1024 * 1024;
const BLOCK_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/evm-block/v1";
const TX_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/evm-tx/v1";
const RECEIPT_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/evm-receipt/v1";
const LOG_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/evm-log/v1";
const HYPE_DECIMALS: u8 = 18;

pub const MAINNET_CHAIN_ID: u64 = 999;
pub const TESTNET_CHAIN_ID: u64 = 998;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EvmChainId(u64);

impl EvmChainId {
    pub const MAINNET: Self = Self(MAINNET_CHAIN_ID);
    pub const TESTNET: Self = Self(TESTNET_CHAIN_ID);

    pub const fn new(id: u64) -> Result<Self, EvmError> {
        match id {
            MAINNET_CHAIN_ID | TESTNET_CHAIN_ID => Ok(Self(id)),
            _ => Err(EvmError::UnsupportedChainId(id)),
        }
    }

    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for EvmChainId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Hash32([u8; 32]);

impl Hash32 {
    #[must_use]
    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn parse_api(input: &str) -> Result<Self, EvmError> {
        let hex_value = input.strip_prefix("0x").ok_or(EvmError::InvalidHash)?;
        if hex_value.len() != 64 || hex_value.bytes().any(|byte| byte.is_ascii_uppercase()) {
            return Err(EvmError::InvalidHash);
        }
        let mut bytes = [0_u8; 32];
        hex::decode_to_slice(hex_value, &mut bytes).map_err(|_| EvmError::InvalidHash)?;
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn to_api_string(self) -> String {
        format!("0x{}", hex::encode(self.0))
    }
}

impl fmt::Display for Hash32 {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_api_string())
    }
}

impl Serialize for Hash32 {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_api_string())
    }
}

impl<'de> Deserialize<'de> for Hash32 {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::parse_api(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Wei([u8; 32]);

impl Wei {
    pub const ZERO: Self = Self([0; 32]);

    #[must_use]
    pub const fn from_be_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    #[must_use]
    pub fn from_u64(value: u64) -> Self {
        let mut bytes = [0_u8; 32];
        bytes[24..].copy_from_slice(&value.to_be_bytes());
        Self(bytes)
    }

    pub fn from_be_slice(bytes: &[u8]) -> Result<Self, EvmError> {
        if bytes.len() > 32 {
            return Err(EvmError::InvalidQuantity);
        }
        let mut padded = [0_u8; 32];
        padded[32 - bytes.len()..].copy_from_slice(bytes);
        Ok(Self(padded))
    }

    #[must_use]
    pub const fn as_be_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_u64(self) -> Result<u64, EvmError> {
        if self.0[..24].iter().any(|byte| *byte != 0) {
            return Err(EvmError::QuantityOverflow);
        }
        Ok(u64::from_be_bytes(
            self.0[24..].try_into().expect("8 trailing bytes"),
        ))
    }

    pub fn to_i128(self) -> Result<i128, EvmError> {
        if self.0[0] & 0x80 != 0 || self.0[..16].iter().any(|byte| *byte != 0) {
            return Err(EvmError::QuantityOverflow);
        }
        let mut wide = [0_u8; 16];
        wide.copy_from_slice(&self.0[16..]);
        let unsigned = u128::from_be_bytes(wide);
        i128::try_from(unsigned).map_err(|_| EvmError::QuantityOverflow)
    }

    pub fn to_hype_decimal(self) -> Result<Decimal, EvmError> {
        Decimal::from_raw(self.to_i128()?, HYPE_DECIMALS).map_err(EvmError::from_value)
    }

    pub fn from_hype_decimal(amount: Decimal) -> Result<Self, EvmError> {
        let scaled = amount
            .rescale(HYPE_DECIMALS, RoundingMode::TowardZero)
            .map_err(EvmError::from_value)?;
        if scaled.raw() < 0 {
            return Err(EvmError::InvalidQuantity);
        }
        let unsigned = u128::try_from(scaled.raw()).map_err(|_| EvmError::QuantityOverflow)?;
        let mut bytes = [0_u8; 32];
        bytes[16..].copy_from_slice(&unsigned.to_be_bytes());
        Ok(Self(bytes))
    }

    #[must_use]
    pub fn is_zero(self) -> bool {
        self.0.iter().all(|byte| *byte == 0)
    }

    #[must_use]
    pub fn to_decimal_string(self) -> String {
        let mut start = 0;
        while start < 31 && self.0[start] == 0 {
            start += 1;
        }
        hex_to_dec(&self.0[start..])
    }
}

impl fmt::Display for Wei {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.to_decimal_string())
    }
}

impl Serialize for Wei {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_decimal_string())
    }
}

impl<'de> Deserialize<'de> for Wei {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        parse_wei_decimal(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum EvmError {
    #[error("unsupported HyperEVM chain id {0}")]
    UnsupportedChainId(u64),
    #[error("malformed HyperEVM payload: {0}")]
    MalformedPayload(String),
    #[error("HyperEVM schema drift: {0}")]
    SchemaDrift(String),
    #[error("invalid HyperEVM hash")]
    InvalidHash,
    #[error("invalid HyperEVM address")]
    InvalidAddress,
    #[error("invalid HyperEVM quantity")]
    InvalidQuantity,
    #[error("HyperEVM quantity overflows the conversion range")]
    QuantityOverflow,
    #[error("invalid HyperEVM identity")]
    InvalidIdentity,
}

impl EvmError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::UnsupportedChainId(_) => "evm.unsupported_chain_id",
            Self::MalformedPayload(_) => "evm.malformed_payload",
            Self::SchemaDrift(_) => "evm.schema_drift",
            Self::InvalidHash => "evm.invalid_hash",
            Self::InvalidAddress => "evm.invalid_address",
            Self::InvalidQuantity => "evm.invalid_quantity",
            Self::QuantityOverflow => "evm.quantity_overflow",
            Self::InvalidIdentity => "evm.invalid_identity",
        }
    }

    #[must_use]
    pub const fn disposition(&self) -> ErrorDisposition {
        match self {
            Self::UnsupportedChainId(_)
            | Self::MalformedPayload(_)
            | Self::SchemaDrift(_)
            | Self::InvalidHash
            | Self::InvalidAddress
            | Self::InvalidQuantity
            | Self::QuantityOverflow
            | Self::InvalidIdentity => ErrorDisposition::Quarantine,
        }
    }

    fn from_value(error: ValueError) -> Self {
        match error {
            ValueError::Overflow | ValueError::OutOfRange => Self::QuantityOverflow,
            _ => Self::InvalidQuantity,
        }
    }
}

impl From<EvmError> for SourceError {
    fn from(error: EvmError) -> Self {
        match error {
            EvmError::SchemaDrift(detail) => Self::SchemaDrift(detail),
            other => Self::MalformedPayload(other.to_string()),
        }
    }
}

pub fn decode_rmp_lz4(
    bytes: &[u8],
    fallback_chain: EvmChainId,
) -> Result<Vec<EvmBlockAndReceipts>, EvmError> {
    let msgpack = wire::decompress_lz4_frame(bytes)?;
    if msgpack.is_empty() || msgpack.len() > MAX_ARCHIVE_BYTES {
        return Err(EvmError::MalformedPayload(
            "decompressed HyperEVM archive size is outside the supported range".to_owned(),
        ));
    }
    let root = wire::parse_msgpack(&msgpack)?;
    let objects = match &root {
        wire::WireValue::Array(items) => items.as_slice(),
        _ => std::slice::from_ref(&root),
    };
    objects
        .iter()
        .map(|object| EvmBlockAndReceipts::from_wire(object, fallback_chain))
        .collect()
}

pub fn encode_rmp_lz4(records: &[EvmBlockAndReceipts]) -> Result<Vec<u8>, EvmError> {
    let root = match records {
        [single] => single.to_wire(),
        _ => wire::WireValue::Array(records.iter().map(EvmBlockAndReceipts::to_wire).collect()),
    };
    let packed = wire::encode_msgpack(&root)?;
    wire::compress_lz4_frame(&packed)
}

pub(crate) fn hash_fact(context: &str, parts: &[&[u8]]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(context);
    for part in parts {
        let length = u64::try_from(part.len()).expect("fact field fits u64 length prefix");
        hasher.update(&length.to_be_bytes());
        hasher.update(part);
    }
    *hasher.finalize().as_bytes()
}

fn parse_wei_decimal(input: &str) -> Result<Wei, EvmError> {
    if input.is_empty() || input.as_bytes().iter().any(|byte| !byte.is_ascii_digit()) {
        return Err(EvmError::InvalidQuantity);
    }
    let mut bytes = [0_u8; 32];
    for digit in input.bytes() {
        mul_add_digit(&mut bytes, digit - b'0')?;
    }
    Ok(Wei(bytes))
}

fn mul_add_digit(bytes: &mut [u8; 32], digit: u8) -> Result<(), EvmError> {
    let mut carry = u16::from(digit);
    for slot in bytes.iter_mut().rev() {
        let product = u16::from(*slot) * 10 + carry;
        *slot = (product & 0xff) as u8;
        carry = product >> 8;
    }
    if carry == 0 {
        Ok(())
    } else {
        Err(EvmError::QuantityOverflow)
    }
}

fn hex_to_dec(bytes: &[u8]) -> String {
    if bytes.iter().all(|byte| *byte == 0) {
        return "0".to_owned();
    }
    let mut digits = vec![0_u8; bytes.len() * 3];
    for byte in bytes {
        let mut carry = u32::from(*byte);
        for digit in digits.iter_mut().rev() {
            carry += u32::from(*digit) << 8;
            *digit = (carry % 10) as u8;
            carry /= 10;
        }
    }
    let start = digits.iter().position(|digit| *digit != 0).unwrap_or(0);
    digits[start..]
        .iter()
        .map(|digit| char::from(b'0' + *digit))
        .collect()
}

pub(crate) fn hash32_from_wire(value: Option<&wire::WireValue>) -> Result<Hash32, EvmError> {
    let bytes = wire::as_bytes(value.ok_or(EvmError::InvalidHash)?)?;
    let bytes: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .map_err(|_| EvmError::InvalidHash)?;
    Ok(Hash32::from_bytes(bytes))
}

pub(crate) fn optional_hash32(value: Option<&wire::WireValue>) -> Result<Option<Hash32>, EvmError> {
    match value {
        None | Some(wire::WireValue::Nil) => Ok(None),
        Some(other) => Ok(Some(hash32_from_wire(Some(other))?)),
    }
}

pub(crate) fn address_from_wire(value: Option<&wire::WireValue>) -> Result<Address, EvmError> {
    let bytes = wire::as_bytes(value.ok_or(EvmError::InvalidAddress)?)?;
    match bytes.len() {
        20 => Ok(Address::from_bytes(
            bytes.as_slice().try_into().expect("20-byte address"),
        )),
        32 if bytes[..12].iter().all(|byte| *byte == 0) => Ok(Address::from_bytes(
            bytes[12..].try_into().expect("address in topic"),
        )),
        _ => Err(EvmError::InvalidAddress),
    }
}

pub(crate) fn optional_address(
    value: Option<&wire::WireValue>,
) -> Result<Option<Address>, EvmError> {
    match value {
        None | Some(wire::WireValue::Nil) => Ok(None),
        Some(other) => Ok(Some(address_from_wire(Some(other))?)),
    }
}

pub(crate) fn wei_from_wire(value: Option<&wire::WireValue>) -> Result<Wei, EvmError> {
    match value {
        None | Some(wire::WireValue::Nil) => Ok(Wei::ZERO),
        Some(wire::WireValue::Uint(value)) => Ok(Wei::from_u64(*value)),
        Some(wire::WireValue::Int(value)) if *value >= 0 => Ok(Wei::from_u64(*value as u64)),
        Some(other) => Wei::from_be_slice(&wire::as_bytes(other)?),
    }
}

pub(crate) fn u64_from_wire(value: Option<&wire::WireValue>) -> Result<u64, EvmError> {
    wei_from_wire(value)?.to_u64()
}

pub(crate) fn optional_wei(value: Option<&wire::WireValue>) -> Result<Option<Wei>, EvmError> {
    match value {
        None | Some(wire::WireValue::Nil) => Ok(None),
        Some(other) => Ok(Some(wei_from_wire(Some(other))?)),
    }
}

pub(crate) fn optional_u64(value: Option<&wire::WireValue>) -> Result<Option<u64>, EvmError> {
    match value {
        None | Some(wire::WireValue::Nil) => Ok(None),
        Some(other) => Ok(Some(u64_from_wire(Some(other))?)),
    }
}

pub(crate) fn required_u64(
    value: Option<&wire::WireValue>,
    field: &'static str,
) -> Result<u64, EvmError> {
    match value {
        None | Some(wire::WireValue::Nil) => {
            Err(EvmError::SchemaDrift(format!("header is missing {field}")))
        }
        Some(other) => u64_from_wire(Some(other)),
    }
}

pub(crate) fn chain_from_tx(
    tx_chain: Option<u64>,
    fallback: EvmChainId,
) -> Result<EvmChainId, EvmError> {
    match tx_chain {
        None => Ok(fallback),
        Some(id) => {
            let parsed = EvmChainId::new(id)?;
            if parsed == fallback {
                Ok(parsed)
            } else {
                Err(EvmError::SchemaDrift(format!(
                    "transaction chain id {id} disagrees with archive chain {}",
                    fallback.get()
                )))
            }
        }
    }
}

#[cfg(test)]
mod decode_edges;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mainnet_and_testnet_chain_ids_are_accepted() {
        assert_eq!(EvmChainId::new(999).unwrap(), EvmChainId::MAINNET);
        assert_eq!(EvmChainId::new(998).unwrap(), EvmChainId::TESTNET);
        assert_eq!(
            EvmChainId::new(1).unwrap_err(),
            EvmError::UnsupportedChainId(1)
        );
    }

    #[test]
    fn wei_hype_roundtrip_uses_eighteen_decimals() {
        let one = Wei::from_hype_decimal(Decimal::from_raw(1, 0).unwrap()).unwrap();
        assert_eq!(
            one.to_hype_decimal().unwrap().to_string(),
            "1.000000000000000000"
        );
        let wei = Wei::from_u64(1);
        assert_eq!(
            wei.to_hype_decimal().unwrap().to_string(),
            "0.000000000000000001"
        );
    }
}
