use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::ModelError;
use crate::schema::FeatureSchema;
use crate::signature::{BundleSignatureVerifier, Ed25519Verifier, verify_against_approved_keys};

pub const REQUIRED_FILES: [&str; 8] = [
    "manifest.toml",
    "feature-schema.json",
    "preprocessing.json",
    "calibration.json",
    "evaluation.json",
    "training-data-manifest.json",
    "model-card.md",
    "signature.ed25519",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtifactKind {
    DeterministicLinearV1,
    Onnx,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
    model_id: String,
    semantic_version: String,
    feature_set_version: String,
    artifact_kind: ArtifactKind,
    review_expires_unix_micros: i64,
    approved_use: Vec<String>,
    prohibited_use: Vec<String>,
}

impl BundleManifest {
    pub fn parse(toml_text: &str) -> Result<Self, ModelError> {
        let manifest: Self = toml::from_str(toml_text).map_err(|_| ModelError::InvalidManifest)?;
        if manifest.model_id.trim().is_empty()
            || manifest.semantic_version.trim().is_empty()
            || manifest.feature_set_version.trim().is_empty()
            || manifest.approved_use.is_empty()
        {
            return Err(ModelError::InvalidManifest);
        }
        if manifest
            .prohibited_use
            .iter()
            .any(|item| item == "live-trading")
            && manifest
                .approved_use
                .iter()
                .any(|item| item == "live-trading")
        {
            return Err(ModelError::InvalidManifest);
        }
        Ok(manifest)
    }

    #[must_use]
    pub fn model_id(&self) -> &str {
        &self.model_id
    }

    #[must_use]
    pub const fn artifact_kind(&self) -> ArtifactKind {
        self.artifact_kind
    }

    #[must_use]
    pub fn feature_set_version(&self) -> &str {
        &self.feature_set_version
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedBundle {
    files: BTreeMap<String, Vec<u8>>,
    manifest: BundleManifest,
    schema: FeatureSchema,
    bundle_hash: [u8; 32],
    signer: [u8; 32],
}

impl SignedBundle {
    pub fn load_dir(
        path: &Path,
        approved_keys: &[[u8; 32]],
        verifier: &impl BundleSignatureVerifier,
    ) -> Result<Self, ModelError> {
        let mut files = BTreeMap::new();
        for name in REQUIRED_FILES {
            let bytes = fs::read(path.join(name)).map_err(|_| ModelError::MissingFile { name })?;
            files.insert(name.to_owned(), bytes);
        }
        let artifact_name = if path.join("model.linear-v1.json").exists() {
            "model.linear-v1.json"
        } else if path.join("model.onnx").exists() {
            "model.onnx"
        } else {
            return Err(ModelError::MissingFile {
                name: "model.linear-v1.json",
            });
        };
        files.insert(
            artifact_name.to_owned(),
            fs::read(path.join(artifact_name)).map_err(|_| ModelError::MissingFile {
                name: "model.linear-v1.json",
            })?,
        );
        Self::from_files(files, approved_keys, verifier)
    }

    pub fn from_files(
        files: BTreeMap<String, Vec<u8>>,
        approved_keys: &[[u8; 32]],
        verifier: &impl BundleSignatureVerifier,
    ) -> Result<Self, ModelError> {
        for name in REQUIRED_FILES {
            if !files.contains_key(name) {
                return Err(ModelError::MissingFile { name });
            }
        }
        let signature = files.get("signature.ed25519").ok_or(ModelError::Unsigned)?;
        if signature.is_empty() {
            return Err(ModelError::Unsigned);
        }
        let message = canonical_message(&files);
        let signer = verify_against_approved_keys(verifier, &message, signature, approved_keys)?;
        let manifest = BundleManifest::parse(
            std::str::from_utf8(
                files
                    .get("manifest.toml")
                    .ok_or(ModelError::InvalidManifest)?,
            )
            .map_err(|_| ModelError::InvalidManifest)?,
        )?;
        match manifest.artifact_kind {
            ArtifactKind::DeterministicLinearV1 => {
                if !files.contains_key("model.linear-v1.json") {
                    return Err(ModelError::MissingFile {
                        name: "model.linear-v1.json",
                    });
                }
            }
            ArtifactKind::Onnx => {
                if !files.contains_key("model.onnx") {
                    return Err(ModelError::MissingFile { name: "model.onnx" });
                }
            }
        }
        let schema: FeatureSchema = serde_json::from_slice(
            files
                .get("feature-schema.json")
                .ok_or(ModelError::InvalidManifest)?,
        )
        .map_err(|_| ModelError::InvalidManifest)?;
        FeatureSchema::new(schema.ordered_features().to_vec())?;
        Ok(Self {
            files,
            manifest,
            schema,
            bundle_hash: message,
            signer,
        })
    }

    pub fn verify_default(
        files: BTreeMap<String, Vec<u8>>,
        approved_keys: &[[u8; 32]],
    ) -> Result<Self, ModelError> {
        Self::from_files(files, approved_keys, &Ed25519Verifier)
    }

    #[must_use]
    pub const fn bundle_hash(&self) -> [u8; 32] {
        self.bundle_hash
    }

    #[must_use]
    pub const fn signer(&self) -> [u8; 32] {
        self.signer
    }

    #[must_use]
    pub const fn manifest(&self) -> &BundleManifest {
        &self.manifest
    }

    #[must_use]
    pub const fn schema(&self) -> &FeatureSchema {
        &self.schema
    }

    pub fn artifact_bytes(&self) -> Result<&[u8], ModelError> {
        match self.manifest.artifact_kind {
            ArtifactKind::DeterministicLinearV1 => self
                .files
                .get("model.linear-v1.json")
                .map(Vec::as_slice)
                .ok_or(ModelError::MissingFile {
                    name: "model.linear-v1.json",
                }),
            ArtifactKind::Onnx => self
                .files
                .get("model.onnx")
                .map(Vec::as_slice)
                .ok_or(ModelError::OnnxProductionUnavailable),
        }
    }
}

pub fn canonical_message(files: &BTreeMap<String, Vec<u8>>) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hl.model-bundle.v1");
    for (name, bytes) in files {
        if name == "signature.ed25519" {
            continue;
        }
        hasher.update(name.as_bytes());
        hasher.update(&u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
        hasher.update(bytes);
    }
    *hasher.finalize().as_bytes()
}

pub fn sign_files(
    files: &BTreeMap<String, Vec<u8>>,
    signing_key: &ed25519_dalek::SigningKey,
) -> [u8; 64] {
    use ed25519_dalek::Signer;
    let message = canonical_message(files);
    signing_key.sign(&message).to_bytes()
}
