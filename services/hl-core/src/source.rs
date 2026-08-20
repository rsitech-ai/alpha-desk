use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};

use canonical_events::{BlockEnvelope, ConfirmationClass};
use domain_types::{BlockHeight, ChainId, ProtocolTime, SourceId};
use serde::Deserialize;

pub const LOCAL_REPLAY_BLOCK_SCHEMA: &str = "hl.core.local-replay-block.v1";
pub const SYNTHETIC_UNASSESSED: &str = "synthetic_unassessed";

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlockSourceError {
    #[error("local replay block source failed: {0}")]
    Decode(&'static str),
    #[error("local replay fixture must remain synthetic_unassessed with Stage 1/2 false")]
    Qualification,
    #[error("local replay block file could not be read")]
    Io,
}

pub trait CanonicalBlockSource {
    fn next_block(&mut self) -> Result<Option<BlockEnvelope>, BlockSourceError>;
}

#[derive(Debug, Default)]
pub struct InMemoryBlockSource {
    blocks: VecDeque<BlockEnvelope>,
}

impl InMemoryBlockSource {
    #[must_use]
    pub fn new(blocks: impl IntoIterator<Item = BlockEnvelope>) -> Self {
        Self {
            blocks: blocks.into_iter().collect(),
        }
    }
}

impl CanonicalBlockSource for InMemoryBlockSource {
    fn next_block(&mut self) -> Result<Option<BlockEnvelope>, BlockSourceError> {
        Ok(self.blocks.pop_front())
    }
}

#[derive(Debug)]
pub struct DirectoryBlockSource {
    files: VecDeque<PathBuf>,
}

impl DirectoryBlockSource {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, BlockSourceError> {
        let mut files = fs::read_dir(root.as_ref())
            .map_err(|_| BlockSourceError::Io)?
            .map(|entry| {
                entry
                    .map(|item| item.path())
                    .map_err(|_| BlockSourceError::Io)
            })
            .collect::<Result<Vec<_>, _>>()?;
        files.retain(|path| path.extension().is_some_and(|ext| ext == "json"));
        files.sort();
        Ok(Self {
            files: files.into(),
        })
    }
}

impl CanonicalBlockSource for DirectoryBlockSource {
    fn next_block(&mut self) -> Result<Option<BlockEnvelope>, BlockSourceError> {
        let Some(path) = self.files.pop_front() else {
            return Ok(None);
        };
        let json = fs::read_to_string(path).map_err(|_| BlockSourceError::Io)?;
        Ok(Some(decode_local_replay_block(&json)?))
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalReplayBlockFile {
    schema: String,
    source_qualification: String,
    stage_1_qualified: bool,
    stage_2_qualified: bool,
    chain_id: String,
    block_height: u64,
    block_time_micros: i64,
    confirmation_class: String,
    source_block_hashes: BTreeMap<String, String>,
}

pub fn decode_local_replay_block(json: &str) -> Result<BlockEnvelope, BlockSourceError> {
    let file: LocalReplayBlockFile =
        serde_json::from_str(json).map_err(|_| BlockSourceError::Decode("invalid json"))?;
    if file.schema != LOCAL_REPLAY_BLOCK_SCHEMA {
        return Err(BlockSourceError::Decode("unsupported schema"));
    }
    if file.source_qualification != SYNTHETIC_UNASSESSED
        || file.stage_1_qualified
        || file.stage_2_qualified
    {
        return Err(BlockSourceError::Qualification);
    }
    let confirmation = parse_confirmation(&file.confirmation_class)?;
    let mut hashes = BTreeMap::new();
    for (source, digest) in file.source_block_hashes {
        let source =
            SourceId::new(source).map_err(|_| BlockSourceError::Decode("invalid source"))?;
        hashes.insert(source, parse_hash32(&digest)?);
    }
    BlockEnvelope::try_new(
        ChainId::new(file.chain_id).map_err(|_| BlockSourceError::Decode("invalid chain"))?,
        BlockHeight::new(file.block_height),
        ProtocolTime::from_unix_micros(file.block_time_micros)
            .map_err(|_| BlockSourceError::Decode("invalid block time"))?,
        confirmation,
        Vec::new(),
        hashes,
    )
    .map_err(|_| BlockSourceError::Decode("invalid block envelope"))
}

#[must_use]
pub const fn confirmation_label(class: ConfirmationClass) -> &'static str {
    match class {
        ConfirmationClass::ProvisionalSource => "provisional-source",
        ConfirmationClass::CommittedPrimary => "committed-primary",
        ConfirmationClass::CommittedIndependent => "committed-independent",
        ConfirmationClass::ReconciledSnapshot => "reconciled-snapshot",
        ConfirmationClass::Corrected => "corrected",
        ConfirmationClass::Expired => "expired",
    }
}

fn parse_confirmation(value: &str) -> Result<ConfirmationClass, BlockSourceError> {
    match value {
        "provisional-source" => Ok(ConfirmationClass::ProvisionalSource),
        "committed-primary" => Ok(ConfirmationClass::CommittedPrimary),
        "committed-independent" => Ok(ConfirmationClass::CommittedIndependent),
        "reconciled-snapshot" => Ok(ConfirmationClass::ReconciledSnapshot),
        "corrected" => Ok(ConfirmationClass::Corrected),
        "expired" => Ok(ConfirmationClass::Expired),
        _ => Err(BlockSourceError::Decode("unknown confirmation class")),
    }
}

fn parse_hash32(value: &str) -> Result<[u8; 32], BlockSourceError> {
    let bytes = hex::decode(value).map_err(|_| BlockSourceError::Decode("invalid hash"))?;
    <[u8; 32]>::try_from(bytes).map_err(|_| BlockSourceError::Decode("hash must be 32 bytes"))
}
