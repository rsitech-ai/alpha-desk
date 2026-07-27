#![forbid(unsafe_code)]

use prost::Message;

#[allow(clippy::all, dead_code)]
mod generated {
    pub(crate) mod hl {
        pub(crate) mod common {
            pub(crate) mod v1 {
                include!(concat!(env!("OUT_DIR"), "/hl.common.v1.rs"));
            }
        }

        pub(crate) mod canonical {
            pub(crate) mod v1 {
                include!(concat!(env!("OUT_DIR"), "/hl.canonical.v1.rs"));
            }
        }

        pub(crate) mod stream {
            pub(crate) mod v1 {
                include!(concat!(env!("OUT_DIR"), "/hl.stream.v1.rs"));
            }
        }
    }
}

pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/alpha-desk-v1.pb"));

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireSourceEvidence {
    pub source_id: String,
    pub source_version: String,
    pub source_offset: String,
    pub content_hash: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireCanonicalEventEnvelope {
    pub schema_version: String,
    pub chain_id: String,
    pub block_height: u64,
    pub block_time_micros: i64,
    pub transaction_id: String,
    pub transaction_index: u32,
    pub event_index: u32,
    pub event_id: String,
    pub event_kind: String,
    pub market_ids: Vec<String>,
    pub account_ids: Vec<String>,
    pub source_evidence: Vec<WireSourceEvidence>,
    pub confirmation_class: i32,
    pub observed_at_micros: i64,
    pub ingested_at_micros: i64,
    pub canonicalized_at_micros: i64,
    pub payload_hash: Vec<u8>,
    pub parser_version: String,
    pub payload: Vec<u8>,
}

impl WireCanonicalEventEnvelope {
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        generated::hl::canonical::v1::CanonicalEventEnvelope::from(self).encode_to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        generated::hl::canonical::v1::CanonicalEventEnvelope::decode(bytes).map(Into::into)
    }
}

impl From<&WireCanonicalEventEnvelope> for generated::hl::canonical::v1::CanonicalEventEnvelope {
    fn from(value: &WireCanonicalEventEnvelope) -> Self {
        Self {
            schema_version: value.schema_version.clone(),
            chain_id: value.chain_id.clone(),
            block_height: value.block_height,
            block_time_micros: value.block_time_micros,
            transaction_id: value.transaction_id.clone(),
            transaction_index: value.transaction_index,
            event_index: value.event_index,
            event_id: value.event_id.clone(),
            event_kind: value.event_kind.clone(),
            market_ids: value.market_ids.clone(),
            account_ids: value.account_ids.clone(),
            source_evidence: value.source_evidence.iter().map(Into::into).collect(),
            confirmation_class: value.confirmation_class,
            observed_at_micros: value.observed_at_micros,
            ingested_at_micros: value.ingested_at_micros,
            canonicalized_at_micros: value.canonicalized_at_micros,
            payload_hash: value.payload_hash.clone(),
            parser_version: value.parser_version.clone(),
            payload: value.payload.clone(),
        }
    }
}

impl From<&WireSourceEvidence> for generated::hl::canonical::v1::SourceEvidence {
    fn from(value: &WireSourceEvidence) -> Self {
        Self {
            source_id: value.source_id.clone(),
            source_version: value.source_version.clone(),
            source_offset: value.source_offset.clone(),
            content_hash: value.content_hash.clone(),
        }
    }
}

impl From<generated::hl::canonical::v1::CanonicalEventEnvelope> for WireCanonicalEventEnvelope {
    fn from(value: generated::hl::canonical::v1::CanonicalEventEnvelope) -> Self {
        Self {
            schema_version: value.schema_version,
            chain_id: value.chain_id,
            block_height: value.block_height,
            block_time_micros: value.block_time_micros,
            transaction_id: value.transaction_id,
            transaction_index: value.transaction_index,
            event_index: value.event_index,
            event_id: value.event_id,
            event_kind: value.event_kind,
            market_ids: value.market_ids,
            account_ids: value.account_ids,
            source_evidence: value.source_evidence.into_iter().map(Into::into).collect(),
            confirmation_class: value.confirmation_class,
            observed_at_micros: value.observed_at_micros,
            ingested_at_micros: value.ingested_at_micros,
            canonicalized_at_micros: value.canonicalized_at_micros,
            payload_hash: value.payload_hash,
            parser_version: value.parser_version,
            payload: value.payload,
        }
    }
}

impl From<generated::hl::canonical::v1::SourceEvidence> for WireSourceEvidence {
    fn from(value: generated::hl::canonical::v1::SourceEvidence) -> Self {
        Self {
            source_id: value.source_id,
            source_version: value.source_version,
            source_offset: value.source_offset,
            content_hash: value.content_hash,
        }
    }
}
