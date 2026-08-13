use serde::{Deserialize, Serialize};

use crate::error::ModelError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSchema {
    ordered_features: Vec<String>,
}

impl FeatureSchema {
    pub fn new(ordered_features: Vec<String>) -> Result<Self, ModelError> {
        if ordered_features.is_empty() || ordered_features.iter().any(|name| name.trim().is_empty())
        {
            return Err(ModelError::InvalidManifest);
        }
        Ok(Self { ordered_features })
    }

    #[must_use]
    pub fn ordered_features(&self) -> &[String] {
        &self.ordered_features
    }

    pub fn require_exact_match(&self, observed: &[String]) -> Result<(), ModelError> {
        if self.ordered_features == observed {
            Ok(())
        } else {
            Err(ModelError::SchemaMismatch)
        }
    }
}
