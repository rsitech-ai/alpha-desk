use serde::Serialize;

use crate::error::ResearchError;
use crate::estimator::EstimatorClass;
use crate::metrics::PerformanceMetrics;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VariantStatus {
    Attempted,
    Rejected,
    ResearchOnly,
}

impl VariantStatus {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Attempted => "attempted",
            Self::Rejected => "rejected",
            Self::ResearchOnly => "research_only",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct VariantRecord {
    pub variant_id: String,
    pub family_id: String,
    pub estimator_class: EstimatorClass,
    pub status: VariantStatus,
    pub metrics: Option<PerformanceMetrics>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct VariantLedger {
    records: Vec<VariantRecord>,
}

impl VariantLedger {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.records.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    #[must_use]
    pub fn records(&self) -> &[VariantRecord] {
        &self.records
    }

    pub fn register(
        &mut self,
        family_id: &str,
        class: EstimatorClass,
    ) -> Result<String, ResearchError> {
        if family_id.trim().is_empty() {
            return Err(ResearchError::InvalidFixture);
        }
        let variant_id = variant_identity(family_id, class);
        if let Some(existing) = self
            .records
            .iter()
            .find(|record| record.variant_id == variant_id)
        {
            return Ok(existing.variant_id.clone());
        }
        self.records.push(VariantRecord {
            variant_id: variant_id.clone(),
            family_id: family_id.to_owned(),
            estimator_class: class,
            status: VariantStatus::Attempted,
            metrics: None,
        });
        Ok(variant_id)
    }

    pub fn record_metrics(
        &mut self,
        variant_id: &str,
        metrics: PerformanceMetrics,
    ) -> Result<(), ResearchError> {
        let record = self
            .records
            .iter_mut()
            .find(|record| record.variant_id == variant_id)
            .ok_or(ResearchError::InvalidFixture)?;
        if record.metrics.is_some() {
            return Err(ResearchError::ImmutableVariant);
        }
        record.metrics = Some(metrics);
        Ok(())
    }

    pub fn mark_research_only(&mut self, variant_id: &str) -> Result<(), ResearchError> {
        let record = self
            .records
            .iter_mut()
            .find(|record| record.variant_id == variant_id)
            .ok_or(ResearchError::InvalidFixture)?;
        record.status = VariantStatus::ResearchOnly;
        Ok(())
    }

    pub fn reject(&mut self, variant_id: &str) -> Result<(), ResearchError> {
        let record = self
            .records
            .iter_mut()
            .find(|record| record.variant_id == variant_id)
            .ok_or(ResearchError::InvalidFixture)?;
        record.status = VariantStatus::Rejected;
        Ok(())
    }

    pub fn claim_significance(&self) -> Result<(), ResearchError> {
        Err(ResearchError::SignificanceNotClaimed)
    }

    pub fn accept_holdout(&self, _variant_id: &str) -> Result<(), ResearchError> {
        Err(ResearchError::HoldoutNotImplemented)
    }
}

pub fn variant_identity(family_id: &str, class: EstimatorClass) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hl.research.variant.v1");
    hasher.update(family_id.as_bytes());
    hasher.update(class.as_str().as_bytes());
    hex::encode(hasher.finalize().as_bytes())
}
