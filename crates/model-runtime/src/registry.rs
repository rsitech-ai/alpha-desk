use std::collections::BTreeMap;

use domain_types::ModelVersion;
use serde::{Deserialize, Serialize};

use crate::bundle::SignedBundle;
use crate::error::ModelError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelState {
    Draft,
    ResearchPassed,
    HoldoutPassed,
    Shadow,
    Approved,
    Canary,
    Production,
    Degraded,
    Retired,
    Revoked,
}

impl ModelState {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Draft => "DRAFT",
            Self::ResearchPassed => "RESEARCH_PASSED",
            Self::HoldoutPassed => "HOLDOUT_PASSED",
            Self::Shadow => "SHADOW",
            Self::Approved => "APPROVED",
            Self::Canary => "CANARY",
            Self::Production => "PRODUCTION",
            Self::Degraded => "DEGRADED",
            Self::Retired => "RETIRED",
            Self::Revoked => "REVOKED",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionEvidence {
    SyntheticResearch,
    HoldoutEvaluation,
    ShadowLive,
    ProductionApproval,
    Degrade,
    Retire,
    Revoke,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryRecord {
    bundle_hash: [u8; 32],
    state: ModelState,
    events: Vec<RegistryEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEvent {
    from: ModelState,
    to: ModelState,
    evidence: &'static str,
}

impl RegistryRecord {
    #[must_use]
    pub const fn state(&self) -> ModelState {
        self.state
    }

    #[must_use]
    pub const fn bundle_hash(&self) -> [u8; 32] {
        self.bundle_hash
    }

    #[must_use]
    pub fn events(&self) -> &[RegistryEvent] {
        &self.events
    }
}

#[derive(Debug, Default, Clone)]
pub struct ModelRegistry {
    records: BTreeMap<String, RegistryRecord>,
}

impl ModelRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self {
            records: BTreeMap::new(),
        }
    }

    pub fn register(&mut self, bundle: &SignedBundle) -> Result<ModelVersion, ModelError> {
        let version = ModelVersion::new(format!(
            "{}:{}",
            bundle.manifest().model_id(),
            hex::encode(bundle.bundle_hash())
        ))
        .map_err(|_| ModelError::InvalidManifest)?;
        if self.records.contains_key(version.as_str()) {
            return Err(ModelError::InvalidManifest);
        }
        self.records.insert(
            version.as_str().to_owned(),
            RegistryRecord {
                bundle_hash: bundle.bundle_hash(),
                state: ModelState::Draft,
                events: Vec::new(),
            },
        );
        Ok(version)
    }

    pub fn advance(
        &mut self,
        version: &ModelVersion,
        to: ModelState,
        evidence: TransitionEvidence,
    ) -> Result<ModelState, ModelError> {
        let record = self
            .records
            .get_mut(version.as_str())
            .ok_or(ModelError::Unregistered)?;
        if record.state == ModelState::Revoked {
            return Err(ModelError::Revoked);
        }
        match (record.state, to, evidence) {
            (
                ModelState::Draft,
                ModelState::ResearchPassed,
                TransitionEvidence::SyntheticResearch,
            ) => {}
            (_, ModelState::HoldoutPassed, _) => return Err(ModelError::HoldoutNotImplemented),
            (_, ModelState::Shadow, _) => return Err(ModelError::ShadowLiveNotImplemented),
            (_, ModelState::Approved | ModelState::Canary | ModelState::Production, _) => {
                return Err(ModelError::ProductionNotImplemented);
            }
            (
                ModelState::Draft | ModelState::ResearchPassed | ModelState::Degraded,
                ModelState::Revoked,
                TransitionEvidence::Revoke,
            ) => {}
            (
                ModelState::ResearchPassed | ModelState::Degraded,
                ModelState::Retired,
                TransitionEvidence::Retire,
            ) => {}
            (ModelState::ResearchPassed, ModelState::Degraded, TransitionEvidence::Degrade) => {}
            (from, target, _) => {
                return Err(ModelError::IllegalTransition {
                    from: from.as_str(),
                    to: target.as_str(),
                });
            }
        }
        record.events.push(RegistryEvent {
            from: record.state,
            to,
            evidence: match evidence {
                TransitionEvidence::SyntheticResearch => "synthetic_research",
                TransitionEvidence::HoldoutEvaluation => "holdout_evaluation",
                TransitionEvidence::ShadowLive => "shadow_live",
                TransitionEvidence::ProductionApproval => "production_approval",
                TransitionEvidence::Degrade => "degrade",
                TransitionEvidence::Retire => "retire",
                TransitionEvidence::Revoke => "revoke",
            },
        });
        record.state = to;
        Ok(to)
    }

    pub fn stamp_holdout_passed(&self, version: &ModelVersion) -> Result<ModelState, ModelError> {
        let _ = self
            .records
            .get(version.as_str())
            .ok_or(ModelError::Unregistered)?;
        Err(ModelError::HoldoutNotImplemented)
    }

    pub fn require_loadable(&self, version: &ModelVersion) -> Result<&RegistryRecord, ModelError> {
        let record = self
            .records
            .get(version.as_str())
            .ok_or(ModelError::Unregistered)?;
        match record.state {
            ModelState::Revoked | ModelState::Retired => Err(ModelError::Revoked),
            ModelState::Draft
            | ModelState::Degraded
            | ModelState::HoldoutPassed
            | ModelState::Shadow
            | ModelState::Approved
            | ModelState::Canary
            | ModelState::Production => Err(ModelError::IllegalTransition {
                from: record.state.as_str(),
                to: "INFERENCE",
            }),
            ModelState::ResearchPassed => Ok(record),
        }
    }
}
