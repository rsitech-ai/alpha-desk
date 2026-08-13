use std::path::{Component, Path, PathBuf};

use domain_types::{ChainId, SourceId};
use storage_ports::ArchiveError;

use super::manifest;

pub(super) const LEGACY_DATASET: &str = "raw_source_observations";
pub(super) const BYTE_V2_DATASET: &str = "raw_source_observations_byte_v2";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RawPolicy {
    LegacyContiguous,
    MonotonicByteV2,
    MonotonicByteV3,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct ActivePolicies {
    legacy: bool,
    byte_v2: bool,
    byte_v3: bool,
}

impl ActivePolicies {
    pub(super) const fn active(self, policy: RawPolicy) -> bool {
        match policy {
            RawPolicy::LegacyContiguous => self.legacy,
            RawPolicy::MonotonicByteV2 => self.byte_v2,
            RawPolicy::MonotonicByteV3 => self.byte_v3,
        }
    }

    pub(super) const fn conflicts(self) -> bool {
        self.legacy as u32 + self.byte_v2 as u32 + self.byte_v3 as u32 > 1
    }
}

pub(super) fn active_policies(
    root: &Path,
    chain: &ChainId,
    source: &SourceId,
) -> Result<ActivePolicies, ArchiveError> {
    Ok(ActivePolicies {
        legacy: checked_current_exists(
            root,
            &dataset_relative(chain, source, RawPolicy::LegacyContiguous),
        )?,
        byte_v2: checked_current_exists(
            root,
            &dataset_relative(chain, source, RawPolicy::MonotonicByteV2),
        )?,
        byte_v3: checked_current_exists(
            root,
            &dataset_relative(chain, source, RawPolicy::MonotonicByteV3),
        )?,
    })
}

pub(super) fn ensure_append_policy(
    root: &Path,
    chain: &ChainId,
    source: &SourceId,
    requested: RawPolicy,
) -> Result<(), ArchiveError> {
    let active = active_policies(root, chain, source)?;
    if active.conflicts() {
        return Err(ArchiveError::ManifestVerification(
            "raw source has more than one active cursor policy",
        ));
    }
    let other_active = match requested {
        RawPolicy::LegacyContiguous => active.byte_v2 || active.byte_v3,
        RawPolicy::MonotonicByteV2 => active.legacy || active.byte_v3,
        RawPolicy::MonotonicByteV3 => active.legacy || active.byte_v2,
    };
    if other_active {
        return Err(ArchiveError::InvalidInput(
            "raw source already uses a different archive cursor policy",
        ));
    }
    Ok(())
}

pub(super) fn ensure_read_policy(
    root: &Path,
    chain: &ChainId,
    source: &SourceId,
    requested: RawPolicy,
) -> Result<bool, ArchiveError> {
    let active = active_policies(root, chain, source)?;
    if active.conflicts() {
        return Err(ArchiveError::ManifestVerification(
            "raw source has more than one active cursor policy",
        ));
    }
    Ok(active.active(requested))
}

pub(super) fn writer_lock_relative(chain: &ChainId, source: &SourceId) -> PathBuf {
    dataset_relative(chain, source, RawPolicy::LegacyContiguous).join(".writer.lock")
}

pub(super) fn dataset_relative(chain: &ChainId, source: &SourceId, policy: RawPolicy) -> PathBuf {
    let dataset = match policy {
        RawPolicy::LegacyContiguous => LEGACY_DATASET,
        RawPolicy::MonotonicByteV2 => BYTE_V2_DATASET,
        RawPolicy::MonotonicByteV3 => super::raw_v3::RAW_BYTE_DATASET_V3,
    };
    PathBuf::from(format!(
        "chain={}",
        manifest::encoded_component(chain.as_str())
    ))
    .join(format!("dataset={dataset}"))
    .join(format!(
        "source={}",
        manifest::encoded_component(source.as_str())
    ))
}

fn checked_current_exists(root: &Path, dataset: &Path) -> Result<bool, ArchiveError> {
    let relative = dataset.join("CURRENT");
    super::fs::validate_relative(&relative)?;
    let mut current = root.to_path_buf();
    let component_count = relative.components().count();
    for (index, component) in relative.components().enumerate() {
        let Component::Normal(component) = component else {
            return Err(ArchiveError::UnsafePath);
        };
        current.push(component);
        let metadata = match std::fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
            Err(_) => return Err(ArchiveError::Io("inspecting raw policy current pointer")),
        };
        if metadata.file_type().is_symlink() {
            return Err(ArchiveError::UnsafePath);
        }
        let last = index + 1 == component_count;
        if (last && !metadata.is_file()) || (!last && !metadata.is_dir()) {
            return Err(ArchiveError::UnsafePath);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_lock_stays_at_the_frozen_v1_location() {
        let chain = ChainId::new("mainnet").unwrap();
        let source = SourceId::new("primary-node").unwrap();
        assert_eq!(
            writer_lock_relative(&chain, &source),
            PathBuf::from(
                "chain=mainnet/dataset=raw_source_observations/source=primary-node/.writer.lock"
            )
        );
    }
}
