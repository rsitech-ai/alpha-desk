use std::path::{Path, PathBuf};

use domain_types::{ChainId, SourceId};
use storage_ports::ArchiveError;

use super::{LocalParquetArchive, manifest, raw, reader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveDataset {
    CanonicalEvents,
    RawSourceObservations,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectedObject {
    dataset: ArchiveDataset,
    relative_path: PathBuf,
    sha256: [u8; 32],
    size_bytes: u64,
    row_count: u64,
}

impl InspectedObject {
    pub(crate) fn new(
        dataset: ArchiveDataset,
        relative_path: PathBuf,
        sha256: [u8; 32],
        size_bytes: u64,
        row_count: u64,
    ) -> Self {
        Self {
            dataset,
            relative_path,
            sha256,
            size_bytes,
            row_count,
        }
    }

    #[must_use]
    pub const fn dataset(&self) -> ArchiveDataset {
        self.dataset
    }

    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    #[must_use]
    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    #[must_use]
    pub const fn size_bytes(&self) -> u64 {
        self.size_bytes
    }

    #[must_use]
    pub const fn row_count(&self) -> u64 {
        self.row_count
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ArchiveInspection {
    canonical_chains: u64,
    raw_sources: u64,
    canonical_blocks: u64,
    canonical_events: u64,
    raw_observations: u64,
    objects: Vec<InspectedObject>,
}

impl ArchiveInspection {
    #[must_use]
    pub const fn canonical_chains(&self) -> u64 {
        self.canonical_chains
    }

    #[must_use]
    pub const fn raw_sources(&self) -> u64 {
        self.raw_sources
    }

    #[must_use]
    pub const fn canonical_blocks(&self) -> u64 {
        self.canonical_blocks
    }

    #[must_use]
    pub const fn canonical_events(&self) -> u64 {
        self.canonical_events
    }

    #[must_use]
    pub const fn raw_observations(&self) -> u64 {
        self.raw_observations
    }

    #[must_use]
    pub fn objects(&self) -> &[InspectedObject] {
        &self.objects
    }

    fn merge(&mut self, other: Self) -> Result<(), ArchiveError> {
        self.canonical_chains = checked_add(self.canonical_chains, other.canonical_chains)?;
        self.raw_sources = checked_add(self.raw_sources, other.raw_sources)?;
        self.canonical_blocks = checked_add(self.canonical_blocks, other.canonical_blocks)?;
        self.canonical_events = checked_add(self.canonical_events, other.canonical_events)?;
        self.raw_observations = checked_add(self.raw_observations, other.raw_observations)?;
        self.objects.extend(other.objects);
        Ok(())
    }

    pub(crate) fn canonical(blocks: u64, events: u64, objects: Vec<InspectedObject>) -> Self {
        Self {
            canonical_chains: 1,
            canonical_blocks: blocks,
            canonical_events: events,
            objects,
            ..Self::default()
        }
    }

    pub(crate) fn raw(observations: u64, objects: Vec<InspectedObject>) -> Self {
        Self {
            raw_sources: 1,
            raw_observations: observations,
            objects,
            ..Self::default()
        }
    }
}

pub fn inspect(archive: &LocalParquetArchive) -> Result<ArchiveInspection, ArchiveError> {
    let mut inspection = ArchiveInspection::default();
    for chain in discover_chains(archive.root())? {
        if let Some(canonical) = reader::inspect_chain(archive, &chain)? {
            inspection.merge(canonical)?;
        }
        for source in discover_raw_sources(archive.root(), &chain)? {
            if let Some(raw) = raw::inspect_source(archive, &chain, &source)? {
                inspection.merge(raw)?;
            }
        }
    }
    inspection
        .objects
        .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(inspection)
}

fn discover_chains(root: &Path) -> Result<Vec<ChainId>, ArchiveError> {
    let mut chains = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|_| ArchiveError::Io("listing archive root"))? {
        let entry = entry.map_err(|_| ArchiveError::Io("reading archive root entry"))?;
        let metadata = entry
            .file_type()
            .map_err(|_| ArchiveError::Io("inspecting archive root entry"))?;
        if metadata.is_symlink() {
            return Err(ArchiveError::UnsafePath);
        }
        if !metadata.is_dir() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ArchiveError::UnsafePath)?;
        let Some(encoded) = name.strip_prefix("chain=") else {
            continue;
        };
        let chain = ChainId::new(manifest::decoded_component(encoded)?)
            .map_err(|_| ArchiveError::ManifestVerification("invalid discovered chain ID"))?;
        chains.push(chain);
    }
    chains.sort();
    Ok(chains)
}

fn discover_raw_sources(root: &Path, chain: &ChainId) -> Result<Vec<SourceId>, ArchiveError> {
    let dataset = root
        .join(format!(
            "chain={}",
            manifest::encoded_component(chain.as_str())
        ))
        .join("dataset=raw_source_observations");
    if !dataset.exists() {
        return Ok(Vec::new());
    }
    let metadata = std::fs::symlink_metadata(&dataset)
        .map_err(|_| ArchiveError::Io("inspecting raw archive dataset"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ArchiveError::UnsafePath);
    }
    let mut sources = Vec::new();
    for entry in
        std::fs::read_dir(dataset).map_err(|_| ArchiveError::Io("listing raw archive sources"))?
    {
        let entry = entry.map_err(|_| ArchiveError::Io("reading raw source entry"))?;
        let file_type = entry
            .file_type()
            .map_err(|_| ArchiveError::Io("inspecting raw source entry"))?;
        if file_type.is_symlink() {
            return Err(ArchiveError::UnsafePath);
        }
        if !file_type.is_dir() {
            continue;
        }
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| ArchiveError::UnsafePath)?;
        let Some(encoded) = name.strip_prefix("source=") else {
            continue;
        };
        let source = SourceId::new(manifest::decoded_component(encoded)?)
            .map_err(|_| ArchiveError::ManifestVerification("invalid discovered source ID"))?;
        sources.push(source);
    }
    sources.sort();
    Ok(sources)
}

fn checked_add(left: u64, right: u64) -> Result<u64, ArchiveError> {
    left.checked_add(right).ok_or(ArchiveError::InvalidInput(
        "archive inspection count overflows",
    ))
}
