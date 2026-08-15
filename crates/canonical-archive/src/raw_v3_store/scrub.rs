use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use domain_types::{ChainId, ManifestId, SourceId};
use storage_ports::{ArchiveError, RawArchiveMaintenanceStatistics};

use super::{
    RawV3Archive, dataset_relative, lease_root, load_current_root, load_logical_commit,
    load_packs_for_tree, load_verified_pack_rows, root_relative, verify_loaded_commit,
    walk_logical_leaves,
};
use crate::{
    fs, manifest,
    raw_v3::{
        RootBundleV3, SequenceNodeRefV3, SequenceStorageRefV3, load_sequence_internal,
        parse_logical_commit_manifest, root_bundle_hash,
    },
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawArchiveScrubReport {
    root_sha256: [u8; 32],
    logical_manifest_count: u64,
    packed_range_count: u64,
}

impl RawArchiveScrubReport {
    #[must_use]
    pub const fn root_sha256(&self) -> [u8; 32] {
        self.root_sha256
    }

    #[must_use]
    pub const fn logical_manifest_count(&self) -> u64 {
        self.logical_manifest_count
    }

    #[must_use]
    pub const fn packed_range_count(&self) -> u64 {
        self.packed_range_count
    }
}

pub fn verify_all_sources(archive: &RawV3Archive) -> Result<(), ArchiveError> {
    for (chain, source) in super::discover_v3_sources(archive)? {
        if load_current_root(archive, &chain, &source)?.is_none() {
            return Err(ArchiveError::ManifestVerification(
                "raw V3 source directory is missing CURRENT",
            ));
        }
        let _ = scrub_source(archive, &chain, &source)?;
    }
    Ok(())
}

pub fn scrub_source(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<RawArchiveScrubReport, ArchiveError> {
    let (root, journal_bytes) =
        load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
    let _lease = lease_root(archive, chain, source, &root)?;
    let stats = inspect_tree(archive, chain, source, &root, &journal_bytes, true)?;
    super::hint::verify_live_hints(archive, chain, source)?;
    verify_checkpoint(archive, chain, source)?;
    Ok(RawArchiveScrubReport {
        root_sha256: root_bundle_hash(&root)?,
        logical_manifest_count: stats.logical_manifest_count(),
        packed_range_count: stats.packed_range_count(),
    })
}

pub fn maintenance_statistics(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<RawArchiveMaintenanceStatistics, ArchiveError> {
    let Some((root, journal_bytes)) = load_current_root(archive, chain, source)? else {
        return RawArchiveMaintenanceStatistics::try_new(0, 0, 0, 0, 0, 0, 0, 0, 0);
    };
    let _lease = lease_root(archive, chain, source, &root)?;
    inspect_tree(archive, chain, source, &root, &journal_bytes, false)
}

fn inspect_tree(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &RootBundleV3,
    journal_bytes: &[u8],
    decode: bool,
) -> Result<RawArchiveMaintenanceStatistics, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), journal_bytes)?;
    let mut logical_manifest_count = 0_u64;
    let mut logical_row_count = 0_u64;
    let mut logical_data_bytes = 0_u64;
    let mut packed_range_count = 0_u64;
    let mut pending_pack_manifest_count = 0_u64;
    let mut physical = BTreeSet::new();
    let mut expected_sequence = root.sequence_root().first_local_sequence();
    walk_logical_leaves(root.sequence_root(), journal_bytes, &packs, &mut |entry| {
        if entry.first_local_sequence() != expected_sequence {
            return Err(ArchiveError::ManifestVerification(
                "sequence tree leaves are not exactly contiguous",
            ));
        }
        expected_sequence = entry.last_local_sequence().checked_add(1).ok_or(
            ArchiveError::ManifestVerification("sequence leaf coverage overflows"),
        )?;
        match entry.storage() {
            SequenceStorageRefV3::Logical {
                manifest_relative_path,
                manifest_sha256,
            } => {
                pending_pack_manifest_count = pending_pack_manifest_count
                    .checked_add(1)
                    .ok_or(ArchiveError::InvalidInput("pending pack count overflows"))?;
                let hash = manifest::parse_hash(manifest_sha256)?;
                let loaded = load_logical_commit(archive, Path::new(manifest_relative_path), hash)?;
                if decode {
                    verify_loaded_commit(archive, &loaded, hash)?;
                }
                logical_manifest_count =
                    logical_manifest_count
                        .checked_add(1)
                        .ok_or(ArchiveError::InvalidInput(
                            "logical manifest count overflows",
                        ))?;
                logical_row_count = logical_row_count
                    .checked_add(loaded.object().row_count())
                    .ok_or(ArchiveError::InvalidInput("logical row count overflows"))?;
                logical_data_bytes = logical_data_bytes
                    .checked_add(loaded.object().size_bytes())
                    .ok_or(ArchiveError::InvalidInput("logical data bytes overflow"))?;
                physical.insert(PathBuf::from(loaded.object().relative_path()));
            }
            SequenceStorageRefV3::Packed { .. } => {
                packed_range_count = packed_range_count
                    .checked_add(1)
                    .ok_or(ArchiveError::InvalidInput("packed range count overflows"))?;
                let pack = if decode {
                    load_verified_pack_rows(archive, chain, source, entry)?.0
                } else {
                    super::load_pack_manifest(archive, chain, source, entry)?
                };
                for input in pack.inputs() {
                    let commit =
                        parse_logical_commit_manifest(input.canonical_manifest_json().as_bytes())?;
                    logical_manifest_count =
                        logical_manifest_count
                            .checked_add(1)
                            .ok_or(ArchiveError::InvalidInput(
                                "logical manifest count overflows",
                            ))?;
                    logical_row_count = logical_row_count
                        .checked_add(commit.object().row_count())
                        .ok_or(ArchiveError::InvalidInput("logical row count overflows"))?;
                    logical_data_bytes = logical_data_bytes
                        .checked_add(commit.object().size_bytes())
                        .ok_or(ArchiveError::InvalidInput("logical data bytes overflow"))?;
                }
                physical.insert(dataset.join(pack.object().relative_path()));
            }
        }
        Ok(false)
    })?;
    if expected_sequence
        != root
            .sequence_root()
            .last_local_sequence()
            .checked_add(1)
            .ok_or(ArchiveError::ManifestVerification(
                "sequence tree end overflows",
            ))?
    {
        return Err(ArchiveError::ManifestVerification(
            "sequence tree does not cover the authenticated root span",
        ));
    }
    if logical_manifest_count != root.logical_manifest_count()
        || logical_row_count != root.sequence_root().row_count()
    {
        return Err(ArchiveError::ManifestVerification(
            "tree walk disagrees with the authenticated root counts",
        ));
    }
    let mut physical_data_bytes = 0_u64;
    for relative in &physical {
        let (_, length) =
            fs::regular_digest(&archive.root, relative, archive.config.max_read_bytes())?;
        physical_data_bytes = physical_data_bytes
            .checked_add(length)
            .ok_or(ArchiveError::InvalidInput("physical data bytes overflow"))?;
    }
    let index_paths = collect_index_paths(archive, chain, source, root, journal_bytes)?;
    let mut index_bytes = 0_u64;
    for relative in &index_paths {
        if !fs::exists_regular(&archive.root, relative)? {
            return Err(ArchiveError::ManifestVerification(
                "reachable index artifact is missing",
            ));
        }
        let (_, length) =
            fs::regular_digest(&archive.root, relative, archive.config.max_read_bytes())?;
        index_bytes = index_bytes
            .checked_add(length)
            .ok_or(ArchiveError::InvalidInput("index bytes overflow"))?;
    }
    let physical_data_object_count = u64::try_from(physical.len())
        .map_err(|_| ArchiveError::InvalidInput("physical object count exceeds u64"))?;
    let index_inode_count = u64::try_from(index_paths.len())
        .map_err(|_| ArchiveError::InvalidInput("index inode count exceeds u64"))?;
    RawArchiveMaintenanceStatistics::try_new(
        logical_manifest_count,
        logical_row_count,
        physical_data_object_count,
        packed_range_count,
        logical_data_bytes,
        physical_data_bytes,
        index_bytes,
        index_inode_count,
        pending_pack_manifest_count,
    )
}

fn collect_index_paths(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
    root: &RootBundleV3,
    journal_bytes: &[u8],
) -> Result<BTreeSet<PathBuf>, ArchiveError> {
    let dataset = dataset_relative(chain, source);
    let mut paths = BTreeSet::new();
    paths.insert(dataset.join("CURRENT"));
    paths.insert(root_relative(&dataset, root_bundle_hash(root)?));
    paths.insert(PathBuf::from(root.journal_prefix().relative_path()));
    let packs = load_packs_for_tree(archive, chain, source, root.sequence_root(), journal_bytes)?;
    collect_index_locators(
        &dataset,
        journal_bytes,
        &packs,
        root.sequence_root(),
        &mut paths,
    )?;
    walk_logical_leaves(root.sequence_root(), journal_bytes, &packs, &mut |entry| {
        if let SequenceStorageRefV3::Packed {
            pack_manifest_relative_path,
            ..
        } = entry.storage()
        {
            paths.insert(PathBuf::from(pack_manifest_relative_path));
        }
        Ok(false)
    })?;
    if let Some(live) = super::hint::live_hint_protection(archive, chain, source)? {
        paths.extend(live.protected);
    }
    Ok(paths)
}

fn collect_index_locators(
    dataset: &Path,
    journal_bytes: &[u8],
    packs: &crate::raw_v3::IndexPackBytes,
    node: &SequenceNodeRefV3,
    paths: &mut BTreeSet<PathBuf>,
) -> Result<(), ArchiveError> {
    if let Some(relative) = node.locator().index_pack_relative_path() {
        paths.insert(dataset.join(relative));
        if let Some(hash) = node.locator().index_pack_sha256()? {
            paths.insert(dataset.join(format!("index-packs/{}.manifest.json", hex::encode(hash))));
        }
    }
    if node.depth() == 0 {
        return Ok(());
    }
    let page = load_sequence_internal(journal_bytes, packs, node)?;
    for child in page.children() {
        collect_index_locators(dataset, journal_bytes, packs, child, paths)?;
    }
    Ok(())
}

fn verify_checkpoint(
    archive: &RawV3Archive,
    chain: &ChainId,
    source: &SourceId,
) -> Result<(), ArchiveError> {
    match super::checkpoint::load_checkpoint(archive, chain, source)? {
        None => Ok(()),
        Some(super::RawArchiveCheckpoint::V1(checkpoint)) => {
            for encoded in checkpoint.original_manifest_ids() {
                let manifest_id = ManifestId::new(encoded.clone()).map_err(|_| {
                    ArchiveError::ManifestVerification("checkpoint V1 manifest ID is invalid")
                })?;
                super::verify_raw_manifest(archive, &manifest_id)?;
            }
            Ok(())
        }
        Some(super::RawArchiveCheckpoint::V2(checkpoint)) => {
            let (root, journal_bytes) =
                load_current_root(archive, chain, source)?.ok_or(ArchiveError::RangeUnavailable)?;
            for entry in checkpoint.entries().entries() {
                super::verify_logical_at_sequence(
                    archive,
                    &root,
                    &journal_bytes,
                    manifest::hash_from_manifest_id(entry.manifest_id())?,
                    entry.local_sequence_range(),
                )?;
            }
            Ok(())
        }
    }
}
