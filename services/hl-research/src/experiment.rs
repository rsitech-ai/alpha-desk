use domain_types::{BlockRange, ExperimentId, FeatureSetVersion, LabelDefinitionId};
use serde::{Deserialize, Serialize};

use crate::error::ResearchError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperimentStatus {
    Exploratory,
    Registered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperimentManifest {
    pub hypothesis: String,
    pub owner: String,
    pub code_commit: String,
    pub rust_toolchain: String,
    pub feature_set_version: FeatureSetVersion,
    pub label_definition: LabelDefinitionId,
    pub market_universe_version: String,
    pub wallet_score_version: String,
    pub cluster_version_policy: String,
    pub training_range: BlockRange,
    pub validation_ranges: Vec<BlockRange>,
    pub holdout_range: BlockRange,
    pub data_manifest_hash: String,
    pub model_config_hash: String,
    pub random_seed: u64,
    pub cost_model_version: String,
    pub execution_latency_assumptions: String,
    pub promotion_metrics: Vec<String>,
    pub reviewers: Vec<String>,
}

impl ExperimentManifest {
    pub fn missing_field(&self) -> Option<&'static str> {
        if self.hypothesis.trim().is_empty() {
            return Some("hypothesis");
        }
        if self.owner.trim().is_empty() {
            return Some("owner");
        }
        if self.code_commit.trim().is_empty() {
            return Some("code_commit");
        }
        if self.rust_toolchain.trim().is_empty() {
            return Some("rust_toolchain");
        }
        if self.market_universe_version.trim().is_empty() {
            return Some("market_universe_version");
        }
        if self.wallet_score_version.trim().is_empty() {
            return Some("wallet_score_version");
        }
        if self.cluster_version_policy.trim().is_empty() {
            return Some("cluster_version_policy");
        }
        if self.validation_ranges.is_empty() {
            return Some("validation_ranges");
        }
        if self.data_manifest_hash.trim().is_empty() {
            return Some("data_manifest_hash");
        }
        if self.model_config_hash.trim().is_empty() {
            return Some("model_config_hash");
        }
        if self.cost_model_version.trim().is_empty() {
            return Some("cost_model_version");
        }
        if self.execution_latency_assumptions.trim().is_empty() {
            return Some("execution_latency_assumptions");
        }
        if self.promotion_metrics.is_empty() {
            return Some("promotion_metrics");
        }
        if self.reviewers.is_empty() {
            return Some("reviewers");
        }
        None
    }

    pub fn content_hash(&self) -> [u8; 32] {
        let encoded = serde_json::to_vec(self).unwrap_or_default();
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"hl.research.manifest.v1");
        hasher.update(&encoded);
        *hasher.finalize().as_bytes()
    }

    pub fn experiment_id(&self) -> Result<ExperimentId, ResearchError> {
        ExperimentId::new(hex::encode(self.content_hash()))
            .map_err(|_| ResearchError::InvalidFixture)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExperimentRecord {
    pub experiment_id: ExperimentId,
    pub status: ExperimentStatus,
    pub manifest: ExperimentManifest,
}

#[derive(Debug, Default, Clone)]
pub struct ExperimentRegistry {
    records: Vec<ExperimentRecord>,
}

impl ExperimentRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
        }
    }

    pub fn submit(
        &mut self,
        manifest: ExperimentManifest,
    ) -> Result<ExperimentRecord, ResearchError> {
        if let Some(field) = manifest.missing_field() {
            return Err(ResearchError::IncompleteManifest { field });
        }
        let experiment_id = manifest.experiment_id()?;
        if let Some(existing) = self
            .records
            .iter()
            .find(|record| record.experiment_id == experiment_id)
        {
            return Ok(existing.clone());
        }
        let record = ExperimentRecord {
            experiment_id,
            status: ExperimentStatus::Registered,
            manifest,
        };
        self.records.push(record.clone());
        Ok(record)
    }

    pub fn replace_registered(
        &self,
        experiment_id: &ExperimentId,
        _manifest: ExperimentManifest,
    ) -> Result<(), ResearchError> {
        if self
            .records
            .iter()
            .any(|record| record.experiment_id == *experiment_id)
        {
            Err(ResearchError::ImmutableExperiment)
        } else {
            Err(ResearchError::InvalidFixture)
        }
    }

    pub fn get(&self, experiment_id: &ExperimentId) -> Result<&ExperimentRecord, ResearchError> {
        self.records
            .iter()
            .find(|record| record.experiment_id == *experiment_id)
            .ok_or(ResearchError::InvalidFixture)
    }

    pub fn open_locked_holdout(&self, experiment_id: &ExperimentId) -> Result<(), ResearchError> {
        let _ = self.get(experiment_id)?;
        Err(ResearchError::HoldoutNotImplemented)
    }

    pub fn open_holdout(&self, experiment_id: &ExperimentId) -> Result<(), ResearchError> {
        self.open_locked_holdout(experiment_id)
    }
}
