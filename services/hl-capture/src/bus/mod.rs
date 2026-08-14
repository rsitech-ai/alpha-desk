mod jetstream;
mod subjects;

use std::collections::BTreeMap;

use async_trait::async_trait;
use bytes::Bytes;
use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId};
use sha2::{Digest as _, Sha256};
use storage_ports::ArchiveReceipt;

pub use jetstream::{
    JetStreamAuthentication, JetStreamConfig, JetStreamConfigError, JetStreamPublisher,
    ReconnectingJetStreamPublisher,
};
pub use subjects::{
    CANONICAL_STREAM, DEAD_LETTER_STREAM, FEATURE_STREAM, HEALTH_STREAM, SIGNAL_STREAM,
    STATE_STREAM, Subject, subject_for_event_kind,
};

// Frozen JetStream marker schema. Layout lock: tests/block_marker_freeze.rs
// (empty-primary digest, Independent tag `3`, and one event-row payload).
const BLOCK_MARKER_SCHEMA_V1: &str = "hyperliquid-alpha-desk/block-publication/v1";
const MAX_IDENTITY_BYTES: usize = 512;
const MAX_PUBLICATION_PAYLOAD_BYTES: usize = 7_500_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationMessage {
    subject: Subject,
    message_id: String,
    schema_version: String,
    chain_id: ChainId,
    block_height: BlockHeight,
    canonical_block_hash: [u8; 32],
    archive_receipt_id: String,
    archive_manifest_sha256: [u8; 32],
    publication_sha256: [u8; 32],
    payload: Bytes,
}

#[async_trait]
pub trait CanonicalPublisher: Send + Sync {
    async fn publish(
        &self,
        message: &PublicationMessage,
    ) -> Result<PublicationAck, PublicationError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicationAck {
    message_id: String,
    stream: String,
    stream_sequence: u64,
    duplicate: bool,
    publication_sha256: [u8; 32],
}

impl PublicationAck {
    pub fn try_new(
        message: &PublicationMessage,
        stream: String,
        stream_sequence: u64,
        duplicate: bool,
    ) -> Result<Self, PublicationError> {
        if stream != message.stream() {
            return Err(PublicationError::UnexpectedAckStream);
        }
        if stream_sequence == 0 {
            return Err(PublicationError::InvalidAck);
        }
        Ok(Self {
            message_id: message.message_id.clone(),
            stream,
            stream_sequence,
            duplicate,
            publication_sha256: message.publication_sha256,
        })
    }

    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    #[must_use]
    pub fn stream(&self) -> &str {
        &self.stream
    }

    #[must_use]
    pub const fn stream_sequence(&self) -> u64 {
        self.stream_sequence
    }

    #[must_use]
    pub const fn duplicate(&self) -> bool {
        self.duplicate
    }

    #[must_use]
    pub const fn publication_sha256(&self) -> [u8; 32] {
        self.publication_sha256
    }
}

impl PublicationMessage {
    #[must_use]
    pub const fn subject(&self) -> Subject {
        self.subject
    }

    #[must_use]
    pub const fn stream(&self) -> &'static str {
        self.subject.stream()
    }

    #[must_use]
    pub fn message_id(&self) -> &str {
        &self.message_id
    }

    #[must_use]
    pub fn schema_version(&self) -> &str {
        &self.schema_version
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn block_height(&self) -> BlockHeight {
        self.block_height
    }

    #[must_use]
    pub const fn canonical_block_hash(&self) -> [u8; 32] {
        self.canonical_block_hash
    }

    #[must_use]
    pub fn archive_receipt_id(&self) -> &str {
        &self.archive_receipt_id
    }

    #[must_use]
    pub const fn archive_manifest_sha256(&self) -> [u8; 32] {
        self.archive_manifest_sha256
    }

    #[must_use]
    pub const fn publication_sha256(&self) -> [u8; 32] {
        self.publication_sha256
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommittedPublicationBatch {
    block: PublicationMessage,
    events: Vec<PublicationMessage>,
}

impl CommittedPublicationBatch {
    pub fn try_new(
        block: &BlockEnvelope,
        receipt: &ArchiveReceipt,
    ) -> Result<Self, PublicationError> {
        if receipt.block_height() != block.block_height()
            || receipt.canonical_block_hash() != block.canonical_block_hash()
        {
            return Err(PublicationError::ArchiveReceiptMismatch);
        }
        admit_committed_publication(block.confirmation_class())?;

        let block_payload = encode_block_marker(block, receipt)?;
        let block_message = PublicationMessage::try_new(
            Subject::BlockCommitted,
            format!("blk_{}", hex::encode(block.canonical_block_hash())),
            BLOCK_MARKER_SCHEMA_V1,
            block,
            receipt,
            block_payload,
        )?;

        let mut events = Vec::with_capacity(block.events().len());
        for event in block.events() {
            let payload = event
                .encode_to_vec()
                .map_err(|_| PublicationError::CanonicalCodec)?;
            events.push(PublicationMessage::try_new(
                subject_for_event_kind(event.event_kind()),
                event.event_id().as_str().to_owned(),
                event.schema_version(),
                block,
                receipt,
                payload,
            )?);
        }

        Ok(Self {
            block: block_message,
            events,
        })
    }

    #[must_use]
    pub const fn block(&self) -> &PublicationMessage {
        &self.block
    }

    #[must_use]
    pub fn events(&self) -> &[PublicationMessage] {
        &self.events
    }

    pub fn iter(&self) -> impl Iterator<Item = &PublicationMessage> {
        std::iter::once(&self.block).chain(self.events.iter())
    }
}

impl PublicationMessage {
    fn try_new(
        subject: Subject,
        message_id: String,
        schema_version: &str,
        block: &BlockEnvelope,
        receipt: &ArchiveReceipt,
        payload: Vec<u8>,
    ) -> Result<Self, PublicationError> {
        validate_identity(&message_id)?;
        validate_identity(schema_version)?;
        if message_id != message_id.to_ascii_lowercase() {
            return Err(PublicationError::NonCanonicalMessageId);
        }
        if payload.is_empty() || payload.len() > MAX_PUBLICATION_PAYLOAD_BYTES {
            return Err(PublicationError::PayloadSize {
                actual: payload.len(),
                limit: MAX_PUBLICATION_PAYLOAD_BYTES,
            });
        }
        let publication_sha256 = Sha256::digest(&payload).into();
        Ok(Self {
            subject,
            message_id,
            schema_version: schema_version.to_owned(),
            chain_id: block.chain_id().clone(),
            block_height: block.block_height(),
            canonical_block_hash: block.canonical_block_hash(),
            archive_receipt_id: receipt.receipt_id().to_owned(),
            archive_manifest_sha256: receipt.manifest_sha256(),
            publication_sha256,
            payload: Bytes::from(payload),
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicationDisposition {
    New,
    IdenticalDuplicate,
}

#[derive(Debug)]
pub struct PublicationLedger {
    limit: usize,
    hashes: BTreeMap<String, PublicationFingerprint>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PublicationFingerprint {
    publication_sha256: [u8; 32],
    canonical_block_hash: [u8; 32],
    archive_manifest_sha256: [u8; 32],
    subject: Subject,
}

impl PublicationLedger {
    pub fn new(limit: usize) -> Result<Self, PublicationError> {
        if limit == 0 {
            return Err(PublicationError::InvalidLedgerCapacity);
        }
        Ok(Self {
            limit,
            hashes: BTreeMap::new(),
        })
    }

    pub fn record(
        &mut self,
        message: &PublicationMessage,
    ) -> Result<PublicationDisposition, PublicationError> {
        let fingerprint = PublicationFingerprint {
            publication_sha256: message.publication_sha256,
            canonical_block_hash: message.canonical_block_hash,
            archive_manifest_sha256: message.archive_manifest_sha256,
            subject: message.subject,
        };
        if let Some(existing) = self.hashes.get(message.message_id()) {
            return if existing == &fingerprint {
                Ok(PublicationDisposition::IdenticalDuplicate)
            } else {
                Err(PublicationError::DivergentMessageId {
                    message_id: message.message_id.clone(),
                })
            };
        }
        if self.hashes.len() >= self.limit {
            return Err(PublicationError::LedgerCapacityExceeded { limit: self.limit });
        }
        self.hashes.insert(message.message_id.clone(), fingerprint);
        Ok(PublicationDisposition::New)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.hashes.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.hashes.is_empty()
    }
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum PublicationError {
    #[error("archive receipt does not bind the canonical block")]
    ArchiveReceiptMismatch,
    #[error("only committed canonical blocks may use committed subjects")]
    NotCommitted,
    #[error("canonical publication codec failed")]
    CanonicalCodec,
    #[error("publication identity is invalid")]
    InvalidIdentity,
    #[error("publication message ID is not its canonical lowercase form")]
    NonCanonicalMessageId,
    #[error("publication payload size {actual} is outside the limit {limit}")]
    PayloadSize { actual: usize, limit: usize },
    #[error("publication ledger capacity must be greater than zero")]
    InvalidLedgerCapacity,
    #[error("publication ledger capacity {limit} is exhausted")]
    LedgerCapacityExceeded { limit: usize },
    #[error("message ID {message_id} was reused with divergent content")]
    DivergentMessageId { message_id: String },
    #[error("block publication count exceeds the wire limit")]
    CountOverflow,
    #[error("JetStream connection failed")]
    TransportConnect,
    #[error("JetStream publish request failed")]
    TransportPublish,
    #[error("JetStream publish acknowledgement failed")]
    TransportAck,
    #[error("JetStream acknowledged the publication from an unexpected stream")]
    UnexpectedAckStream,
    #[error("JetStream returned an invalid publication acknowledgement")]
    InvalidAck,
}

impl PublicationError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::ArchiveReceiptMismatch => "publication.archive_receipt_mismatch",
            Self::NotCommitted => "publication.not_committed",
            Self::CanonicalCodec => "publication.canonical_codec",
            Self::InvalidIdentity => "publication.invalid_identity",
            Self::NonCanonicalMessageId => "publication.noncanonical_message_id",
            Self::PayloadSize { .. } => "publication.payload_size",
            Self::InvalidLedgerCapacity => "publication.invalid_ledger_capacity",
            Self::LedgerCapacityExceeded { .. } => "publication.ledger_capacity_exceeded",
            Self::DivergentMessageId { .. } => "publication.divergent_message_id",
            Self::CountOverflow => "publication.count_overflow",
            Self::TransportConnect => "publication.transport_connect",
            Self::TransportPublish => "publication.transport_publish",
            Self::TransportAck => "publication.transport_ack",
            Self::UnexpectedAckStream => "publication.unexpected_ack_stream",
            Self::InvalidAck => "publication.invalid_ack",
        }
    }
}

fn validate_identity(value: &str) -> Result<(), PublicationError> {
    if value.is_empty()
        || value.trim() != value
        || value.len() > MAX_IDENTITY_BYTES
        || value.chars().any(char::is_control)
    {
        Err(PublicationError::InvalidIdentity)
    } else {
        Ok(())
    }
}

fn admit_committed_publication(class: ConfirmationClass) -> Result<(), PublicationError> {
    match class {
        ConfirmationClass::CommittedPrimary | ConfirmationClass::CommittedIndependent => Ok(()),
        ConfirmationClass::ProvisionalSource
        | ConfirmationClass::ReconciledSnapshot
        | ConfirmationClass::Corrected
        | ConfirmationClass::Expired => Err(PublicationError::NotCommitted),
    }
}

// Frozen `hyperliquid-alpha-desk/block-publication/v1` layout. Field order,
// confirmation-class tags `2`/`3`, counted identities, and SHA-256 envelope
// hashes must not change without a schema version bump.
fn encode_block_marker(
    block: &BlockEnvelope,
    receipt: &ArchiveReceipt,
) -> Result<Vec<u8>, PublicationError> {
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
        | ConfirmationClass::Expired => return Err(PublicationError::NotCommitted),
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
            .map_err(|_| PublicationError::CanonicalCodec)?;
        output.extend_from_slice(&Sha256::digest(encoded));
    }
    Ok(output)
}

fn push_count(output: &mut Vec<u8>, count: usize) -> Result<(), PublicationError> {
    let count = u64::try_from(count).map_err(|_| PublicationError::CountOverflow)?;
    output.extend_from_slice(&count.to_be_bytes());
    Ok(())
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), PublicationError> {
    push_count(output, value.len())?;
    output.extend_from_slice(value);
    Ok(())
}
