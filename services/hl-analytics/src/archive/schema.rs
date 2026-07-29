use std::sync::Arc;

use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use storage_ports::ArchiveError;

pub const CANONICAL_SCHEMA_DOCUMENT: &[u8] =
    include_bytes!("../../../../schemas/parquet/canonical-events-v1.json");
pub const RAW_SCHEMA_DOCUMENT: &[u8] =
    include_bytes!("../../../../schemas/parquet/raw-observations-v1.json");

pub fn canonical_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("chain_id", DataType::Utf8, false),
        Field::new("block_height", DataType::UInt64, false),
        Field::new("block_time_micros", DataType::Int64, false),
        Field::new("canonical_block_hash", DataType::FixedSizeBinary(32), false),
        Field::new("confirmation_class", DataType::Utf8, false),
        Field::new("transaction_id", DataType::Utf8, false),
        Field::new("transaction_index", DataType::UInt32, false),
        Field::new("canonical_event_index", DataType::UInt32, false),
        Field::new("event_id", DataType::Utf8, false),
        Field::new("event_kind", DataType::Utf8, false),
        Field::new("schema_version", DataType::Utf8, false),
        Field::new("payload_hash", DataType::FixedSizeBinary(32), false),
        Field::new("canonical_event_envelope_pb", DataType::Binary, false),
    ]))
}

pub fn canonical_schema_fingerprint() -> Result<[u8; 32], ArchiveError> {
    schema_fingerprint(CANONICAL_SCHEMA_DOCUMENT, expected_canonical_document())
}

pub fn raw_schema() -> SchemaRef {
    Arc::new(Schema::new(vec![
        Field::new("chain_id", DataType::Utf8, false),
        Field::new("source_id", DataType::Utf8, false),
        Field::new("source_version", DataType::Utf8, false),
        Field::new("observation_class", DataType::Utf8, false),
        Field::new("cursor_epoch", DataType::Utf8, false),
        Field::new("cursor_offset", DataType::UInt64, false),
        Field::new("received_wall_micros", DataType::Int64, false),
        Field::new("received_monotonic_nanos", DataType::UInt64, false),
        Field::new("parser_schema_version", DataType::Utf8, false),
        Field::new("content_hash", DataType::FixedSizeBinary(32), false),
        Field::new("warnings_json", DataType::Utf8, false),
        Field::new("payload", DataType::Binary, false),
    ]))
}

pub fn raw_schema_fingerprint() -> Result<[u8; 32], ArchiveError> {
    schema_fingerprint(RAW_SCHEMA_DOCUMENT, expected_raw_document())
}

fn schema_fingerprint(bytes: &[u8], expected: SchemaDocument) -> Result<[u8; 32], ArchiveError> {
    let document: SchemaDocument =
        serde_json::from_slice(bytes).map_err(|_| ArchiveError::SchemaMismatch)?;
    if document != expected {
        return Err(ArchiveError::SchemaMismatch);
    }
    let value = serde_json::to_value(document).map_err(|_| ArchiveError::SchemaMismatch)?;
    let canonical = serde_json::to_vec(&value).map_err(|_| ArchiveError::SchemaMismatch)?;
    Ok(Sha256::digest(canonical).into())
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SchemaDocument {
    format: String,
    dataset: String,
    semantic_version: String,
    authoritative_columns: Vec<String>,
    fields: Vec<SchemaField>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct SchemaField {
    name: String,
    r#type: String,
    nullable: bool,
}

fn expected_canonical_document() -> SchemaDocument {
    SchemaDocument {
        format: "hyperliquid-alpha-desk/parquet-schema/v1".to_owned(),
        dataset: "canonical_events".to_owned(),
        semantic_version: "1.0.0".to_owned(),
        authoritative_columns: vec!["canonical_event_envelope_pb".to_owned()],
        fields: vec![
            field("chain_id", "utf8"),
            field("block_height", "uint64"),
            field("block_time_micros", "int64"),
            field("canonical_block_hash", "fixed_size_binary[32]"),
            field("confirmation_class", "utf8"),
            field("transaction_id", "utf8"),
            field("transaction_index", "uint32"),
            field("canonical_event_index", "uint32"),
            field("event_id", "utf8"),
            field("event_kind", "utf8"),
            field("schema_version", "utf8"),
            field("payload_hash", "fixed_size_binary[32]"),
            field("canonical_event_envelope_pb", "binary"),
        ],
    }
}

fn expected_raw_document() -> SchemaDocument {
    SchemaDocument {
        format: "hyperliquid-alpha-desk/parquet-schema/v1".to_owned(),
        dataset: "raw_source_observations".to_owned(),
        semantic_version: "1.0.0".to_owned(),
        authoritative_columns: vec!["payload".to_owned()],
        fields: vec![
            field("chain_id", "utf8"),
            field("source_id", "utf8"),
            field("source_version", "utf8"),
            field("observation_class", "utf8"),
            field("cursor_epoch", "utf8"),
            field("cursor_offset", "uint64"),
            field("received_wall_micros", "int64"),
            field("received_monotonic_nanos", "uint64"),
            field("parser_schema_version", "utf8"),
            field("content_hash", "fixed_size_binary[32]"),
            field("warnings_json", "utf8"),
            field("payload", "binary"),
        ],
    }
}

fn field(name: &str, r#type: &str) -> SchemaField {
    SchemaField {
        name: name.to_owned(),
        r#type: r#type.to_owned(),
        nullable: false,
    }
}
