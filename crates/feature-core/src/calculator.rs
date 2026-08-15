use std::collections::BTreeMap;

use domain_types::{BlockHeight, FeatureSetVersion, KnownTime, ProtocolTime};

use crate::{
    FeatureError, FeatureKey, FeatureManifest, FeatureSnapshot, FeatureSubject, FeatureValue,
    HealthState,
};

/// Point-in-time calculator context derived from reconstructed local state.
///
/// This type is intentionally independent of `canonical-ledger::StateDelta` so
/// `feature-core` does not depend on the state crate graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureContext {
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub input_watermark: BlockHeight,
    pub data_health: HealthState,
}

impl FeatureContext {
    pub fn try_new(
        feature_set_version: FeatureSetVersion,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        input_watermark: BlockHeight,
        data_health: HealthState,
    ) -> Result<Self, FeatureError> {
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(FeatureError::TemporalInversion);
        }
        Ok(Self {
            feature_set_version,
            effective_at,
            known_at,
            input_watermark,
            data_health,
        })
    }
}

/// Portable reconstructed-state observation consumed by feature calculators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeatureDelta {
    pub subject: FeatureSubject,
    pub values: BTreeMap<FeatureKey, FeatureValue>,
}

impl FeatureDelta {
    pub fn try_new(
        subject: FeatureSubject,
        values: BTreeMap<FeatureKey, FeatureValue>,
    ) -> Result<Self, FeatureError> {
        if values.is_empty() {
            return Err(FeatureError::Malformed {
                what: "feature_delta",
                reason: "empty values",
            });
        }
        Ok(Self { subject, values })
    }
}

/// Deterministic feature calculator over reconstructed observations.
///
/// Live and synthetic replay paths must invoke the same implementation. The
/// calculator never invents fills or USD equity; missing reconstructed inputs
/// must arrive as [`FeatureValue::Missing`].
pub trait FeatureCalculator {
    fn on_delta(
        &mut self,
        delta: &FeatureDelta,
        ctx: &FeatureContext,
        manifest: Option<&FeatureManifest>,
    ) -> Result<Vec<FeatureSnapshot>, FeatureError>;
}

/// Append-only bitemporal snapshot calculator for local PIT materialization.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PitSnapshotCalculator {
    revisions: BTreeMap<FeatureSubject, u32>,
    snapshots: Vec<FeatureSnapshot>,
}

impl PitSnapshotCalculator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn snapshots(&self) -> &[FeatureSnapshot] {
        &self.snapshots
    }
}

impl FeatureCalculator for PitSnapshotCalculator {
    fn on_delta(
        &mut self,
        delta: &FeatureDelta,
        ctx: &FeatureContext,
        manifest: Option<&FeatureManifest>,
    ) -> Result<Vec<FeatureSnapshot>, FeatureError> {
        let revision = self
            .revisions
            .get(&delta.subject)
            .copied()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(FeatureError::Malformed {
                what: "feature_snapshot",
                reason: "revision overflow",
            })?;
        let snapshot = FeatureSnapshot::try_new(
            delta.subject.clone(),
            ctx.feature_set_version.clone(),
            ctx.effective_at,
            ctx.known_at,
            None,
            revision,
            delta.values.clone(),
            ctx.input_watermark,
            ctx.data_health,
            manifest,
        )?;
        self.revisions.insert(delta.subject.clone(), revision);
        self.snapshots.push(snapshot.clone());
        Ok(vec![snapshot])
    }
}
