use domain_types::{Decimal, RoundingMode};
use serde::Deserialize;

use crate::bundle::{ArtifactKind, SignedBundle};
use crate::error::ModelError;
use crate::registry::ModelRegistry;
use domain_types::ModelVersion;

#[derive(Debug, Deserialize)]
struct LinearArtifact {
    kind: String,
    weights: Vec<String>,
    intercept: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResearchScore {
    value: Decimal,
}

impl ResearchScore {
    #[must_use]
    pub const fn value(&self) -> Decimal {
        self.value
    }
}

pub fn score_research_bundle(
    registry: &ModelRegistry,
    version: &ModelVersion,
    bundle: &SignedBundle,
    feature_names: &[String],
    feature_values: &[Decimal],
) -> Result<ResearchScore, ModelError> {
    registry.require_loadable(version)?;
    bundle.schema().require_exact_match(feature_names)?;
    match bundle.manifest().artifact_kind() {
        ArtifactKind::Onnx => Err(ModelError::OnnxProductionUnavailable),
        ArtifactKind::DeterministicLinearV1 => {
            if feature_values.len() != feature_names.len() {
                return Err(ModelError::InvalidInput);
            }
            let artifact: LinearArtifact = serde_json::from_slice(bundle.artifact_bytes()?)
                .map_err(|_| ModelError::InvalidManifest)?;
            if artifact.kind != "deterministic-linear-v1" {
                return Err(ModelError::UnsupportedArtifact);
            }
            if artifact.weights.len() != feature_values.len() {
                return Err(ModelError::SchemaMismatch);
            }
            let intercept = artifact
                .intercept
                .parse::<Decimal>()
                .map_err(|_| ModelError::InvalidManifest)?
                .rescale(8, RoundingMode::TowardZero)
                .map_err(|_| ModelError::InvalidInput)?;
            let mut acc = intercept;
            for (weight, value) in artifact.weights.iter().zip(feature_values) {
                let weight = weight
                    .parse::<Decimal>()
                    .map_err(|_| ModelError::InvalidManifest)?
                    .rescale(8, RoundingMode::TowardZero)
                    .map_err(|_| ModelError::InvalidInput)?;
                let value = value
                    .rescale(8, RoundingMode::TowardZero)
                    .map_err(|_| ModelError::InvalidInput)?;
                let term = weight
                    .checked_mul(value, 8, RoundingMode::TowardZero)
                    .map_err(|_| ModelError::InvalidInput)?;
                acc = acc
                    .checked_add(term)
                    .map_err(|_| ModelError::InvalidInput)?;
            }
            Ok(ResearchScore { value: acc })
        }
    }
}
