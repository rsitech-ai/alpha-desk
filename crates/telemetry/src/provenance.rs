use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildProvenance {
    pub git_sha: String,
    pub dirty: bool,
    pub rustc_version: String,
    pub target_triple: String,
    pub build_epoch: Option<u64>,
    pub reproducible: bool,
    pub schema_fingerprint: String,
    pub cargo_lock_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ProvenanceError {
    #[error("compile-time Git SHA must be exactly 40 hexadecimal characters")]
    InvalidGitSha,
    #[error("{0} must be a non-empty single-line value")]
    InvalidText(&'static str),
    #[error("{0} must be exactly 64 hexadecimal characters")]
    InvalidSha256(&'static str),
    #[error("compile-time build epoch is invalid")]
    InvalidBuildEpoch,
    #[error("compile-time reproducibility flag is invalid")]
    InvalidReproducibleFlag,
    #[error("compile-time dirty flag is invalid")]
    InvalidDirtyFlag,
    #[error("compile-time reproducibility fields disagree")]
    ReproducibilityMismatch,
    #[error("build provenance serialization failed")]
    Serialization,
}

impl BuildProvenance {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        git_sha: impl Into<String>,
        dirty: bool,
        rustc_version: impl Into<String>,
        target_triple: impl Into<String>,
        build_epoch: Option<u64>,
        schema_fingerprint: impl Into<String>,
        cargo_lock_sha256: impl Into<String>,
    ) -> Result<Self, ProvenanceError> {
        let git_sha = git_sha.into();
        let rustc_version = rustc_version.into();
        let target_triple = target_triple.into();
        let schema_fingerprint = schema_fingerprint.into();
        let cargo_lock_sha256 = cargo_lock_sha256.into();

        if git_sha.len() != 40 || !git_sha.bytes().all(is_lower_hex) {
            return Err(ProvenanceError::InvalidGitSha);
        }
        validate_text(&rustc_version, "rustc_version")?;
        validate_text(&target_triple, "target_triple")?;
        validate_sha256(&schema_fingerprint, "schema_fingerprint")?;
        validate_sha256(&cargo_lock_sha256, "cargo_lock_sha256")?;

        Ok(Self {
            git_sha,
            dirty,
            rustc_version,
            target_triple,
            build_epoch,
            reproducible: build_epoch.is_some(),
            schema_fingerprint,
            cargo_lock_sha256,
        })
    }

    pub fn current() -> Result<Self, ProvenanceError> {
        let dirty = match env!("ALPHA_DESK_GIT_DIRTY") {
            "true" => true,
            "false" => false,
            _ => return Err(ProvenanceError::InvalidDirtyFlag),
        };
        let epoch_text = env!("ALPHA_DESK_BUILD_EPOCH");
        let build_epoch = if epoch_text.is_empty() {
            None
        } else {
            Some(
                epoch_text
                    .parse::<u64>()
                    .map_err(|_| ProvenanceError::InvalidBuildEpoch)?,
            )
        };
        let expected_reproducible = match env!("ALPHA_DESK_REPRODUCIBLE") {
            "true" => true,
            "false" => false,
            _ => return Err(ProvenanceError::InvalidReproducibleFlag),
        };
        let build = Self::try_new(
            env!("ALPHA_DESK_GIT_SHA"),
            dirty,
            env!("ALPHA_DESK_RUSTC_VERSION"),
            env!("ALPHA_DESK_TARGET_TRIPLE"),
            build_epoch,
            env!("ALPHA_DESK_SCHEMA_FINGERPRINT"),
            env!("ALPHA_DESK_CARGO_LOCK_SHA256"),
        )?;
        if build.reproducible != expected_reproducible {
            return Err(ProvenanceError::ReproducibilityMismatch);
        }
        Ok(build)
    }

    pub fn to_json(&self) -> Result<String, ProvenanceError> {
        serde_json::to_string(self).map_err(|_| ProvenanceError::Serialization)
    }
}

fn validate_text(value: &str, field: &'static str) -> Result<(), ProvenanceError> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err(ProvenanceError::InvalidText(field));
    }
    Ok(())
}

fn validate_sha256(value: &str, field: &'static str) -> Result<(), ProvenanceError> {
    if value.len() != 64 || !value.bytes().all(is_lower_hex) {
        return Err(ProvenanceError::InvalidSha256(field));
    }
    Ok(())
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}
