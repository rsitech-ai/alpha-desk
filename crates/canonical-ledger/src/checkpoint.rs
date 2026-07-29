use domain_types::{ChainId, CheckpointId, ManifestId};
use serde::{Deserialize, Serialize};

use crate::{
    CheckpointError, StateCheckpoint, StateImage, StateImageLimits, error::valid_reducer_version,
};

const CHECKPOINT_MANIFEST_SCHEMA: &str = "hyperliquid-alpha-desk/state-checkpoint-manifest/v1";
const CHECKPOINT_ID_CONTEXT: &str = "hyperliquid-alpha-desk/state-checkpoint-id/v1";
const STATE_FILE_HASH_CONTEXT: &str = "hyperliquid-alpha-desk/state-checkpoint-file/v1";
const MAX_MANIFEST_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointCompatibility {
    chain_id: ChainId,
    reducer_set_version: String,
    archive_manifest_id: ManifestId,
    archive_manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
}

impl CheckpointCompatibility {
    pub fn try_new(
        chain_id: ChainId,
        reducer_set_version: impl Into<String>,
        archive_manifest_id: ManifestId,
        archive_manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
    ) -> Result<Self, CheckpointError> {
        let reducer_set_version = reducer_set_version.into();
        if !valid_reducer_version(&reducer_set_version)
            || zero_hash(archive_manifest_sha256)
            || zero_hash(schema_fingerprint)
        {
            return Err(CheckpointError::InvalidInput);
        }
        Ok(Self {
            chain_id,
            reducer_set_version,
            archive_manifest_id,
            archive_manifest_sha256,
            schema_fingerprint,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckpointArtifact {
    checkpoint_id: CheckpointId,
    checkpoint: StateCheckpoint,
    state_image: StateImage,
    state_image_bytes: Vec<u8>,
    state_image_blake3: [u8; 32],
    archive_manifest_id: ManifestId,
    archive_manifest_sha256: [u8; 32],
    schema_fingerprint: [u8; 32],
}

impl CheckpointArtifact {
    pub fn try_new(
        checkpoint: StateCheckpoint,
        state_image: StateImage,
        archive_manifest_id: ManifestId,
        archive_manifest_sha256: [u8; 32],
        schema_fingerprint: [u8; 32],
    ) -> Result<Self, CheckpointError> {
        if zero_hash(archive_manifest_sha256) || zero_hash(schema_fingerprint) {
            return Err(CheckpointError::InvalidInput);
        }
        let watermark = state_image
            .watermark()
            .ok_or(CheckpointError::InvalidInput)?;
        if state_image.chain_id() != checkpoint.chain_id()
            || watermark.block_height != checkpoint.block_height()
            || watermark.canonical_block_hash != checkpoint.canonical_block_hash()
            || state_image.reducer_set_version() != checkpoint.reducer_set_version()
            || state_image.state_hash() != checkpoint.state_hash()
        {
            return Err(CheckpointError::Integrity);
        }
        let state_image_bytes = state_image.canonical_bytes();
        let state_image_blake3 = hash_state_file(&state_image_bytes);
        let material = ManifestMaterial {
            schema_version: CHECKPOINT_MANIFEST_SCHEMA,
            chain_id: checkpoint.chain_id().as_str(),
            block_height: checkpoint.block_height().get(),
            canonical_block_hash: hex::encode(checkpoint.canonical_block_hash()),
            state_hash: hex::encode(checkpoint.state_hash()),
            reducer_set_version: checkpoint.reducer_set_version(),
            archive_manifest_id: archive_manifest_id.as_str(),
            archive_manifest_sha256: hex::encode(archive_manifest_sha256),
            schema_fingerprint: hex::encode(schema_fingerprint),
            state_image_blake3: hex::encode(state_image_blake3),
            state_image_bytes: u64::try_from(state_image_bytes.len())
                .map_err(|_| CheckpointError::InvalidInput)?,
        };
        let checkpoint_id = checkpoint_id(&material)?;
        Ok(Self {
            checkpoint_id,
            checkpoint,
            state_image,
            state_image_bytes,
            state_image_blake3,
            archive_manifest_id,
            archive_manifest_sha256,
            schema_fingerprint,
        })
    }

    pub fn decode(
        manifest_bytes: &[u8],
        state_image_bytes: &[u8],
        limits: StateImageLimits,
    ) -> Result<Self, CheckpointError> {
        if manifest_bytes.is_empty() || manifest_bytes.len() > MAX_MANIFEST_BYTES {
            return Err(CheckpointError::InvalidManifest);
        }
        let stored: StoredManifest =
            serde_json::from_slice(manifest_bytes).map_err(|_| CheckpointError::InvalidManifest)?;
        if stored.schema_version != CHECKPOINT_MANIFEST_SCHEMA
            || stored.state_image_bytes
                != u64::try_from(state_image_bytes.len())
                    .map_err(|_| CheckpointError::InvalidManifest)?
        {
            return Err(CheckpointError::InvalidManifest);
        }
        let state_image = StateImage::decode_canonical(state_image_bytes, limits)?;
        let checkpoint = StateCheckpoint::from_parts(
            ChainId::new(stored.chain_id).map_err(|_| CheckpointError::InvalidManifest)?,
            domain_types::BlockHeight::new(stored.block_height),
            decode_hash(&stored.canonical_block_hash)?,
            decode_hash(&stored.state_hash)?,
            stored.reducer_set_version,
        );
        let artifact = Self::try_new(
            checkpoint,
            state_image,
            ManifestId::new(stored.archive_manifest_id)
                .map_err(|_| CheckpointError::InvalidManifest)?,
            decode_hash(&stored.archive_manifest_sha256)?,
            decode_hash(&stored.schema_fingerprint)?,
        )?;
        let stored_id = CheckpointId::new(stored.checkpoint_id)
            .map_err(|_| CheckpointError::InvalidManifest)?;
        if stored_id != artifact.checkpoint_id
            || decode_hash(&stored.state_image_blake3)? != artifact.state_image_blake3
        {
            return Err(CheckpointError::Integrity);
        }
        if artifact.encode_manifest()? != manifest_bytes {
            return Err(CheckpointError::NonCanonicalManifest);
        }
        Ok(artifact)
    }

    pub fn encode_manifest(&self) -> Result<Vec<u8>, CheckpointError> {
        let material = self.material();
        serde_json::to_vec(&StoredManifest {
            schema_version: CHECKPOINT_MANIFEST_SCHEMA.to_owned(),
            checkpoint_id: self.checkpoint_id.as_str().to_owned(),
            chain_id: material.chain_id.to_owned(),
            block_height: material.block_height,
            canonical_block_hash: material.canonical_block_hash,
            state_hash: material.state_hash,
            reducer_set_version: material.reducer_set_version.to_owned(),
            archive_manifest_id: material.archive_manifest_id.to_owned(),
            archive_manifest_sha256: material.archive_manifest_sha256,
            schema_fingerprint: material.schema_fingerprint,
            state_image_blake3: material.state_image_blake3,
            state_image_bytes: material.state_image_bytes,
        })
        .map_err(|_| CheckpointError::InvalidManifest)
    }

    pub fn validate_compatibility(
        &self,
        expected: &CheckpointCompatibility,
    ) -> Result<(), CheckpointError> {
        if self.checkpoint.chain_id() == &expected.chain_id
            && self.checkpoint.reducer_set_version() == expected.reducer_set_version
            && self.archive_manifest_id == expected.archive_manifest_id
            && self.archive_manifest_sha256 == expected.archive_manifest_sha256
            && self.schema_fingerprint == expected.schema_fingerprint
        {
            Ok(())
        } else {
            Err(CheckpointError::Incompatible)
        }
    }

    #[must_use]
    pub const fn checkpoint_id(&self) -> &CheckpointId {
        &self.checkpoint_id
    }

    #[must_use]
    pub const fn checkpoint(&self) -> &StateCheckpoint {
        &self.checkpoint
    }

    #[must_use]
    pub const fn state_image(&self) -> &StateImage {
        &self.state_image
    }

    #[must_use]
    pub fn state_image_bytes(&self) -> &[u8] {
        &self.state_image_bytes
    }

    #[must_use]
    pub const fn archive_manifest_id(&self) -> &ManifestId {
        &self.archive_manifest_id
    }

    #[must_use]
    pub const fn archive_manifest_sha256(&self) -> [u8; 32] {
        self.archive_manifest_sha256
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> [u8; 32] {
        self.schema_fingerprint
    }

    fn material(&self) -> ManifestMaterial<'_> {
        ManifestMaterial {
            schema_version: CHECKPOINT_MANIFEST_SCHEMA,
            chain_id: self.checkpoint.chain_id().as_str(),
            block_height: self.checkpoint.block_height().get(),
            canonical_block_hash: hex::encode(self.checkpoint.canonical_block_hash()),
            state_hash: hex::encode(self.checkpoint.state_hash()),
            reducer_set_version: self.checkpoint.reducer_set_version(),
            archive_manifest_id: self.archive_manifest_id.as_str(),
            archive_manifest_sha256: hex::encode(self.archive_manifest_sha256),
            schema_fingerprint: hex::encode(self.schema_fingerprint),
            state_image_blake3: hex::encode(self.state_image_blake3),
            state_image_bytes: u64::try_from(self.state_image_bytes.len())
                .expect("validated state image length fits u64"),
        }
    }
}

#[derive(Debug, Serialize)]
struct ManifestMaterial<'a> {
    schema_version: &'static str,
    chain_id: &'a str,
    block_height: u64,
    canonical_block_hash: String,
    state_hash: String,
    reducer_set_version: &'a str,
    archive_manifest_id: &'a str,
    archive_manifest_sha256: String,
    schema_fingerprint: String,
    state_image_blake3: String,
    state_image_bytes: u64,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredManifest {
    schema_version: String,
    checkpoint_id: String,
    chain_id: String,
    block_height: u64,
    canonical_block_hash: String,
    state_hash: String,
    reducer_set_version: String,
    archive_manifest_id: String,
    archive_manifest_sha256: String,
    schema_fingerprint: String,
    state_image_blake3: String,
    state_image_bytes: u64,
}

fn checkpoint_id(material: &ManifestMaterial<'_>) -> Result<CheckpointId, CheckpointError> {
    let bytes = serde_json::to_vec(material).map_err(|_| CheckpointError::InvalidManifest)?;
    let mut hasher = blake3::Hasher::new_derive_key(CHECKPOINT_ID_CONTEXT);
    hasher.update(&bytes);
    CheckpointId::new(format!(
        "state-checkpoint-v1-{}",
        hasher.finalize().to_hex()
    ))
    .map_err(|_| CheckpointError::InvalidInput)
}

fn hash_state_file(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new_derive_key(STATE_FILE_HASH_CONTEXT);
    hasher.update(bytes);
    *hasher.finalize().as_bytes()
}

fn decode_hash(value: &str) -> Result<[u8; 32], CheckpointError> {
    if value.len() != 64
        || value
            .bytes()
            .any(|byte| !byte.is_ascii_digit() && !(b'a'..=b'f').contains(&byte))
    {
        return Err(CheckpointError::InvalidManifest);
    }
    let mut hash = [0_u8; 32];
    hex::decode_to_slice(value, &mut hash).map_err(|_| CheckpointError::InvalidManifest)?;
    if zero_hash(hash) {
        return Err(CheckpointError::InvalidManifest);
    }
    Ok(hash)
}

fn zero_hash(hash: [u8; 32]) -> bool {
    hash == [0_u8; 32]
}
