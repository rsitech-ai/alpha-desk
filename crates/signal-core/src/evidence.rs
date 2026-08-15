use domain_types::{BlockHeight, EntityId, Horizon, ProbabilityPpm, SignalId, UsdAmount};
use feature_core::EvidenceRef;
use market_intelligence::{
    AnalogueSet, MarketFeatureSnapshot, ObservationAdmission, ObservationMintKind,
};
use serde::{Deserialize, Serialize};

use crate::{SignalError, invalidation::InvalidationRule};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceBundle {
    pub signal_id: SignalId,
    pub canonical_event_refs: Vec<EvidenceRef>,
    pub entities: Vec<(EntityId, ProbabilityPpm)>,
    pub feature_before: MarketFeatureSnapshot,
    pub feature_after: MarketFeatureSnapshot,
    pub watermark: BlockHeight,
    pub source_confidence: ProbabilityPpm,
    pub model_artifact_hash: [u8; 32],
    pub code_commit: String,
    pub cost_assumptions_hash: [u8; 32],
    pub analogues: AnalogueSet,
    pub invalidation_rules: Vec<InvalidationRule>,
    pub capacity: UsdAmount,
    pub half_life: Horizon,
    pub limitations: Vec<String>,
    pub content_hash: [u8; 32],
}

impl EvidenceBundle {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        signal_id: SignalId,
        canonical_event_refs: Vec<EvidenceRef>,
        entities: Vec<(EntityId, ProbabilityPpm)>,
        feature_before: MarketFeatureSnapshot,
        feature_after: MarketFeatureSnapshot,
        watermark: BlockHeight,
        source_confidence: ProbabilityPpm,
        model_artifact_hash: [u8; 32],
        code_commit: String,
        cost_assumptions_hash: [u8; 32],
        analogues: AnalogueSet,
        invalidation_rules: Vec<InvalidationRule>,
        capacity: UsdAmount,
        half_life: Horizon,
        limitations: Vec<String>,
    ) -> Result<Self, SignalError> {
        if code_commit.trim().is_empty() {
            return Err(SignalError::EmptyIdentifier {
                field: "code_commit",
            });
        }
        if limitations.iter().any(|item| item.trim().is_empty()) {
            return Err(SignalError::EmptyIdentifier {
                field: "limitations",
            });
        }
        let mut bundle = Self {
            signal_id,
            canonical_event_refs,
            entities,
            feature_before,
            feature_after,
            watermark,
            source_confidence,
            model_artifact_hash,
            code_commit,
            cost_assumptions_hash,
            analogues,
            invalidation_rules,
            capacity,
            half_life,
            limitations,
            content_hash: [0_u8; 32],
        };
        bundle.content_hash = bundle.compute_hash();
        Ok(bundle)
    }

    #[must_use]
    pub fn compute_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"alpha-desk/signal-evidence/v1");
        hasher.update(self.signal_id.as_str().as_bytes());
        hasher.update(&self.watermark.get().to_le_bytes());
        hasher.update(&self.source_confidence.ppm().to_le_bytes());
        hasher.update(&self.model_artifact_hash);
        hasher.update(self.code_commit.as_bytes());
        hasher.update(&self.cost_assumptions_hash);
        hasher.update(&self.feature_before.provenance_hash);
        hasher.update(&self.feature_after.provenance_hash);
        hasher.update(&self.capacity.raw().to_le_bytes());
        hasher.update(&self.half_life.as_micros().to_le_bytes());
        hasher.update(&(u64::try_from(self.canonical_event_refs.len()).unwrap_or(0)).to_le_bytes());
        hasher.update(&(u64::try_from(self.invalidation_rules.len()).unwrap_or(0)).to_le_bytes());
        hasher.update(self.limitations.join("\0").as_bytes());
        *hasher.finalize().as_bytes()
    }

    #[must_use]
    pub fn missing_for_admission(&self) -> Vec<String> {
        let mut missing = Vec::new();
        if self.canonical_event_refs.is_empty() {
            missing.push("canonical_event_refs".to_owned());
        }
        if self.model_artifact_hash.iter().all(|byte| *byte == 0) {
            missing.push("model_artifact_hash".to_owned());
        }
        if self.cost_assumptions_hash.iter().all(|byte| *byte == 0) {
            missing.push("cost_assumptions".to_owned());
        }
        if self.invalidation_rules.is_empty() {
            missing.push("invalidation_rules".to_owned());
        }
        if self.limitations.is_empty() {
            missing.push("limitations".to_owned());
        }
        if self.watermark.get() == 0 {
            missing.push("data_watermark".to_owned());
        }
        if snapshot_lacks_observation(
            &self.feature_before,
            "book",
            ObservationMintKind::DecimalDepth,
        ) || snapshot_lacks_observation(
            &self.feature_after,
            "book",
            ObservationMintKind::DecimalDepth,
        ) {
            missing.push("book".to_owned());
        }
        if snapshot_lacks_observation(
            &self.feature_before,
            "fills",
            ObservationMintKind::BooleanPresence,
        ) || snapshot_lacks_observation(
            &self.feature_after,
            "fills",
            ObservationMintKind::BooleanPresence,
        ) {
            missing.push("fills".to_owned());
        }
        if matches!(
            inventory_admission(&self.feature_before),
            ObservationAdmission::Missing
        ) || matches!(
            inventory_admission(&self.feature_after),
            ObservationAdmission::Missing
        ) {
            missing.push("inventory".to_owned());
        }
        missing
    }

    #[must_use]
    pub fn malformed_for_admission(&self) -> Option<(&'static str, &'static str)> {
        for snapshot in [&self.feature_before, &self.feature_after] {
            match inventory_admission(snapshot) {
                ObservationAdmission::Malformed { what, reason } => {
                    return Some((what, reason));
                }
                ObservationAdmission::Observed | ObservationAdmission::Missing => {}
            }
        }
        None
    }
}

fn snapshot_lacks_observation(
    snapshot: &MarketFeatureSnapshot,
    name: &'static str,
    kind: ObservationMintKind,
) -> bool {
    match snapshot.admit_observation(name, kind) {
        ObservationAdmission::Observed => false,
        ObservationAdmission::Missing | ObservationAdmission::Malformed { .. } => true,
    }
}

fn inventory_admission(snapshot: &MarketFeatureSnapshot) -> ObservationAdmission {
    snapshot.admit_observation("inventory", ObservationMintKind::DecimalDepth)
}
