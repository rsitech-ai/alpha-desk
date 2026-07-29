use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, ConfirmationClass};
use chrono::{DateTime, Utc};
use domain_types::{BlockHeight, ManifestId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use storage_ports::ArchiveError;

pub const BLOCK_MANIFEST_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-block-manifest/v1";
pub const PARTITION_MANIFEST_SCHEMA_V1: &str =
    "hyperliquid-alpha-desk/archive-partition-manifest/v1";
pub const CATALOG_MANIFEST_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-catalog-manifest/v1";
pub const CURRENT_POINTER_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-current-pointer/v1";
pub const CANONICAL_DATASET: &str = "canonical_events";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectDescriptorV1 {
    pub relative_path: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub row_count: u64,
    pub schema_fingerprint_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockDescriptorV1 {
    pub chain_id: String,
    pub block_height: u64,
    pub block_time_micros: i64,
    pub confirmation_class: String,
    pub canonical_block_blake3: String,
    pub event_count: u64,
    pub source_block_hashes_blake3: BTreeMap<String, String>,
}

impl BlockDescriptorV1 {
    pub fn from_block(block: &BlockEnvelope) -> Result<Self, ArchiveError> {
        let event_count = u64::try_from(block.events().len())
            .map_err(|_| ArchiveError::InvalidInput("canonical event count exceeds u64"))?;
        Ok(Self {
            chain_id: block.chain_id().as_str().to_owned(),
            block_height: block.block_height().get(),
            block_time_micros: block.block_time().unix_micros(),
            confirmation_class: confirmation_name(block.confirmation_class()).to_owned(),
            canonical_block_blake3: hex::encode(block.canonical_block_hash()),
            event_count,
            source_block_hashes_blake3: block
                .source_block_hashes()
                .iter()
                .map(|(source, hash)| (source.as_str().to_owned(), hex::encode(hash)))
                .collect(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockManifestV1 {
    pub schema: String,
    pub producer_build_id: String,
    pub created_at_micros: i64,
    pub input_object_count: u64,
    pub rolling_content_sha256: String,
    pub blocks: Vec<BlockDescriptorV1>,
    pub object: ObjectDescriptorV1,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BlockManifestRefV1 {
    pub block_height: u64,
    pub canonical_block_blake3: String,
    pub manifest_relative_path: String,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionManifestV1 {
    pub schema: String,
    pub chain_id: String,
    pub dataset: String,
    pub partition: String,
    pub generation: u64,
    pub producer_build_id: String,
    pub created_at_micros: i64,
    pub previous_manifest_sha256: Option<String>,
    pub transition: PartitionTransitionV1,
    pub blocks: Vec<BlockManifestRefV1>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PartitionTransitionV1 {
    Append,
    Compaction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartitionManifestRefV1 {
    pub partition: String,
    pub manifest_relative_path: String,
    pub manifest_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogManifestV1 {
    pub schema: String,
    pub chain_id: String,
    pub dataset: String,
    pub generation: u64,
    pub producer_build_id: String,
    pub created_at_micros: i64,
    pub previous_manifest_sha256: Option<String>,
    pub partitions: BTreeMap<String, PartitionManifestRefV1>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurrentPointerV1 {
    pub schema: String,
    pub manifest_relative_path: String,
    pub manifest_sha256: String,
}

pub fn canonical_json<T: Serialize>(value: &T) -> Result<Vec<u8>, ArchiveError> {
    serde_json::to_vec(value)
        .map_err(|_| ArchiveError::Codec("serializing archive manifest".into()))
}

pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn parse_hash(value: &str) -> Result<[u8; 32], ArchiveError> {
    if value.len() != 64 || value.bytes().any(|byte| !byte.is_ascii_hexdigit()) {
        return Err(ArchiveError::ManifestVerification(
            "expected a lowercase 32-byte hexadecimal hash",
        ));
    }
    if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return Err(ArchiveError::ManifestVerification(
            "manifest hashes must use lowercase hexadecimal",
        ));
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash)
        .map_err(|_| ArchiveError::ManifestVerification("manifest contains an invalid hash"))?;
    Ok(hash)
}

pub fn manifest_id(hash: [u8; 32]) -> Result<ManifestId, ArchiveError> {
    ManifestId::new(format!("archive-manifest-v1-{}", hex::encode(hash)))
        .map_err(|_| ArchiveError::InvalidInput("manifest ID"))
}

pub fn hash_from_manifest_id(manifest: &ManifestId) -> Result<[u8; 32], ArchiveError> {
    let value = manifest
        .as_str()
        .strip_prefix("archive-manifest-v1-")
        .ok_or(ArchiveError::ManifestVerification(
            "manifest ID has an unsupported format",
        ))?;
    parse_hash(value)
}

pub fn encoded_component(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
            output.push(char::from(byte));
        } else {
            output.push('%');
            output.push_str(&format!("{byte:02X}"));
        }
    }
    output
}

pub fn decoded_component(value: &str) -> Result<String, ArchiveError> {
    let bytes = value.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let encoded = bytes
                .get(index + 1..index + 3)
                .ok_or(ArchiveError::UnsafePath)?;
            let encoded = std::str::from_utf8(encoded).map_err(|_| ArchiveError::UnsafePath)?;
            let byte = u8::from_str_radix(encoded, 16).map_err(|_| ArchiveError::UnsafePath)?;
            decoded.push(byte);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    let decoded = String::from_utf8(decoded).map_err(|_| ArchiveError::UnsafePath)?;
    if encoded_component(&decoded) != value {
        return Err(ArchiveError::UnsafePath);
    }
    Ok(decoded)
}

pub fn partition_for(block_time_micros: i64) -> Result<String, ArchiveError> {
    let timestamp = DateTime::<Utc>::from_timestamp_micros(block_time_micros).ok_or(
        ArchiveError::InvalidInput("block time cannot be partitioned"),
    )?;
    Ok(timestamp.format("date=%Y-%m-%d/hour=%H").to_string())
}

pub fn confirmation_name(value: ConfirmationClass) -> &'static str {
    match value {
        ConfirmationClass::ProvisionalSource => "provisional-source",
        ConfirmationClass::CommittedPrimary => "committed-primary",
        ConfirmationClass::CommittedIndependent => "committed-independent",
        ConfirmationClass::ReconciledSnapshot => "reconciled-snapshot",
        ConfirmationClass::Corrected => "corrected",
        ConfirmationClass::Expired => "expired",
    }
}

pub fn parse_confirmation(value: &str) -> Result<ConfirmationClass, ArchiveError> {
    match value {
        "provisional-source" => Ok(ConfirmationClass::ProvisionalSource),
        "committed-primary" => Ok(ConfirmationClass::CommittedPrimary),
        "committed-independent" => Ok(ConfirmationClass::CommittedIndependent),
        "reconciled-snapshot" => Ok(ConfirmationClass::ReconciledSnapshot),
        "corrected" => Ok(ConfirmationClass::Corrected),
        "expired" => Ok(ConfirmationClass::Expired),
        _ => Err(ArchiveError::ManifestVerification(
            "manifest has an unknown confirmation class",
        )),
    }
}

pub fn validate_block_ref_order(blocks: &[BlockManifestRefV1]) -> Result<(), ArchiveError> {
    let mut previous: Option<BlockHeight> = None;
    for block in blocks {
        let height = BlockHeight::new(block.block_height);
        if previous.is_some_and(|value| value >= height) {
            return Err(ArchiveError::ManifestVerification(
                "partition block references are not strictly ordered",
            ));
        }
        parse_hash(&block.canonical_block_blake3)?;
        parse_hash(&block.manifest_sha256)?;
        previous = Some(height);
    }
    Ok(())
}
