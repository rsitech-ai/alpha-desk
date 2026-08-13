use std::collections::BTreeMap;

use canonical_events::{
    BlockEnvelope, CanonicalEventEnvelope, CanonicalEventInput, ConfirmationClass, EventPayload,
    SourceEvidence, TradeMatched,
};
use domain_types::{
    BlockHeight, ChainId, KnownTime, Price, ProtocolTime, Quantity, SourceId, TransactionId,
};

const FIXTURE_TIME_BASE_MICROS: i64 = 1_700_000_000_000_000;
const PRIMARY_SOURCE_ID: &str = "synthetic-fixture";
const PRIMARY_SOURCE_VERSION: &str = "synthetic-fixture-v1";
const INDEPENDENT_SOURCE_ID: &str = "synthetic-independent-fixture";
const INDEPENDENT_SOURCE_VERSION: &str = "synthetic-independent-fixture-v1";

pub fn synthetic_fixture_block(
    chain_id: &ChainId,
    block_height: BlockHeight,
) -> Result<BlockEnvelope, FixtureError> {
    synthetic_committed_fixture_block(
        chain_id,
        block_height,
        PRIMARY_SOURCE_ID,
        PRIMARY_SOURCE_VERSION,
        ConfirmationClass::CommittedPrimary,
    )
}

pub fn synthetic_independent_fixture_block(
    chain_id: &ChainId,
    block_height: BlockHeight,
) -> Result<BlockEnvelope, FixtureError> {
    synthetic_committed_fixture_block(
        chain_id,
        block_height,
        INDEPENDENT_SOURCE_ID,
        INDEPENDENT_SOURCE_VERSION,
        ConfirmationClass::CommittedIndependent,
    )
}

fn synthetic_committed_fixture_block(
    chain_id: &ChainId,
    block_height: BlockHeight,
    source_id: &str,
    source_version: &str,
    confirmation_class: ConfirmationClass,
) -> Result<BlockEnvelope, FixtureError> {
    match confirmation_class {
        ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => {}
        ConfirmationClass::ProvisionalSource
        | ConfirmationClass::ReconciledSnapshot
        | ConfirmationClass::Corrected
        | ConfirmationClass::Expired => {
            return Err(FixtureError::UnsupportedConfirmation);
        }
    }
    let height_micros = i64::try_from(block_height.get())
        .ok()
        .and_then(|height| height.checked_mul(1_000))
        .and_then(|height| FIXTURE_TIME_BASE_MICROS.checked_add(height))
        .ok_or(FixtureError::HeightOverflow)?;
    let block_time =
        ProtocolTime::from_unix_micros(height_micros).map_err(|_| FixtureError::InvalidTime)?;
    let observed_at =
        KnownTime::from_unix_micros(height_micros).map_err(|_| FixtureError::InvalidTime)?;
    let ingested_at = KnownTime::from_unix_micros(
        height_micros
            .checked_add(1)
            .ok_or(FixtureError::HeightOverflow)?,
    )
    .map_err(|_| FixtureError::InvalidTime)?;
    let canonicalized_at = KnownTime::from_unix_micros(
        height_micros
            .checked_add(2)
            .ok_or(FixtureError::HeightOverflow)?,
    )
    .map_err(|_| FixtureError::InvalidTime)?;
    let source_id = SourceId::new(source_id).map_err(|_| FixtureError::InvalidIdentity)?;
    let transaction_id = TransactionId::new(format!("fixture-tx-{}", block_height.get()))
        .map_err(|_| FixtureError::InvalidIdentity)?;
    let source_hash = fixture_hash(b"source-block", block_height);
    let source_evidence = SourceEvidence::try_new(
        source_id.clone(),
        source_version,
        format!("block:{}", block_height.get()),
        fixture_hash(b"source-event", block_height),
    )
    .map_err(|_| FixtureError::InvalidEvent)?;
    let event = CanonicalEventEnvelope::from_input(CanonicalEventInput {
        schema_version: "1.0.0".to_owned(),
        chain_id: chain_id.clone(),
        block_height,
        block_time,
        transaction_id,
        transaction_index: 0,
        canonical_event_index: 0,
        market_ids: Vec::new(),
        account_ids: Vec::new(),
        source_evidence: vec![source_evidence],
        confirmation_class,
        observed_at,
        ingested_at,
        canonicalized_at,
        parser_version: "synthetic-fixture-parser-v1".to_owned(),
        payload: EventPayload::TradeMatched(TradeMatched::without_identities(
            Price::parse_at_scale("65000", 6).map_err(|_| FixtureError::InvalidDecimal)?,
            Quantity::parse_at_scale("0.01", 8).map_err(|_| FixtureError::InvalidDecimal)?,
            block_height.get(),
        )),
    })
    .map_err(|_| FixtureError::InvalidEvent)?;
    BlockEnvelope::try_new(
        chain_id.clone(),
        block_height,
        block_time,
        confirmation_class,
        vec![event],
        BTreeMap::from([(source_id, source_hash)]),
    )
    .map_err(|_| FixtureError::InvalidBlock)
}

fn fixture_hash(domain: &[u8], block_height: BlockHeight) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hyperliquid-alpha-desk/synthetic-fixture/v1\0");
    hasher.update(domain);
    hasher.update(&block_height.get().to_be_bytes());
    *hasher.finalize().as_bytes()
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum FixtureError {
    #[error("fixture block height overflows the deterministic time domain")]
    HeightOverflow,
    #[error("fixture time is invalid")]
    InvalidTime,
    #[error("fixture identity is invalid")]
    InvalidIdentity,
    #[error("fixture decimal is invalid")]
    InvalidDecimal,
    #[error("fixture event is invalid")]
    InvalidEvent,
    #[error("fixture block is invalid")]
    InvalidBlock,
    #[error("synthetic fixtures do not emit reconciliation confirmation classes")]
    UnsupportedConfirmation,
}

impl FixtureError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::HeightOverflow => "capture_fixture.height_overflow",
            Self::InvalidTime => "capture_fixture.invalid_time",
            Self::InvalidIdentity => "capture_fixture.invalid_identity",
            Self::InvalidDecimal => "capture_fixture.invalid_decimal",
            Self::InvalidEvent => "capture_fixture.invalid_event",
            Self::InvalidBlock => "capture_fixture.invalid_block",
            Self::UnsupportedConfirmation => "capture_fixture.unsupported_confirmation",
        }
    }
}
