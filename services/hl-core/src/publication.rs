use std::collections::BTreeMap;

use canonical_events::{BlockEnvelope, CanonicalEventEnvelope, ConfirmationClass, EventKind};
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};
use sha2::{Digest as _, Sha256};
use storage_ports::ArchiveReceipt;

pub const CANONICAL_STREAM: &str = "HL_CANONICAL";
pub const BLOCK_COMMITTED_SUBJECT: &str = "hl.v1.block.committed";
pub const BLOCK_PROVISIONAL_SUBJECT: &str = "hl.v1.block.provisional";
pub const SNAPSHOT_ACCOUNT_SUBJECT: &str = "hl.v1.snapshot.account";
pub const SNAPSHOT_MARKET_SUBJECT: &str = "hl.v1.snapshot.market";
pub const SNAPSHOT_ECOSYSTEM_SUBJECT: &str = "hl.v1.snapshot.ecosystem";
pub const HEALTH_SOURCE_SUBJECT: &str = "hl.v1.health.source";
pub const BLOCK_MARKER_SCHEMA_V1: &str = "hyperliquid-alpha-desk/block-publication/v1";
pub const HEADER_SCHEMA: &str = "Alpha-Desk-Schema";
pub const HEADER_CHAIN: &str = "Alpha-Desk-Chain";
pub const HEADER_BLOCK_HEIGHT: &str = "Alpha-Desk-Block-Height";
pub const HEADER_BLOCK_HASH: &str = "Alpha-Desk-Block-Hash";
pub const HEADER_ARCHIVE_RECEIPT: &str = "Alpha-Desk-Archive-Receipt";
pub const HEADER_ARCHIVE_MANIFEST_SHA256: &str = "Alpha-Desk-Archive-Manifest-SHA256";
pub const HEADER_PUBLICATION_SHA256: &str = "Alpha-Desk-Publication-SHA256";

const MAX_IDENTITY_BYTES: usize = 512;
const MAX_PUBLICATION_PAYLOAD_BYTES: usize = 7_500_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalSubject {
    BlockCommitted,
    EventFill,
    EventOrder,
    EventLedger,
    EventMarketMeta,
    EventOracle,
}

impl CanonicalSubject {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BlockCommitted => BLOCK_COMMITTED_SUBJECT,
            Self::EventFill => "hl.v1.event.fill",
            Self::EventOrder => "hl.v1.event.order",
            Self::EventLedger => "hl.v1.event.ledger",
            Self::EventMarketMeta => "hl.v1.event.market_meta",
            Self::EventOracle => "hl.v1.event.oracle",
        }
    }

    pub fn parse(value: &str) -> Result<Self, BlockMarkerError> {
        match value {
            BLOCK_COMMITTED_SUBJECT => Ok(Self::BlockCommitted),
            "hl.v1.event.fill" => Ok(Self::EventFill),
            "hl.v1.event.order" => Ok(Self::EventOrder),
            "hl.v1.event.ledger" => Ok(Self::EventLedger),
            "hl.v1.event.market_meta" => Ok(Self::EventMarketMeta),
            "hl.v1.event.oracle" => Ok(Self::EventOracle),
            BLOCK_PROVISIONAL_SUBJECT
            | SNAPSHOT_ACCOUNT_SUBJECT
            | SNAPSHOT_MARKET_SUBJECT
            | SNAPSHOT_ECOSYSTEM_SUBJECT => Err(BlockMarkerError::Provisional),
            _ => Err(BlockMarkerError::UnexpectedSubject),
        }
    }
}

#[must_use]
pub const fn subject_for_event_kind(kind: EventKind) -> CanonicalSubject {
    match kind {
        EventKind::OrderPartiallyFilled
        | EventKind::OrderFilled
        | EventKind::TwapSliceFilled
        | EventKind::TradeMatched
        | EventKind::LiquidationFill
        | EventKind::BackstopLiquidation => CanonicalSubject::EventFill,
        EventKind::OrderAccepted
        | EventKind::OrderRested
        | EventKind::OrderModified
        | EventKind::OrderCancelled
        | EventKind::OrderRejected
        | EventKind::TriggerOrderActivated
        | EventKind::TwapStarted
        | EventKind::TwapCompleted => CanonicalSubject::EventOrder,
        EventKind::OracleUpdated | EventKind::FundingRateUpdated => CanonicalSubject::EventOracle,
        EventKind::MarketHalted
        | EventKind::MarketResumed
        | EventKind::OpenInterestCapChanged
        | EventKind::MarginTableChanged
        | EventKind::MarketCreated
        | EventKind::MarketMetadataChanged
        | EventKind::AssetContextUpdated
        | EventKind::DexCreated
        | EventKind::OutcomeCreated
        | EventKind::OutcomeResolved => CanonicalSubject::EventMarketMeta,
        EventKind::DepositCredited
        | EventKind::WithdrawalDebited
        | EventKind::SpotTransfer
        | EventKind::PerpTransfer
        | EventKind::SubaccountTransfer
        | EventKind::VaultDeposit
        | EventKind::VaultWithdrawal
        | EventKind::FeeCharged
        | EventKind::BuilderFeeCharged
        | EventKind::FundingPaid
        | EventKind::FundingReceived
        | EventKind::ReferralReward
        | EventKind::AccountModeChanged
        | EventKind::MarginModeChanged
        | EventKind::LeverageChanged
        | EventKind::LiquidationStarted
        | EventKind::PositionSettled => CanonicalSubject::EventLedger,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkerEvent {
    pub event_id: String,
    pub event_kind: EventKind,
    pub payload_hash: [u8; 32],
    pub envelope_sha256: [u8; 32],
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BlockMarkerError {
    #[error("committed block marker is truncated or malformed")]
    Malformed,
    #[error("committed block marker uses an unsupported schema")]
    UnsupportedSchema,
    #[error("committed block marker is not a committed confirmation class")]
    NotCommitted,
    #[error("JetStream subject is not a committed canonical consumer subject")]
    UnexpectedSubject,
    #[error("provisional canonical subjects are rejected by the file-store replay consumer")]
    Provisional,
    #[error("canonical publication payload size is outside the supported bound")]
    PayloadSize,
    #[error("canonical publication identity is invalid")]
    InvalidIdentity,
}

impl BlockMarkerError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Malformed => "core.jetstream_decode",
            Self::UnsupportedSchema => "core.jetstream_unsupported_schema",
            Self::NotCommitted => "core.jetstream_not_committed",
            Self::UnexpectedSubject => "core.jetstream_unexpected_subject",
            Self::Provisional => "core.jetstream_provisional",
            Self::PayloadSize => "core.jetstream_payload_size",
            Self::InvalidIdentity => "core.jetstream_invalid_identity",
        }
    }
}

pub fn encode_committed_block_marker(
    block: &BlockEnvelope,
    receipt: &ArchiveReceipt,
) -> Result<Vec<u8>, BlockMarkerError> {
    if receipt.block_height() != block.block_height()
        || receipt.canonical_block_hash() != block.canonical_block_hash()
    {
        return Err(BlockMarkerError::Malformed);
    }
    if !matches!(
        block.confirmation_class(),
        ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent
    ) {
        return Err(BlockMarkerError::NotCommitted);
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
        | ConfirmationClass::Expired => return Err(BlockMarkerError::NotCommitted),
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
            .map_err(|_| BlockMarkerError::Malformed)?;
        output.extend_from_slice(&Sha256::digest(&encoded));
    }
    validate_payload_size(output.len())?;
    Ok(output)
}

pub fn decode_committed_block_marker(
    bytes: &[u8],
) -> Result<CommittedBlockMarker, BlockMarkerError> {
    let mut cursor = bytes;
    let schema = read_utf8(&mut cursor)?;
    if schema != BLOCK_MARKER_SCHEMA_V1 {
        return Err(BlockMarkerError::UnsupportedSchema);
    }
    let chain_id =
        ChainId::new(read_utf8(&mut cursor)?).map_err(|_| BlockMarkerError::Malformed)?;
    let block_height = BlockHeight::new(read_u64(&mut cursor)?);
    let block_time = ProtocolTime::from_unix_micros(read_i64(&mut cursor)?)
        .map_err(|_| BlockMarkerError::Malformed)?;
    let confirmation_class = match read_u8(&mut cursor)? {
        2 => ConfirmationClass::CommittedPrimary,
        3 => ConfirmationClass::CommittedIndependent,
        _ => return Err(BlockMarkerError::NotCommitted),
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
            SourceId::new(read_utf8(&mut cursor)?).map_err(|_| BlockMarkerError::Malformed)?;
        let hash = read_hash32(&mut cursor)?;
        if source_block_hashes.insert(source, hash).is_some() {
            return Err(BlockMarkerError::Malformed);
        }
    }

    let event_count = read_usize(&mut cursor)?;
    let mut events = Vec::with_capacity(event_count);
    for _ in 0..event_count {
        let event_id = read_utf8(&mut cursor)?.to_owned();
        let event_kind = EventKind::try_from(read_utf8(&mut cursor)?)
            .map_err(|_| BlockMarkerError::Malformed)?;
        events.push(MarkerEvent {
            event_id,
            event_kind,
            payload_hash: read_hash32(&mut cursor)?,
            envelope_sha256: read_hash32(&mut cursor)?,
        });
    }
    if !cursor.is_empty() {
        return Err(BlockMarkerError::Malformed);
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

pub fn encode_event_payload(event: &CanonicalEventEnvelope) -> Result<Vec<u8>, BlockMarkerError> {
    let payload = event
        .encode_to_vec()
        .map_err(|_| BlockMarkerError::Malformed)?;
    validate_payload_size(payload.len())?;
    Ok(payload)
}

fn validate_payload_size(actual: usize) -> Result<(), BlockMarkerError> {
    if actual == 0 || actual > MAX_PUBLICATION_PAYLOAD_BYTES {
        Err(BlockMarkerError::PayloadSize)
    } else {
        Ok(())
    }
}

fn push_count(output: &mut Vec<u8>, count: usize) -> Result<(), BlockMarkerError> {
    let count = u64::try_from(count).map_err(|_| BlockMarkerError::Malformed)?;
    output.extend_from_slice(&count.to_be_bytes());
    Ok(())
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), BlockMarkerError> {
    push_count(output, value.len())?;
    output.extend_from_slice(value);
    Ok(())
}

fn read_u8(cursor: &mut &[u8]) -> Result<u8, BlockMarkerError> {
    let (byte, rest) = cursor.split_first().ok_or(BlockMarkerError::Malformed)?;
    *cursor = rest;
    Ok(*byte)
}

fn read_u64(cursor: &mut &[u8]) -> Result<u64, BlockMarkerError> {
    let (bytes, rest) = cursor
        .split_first_chunk::<8>()
        .ok_or(BlockMarkerError::Malformed)?;
    *cursor = rest;
    Ok(u64::from_be_bytes(*bytes))
}

fn read_i64(cursor: &mut &[u8]) -> Result<i64, BlockMarkerError> {
    let (bytes, rest) = cursor
        .split_first_chunk::<8>()
        .ok_or(BlockMarkerError::Malformed)?;
    *cursor = rest;
    Ok(i64::from_be_bytes(*bytes))
}

fn read_hash32(cursor: &mut &[u8]) -> Result<[u8; 32], BlockMarkerError> {
    let (bytes, rest) = cursor
        .split_first_chunk::<32>()
        .ok_or(BlockMarkerError::Malformed)?;
    *cursor = rest;
    Ok(*bytes)
}

fn read_counted<'a>(cursor: &mut &'a [u8]) -> Result<&'a [u8], BlockMarkerError> {
    let len = read_usize(cursor)?;
    if cursor.len() < len {
        return Err(BlockMarkerError::Malformed);
    }
    let (head, tail) = cursor.split_at(len);
    *cursor = tail;
    Ok(head)
}

fn read_utf8<'a>(cursor: &mut &'a [u8]) -> Result<&'a str, BlockMarkerError> {
    let bytes = read_counted(cursor)?;
    let value = std::str::from_utf8(bytes).map_err(|_| BlockMarkerError::Malformed)?;
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTITY_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(BlockMarkerError::InvalidIdentity);
    }
    Ok(value)
}

fn read_usize(cursor: &mut &[u8]) -> Result<usize, BlockMarkerError> {
    usize::try_from(read_u64(cursor)?).map_err(|_| BlockMarkerError::Malformed)
}
