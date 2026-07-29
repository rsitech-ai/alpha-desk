#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use canonical_events::{
    BlockEnvelope, BlockError, CanonicalEventEnvelope, MappingDisposition, MappingError,
    MarketCatalogV1, NodeV1MappingContext, map_node_v1_record,
};
use domain_types::{ChainId, KnownTime, MarketId, SourceId};
use hl_protocol::SourceError;
use hl_protocol::node::v1::{NodeStreamKind, parse_node_record};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const OUTPUT_SCHEMA: &str = "alpha-desk.canonical-inspect.v1";
const QUALIFIED_CORPUS: &str = "normalized-public-documentation-example";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectionSummary {
    event_count: usize,
    output_sha256: String,
}

impl InspectionSummary {
    #[must_use]
    pub const fn event_count(&self) -> usize {
        self.event_count
    }

    #[must_use]
    pub fn output_sha256(&self) -> &str {
        &self.output_sha256
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InspectManifest {
    schema_version: u32,
    fixture_id: String,
    qualification: String,
    production_recording: bool,
    source_path: String,
    source_sha256: String,
    stream: NodeStreamKind,
    chain_id: String,
    source_id: String,
    source_version: String,
    source_offset: String,
    observed_at_micros: i64,
    ingested_at_micros: i64,
    canonicalized_at_micros: i64,
    mapper_version: String,
    catalog_version: String,
    expected_disposition: ExpectedDisposition,
    #[serde(default)]
    market: Vec<MarketEntry>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum ExpectedDisposition {
    Mapped,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct MarketEntry {
    symbol: String,
    market_id: String,
}

#[derive(Debug, Serialize)]
struct InspectionManifest {
    schema: &'static str,
    fixture_id: String,
    qualification: String,
    production_recording: bool,
    source_path: String,
    source_sha256: String,
    source_blake3: String,
    mapping_disposition: &'static str,
    market_catalog_version: String,
    canonical_schema_versions: Vec<String>,
    event_count: usize,
    events: Vec<EventInspection>,
    block: BlockInspection,
}

#[derive(Debug, Serialize)]
struct EventInspection {
    transaction_id: String,
    transaction_index: u32,
    event_index: u32,
    event_id: String,
    event_kind: String,
    payload_blake3: String,
    source_event_index: Option<u32>,
}

#[derive(Debug, Serialize)]
struct BlockInspection {
    chain_id: String,
    block_height: u64,
    block_time_micros: i64,
    confirmation_class: &'static str,
    canonical_block_blake3: String,
}

pub fn canonicalize(
    root: impl AsRef<Path>,
    manifest_path: impl AsRef<Path>,
    output_path: impl AsRef<Path>,
) -> Result<InspectionSummary, InspectError> {
    let root = fs::canonicalize(root.as_ref()).map_err(|source| InspectError::Io {
        operation: "canonicalize root",
        source,
    })?;
    if !root.is_dir() {
        return Err(InspectError::UnsafePath);
    }
    let manifest_path = resolve_regular_file(&root, manifest_path.as_ref())?;
    let manifest_bytes = fs::read(&manifest_path).map_err(|source| InspectError::Io {
        operation: "read manifest",
        source,
    })?;
    let manifest: InspectManifest =
        toml::from_slice(&manifest_bytes).map_err(|error| InspectError::InvalidManifest {
            reason: error.to_string(),
        })?;
    validate_manifest(&manifest)?;

    let source_path = resolve_regular_file(&root, Path::new(&manifest.source_path))?;
    let source_bytes = fs::read(&source_path).map_err(|source| InspectError::Io {
        operation: "read source fixture",
        source,
    })?;
    let source_sha256 = hex::encode(Sha256::digest(&source_bytes));
    if source_sha256 != manifest.source_sha256 {
        return Err(InspectError::SourceHashMismatch);
    }
    let record = parse_node_record(manifest.stream, source_bytes.into())?;
    let catalog_entries = manifest
        .market
        .iter()
        .map(|entry| {
            MarketId::new(entry.market_id.clone())
                .map(|market_id| (entry.symbol.as_str(), market_id))
                .map_err(|error| InspectError::InvalidManifest {
                    reason: format!("invalid market_id: {error}"),
                })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let catalog = MarketCatalogV1::try_new(manifest.catalog_version.clone(), catalog_entries)?;
    let source_id = SourceId::new(manifest.source_id.clone()).map_err(|error| {
        InspectError::InvalidManifest {
            reason: format!("invalid source_id: {error}"),
        }
    })?;
    let context = NodeV1MappingContext {
        chain_id: ChainId::new(manifest.chain_id.clone()).map_err(|error| {
            InspectError::InvalidManifest {
                reason: format!("invalid chain_id: {error}"),
            }
        })?,
        source_id: source_id.clone(),
        source_version: manifest.source_version.clone(),
        source_offset: manifest.source_offset.clone(),
        observed_at: known_time(manifest.observed_at_micros, "observed_at_micros")?,
        ingested_at: known_time(manifest.ingested_at_micros, "ingested_at_micros")?,
        canonicalized_at: known_time(manifest.canonicalized_at_micros, "canonicalized_at_micros")?,
        mapper_version: manifest.mapper_version.clone(),
    };
    let disposition = map_node_v1_record(&record, &catalog, &context)?;
    let MappingDisposition::Mapped(events) = disposition else {
        return Err(InspectError::UnexpectedDisposition);
    };
    if manifest.expected_disposition != ExpectedDisposition::Mapped || events.is_empty() {
        return Err(InspectError::UnexpectedDisposition);
    }

    let block = build_block(&events, &source_id, *record.content_hash().as_bytes())?;
    let output = build_output(manifest, source_sha256, &record, &events, &block);
    let mut bytes = serde_json::to_vec(&output).map_err(InspectError::Serialize)?;
    bytes.push(b'\n');
    write_atomic_new(output_path.as_ref(), &bytes)?;

    Ok(InspectionSummary {
        event_count: events.len(),
        output_sha256: hex::encode(Sha256::digest(&bytes)),
    })
}

fn validate_manifest(manifest: &InspectManifest) -> Result<(), InspectError> {
    if manifest.schema_version != 1
        || manifest.qualification != QUALIFIED_CORPUS
        || manifest.production_recording
        || manifest.fixture_id.is_empty()
        || manifest.fixture_id.trim() != manifest.fixture_id
        || !is_sha256(&manifest.source_sha256)
    {
        return Err(InspectError::InvalidManifest {
            reason: "manifest qualification or identity contract is invalid".to_owned(),
        });
    }
    Ok(())
}

fn build_block(
    events: &[CanonicalEventEnvelope],
    source_id: &SourceId,
    source_hash: [u8; 32],
) -> Result<BlockEnvelope, InspectError> {
    let first = events.first().ok_or(InspectError::UnexpectedDisposition)?;
    let mut source_hashes = BTreeMap::new();
    source_hashes.insert(source_id.clone(), source_hash);
    BlockEnvelope::try_new(
        first.chain_id().clone(),
        first.block_height(),
        first.block_time(),
        first.confirmation_class(),
        events.to_vec(),
        source_hashes,
    )
    .map_err(Into::into)
}

fn build_output(
    manifest: InspectManifest,
    source_sha256: String,
    record: &hl_protocol::node::v1::NodeRecordV1,
    events: &[CanonicalEventEnvelope],
    block: &BlockEnvelope,
) -> InspectionManifest {
    let canonical_schema_versions = events
        .iter()
        .map(|event| event.schema_version().to_owned())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let events = events
        .iter()
        .map(|event| EventInspection {
            transaction_id: event.transaction_id().to_string(),
            transaction_index: event.transaction_index(),
            event_index: event.canonical_event_index(),
            event_id: event.event_id().to_string(),
            event_kind: event.event_kind().as_wire_name().to_owned(),
            payload_blake3: hex::encode(event.payload_hash()),
            source_event_index: event.source_evidence()[0].source_event_index(),
        })
        .collect::<Vec<_>>();

    InspectionManifest {
        schema: OUTPUT_SCHEMA,
        fixture_id: manifest.fixture_id,
        qualification: manifest.qualification,
        production_recording: manifest.production_recording,
        source_path: manifest.source_path,
        source_sha256,
        source_blake3: record.content_hash().to_hex().to_string(),
        mapping_disposition: "mapped-provisional",
        market_catalog_version: manifest.catalog_version,
        canonical_schema_versions,
        event_count: events.len(),
        events,
        block: BlockInspection {
            chain_id: block.chain_id().to_string(),
            block_height: block.block_height().get(),
            block_time_micros: block.block_time().unix_micros(),
            confirmation_class: "provisional-source",
            canonical_block_blake3: hex::encode(block.canonical_block_hash()),
        },
    }
}

fn known_time(value: i64, field: &'static str) -> Result<KnownTime, InspectError> {
    KnownTime::from_unix_micros(value).map_err(|error| InspectError::InvalidManifest {
        reason: format!("invalid {field}: {error}"),
    })
}

fn resolve_regular_file(root: &Path, relative: &Path) -> Result<PathBuf, InspectError> {
    if relative.as_os_str().is_empty()
        || relative.is_absolute()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(InspectError::UnsafePath);
    }
    let mut candidate = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(InspectError::UnsafePath);
        };
        candidate.push(component);
        let metadata = fs::symlink_metadata(&candidate).map_err(|source| InspectError::Io {
            operation: "inspect input path",
            source,
        })?;
        if metadata.file_type().is_symlink() {
            return Err(InspectError::UnsafePath);
        }
    }
    if !candidate.is_file() {
        return Err(InspectError::UnsafePath);
    }
    Ok(candidate)
}

fn write_atomic_new(path: &Path, bytes: &[u8]) -> Result<(), InspectError> {
    if path.as_os_str().is_empty() || path.file_name().is_none() {
        return Err(InspectError::UnsafeOutputPath);
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    if path.exists() {
        return Err(InspectError::OutputExists);
    }
    let mut staged =
        tempfile::NamedTempFile::new_in(parent).map_err(|source| InspectError::Io {
            operation: "create staged output",
            source,
        })?;
    staged
        .write_all(bytes)
        .and_then(|()| staged.flush())
        .and_then(|()| staged.as_file().sync_all())
        .map_err(|source| InspectError::Io {
            operation: "write staged output",
            source,
        })?;
    staged.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            InspectError::OutputExists
        } else {
            InspectError::Io {
                operation: "publish output",
                source: error.error,
            }
        }
    })?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| InspectError::Io {
            operation: "sync output directory",
            source,
        })
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Debug, thiserror::Error)]
pub enum InspectError {
    #[error("unsafe input path")]
    UnsafePath,
    #[error("unsafe output path")]
    UnsafeOutputPath,
    #[error("invalid inspection manifest: {reason}")]
    InvalidManifest { reason: String },
    #[error("source fixture SHA-256 does not match the manifest")]
    SourceHashMismatch,
    #[error("source fixture failed parsing: {0}")]
    Source(#[from] SourceError),
    #[error("source fixture failed canonical mapping: {0}")]
    Mapping(#[from] MappingError),
    #[error("mapping disposition does not match the required mapped contract")]
    UnexpectedDisposition,
    #[error("canonical block validation failed: {0}")]
    Block(#[from] BlockError),
    #[error("inspection output serialization failed: {0}")]
    Serialize(#[source] serde_json::Error),
    #[error("inspection output already exists")]
    OutputExists,
    #[error("{operation} failed: {source}")]
    Io {
        operation: &'static str,
        #[source]
        source: std::io::Error,
    },
}

impl InspectError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::UnsafePath => "canonical_inspect.unsafe_input_path",
            Self::UnsafeOutputPath => "canonical_inspect.unsafe_output_path",
            Self::InvalidManifest { .. } => "canonical_inspect.invalid_manifest",
            Self::SourceHashMismatch => "canonical_inspect.source_hash_mismatch",
            Self::Source(_) => "canonical_inspect.source_rejected",
            Self::Mapping(_) => "canonical_inspect.mapping_rejected",
            Self::UnexpectedDisposition => "canonical_inspect.unexpected_disposition",
            Self::Block(_) => "canonical_inspect.block_rejected",
            Self::Serialize(_) => "canonical_inspect.serialize_failed",
            Self::OutputExists => "canonical_inspect.output_exists",
            Self::Io { .. } => "canonical_inspect.io_failed",
        }
    }
}
