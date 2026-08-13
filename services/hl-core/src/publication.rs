//! Canonical JetStream subjects, headers, and the committed-marker codec surface.
//!
//! The frozen marker byte layout lives in `jetstream-marker`. This module keeps
//! consumer-facing subject/header types and maps codec errors onto
//! [`BlockMarkerError`].
//!
//! `hl-capture` still encodes markers with a private copy in
//! `services/hl-capture/src/bus/mod.rs` until the capture stack merges onto
//! `jetstream-marker`.

use canonical_events::{BlockEnvelope, CanonicalEventEnvelope, EventKind};
use jetstream_marker::{
    MAX_PUBLICATION_PAYLOAD_BYTES, MarkerCodecError,
    decode_committed_block_marker as decode_marker, encode_committed_block_marker as encode_marker,
};
use storage_ports::ArchiveReceipt;

pub use jetstream_marker::{BLOCK_MARKER_SCHEMA_V1, CommittedBlockMarker};

pub const CANONICAL_STREAM: &str = "HL_CANONICAL";
pub const BLOCK_COMMITTED_SUBJECT: &str = "hl.v1.block.committed";
pub const BLOCK_PROVISIONAL_SUBJECT: &str = "hl.v1.block.provisional";
pub const HEADER_SCHEMA: &str = "Alpha-Desk-Schema";
pub const HEADER_CHAIN: &str = "Alpha-Desk-Chain";
pub const HEADER_BLOCK_HEIGHT: &str = "Alpha-Desk-Block-Height";
pub const HEADER_BLOCK_HASH: &str = "Alpha-Desk-Block-Hash";
pub const HEADER_ARCHIVE_RECEIPT: &str = "Alpha-Desk-Archive-Receipt";
pub const HEADER_ARCHIVE_MANIFEST_SHA256: &str = "Alpha-Desk-Archive-Manifest-SHA256";
pub const HEADER_PUBLICATION_SHA256: &str = "Alpha-Desk-Publication-SHA256";

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
            BLOCK_PROVISIONAL_SUBJECT => Err(BlockMarkerError::Provisional),
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

impl From<MarkerCodecError> for BlockMarkerError {
    fn from(error: MarkerCodecError) -> Self {
        match error {
            MarkerCodecError::Malformed => Self::Malformed,
            MarkerCodecError::UnsupportedSchema => Self::UnsupportedSchema,
            MarkerCodecError::NotCommitted => Self::NotCommitted,
            MarkerCodecError::PayloadSize => Self::PayloadSize,
            MarkerCodecError::InvalidIdentity => Self::InvalidIdentity,
        }
    }
}

pub fn encode_committed_block_marker(
    block: &BlockEnvelope,
    receipt: &ArchiveReceipt,
) -> Result<Vec<u8>, BlockMarkerError> {
    encode_marker(block, receipt).map_err(BlockMarkerError::from)
}

pub fn decode_committed_block_marker(
    bytes: &[u8],
) -> Result<CommittedBlockMarker, BlockMarkerError> {
    decode_marker(bytes).map_err(BlockMarkerError::from)
}

pub fn encode_event_payload(event: &CanonicalEventEnvelope) -> Result<Vec<u8>, BlockMarkerError> {
    let payload = event
        .encode_to_vec()
        .map_err(|_| BlockMarkerError::Malformed)?;
    if payload.is_empty() || payload.len() > MAX_PUBLICATION_PAYLOAD_BYTES {
        Err(BlockMarkerError::PayloadSize)
    } else {
        Ok(payload)
    }
}
