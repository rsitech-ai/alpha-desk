//! Frozen JetStream committed-block marker codec.
//!
//! This crate owns the `hyperliquid-alpha-desk/block-publication/v1` byte
//! layout used on `hl.v1.block.committed`. Field order, big-endian widths,
//! confirmation-class tags `2`/`3`, counted UTF-8 identities, and SHA-256
//! envelope hashes are frozen: change them only with a schema version bump.
//!
//! `hl-core` decodes through this crate. `hl-capture` still has a private
//! encoder copy in `services/hl-capture/src/bus/mod.rs` until the capture
//! stack merges onto this dependency. Do not "fix" producer/consumer drift by
//! changing this layout.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, ConfirmationClass, EventKind};
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};
use sha2::{Digest as _, Sha256};
use storage_ports::ArchiveReceipt;

/// Frozen schema identity written as the first counted string in every marker.
pub const BLOCK_MARKER_SCHEMA_V1: &str = "hyperliquid-alpha-desk/block-publication/v1";

/// Maximum UTF-8 identity length accepted by the frozen marker codec.
pub const MAX_IDENTITY_BYTES: usize = 512;

/// Maximum publication payload size accepted by the frozen marker codec.
pub const MAX_PUBLICATION_PAYLOAD_BYTES: usize = 7_500_000;

/// One event row inside a committed-block marker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerEvent {
    pub event_id: String,
    pub event_kind: EventKind,
    pub payload_hash: [u8; 32],
    pub envelope_sha256: [u8; 32],
}

/// Decoded committed-block marker payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedBlockMarker {
    pub chain_id: ChainId,
    pub block_height: BlockHeight,
    pub block_time: ProtocolTime,
    pub confirmation_class: ConfirmationClass,
    pub canonical_block_hash: [u8; 32],
    pub archive_receipt_id: String,
    pub archive_manifest_id: String,
    pub archive_manifest_sha256: [u8; 32],
    pub schema_fingerprint: [u8; 32],
    pub source_block_hashes: BTreeMap<SourceId, [u8; 32]>,
    pub events: Vec<MarkerEvent>,
}

/// Failures while encoding or decoding the frozen committed-block marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MarkerCodecError {
    #[error("committed block marker is truncated or malformed")]
    Malformed,
    #[error("committed block marker uses an unsupported schema")]
    UnsupportedSchema,
    #[error("committed block marker is not a committed confirmation class")]
    NotCommitted,
    #[error("canonical publication payload size is outside the supported bound")]
    PayloadSize,
    #[error("canonical publication identity is invalid")]
    InvalidIdentity,
}

/// Encode a committed canonical block marker using the frozen v1 layout.
///
/// # Errors
///
/// Returns [`MarkerCodecError::Malformed`] when the archive receipt does not
/// bind the block or an event envelope cannot be encoded,
/// [`MarkerCodecError::NotCommitted`] when the confirmation class is not a
/// committed class, and [`MarkerCodecError::PayloadSize`] when the encoded
/// payload is empty or exceeds [`MAX_PUBLICATION_PAYLOAD_BYTES`].
pub fn encode_committed_block_marker(
    block: &BlockEnvelope,
    receipt: &ArchiveReceipt,
) -> Result<Vec<u8>, MarkerCodecError> {
    if receipt.block_height() != block.block_height()
        || receipt.canonical_block_hash() != block.canonical_block_hash()
    {
        return Err(MarkerCodecError::Malformed);
    }
    if !matches!(
        block.confirmation_class(),
        ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent
    ) {
        return Err(MarkerCodecError::NotCommitted);
    }

    let mut output = Vec::new();
    push_bytes(&mut output, BLOCK_MARKER_SCHEMA_V1.as_bytes())?;
    push_bytes(&mut output, block.chain_id().as_str().as_bytes())?;
    output.extend_from_slice(&block.block_height().get().to_be_bytes());
    output.extend_from_slice(&block.block_time().unix_micros().to_be_bytes());
    output.push(match block.confirmation_class() {
        ConfirmationClass::CommittedPrimary => 2,
        ConfirmationClass::CommittedIndependent => 3,
        ConfirmationClass::ProvisionalSource
        | ConfirmationClass::ReconciledSnapshot
        | ConfirmationClass::Corrected
        | ConfirmationClass::Expired => return Err(MarkerCodecError::NotCommitted),
    });
    output.extend_from_slice(&block.canonical_block_hash());
    push_bytes(&mut output, receipt.receipt_id().as_bytes())?;
    push_bytes(&mut output, receipt.manifest_id().as_str().as_bytes())?;
    output.extend_from_slice(&receipt.manifest_sha256());
    output.extend_from_slice(&receipt.schema_fingerprint());

    push_count(&mut output, block.source_block_hashes().len())?;
    for (source_id, source_hash) in block.source_block_hashes() {
        push_bytes(&mut output, source_id.as_str().as_bytes())?;
        output.extend_from_slice(source_hash);
    }

    push_count(&mut output, block.events().len())?;
    for event in block.events() {
        push_bytes(&mut output, event.event_id().as_str().as_bytes())?;
        push_bytes(&mut output, event.event_kind().as_wire_name().as_bytes())?;
        output.extend_from_slice(&event.payload_hash());
        let encoded = event
            .encode_to_vec()
            .map_err(|_| MarkerCodecError::Malformed)?;
        output.extend_from_slice(&Sha256::digest(&encoded));
    }
    validate_payload_size(output.len())?;
    Ok(output)
}

/// Decode a frozen v1 committed-block marker payload.
///
/// # Errors
///
/// Returns [`MarkerCodecError::UnsupportedSchema`] when the leading schema
/// string is not [`BLOCK_MARKER_SCHEMA_V1`], [`MarkerCodecError::NotCommitted`]
/// when the confirmation tag is not `2` or `3`,
/// [`MarkerCodecError::InvalidIdentity`] when a counted string fails identity
/// checks, and [`MarkerCodecError::Malformed`] for truncated, overlong, or
/// trailing-byte payloads.
pub fn decode_committed_block_marker(
    bytes: &[u8],
) -> Result<CommittedBlockMarker, MarkerCodecError> {
    let mut cursor = bytes;
    let schema = read_utf8(&mut cursor)?;
    if schema != BLOCK_MARKER_SCHEMA_V1 {
        return Err(MarkerCodecError::UnsupportedSchema);
    }
    let chain_id =
        ChainId::new(read_utf8(&mut cursor)?).map_err(|_| MarkerCodecError::Malformed)?;
    let block_height = BlockHeight::new(read_u64(&mut cursor)?);
    let block_time = ProtocolTime::from_unix_micros(read_i64(&mut cursor)?)
        .map_err(|_| MarkerCodecError::Malformed)?;
    let confirmation_class = match read_u8(&mut cursor)? {
        2 => ConfirmationClass::CommittedPrimary,
        3 => ConfirmationClass::CommittedIndependent,
        _ => return Err(MarkerCodecError::NotCommitted),
    };
    let canonical_block_hash = read_hash32(&mut cursor)?;
    let archive_receipt_id = read_utf8(&mut cursor)?.to_owned();
    let archive_manifest_id = read_utf8(&mut cursor)?.to_owned();
    let archive_manifest_sha256 = read_hash32(&mut cursor)?;
    let schema_fingerprint = read_hash32(&mut cursor)?;

    let source_count = read_usize(&mut cursor)?;
    let mut source_block_hashes = BTreeMap::new();
    for _ in 0..source_count {
        let source =
            SourceId::new(read_utf8(&mut cursor)?).map_err(|_| MarkerCodecError::Malformed)?;
        let hash = read_hash32(&mut cursor)?;
        if source_block_hashes.insert(source, hash).is_some() {
            return Err(MarkerCodecError::Malformed);
        }
    }

    let event_count = read_usize(&mut cursor)?;
    let mut events = Vec::with_capacity(event_count);
    for _ in 0..event_count {
        let event_id = read_utf8(&mut cursor)?.to_owned();
        let event_kind = EventKind::try_from(read_utf8(&mut cursor)?)
            .map_err(|_| MarkerCodecError::Malformed)?;
        events.push(MarkerEvent {
            event_id,
            event_kind,
            payload_hash: read_hash32(&mut cursor)?,
            envelope_sha256: read_hash32(&mut cursor)?,
        });
    }
    if !cursor.is_empty() {
        return Err(MarkerCodecError::Malformed);
    }
    Ok(CommittedBlockMarker {
        chain_id,
        block_height,
        block_time,
        confirmation_class,
        canonical_block_hash,
        archive_receipt_id,
        archive_manifest_id,
        archive_manifest_sha256,
        schema_fingerprint,
        source_block_hashes,
        events,
    })
}

/// Reject empty or oversized publication payloads.
///
/// # Errors
///
/// Returns [`MarkerCodecError::PayloadSize`] when `actual` is zero or greater
/// than [`MAX_PUBLICATION_PAYLOAD_BYTES`].
pub fn validate_payload_size(actual: usize) -> Result<(), MarkerCodecError> {
    if actual == 0 || actual > MAX_PUBLICATION_PAYLOAD_BYTES {
        Err(MarkerCodecError::PayloadSize)
    } else {
        Ok(())
    }
}

fn push_count(output: &mut Vec<u8>, count: usize) -> Result<(), MarkerCodecError> {
    let count = u64::try_from(count).map_err(|_| MarkerCodecError::Malformed)?;
    output.extend_from_slice(&count.to_be_bytes());
    Ok(())
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), MarkerCodecError> {
    push_count(output, value.len())?;
    output.extend_from_slice(value);
    Ok(())
}

fn read_u8(cursor: &mut &[u8]) -> Result<u8, MarkerCodecError> {
    let (byte, rest) = cursor.split_first().ok_or(MarkerCodecError::Malformed)?;
    *cursor = rest;
    Ok(*byte)
}

fn read_u64(cursor: &mut &[u8]) -> Result<u64, MarkerCodecError> {
    let (bytes, rest) = cursor
        .split_first_chunk::<8>()
        .ok_or(MarkerCodecError::Malformed)?;
    *cursor = rest;
    Ok(u64::from_be_bytes(*bytes))
}

fn read_i64(cursor: &mut &[u8]) -> Result<i64, MarkerCodecError> {
    let (bytes, rest) = cursor
        .split_first_chunk::<8>()
        .ok_or(MarkerCodecError::Malformed)?;
    *cursor = rest;
    Ok(i64::from_be_bytes(*bytes))
}

fn read_hash32(cursor: &mut &[u8]) -> Result<[u8; 32], MarkerCodecError> {
    let (bytes, rest) = cursor
        .split_first_chunk::<32>()
        .ok_or(MarkerCodecError::Malformed)?;
    *cursor = rest;
    Ok(*bytes)
}

fn read_counted<'a>(cursor: &mut &'a [u8]) -> Result<&'a [u8], MarkerCodecError> {
    let len = read_usize(cursor)?;
    if cursor.len() < len {
        return Err(MarkerCodecError::Malformed);
    }
    let (head, tail) = cursor.split_at(len);
    *cursor = tail;
    Ok(head)
}

fn read_utf8<'a>(cursor: &mut &'a [u8]) -> Result<&'a str, MarkerCodecError> {
    let bytes = read_counted(cursor)?;
    let value = std::str::from_utf8(bytes).map_err(|_| MarkerCodecError::Malformed)?;
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTITY_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(MarkerCodecError::InvalidIdentity);
    }
    Ok(value)
}

fn read_usize(cursor: &mut &[u8]) -> Result<usize, MarkerCodecError> {
    usize::try_from(read_u64(cursor)?).map_err(|_| MarkerCodecError::Malformed)
}
