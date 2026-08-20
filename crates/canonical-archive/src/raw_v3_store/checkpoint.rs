use std::path::{Path, PathBuf};

use domain_types::{ChainId, ManifestId, SourceId};
use serde::{Deserialize, Serialize};
use storage_ports::{
    ArchiveError, LocalRecordSequence, LocalRecordSequenceRange, RawArchiveCheckpointEntriesV2,
    RawArchiveCheckpointEntryV2,
};

use super::{
    RawV3Archive, dataset_relative, leaf_contains_manifest, lease_root, load_current_root,
    load_packs_for_tree, load_verified_root, verify_logical_at_sequence, walk_logical_leaves,
};
use crate::{
    fs, manifest,
    raw_v3::{RootBundleV3, root_bundle_hash},
};

pub const RAW_CHECKPOINT_SCHEMA_V1: &str = "hyperliquid-alpha-desk/archive-raw-checkpoint/v1";
pub const RAW_CHECKPOINT_SCHEMA_V2: &str = "hyperliquid-alpha-desk/archive-raw-checkpoint/v2";
const MAX_CHECKPOINT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawArchiveCheckpoint {
    V1(RawArchiveCheckpointV1),
    V2(RawArchiveCheckpointV2),
}

impl RawArchiveCheckpoint {
    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        match self {
            Self::V1(value) => value.sha256,
            Self::V2(value) => value.sha256,
        }
    }

    #[must_use]
    pub const fn root_sha256(&self) -> [u8; 32] {
        match self {
            Self::V1(value) => value.root_hash,
            Self::V2(value) => value.root_hash,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawArchiveCheckpointV1 {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    original_manifest_ids: Vec<String>,
    first_local_sequence: u64,
    last_local_sequence: u64,
    #[serde(skip)]
    sha256: [u8; 32],
    #[serde(skip)]
    root_hash: [u8; 32],
}

impl RawArchiveCheckpointV1 {
    pub fn try_new(
        root_sha256: [u8; 32],
        chain_id: ChainId,
        source_id: SourceId,
        original_manifest_ids: Vec<ManifestId>,
        range: LocalRecordSequenceRange,
    ) -> Result<Self, ArchiveError> {
        if original_manifest_ids.is_empty() || original_manifest_ids.len() > 4_096 {
            return Err(ArchiveError::InvalidInput(
                "raw archive checkpoint V1 receipt count",
            ));
        }
        let mut unique = original_manifest_ids
            .iter()
            .map(ManifestId::as_str)
            .collect::<Vec<_>>();
        unique.sort_unstable();
        if unique.windows(2).any(|pair| pair[0] == pair[1]) {
            return Err(ArchiveError::InvalidInput(
                "raw archive checkpoint V1 receipt keys are duplicated",
            ));
        }
        let value = Self {
            schema: RAW_CHECKPOINT_SCHEMA_V1.to_owned(),
            root_sha256: hex::encode(root_sha256),
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            original_manifest_ids: original_manifest_ids
                .iter()
                .map(|manifest| manifest.as_str().to_owned())
                .collect(),
            first_local_sequence: range.start().get(),
            last_local_sequence: range.end().get(),
            sha256: [0; 32],
            root_hash: root_sha256,
        };
        let bytes = manifest::canonical_json(&value)?;
        Ok(Self {
            sha256: manifest::sha256(&bytes),
            ..value
        })
    }

    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn root_hash(&self) -> [u8; 32] {
        self.root_hash
    }

    #[must_use]
    pub fn original_manifest_ids(&self) -> &[String] {
        &self.original_manifest_ids
    }

    #[must_use]
    pub const fn first_local_sequence(&self) -> u64 {
        self.first_local_sequence
    }

    #[must_use]
    pub const fn last_local_sequence(&self) -> u64 {
        self.last_local_sequence
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawArchiveCheckpointV2 {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    entries: Vec<CheckpointEntryWireV2>,
    #[serde(skip)]
    sha256: [u8; 32],
    #[serde(skip)]
    root_hash: [u8; 32],
    #[serde(skip)]
    decoded_entries: RawArchiveCheckpointEntriesV2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointEntryWireV2 {
    original_manifest_id: String,
    original_manifest_sha256: String,
    first_local_sequence: u64,
    last_local_sequence: u64,
}

impl RawArchiveCheckpointV2 {
    pub fn try_new(
        root_sha256: [u8; 32],
        chain_id: ChainId,
        source_id: SourceId,
        entries: RawArchiveCheckpointEntriesV2,
    ) -> Result<Self, ArchiveError> {
        let wire_entries = entries
            .entries()
            .iter()
            .map(|entry| CheckpointEntryWireV2 {
                original_manifest_id: entry.manifest_id().as_str().to_owned(),
                original_manifest_sha256: hex::encode(entry.manifest_sha256()),
                first_local_sequence: entry.local_sequence_range().start().get(),
                last_local_sequence: entry.local_sequence_range().end().get(),
            })
            .collect();
        let value = Self {
            schema: RAW_CHECKPOINT_SCHEMA_V2.to_owned(),
            root_sha256: hex::encode(root_sha256),
            chain_id: chain_id.as_str().to_owned(),
            source_id: source_id.as_str().to_owned(),
            entries: wire_entries,
            sha256: [0; 32],
            root_hash: root_sha256,
            decoded_entries: entries,
        };
        let bytes = manifest::canonical_json(&value)?;
        Ok(Self {
            sha256: manifest::sha256(&bytes),
            ..value
        })
    }

    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn root_hash(&self) -> [u8; 32] {
        self.root_hash
    }

    #[must_use]
    pub const fn entries(&self) -> &RawArchiveCheckpointEntriesV2 {
        &self.decoded_entries
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointWireV1 {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    original_manifest_ids: Vec<String>,
    first_local_sequence: u64,
    last_local_sequence: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CheckpointWireV2 {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    entries: Vec<CheckpointEntryWireV2>,
}

pub fn publish_checkpoint_v1(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    original_manifest_ids: &[ManifestId],
    range: LocalRecordSequenceRange,
) -> Result<[u8; 32], ArchiveError> {
    let (root, journal_bytes) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let _lease = lease_root(archive, chain, source, &root)?;
    authenticate_v1_ids(archive, &root, &journal_bytes, original_manifest_ids)?;
    let checkpoint = RawArchiveCheckpointV1::try_new(
        root_bundle_hash(&root)?,
        chain.clone(),
        source.clone(),
        original_manifest_ids.to_vec(),
        range,
    )?;
    publish_checkpoint_bytes(
        archive,
        chain,
        source,
        &manifest::canonical_json(&checkpoint)?,
    )
}

pub fn publish_checkpoint_v2(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    entries: RawArchiveCheckpointEntriesV2,
) -> Result<[u8; 32], ArchiveError> {
    let (root, journal_bytes) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    publish_checkpoint_v2_on(archive, chain, source, &root, &journal_bytes, entries)
}

pub fn publish_checkpoint_v2_on(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    entries: RawArchiveCheckpointEntriesV2,
) -> Result<[u8; 32], ArchiveError> {
    let _lease = lease_root(archive, chain, source, root)?;
    authenticate_v2_entries(archive, root, journal_bytes, &entries)?;
    let checkpoint = RawArchiveCheckpointV2::try_new(
        root_bundle_hash(root)?,
        chain.clone(),
        source.clone(),
        entries,
    )?;
    publish_checkpoint_bytes(
        archive,
        chain,
        source,
        &manifest::canonical_json(&checkpoint)?,
    )
}

pub fn switch_checkpoint_current(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    expected_current: Option<[u8; 32]>,
    target: [u8; 32],
) -> Result<(), ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let expected_bytes = match expected_current {
        None => None,
        Some(hash) => Some(pointer_bytes(&dataset, hash)?),
    };
    let loaded = load_checkpoint_file(archive, chain, source, target)?;
    authenticate_loaded(archive, chain, source, &loaded)?;
    let pointer_bytes = pointer_bytes(&dataset, target)?;
    fs::publish_current_cas(
        &archive.root,
        &checkpoint_current_relative(&dataset),
        expected_bytes.as_deref(),
        &pointer_bytes,
    )?;
    let readback = load_checkpoint(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("checkpoint CURRENT readback is missing"),
    )?;
    if readback.sha256() != target {
        return Err(ArchiveError::ManifestVerification(
            "checkpoint CURRENT readback does not bind the switched document",
        ));
    }
    Ok(())
}

pub fn load_checkpoint(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<Option<RawArchiveCheckpoint>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let Some(bytes) = fs::try_read_regular(
        &archive.root,
        &checkpoint_current_relative(&dataset),
        64 * 1024,
    )?
    else {
        return Ok(None);
    };
    let pointer: manifest::CurrentPointerV1 = serde_json::from_slice(&bytes).map_err(|_| {
        ArchiveError::ManifestVerification("invalid raw V3 checkpoint current pointer JSON")
    })?;
    if pointer.schema != manifest::CURRENT_POINTER_SCHEMA_V1 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported raw V3 checkpoint current pointer schema",
        ));
    }
    let hash = manifest::parse_hash(&pointer.manifest_sha256)?;
    let expected = checkpoint_relative(&dataset, hash);
    if Path::new(&pointer.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "checkpoint CURRENT does not bind the exact checkpoint path",
        ));
    }
    let loaded = load_checkpoint_file(archive, chain, source, hash)?;
    if loaded.sha256() != hash {
        return Err(ArchiveError::ManifestVerification(
            "checkpoint CURRENT hash mismatch",
        ));
    }
    Ok(Some(loaded))
}

pub fn checkpoint_root_sha256(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<Option<[u8; 32]>, ArchiveError> {
    Ok(load_checkpoint(archive, chain, source)?.map(|checkpoint| checkpoint.root_sha256()))
}

fn publish_checkpoint_bytes(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    bytes: &[u8],
) -> Result<[u8; 32], ArchiveError> {
    let hash = manifest::sha256(bytes);
    let dataset = dataset_relative(chain, source);
    fs::publish_immutable(&archive.root, &checkpoint_relative(&dataset, hash), bytes)?;
    let readback = fs::read_regular(
        &archive.root,
        &checkpoint_relative(&dataset, hash),
        MAX_CHECKPOINT_BYTES,
    )?;
    if manifest::sha256(&readback) != hash || readback != bytes {
        return Err(ArchiveError::ManifestVerification(
            "checkpoint readback does not match published bytes",
        ));
    }
    Ok(hash)
}

fn load_checkpoint_file(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    hash: [u8; 32],
) -> Result<RawArchiveCheckpoint, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let bytes = fs::read_regular(
        &archive.root,
        &checkpoint_relative(&dataset, hash),
        MAX_CHECKPOINT_BYTES,
    )?;
    if manifest::sha256(&bytes) != hash {
        return Err(ArchiveError::ManifestVerification(
            "checkpoint path does not bind exact bytes",
        ));
    }
    parse_checkpoint(&bytes, chain, source)
}

fn parse_checkpoint(
    bytes: &[u8],
    chain: &ChainId,
    source: &SourceId,
) -> Result<RawArchiveCheckpoint, ArchiveError> {
    if let Ok(wire) = serde_json::from_slice::<CheckpointWireV1>(bytes) {
        if wire.schema != RAW_CHECKPOINT_SCHEMA_V1 {
            return Err(ArchiveError::ManifestVerification(
                "unsupported raw archive checkpoint schema",
            ));
        }
        let ids = wire
            .original_manifest_ids
            .iter()
            .map(|id| {
                ManifestId::new(id.clone()).map_err(|_| {
                    ArchiveError::ManifestVerification("invalid checkpoint manifest ID")
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let reconstructed = RawArchiveCheckpointV1::try_new(
            manifest::parse_hash(&wire.root_sha256)?,
            ChainId::new(wire.chain_id)
                .map_err(|_| ArchiveError::ManifestVerification("invalid checkpoint chain"))?,
            SourceId::new(wire.source_id)
                .map_err(|_| ArchiveError::ManifestVerification("invalid checkpoint source"))?,
            ids,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(wire.first_local_sequence)?,
                LocalRecordSequence::try_new(wire.last_local_sequence)?,
            )?,
        )?;
        if reconstructed.chain_id != chain.as_str() || reconstructed.source_id != source.as_str() {
            return Err(ArchiveError::ManifestVerification(
                "checkpoint chain or source mismatch",
            ));
        }
        if manifest::canonical_json(&reconstructed)? != bytes {
            return Err(ArchiveError::ManifestVerification(
                "checkpoint V1 canonical bytes are invalid",
            ));
        }
        return Ok(RawArchiveCheckpoint::V1(reconstructed));
    }
    let wire: CheckpointWireV2 = serde_json::from_slice(bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid raw archive checkpoint JSON"))?;
    if wire.schema != RAW_CHECKPOINT_SCHEMA_V2 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported raw archive checkpoint schema",
        ));
    }
    let mut entries = Vec::with_capacity(wire.entries.len());
    for entry in wire.entries {
        let manifest_id = ManifestId::new(entry.original_manifest_id)
            .map_err(|_| ArchiveError::ManifestVerification("invalid checkpoint V2 manifest ID"))?;
        entries.push(RawArchiveCheckpointEntryV2::new(
            manifest_id,
            manifest::parse_hash(&entry.original_manifest_sha256)?,
            LocalRecordSequenceRange::try_new(
                LocalRecordSequence::try_new(entry.first_local_sequence)?,
                LocalRecordSequence::try_new(entry.last_local_sequence)?,
            )?,
        ));
    }
    let reconstructed = RawArchiveCheckpointV2::try_new(
        manifest::parse_hash(&wire.root_sha256)?,
        ChainId::new(wire.chain_id)
            .map_err(|_| ArchiveError::ManifestVerification("invalid checkpoint chain"))?,
        SourceId::new(wire.source_id)
            .map_err(|_| ArchiveError::ManifestVerification("invalid checkpoint source"))?,
        RawArchiveCheckpointEntriesV2::try_new(entries)?,
    )?;
    if reconstructed.chain_id != chain.as_str() || reconstructed.source_id != source.as_str() {
        return Err(ArchiveError::ManifestVerification(
            "checkpoint chain or source mismatch",
        ));
    }
    if manifest::canonical_json(&reconstructed)? != bytes {
        return Err(ArchiveError::ManifestVerification(
            "checkpoint V2 canonical bytes are invalid",
        ));
    }
    Ok(RawArchiveCheckpoint::V2(reconstructed))
}

fn authenticate_loaded(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    loaded: &RawArchiveCheckpoint,
) -> Result<(), ArchiveError> {
    let root_hash = loaded.root_sha256();
    let (root, journal_bytes) = load_verified_root(archive, chain, source, root_hash)?;
    let _lease = lease_root(archive, chain, source, &root)?;
    match loaded {
        RawArchiveCheckpoint::V1(value) => {
            let ids = value
                .original_manifest_ids
                .iter()
                .map(|id| {
                    ManifestId::new(id.clone()).map_err(|_| {
                        ArchiveError::ManifestVerification("invalid checkpoint manifest ID")
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            authenticate_v1_ids(archive, &root, &journal_bytes, &ids)
        }
        RawArchiveCheckpoint::V2(value) => {
            authenticate_v2_entries(archive, &root, &journal_bytes, value.entries())
        }
    }
}

fn authenticate_v1_ids(
    archive: &RawV3Archive,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    original_manifest_ids: &[ManifestId],
) -> Result<(), ArchiveError> {
    let chain = root.chain_id()?;
    let source = root.source_id()?;
    let packs = load_packs_for_tree(
        archive,
        &chain,
        &source,
        root.sequence_root(),
        journal_bytes,
    )?;
    for manifest_id in original_manifest_ids {
        let hash = manifest::hash_from_manifest_id(manifest_id)?;
        let mut found = false;
        walk_logical_leaves(root.sequence_root(), journal_bytes, &packs, &mut |entry| {
            if leaf_contains_manifest(archive, &chain, &source, entry, hash)? {
                found = true;
                Ok(true)
            } else {
                Ok(false)
            }
        })?;
        if !found {
            return Err(ArchiveError::ManifestVerification(
                "checkpoint V1 receipt is not authenticated by the bound root",
            ));
        }
    }
    Ok(())
}

fn authenticate_v2_entries(
    archive: &RawV3Archive,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    entries: &RawArchiveCheckpointEntriesV2,
) -> Result<(), ArchiveError> {
    for entry in entries.entries() {
        verify_logical_at_sequence(
            archive,
            root,
            journal_bytes,
            entry.manifest_sha256(),
            entry.local_sequence_range(),
        )?;
    }
    Ok(())
}

fn pointer_bytes(dataset: &Path, hash: [u8; 32]) -> Result<Vec<u8>, ArchiveError> {
    let relative = checkpoint_relative(dataset, hash);
    manifest::canonical_json(&manifest::CurrentPointerV1 {
        schema: manifest::CURRENT_POINTER_SCHEMA_V1.to_owned(),
        manifest_relative_path: crate::raw::path_string(&relative)?,
        manifest_sha256: hex::encode(hash),
    })
}

fn checkpoint_current_relative(dataset: &Path) -> PathBuf {
    dataset.join("checkpoints").join("CURRENT")
}

fn checkpoint_relative(dataset: &Path, hash: [u8; 32]) -> PathBuf {
    dataset
        .join("checkpoints")
        .join(format!("checkpoint-{}.json", hex::encode(hash)))
}
