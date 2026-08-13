use std::collections::BTreeMap;

use domain_types::{BlockHeight, FeatureSetVersion, KnownTime, ProtocolTime};
use serde::{Deserialize, Serialize};

use crate::{FeatureError, FeatureKey, FeatureManifest, FeatureSubject, FeatureValue, HealthState};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureSnapshot {
    pub subject: FeatureSubject,
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub superseded_at: Option<KnownTime>,
    pub revision: u32,
    pub values: BTreeMap<FeatureKey, FeatureValue>,
    pub input_watermark: BlockHeight,
    pub data_health: HealthState,
    pub provenance_hash: [u8; 32],
}

impl FeatureSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        subject: FeatureSubject,
        feature_set_version: FeatureSetVersion,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        superseded_at: Option<KnownTime>,
        revision: u32,
        values: BTreeMap<FeatureKey, FeatureValue>,
        input_watermark: BlockHeight,
        data_health: HealthState,
        manifest: Option<&FeatureManifest>,
    ) -> Result<Self, FeatureError> {
        if revision == 0 {
            return Err(FeatureError::Malformed {
                what: "feature_snapshot",
                reason: "revision must be >= 1",
            });
        }
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(FeatureError::TemporalInversion);
        }
        if let Some(superseded_at) = superseded_at
            && superseded_at.unix_micros() <= known_at.unix_micros()
        {
            return Err(FeatureError::InvalidSupersession);
        }
        if values.is_empty() {
            return Err(FeatureError::Malformed {
                what: "feature_snapshot",
                reason: "empty values",
            });
        }
        if data_health == HealthState::Red
            && !values
                .values()
                .all(|value| matches!(value, FeatureValue::Missing(_)))
        {
            return Err(FeatureError::Malformed {
                what: "feature_snapshot",
                reason: "red data health must emit missing values",
            });
        }
        if let Some(manifest) = manifest {
            for key in values.keys() {
                manifest.require(key)?;
            }
        }
        let mut snapshot = Self {
            subject,
            feature_set_version,
            effective_at,
            known_at,
            superseded_at,
            revision,
            values,
            input_watermark,
            data_health,
            provenance_hash: [0_u8; 32],
        };
        snapshot.provenance_hash = snapshot.compute_provenance_hash();
        Ok(snapshot)
    }

    #[must_use]
    pub fn compute_provenance_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(self.subject.subject_type().as_bytes());
        hasher.update(&[0]);
        hasher.update(self.subject.subject_id().as_bytes());
        hasher.update(&[0]);
        hasher.update(self.feature_set_version.as_str().as_bytes());
        hasher.update(&self.effective_at.unix_micros().to_le_bytes());
        hasher.update(&self.known_at.unix_micros().to_le_bytes());
        hasher.update(&self.revision.to_le_bytes());
        hasher.update(&self.input_watermark.get().to_le_bytes());
        hasher.update(self.data_health.as_wire_name().as_bytes());
        for (key, value) in &self.values {
            hasher.update(key.namespace.as_bytes());
            hasher.update(&[0]);
            hasher.update(key.name.as_bytes());
            hasher.update(&[0]);
            hasher.update(&key.version.to_le_bytes());
            hash_feature_value(&mut hasher, value);
        }
        *hasher.finalize().as_bytes()
    }
}

fn hash_feature_value(hasher: &mut blake3::Hasher, value: &FeatureValue) {
    match value {
        FeatureValue::Decimal { raw, scale } => {
            hasher.update(&[0]);
            hasher.update(&raw.to_le_bytes());
            hasher.update(&scale.to_le_bytes());
        }
        FeatureValue::SignedInteger(raw) => {
            hasher.update(&[1]);
            hasher.update(&raw.to_le_bytes());
        }
        FeatureValue::UnsignedInteger(raw) => {
            hasher.update(&[2]);
            hasher.update(&raw.to_le_bytes());
        }
        FeatureValue::ProbabilityPpm(probability) => {
            hasher.update(&[3]);
            hasher.update(&probability.ppm().to_le_bytes());
        }
        FeatureValue::Category(category) => {
            hasher.update(&[4]);
            hasher.update(category.as_bytes());
        }
        FeatureValue::Boolean(flag) => {
            hasher.update(&[5]);
            hasher.update(&[u8::from(*flag)]);
        }
        FeatureValue::Missing(reason) => {
            hasher.update(&[6]);
            hasher.update(reason.as_wire_name().as_bytes());
        }
    }
}
