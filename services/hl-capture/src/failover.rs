use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};

use domain_types::{BlockHeight, ChainId, SourceId};
use rustix::fs::{Mode, OFlags, open};
use serde::{Deserialize, Serialize};

const FAILOVER_SCHEMA_VERSION: &str = "hl.capture.failover.v1";
const HASH_CONTEXT: &str = "hyperliquid-alpha-desk/committed-source-failover/v1";
const MAX_FAILOVER_BYTES: u64 = 16 * 1024;
const MAX_PATH_BYTES: usize = 4_096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FailoverReason {
    PrimaryRangeUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailoverDecision {
    chain_id: ChainId,
    primary_source_id: SourceId,
    independent_source_id: SourceId,
    failover_height: BlockHeight,
    reason: FailoverReason,
}

impl FailoverDecision {
    pub fn try_new(
        chain_id: ChainId,
        primary_source_id: SourceId,
        independent_source_id: SourceId,
        failover_height: BlockHeight,
        reason: FailoverReason,
    ) -> Result<Self, FailoverError> {
        if primary_source_id == independent_source_id {
            return Err(FailoverError::InvalidDecision);
        }
        Ok(Self {
            chain_id,
            primary_source_id,
            independent_source_id,
            failover_height,
            reason,
        })
    }

    #[must_use]
    pub const fn chain_id(&self) -> &ChainId {
        &self.chain_id
    }

    #[must_use]
    pub const fn primary_source_id(&self) -> &SourceId {
        &self.primary_source_id
    }

    #[must_use]
    pub const fn independent_source_id(&self) -> &SourceId {
        &self.independent_source_id
    }

    #[must_use]
    pub const fn failover_height(&self) -> BlockHeight {
        self.failover_height
    }

    #[must_use]
    pub const fn reason(&self) -> FailoverReason {
        self.reason
    }

    pub fn validate_topology(
        &self,
        chain_id: &ChainId,
        primary_source_id: &SourceId,
        independent_source_id: &SourceId,
    ) -> Result<(), FailoverError> {
        if self.chain_id == *chain_id
            && self.primary_source_id == *primary_source_id
            && self.independent_source_id == *independent_source_id
        {
            Ok(())
        } else {
            Err(FailoverError::TopologyMismatch)
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailoverRecordDisposition {
    Recorded,
    Identical,
}

#[derive(Debug)]
pub struct FailoverStore {
    path: PathBuf,
}

impl FailoverStore {
    pub fn new(path: PathBuf) -> Result<Self, FailoverError> {
        validate_path(&path)?;
        let parent = path.parent().ok_or(FailoverError::UnsafePath)?;
        validate_existing_parent_chain(parent)?;
        fs::create_dir_all(parent).map_err(|_| FailoverError::Io)?;
        validate_parent(parent)?;
        Ok(Self { path })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn load(&self) -> Result<Option<FailoverDecision>, FailoverError> {
        let metadata = match fs::symlink_metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(FailoverError::Io),
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() > MAX_FAILOVER_BYTES
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(FailoverError::UnsafePath);
        }
        let descriptor = open(
            &self.path,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
            Mode::empty(),
        )
        .map_err(|_| FailoverError::UnsafePath)?;
        let file = File::from(descriptor);
        let current = file.metadata().map_err(|_| FailoverError::Io)?;
        if !current.is_file()
            || current.len() > MAX_FAILOVER_BYTES
            || current.permissions().mode() & 0o077 != 0
        {
            return Err(FailoverError::UnsafePath);
        }
        let mut bytes = Vec::with_capacity(
            usize::try_from(current.len()).map_err(|_| FailoverError::TooLarge)?,
        );
        file.take(MAX_FAILOVER_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| FailoverError::Io)?;
        if u64::try_from(bytes.len()).map_err(|_| FailoverError::TooLarge)? > MAX_FAILOVER_BYTES {
            return Err(FailoverError::TooLarge);
        }
        decode_stored(&bytes).map(Some)
    }

    pub fn record(
        &self,
        decision: &FailoverDecision,
    ) -> Result<FailoverRecordDisposition, FailoverError> {
        if let Some(existing) = self.load()? {
            return if existing == *decision {
                Ok(FailoverRecordDisposition::Identical)
            } else {
                Err(FailoverError::ConflictingDecision)
            };
        }
        let parent = self.path.parent().ok_or(FailoverError::UnsafePath)?;
        validate_parent(parent)?;
        let bytes = encode_stored(decision)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(parent).map_err(|_| FailoverError::Io)?;
        temporary
            .as_file()
            .set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| FailoverError::Io)?;
        temporary.write_all(&bytes).map_err(|_| FailoverError::Io)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| FailoverError::Io)?;
        match temporary.persist_noclobber(&self.path) {
            Ok(_) => {
                File::open(parent)
                    .and_then(|directory| directory.sync_all())
                    .map_err(|_| FailoverError::Io)?;
                Ok(FailoverRecordDisposition::Recorded)
            }
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                match self.load()? {
                    Some(existing) if existing == *decision => {
                        Ok(FailoverRecordDisposition::Identical)
                    }
                    Some(_) => Err(FailoverError::ConflictingDecision),
                    None => Err(FailoverError::Io),
                }
            }
            Err(_) => Err(FailoverError::Io),
        }
    }
}

#[derive(Debug, Serialize)]
struct DecisionMaterial<'a> {
    schema_version: &'static str,
    chain_id: &'a str,
    primary_source_id: &'a str,
    independent_source_id: &'a str,
    failover_height: u64,
    reason: FailoverReason,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredDecision {
    schema_version: String,
    chain_id: String,
    primary_source_id: String,
    independent_source_id: String,
    failover_height: u64,
    reason: FailoverReason,
    decision_hash_blake3: String,
}

fn encode_stored(decision: &FailoverDecision) -> Result<Vec<u8>, FailoverError> {
    let material = material(decision);
    let material_bytes = serde_json::to_vec(&material).map_err(|_| FailoverError::Serialization)?;
    let decision_hash_blake3 = hash_material(&material_bytes);
    serde_json::to_vec(&StoredDecision {
        schema_version: FAILOVER_SCHEMA_VERSION.to_owned(),
        chain_id: decision.chain_id.as_str().to_owned(),
        primary_source_id: decision.primary_source_id.as_str().to_owned(),
        independent_source_id: decision.independent_source_id.as_str().to_owned(),
        failover_height: decision.failover_height.get(),
        reason: decision.reason,
        decision_hash_blake3,
    })
    .map_err(|_| FailoverError::Serialization)
}

fn decode_stored(bytes: &[u8]) -> Result<FailoverDecision, FailoverError> {
    let stored: StoredDecision =
        serde_json::from_slice(bytes).map_err(|_| FailoverError::Serialization)?;
    if stored.schema_version != FAILOVER_SCHEMA_VERSION
        || !is_lowercase_hash(&stored.decision_hash_blake3)
    {
        return Err(FailoverError::Integrity);
    }
    let decision = FailoverDecision::try_new(
        ChainId::new(stored.chain_id).map_err(|_| FailoverError::InvalidDecision)?,
        SourceId::new(stored.primary_source_id).map_err(|_| FailoverError::InvalidDecision)?,
        SourceId::new(stored.independent_source_id).map_err(|_| FailoverError::InvalidDecision)?,
        BlockHeight::new(stored.failover_height),
        stored.reason,
    )?;
    let material_bytes =
        serde_json::to_vec(&material(&decision)).map_err(|_| FailoverError::Serialization)?;
    if hash_material(&material_bytes) != stored.decision_hash_blake3 {
        return Err(FailoverError::Integrity);
    }
    Ok(decision)
}

fn material(decision: &FailoverDecision) -> DecisionMaterial<'_> {
    DecisionMaterial {
        schema_version: FAILOVER_SCHEMA_VERSION,
        chain_id: decision.chain_id.as_str(),
        primary_source_id: decision.primary_source_id.as_str(),
        independent_source_id: decision.independent_source_id.as_str(),
        failover_height: decision.failover_height.get(),
        reason: decision.reason,
    }
}

fn hash_material(material: &[u8]) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(HASH_CONTEXT.as_bytes());
    hasher.update(material);
    hasher.finalize().to_hex().to_string()
}

fn is_lowercase_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .as_bytes()
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
}

fn validate_path(path: &Path) -> Result<(), FailoverError> {
    if path.as_os_str().is_empty()
        || path == Path::new("/")
        || path.as_os_str().len() > MAX_PATH_BYTES
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
        || path.file_name().is_none()
    {
        Err(FailoverError::UnsafePath)
    } else {
        Ok(())
    }
}

fn validate_parent(parent: &Path) -> Result<(), FailoverError> {
    for component in absolute_path(parent)?
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        let metadata = fs::symlink_metadata(component).map_err(|_| FailoverError::Io)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(FailoverError::UnsafePath);
        }
    }
    Ok(())
}

fn validate_existing_parent_chain(parent: &Path) -> Result<(), FailoverError> {
    for component in absolute_path(parent)?
        .ancestors()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        match fs::symlink_metadata(component) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(FailoverError::UnsafePath);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(_) => return Err(FailoverError::Io),
        }
    }
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf, FailoverError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|_| FailoverError::Io)
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum FailoverError {
    #[error("committed-source failover decision is invalid")]
    InvalidDecision,
    #[error("committed-source failover path is unsafe")]
    UnsafePath,
    #[error("committed-source failover decision exceeds its size limit")]
    TooLarge,
    #[error("committed-source failover decision serialization failed")]
    Serialization,
    #[error("committed-source failover decision integrity check failed")]
    Integrity,
    #[error("committed-source failover decision conflicts with durable state")]
    ConflictingDecision,
    #[error("committed-source failover decision does not match configured topology")]
    TopologyMismatch,
    #[error("committed-source failover state I/O failed")]
    Io,
}

impl FailoverError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidDecision => "capture_failover.invalid_decision",
            Self::UnsafePath => "capture_failover.unsafe_path",
            Self::TooLarge => "capture_failover.too_large",
            Self::Serialization => "capture_failover.serialization",
            Self::Integrity => "capture_failover.integrity",
            Self::ConflictingDecision => "capture_failover.conflicting_decision",
            Self::TopologyMismatch => "capture_failover.topology_mismatch",
            Self::Io => "capture_failover.io",
        }
    }
}
