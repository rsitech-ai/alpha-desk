use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use domain_types::{ChainId, ManifestId, SourceId};
use serde::{Deserialize, Serialize};
use storage_ports::{ArchiveError, LocalRecordSequence, RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES};

use super::{
    RawV3Archive, dataset_relative, lease_root, load_current_root, load_pack_manifest,
    load_packs_for_tree, verify_logical_at_sequence, walk_logical_leaves,
};
use crate::{
    fs, manifest,
    raw_v3::{
        BuiltIndexPackV3, IndexPackPageKindV3, ReceiptHintEntryV3, ReceiptHintPageV3,
        parse_logical_commit_manifest, parse_receipt_hint_page, root_bundle_hash,
    },
};

const RAW_HINT_INDEX_SCHEMA_V3: &str = "hyperliquid-alpha-desk/archive-raw-receipt-hint-index/v3";
const HINT_PAGE_FANOUT: usize = 256;
const MAX_HINT_INDEX_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReceiptHintIndexV3 {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    pages: Vec<ReceiptHintIndexPageRefV3>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptHintIndexPageRefV3 {
    relative_path: String,
    sha256: String,
    first_manifest_sha256: String,
    last_manifest_sha256: String,
    offset: u64,
    length: u64,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptHintIndexWireV3 {
    schema: String,
    root_sha256: String,
    chain_id: String,
    source_id: String,
    pages: Vec<ReceiptHintIndexPageRefV3>,
}

pub fn rebuild_receipt_hints(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<[u8; 32], ArchiveError> {
    let (root, journal_bytes) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let _lease = lease_root(archive, chain, source, &root)?;
    let root_hash = root_bundle_hash(&root)?;
    let pages = pages_for_tree(archive, chain, source, &root, &journal_bytes)?;
    let dataset = dataset_relative(chain, source);
    let mut refs = Vec::new();
    for page in &pages {
        let bytes = manifest::canonical_json(page)?;
        let hash = manifest::sha256(&bytes);
        let relative = hint_page_relative(&dataset, hash);
        fs::publish_immutable(&archive.root, &relative, &bytes)?;
        let first = page.entries()[0].manifest_sha256()?;
        let last = page.entries()[page.entries().len() - 1].manifest_sha256()?;
        let length = u64::try_from(bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("receipt hint page exceeds u64"))?;
        refs.push(ReceiptHintIndexPageRefV3 {
            relative_path: crate::raw::path_string(&relative)?,
            sha256: hex::encode(hash),
            first_manifest_sha256: hex::encode(first),
            last_manifest_sha256: hex::encode(last),
            offset: 0,
            length,
        });
    }
    publish_hint_index(archive, chain, source, root_hash, &dataset, refs)
}

pub(super) fn pages_for_tree(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &crate::raw_v3::RootBundleV3,
    journal_bytes: &[u8],
) -> Result<Vec<ReceiptHintPageV3>, ArchiveError> {
    let mut entries = collect_hint_entries(archive, chain, source, root, journal_bytes)?;
    let mut keyed = entries
        .drain(..)
        .map(|entry| Ok((entry.manifest_sha256()?, entry)))
        .collect::<Result<Vec<_>, ArchiveError>>()?;
    keyed.sort_by_key(|left| left.0);
    if keyed.windows(2).any(|pair| pair[0].0 == pair[1].0) {
        return Err(ArchiveError::ManifestVerification(
            "receipt hint keys are duplicated",
        ));
    }
    let entries: Vec<_> = keyed.into_iter().map(|(_, entry)| entry).collect();
    if entries.is_empty() {
        return Err(ArchiveError::InvalidInput(
            "receipt hint rebuild requires at least one logical commit",
        ));
    }
    let mut pages = Vec::new();
    for chunk in entries.chunks(HINT_PAGE_FANOUT) {
        pages.push(ReceiptHintPageV3::try_new(
            chain.clone(),
            source.clone(),
            chunk.to_vec(),
        )?);
    }
    Ok(pages)
}

pub(super) fn publish_from_index_pack(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root_hash: [u8; 32],
    dataset: &Path,
    pack: &BuiltIndexPackV3,
    hint_pages: &[ReceiptHintPageV3],
) -> Result<[u8; 32], ArchiveError> {
    let pack_relative = dataset.join(pack.manifest().object_relative_path());
    let mut refs = Vec::new();
    let pack_pages = pack
        .manifest()
        .pages()
        .iter()
        .filter(|page| page.kind() == IndexPackPageKindV3::ReceiptHint);
    for (built, locator) in hint_pages.iter().zip(pack_pages) {
        let bytes = manifest::canonical_json(built)?;
        let hash = manifest::sha256(&bytes);
        let length = u64::try_from(bytes.len())
            .map_err(|_| ArchiveError::InvalidInput("receipt hint page exceeds u64"))?;
        if locator.length() != length {
            return Err(ArchiveError::ManifestVerification(
                "index pack receipt hint page length mismatch",
            ));
        }
        let start = usize::try_from(locator.offset()).map_err(|_| {
            ArchiveError::ManifestVerification(
                "index pack receipt hint offset exceeds address space",
            )
        })?;
        let end = start
            .checked_add(bytes.len())
            .ok_or(ArchiveError::ManifestVerification(
                "index pack receipt hint page slice overflows",
            ))?;
        if pack.bytes().get(start..end) != Some(bytes.as_slice()) {
            return Err(ArchiveError::ManifestVerification(
                "index pack receipt hint page bytes mismatch",
            ));
        }
        let first = built.entries()[0].manifest_sha256()?;
        let last = built.entries()[built.entries().len() - 1].manifest_sha256()?;
        refs.push(ReceiptHintIndexPageRefV3 {
            relative_path: crate::raw::path_string(&pack_relative)?,
            sha256: hex::encode(hash),
            first_manifest_sha256: hex::encode(first),
            last_manifest_sha256: hex::encode(last),
            offset: locator.offset(),
            length,
        });
    }
    if refs.len() != hint_pages.len() {
        return Err(ArchiveError::ManifestVerification(
            "index pack is missing embedded receipt hint pages",
        ));
    }
    publish_hint_index(archive, chain, source, root_hash, dataset, refs)
}

fn publish_hint_index(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root_hash: [u8; 32],
    dataset: &Path,
    pages: Vec<ReceiptHintIndexPageRefV3>,
) -> Result<[u8; 32], ArchiveError> {
    let index = ReceiptHintIndexV3 {
        schema: RAW_HINT_INDEX_SCHEMA_V3.to_owned(),
        root_sha256: hex::encode(root_hash),
        chain_id: chain.as_str().to_owned(),
        source_id: source.as_str().to_owned(),
        pages,
    };
    let index_bytes = manifest::canonical_json(&index)?;
    let index_hash = manifest::sha256(&index_bytes);
    let index_relative = hint_index_relative(dataset, index_hash);
    fs::publish_immutable(&archive.root, &index_relative, &index_bytes)?;
    let previous = fs::try_read_regular(&archive.root, &hint_current_relative(dataset), 64 * 1024)?;
    let pointer = manifest::CurrentPointerV1 {
        schema: manifest::CURRENT_POINTER_SCHEMA_V1.to_owned(),
        manifest_relative_path: crate::raw::path_string(&index_relative)?,
        manifest_sha256: hex::encode(index_hash),
    };
    fs::publish_current_cas(
        &archive.root,
        &hint_current_relative(dataset),
        previous.as_deref(),
        &manifest::canonical_json(&pointer)?,
    )?;
    Ok(index_hash)
}

pub fn lookup_receipt_hint(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    manifest: &ManifestId,
) -> Result<(u64, u64), ArchiveError> {
    let hash = manifest::hash_from_manifest_id(manifest)?;
    let index = load_hint_index(archive, chain, source)
        .map_err(|_| ArchiveError::ReceiptIndexRebuildRequired)?;
    let encoded = hex::encode(hash);
    let page_ref = index
        .pages
        .iter()
        .find(|page| {
            encoded.as_str() >= page.first_manifest_sha256.as_str()
                && encoded.as_str() <= page.last_manifest_sha256.as_str()
        })
        .ok_or(ArchiveError::ReceiptIndexRebuildRequired)?;
    let page_bytes = load_hint_page_bytes(archive, page_ref)
        .map_err(|_| ArchiveError::ReceiptIndexRebuildRequired)?;
    let page = parse_receipt_hint_page(&page_bytes)
        .map_err(|_| ArchiveError::ReceiptIndexRebuildRequired)?;
    let (first, last) = page
        .candidate_range(hash)
        .ok_or(ArchiveError::ReceiptIndexRebuildRequired)?;
    let (root, journal_bytes) = load_current_root(archive, chain, source)?
        .ok_or(ArchiveError::ReceiptIndexRebuildRequired)?;
    let _lease = lease_root(archive, chain, source, &root)?;
    verify_logical_at_sequence(
        archive,
        &root,
        &journal_bytes,
        hash,
        storage_ports::LocalRecordSequenceRange::try_new(
            LocalRecordSequence::try_new(first)?,
            LocalRecordSequence::try_new(last)?,
        )?,
    )
    .map_err(|_| ArchiveError::ReceiptIndexRebuildRequired)?;
    Ok((first, last))
}

fn collect_hint_entries(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &crate::raw_v3::RootBundleV3,
    journal_bytes: &[u8],
) -> Result<Vec<ReceiptHintEntryV3>, ArchiveError> {
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), journal_bytes)?;
    let mut entries = Vec::new();
    walk_logical_leaves(root.sequence_root(), journal_bytes, &packs, &mut |entry| {
        match entry.storage() {
            crate::raw_v3::SequenceStorageRefV3::Logical {
                manifest_sha256, ..
            } => {
                entries.push(ReceiptHintEntryV3::try_new(
                    manifest::parse_hash(manifest_sha256)?,
                    entry.first_local_sequence(),
                    entry.last_local_sequence(),
                )?);
            }
            crate::raw_v3::SequenceStorageRefV3::Packed { .. } => {
                let pack = load_pack_manifest(archive, chain, source, entry)?;
                for input in pack.inputs() {
                    let commit =
                        parse_logical_commit_manifest(input.canonical_manifest_json().as_bytes())?;
                    entries.push(ReceiptHintEntryV3::try_new(
                        input.manifest_sha256()?,
                        commit.commit().first_local_sequence(),
                        commit.commit().last_local_sequence(),
                    )?);
                }
            }
        }
        Ok(false)
    })?;
    Ok(entries)
}

pub(super) fn live_hint_protection(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<Option<LiveHintProtection>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let current = hint_current_relative(&dataset);
    if !fs::exists_regular(&archive.root, &current)? {
        return Ok(None);
    }
    let index = load_hint_index(archive, chain, source)?;
    let mut protected = BTreeSet::new();
    protected.insert(current);
    protected.insert(PathBuf::from(&index_pointer_path(archive, chain, source)?));
    let mut pack_embedded = !index.pages.is_empty();
    for page in &index.pages {
        let relative = PathBuf::from(&page.relative_path);
        fs::validate_relative(&relative)?;
        if is_standalone_hint_page(&relative) {
            pack_embedded = false;
            protected.insert(relative);
        }
    }
    if index.pages.is_empty() {
        pack_embedded = false;
    }
    Ok(Some(LiveHintProtection {
        protected,
        pack_embedded,
    }))
}

pub(super) struct LiveHintProtection {
    pub protected: BTreeSet<PathBuf>,
    pub pack_embedded: bool,
}

fn index_pointer_path(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<String, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let bytes = fs::read_regular(&archive.root, &hint_current_relative(&dataset), 64 * 1024)?;
    let pointer: manifest::CurrentPointerV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid receipt hint CURRENT JSON"))?;
    Ok(pointer.manifest_relative_path)
}

fn is_standalone_hint_page(relative: &Path) -> bool {
    relative
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("hint-") && name.ends_with(".json"))
}

pub(super) fn verify_live_hints(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<(), ArchiveError> {
    let Some(_) = live_hint_protection(archive, chain, source)? else {
        return Ok(());
    };
    let index = load_hint_index(archive, chain, source)?;
    let (root, journal_bytes) = load_current_root(archive, chain, source)?.ok_or(
        ArchiveError::ManifestVerification("receipt hint exists without a CURRENT root"),
    )?;
    let _lease = lease_root(archive, chain, source, &root)?;
    for page_ref in &index.pages {
        let page_bytes = load_hint_page_bytes(archive, page_ref)?;
        let page = parse_receipt_hint_page(&page_bytes)?;
        for entry in page.entries() {
            verify_logical_at_sequence(
                archive,
                &root,
                &journal_bytes,
                entry.manifest_sha256()?,
                storage_ports::LocalRecordSequenceRange::try_new(
                    LocalRecordSequence::try_new(entry.first_local_sequence())?,
                    LocalRecordSequence::try_new(entry.last_local_sequence())?,
                )?,
            )?;
        }
    }
    Ok(())
}

fn load_hint_index(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<ReceiptHintIndexV3, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let bytes = fs::read_regular(&archive.root, &hint_current_relative(&dataset), 64 * 1024)?;
    let pointer: manifest::CurrentPointerV1 = serde_json::from_slice(&bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid receipt hint CURRENT JSON"))?;
    if pointer.schema != manifest::CURRENT_POINTER_SCHEMA_V1 {
        return Err(ArchiveError::ManifestVerification(
            "unsupported receipt hint CURRENT schema",
        ));
    }
    let hash = manifest::parse_hash(&pointer.manifest_sha256)?;
    let expected = hint_index_relative(&dataset, hash);
    if Path::new(&pointer.manifest_relative_path) != expected {
        return Err(ArchiveError::ManifestVerification(
            "receipt hint CURRENT does not bind the exact index path",
        ));
    }
    let index_bytes = fs::read_regular(&archive.root, &expected, MAX_HINT_INDEX_BYTES)?;
    if manifest::sha256(&index_bytes) != hash {
        return Err(ArchiveError::ManifestVerification(
            "receipt hint index path does not bind exact bytes",
        ));
    }
    let wire: ReceiptHintIndexWireV3 = serde_json::from_slice(&index_bytes)
        .map_err(|_| ArchiveError::ManifestVerification("invalid receipt hint index JSON"))?;
    if wire.schema != RAW_HINT_INDEX_SCHEMA_V3
        || wire.chain_id != chain.as_str()
        || wire.source_id != source.as_str()
    {
        return Err(ArchiveError::ManifestVerification(
            "receipt hint index schema or dataset mismatch",
        ));
    }
    let reconstructed = ReceiptHintIndexV3 {
        schema: wire.schema,
        root_sha256: wire.root_sha256,
        chain_id: wire.chain_id,
        source_id: wire.source_id,
        pages: wire.pages,
    };
    if manifest::canonical_json(&reconstructed)? != index_bytes {
        return Err(ArchiveError::ManifestVerification(
            "receipt hint index canonical bytes are invalid",
        ));
    }
    Ok(reconstructed)
}

fn load_hint_page_bytes(
    archive: &RawV3Archive,
    page_ref: &ReceiptHintIndexPageRefV3,
) -> Result<Vec<u8>, ArchiveError> {
    if page_ref.length == 0 {
        return Err(ArchiveError::ManifestVerification(
            "receipt hint page length is zero",
        ));
    }
    let bytes = fs::read_regular(
        &archive.root,
        Path::new(&page_ref.relative_path),
        RAW_ARCHIVE_MAXIMUM_INDEX_PACK_BYTES,
    )?;
    let start = usize::try_from(page_ref.offset).map_err(|_| {
        ArchiveError::ManifestVerification("receipt hint page offset exceeds address space")
    })?;
    let length = usize::try_from(page_ref.length).map_err(|_| {
        ArchiveError::ManifestVerification("receipt hint page length exceeds address space")
    })?;
    let end = start
        .checked_add(length)
        .ok_or(ArchiveError::ManifestVerification(
            "receipt hint page slice overflows",
        ))?;
    let slice = bytes
        .get(start..end)
        .ok_or(ArchiveError::ManifestVerification(
            "receipt hint page slice is outside the object",
        ))?;
    if hex::encode(manifest::sha256(slice)) != page_ref.sha256 {
        return Err(ArchiveError::ManifestVerification(
            "receipt hint page path does not bind exact bytes",
        ));
    }
    Ok(slice.to_vec())
}

fn hint_current_relative(dataset: &Path) -> PathBuf {
    dataset.join("hints").join("CURRENT")
}

fn hint_index_relative(dataset: &Path, hash: [u8; 32]) -> PathBuf {
    dataset
        .join("hints")
        .join(format!("index-{}.json", hex::encode(hash)))
}

fn hint_page_relative(dataset: &Path, hash: [u8; 32]) -> PathBuf {
    dataset
        .join("hints")
        .join(format!("hint-{}.json", hex::encode(hash)))
}
