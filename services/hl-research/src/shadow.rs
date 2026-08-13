use domain_types::{KnownTime, ProtocolTime};
use serde::{Deserialize, Serialize};

use crate::claims::serialize_unclaimed;
use crate::error::ResearchError;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowDecision {
    pub id: String,
    pub decided_at: ProtocolTime,
    pub known_at: KnownTime,
    pub prediction: String,
    pub expected_cost: String,
    pub model_hash: String,
    pub feature_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowOutcome {
    pub id: String,
    pub observed_at: ProtocolTime,
    pub known_at: KnownTime,
    pub realized_net: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShadowCapture {
    decisions: Vec<ShadowDecision>,
    outcomes: Vec<ShadowOutcome>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ShadowCaptureReport {
    pub schema_version: &'static str,
    pub mode: &'static str,
    pub shadow_live: &'static str,
    pub live_trading: bool,
    pub signer_attached: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_quality_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub alpha_qualified: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub significance_claimed: bool,
    #[serde(serialize_with = "serialize_unclaimed")]
    pub stage_pass_claimed: bool,
    pub decisions: usize,
    pub outcomes: usize,
    pub capture_hash: String,
}

#[derive(Debug, Deserialize)]
struct ShadowFixture {
    evaluation_known_at: KnownTime,
    horizon_micros: u64,
    decisions: Vec<ShadowDecision>,
    outcomes: Vec<ShadowOutcome>,
}

impl ShadowCapture {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn record_decision(&mut self, decision: ShadowDecision) -> Result<(), ResearchError> {
        if decision.id.trim().is_empty()
            || decision.prediction.trim().is_empty()
            || decision.model_hash.trim().is_empty()
            || decision.feature_hash.trim().is_empty()
        {
            return Err(ResearchError::InvalidFixture);
        }
        if decision.known_at.unix_micros() < decision.decided_at.unix_micros() {
            return Err(ResearchError::FutureData {
                field: "decision.known_at",
            });
        }
        if self
            .decisions
            .iter()
            .any(|existing| existing.id == decision.id)
        {
            return Err(ResearchError::ShadowLeakage {
                field: "decision.duplicate",
            });
        }
        self.decisions.push(decision);
        Ok(())
    }

    pub fn record_outcome(
        &mut self,
        outcome: ShadowOutcome,
        evaluation_known_at: KnownTime,
        horizon_micros: u64,
    ) -> Result<(), ResearchError> {
        let decision = self
            .decisions
            .iter()
            .find(|decision| decision.id == outcome.id)
            .ok_or(ResearchError::ShadowLeakage {
                field: "outcome.missing_decision",
            })?;
        if self
            .outcomes
            .iter()
            .any(|existing| existing.id == outcome.id)
        {
            return Err(ResearchError::ShadowLeakage {
                field: "outcome.duplicate",
            });
        }
        if outcome.known_at.unix_micros() > evaluation_known_at.unix_micros() {
            return Err(ResearchError::FutureData {
                field: "outcome.known_at",
            });
        }
        if outcome.known_at.unix_micros() < outcome.observed_at.unix_micros() {
            return Err(ResearchError::FutureData {
                field: "outcome.known_before_protocol",
            });
        }
        let min_known = decision
            .known_at
            .unix_micros()
            .checked_add(i64::try_from(horizon_micros).map_err(|_| ResearchError::InvalidFixture)?)
            .ok_or(ResearchError::InvalidFixture)?;
        if outcome.known_at.unix_micros() < min_known {
            return Err(ResearchError::ShadowLeakage {
                field: "outcome.before_horizon",
            });
        }
        if outcome.observed_at.unix_micros() <= decision.decided_at.unix_micros() {
            return Err(ResearchError::ShadowLeakage {
                field: "outcome.before_decision",
            });
        }
        self.outcomes.push(outcome);
        Ok(())
    }

    pub fn attach_trading_signer(&self, _material: &[u8]) -> Result<(), ResearchError> {
        Err(ResearchError::TradingSignerForbidden)
    }

    pub fn promote_to_live_trading(&self) -> Result<(), ResearchError> {
        Err(ResearchError::TradingSignerForbidden)
    }

    pub fn promote_to_shadow_registry(&self) -> Result<(), ResearchError> {
        Err(ResearchError::ShadowLiveNotImplemented)
    }

    pub fn report(&self) -> ShadowCaptureReport {
        ShadowCaptureReport {
            schema_version: "hl.research.shadow-capture.v1",
            mode: "shadow_capture",
            shadow_live: "capture_only",
            live_trading: false,
            signer_attached: false,
            alpha_quality_claimed: false,
            alpha_qualified: false,
            significance_claimed: false,
            stage_pass_claimed: false,
            decisions: self.decisions.len(),
            outcomes: self.outcomes.len(),
            capture_hash: hex::encode(self.capture_hash()),
        }
    }

    fn capture_hash(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"hl.research.shadow-capture.v1");
        for decision in &self.decisions {
            hasher.update(decision.id.as_bytes());
            hasher.update(&decision.decided_at.unix_micros().to_le_bytes());
            hasher.update(&decision.known_at.unix_micros().to_le_bytes());
            hasher.update(decision.prediction.as_bytes());
            hasher.update(decision.expected_cost.as_bytes());
            hasher.update(decision.model_hash.as_bytes());
            hasher.update(decision.feature_hash.as_bytes());
        }
        for outcome in &self.outcomes {
            hasher.update(outcome.id.as_bytes());
            hasher.update(&outcome.observed_at.unix_micros().to_le_bytes());
            hasher.update(&outcome.known_at.unix_micros().to_le_bytes());
            hasher.update(outcome.realized_net.as_bytes());
        }
        *hasher.finalize().as_bytes()
    }
}

pub fn run_shadow_capture_bytes(bytes: &[u8]) -> Result<ShadowCaptureReport, ResearchError> {
    let fixture: ShadowFixture =
        serde_json::from_slice(bytes).map_err(|_| ResearchError::InvalidFixture)?;
    let mut capture = ShadowCapture::new();
    for decision in fixture.decisions {
        capture.record_decision(decision)?;
    }
    for outcome in fixture.outcomes {
        capture.record_outcome(outcome, fixture.evaluation_known_at, fixture.horizon_micros)?;
    }
    Ok(capture.report())
}
