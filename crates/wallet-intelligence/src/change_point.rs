use domain_types::ProtocolTime;
use serde::{Deserialize, Serialize};

use crate::IntelligenceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ChangeReason {
    CapitalActivation,
    TurnoverShift,
    MakerRatioShift,
    MarketSpecialization,
    LeverageEscalation,
    RiskEscalation,
    LinkedAccountMigration,
    SkillDecay,
    DormantReactivation,
}

impl ChangeReason {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::CapitalActivation => "capital_activation",
            Self::TurnoverShift => "turnover_shift",
            Self::MakerRatioShift => "maker_ratio_shift",
            Self::MarketSpecialization => "market_specialization",
            Self::LeverageEscalation => "leverage_escalation",
            Self::RiskEscalation => "risk_escalation",
            Self::LinkedAccountMigration => "linked_account_migration",
            Self::SkillDecay => "skill_decay",
            Self::DormantReactivation => "dormant_reactivation",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorSample {
    pub protocol_time: ProtocolTime,
    pub maker_ratio_ppm: u32,
    pub turnover_ppm: u32,
    pub leverage_milli: u32,
    pub skill_bps: i64,
    pub dormant: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BehaviorRegime {
    pub regime_id: u32,
    pub started_at: ProtocolTime,
    pub ended_at: Option<ProtocolTime>,
    pub reasons: Vec<ChangeReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangePointDetector {
    threshold: i64,
    drift: i64,
    min_evidence: usize,
    cusum: i64,
    run: usize,
    last_mean: i64,
    next_regime: u32,
    regimes: Vec<BehaviorRegime>,
}

impl ChangePointDetector {
    pub fn try_new(
        threshold: i64,
        drift: i64,
        min_evidence: usize,
    ) -> Result<Self, IntelligenceError> {
        if threshold <= 0 || drift < 0 || min_evidence == 0 {
            return Err(IntelligenceError::Malformed {
                what: "change_point",
                reason: "threshold, drift, and min_evidence must be valid",
            });
        }
        Ok(Self {
            threshold,
            drift,
            min_evidence,
            cusum: 0,
            run: 0,
            last_mean: 0,
            next_regime: 1,
            regimes: Vec::new(),
        })
    }

    pub fn observe(
        &mut self,
        sample: BehaviorSample,
    ) -> Result<Option<BehaviorRegime>, IntelligenceError> {
        if sample.maker_ratio_ppm > 1_000_000 || sample.turnover_ppm > 1_000_000 {
            return Err(IntelligenceError::OutOfRange);
        }
        let observation = i64::from(sample.maker_ratio_ppm);
        if self.regimes.is_empty() {
            self.last_mean = observation;
            self.regimes.push(BehaviorRegime {
                regime_id: self.next_regime,
                started_at: sample.protocol_time,
                ended_at: None,
                reasons: Vec::new(),
            });
            self.next_regime += 1;
            return Ok(None);
        }
        let abs_dev = observation
            .checked_sub(self.last_mean)
            .ok_or(IntelligenceError::Overflow)?
            .unsigned_abs();
        let drift = u64::try_from(self.drift).map_err(|_| IntelligenceError::Overflow)?;
        let deviation = abs_dev.saturating_sub(drift);
        self.cusum = self
            .cusum
            .saturating_add(i64::try_from(deviation).map_err(|_| IntelligenceError::Overflow)?);
        self.run += 1;
        let mut reasons = Vec::new();
        if sample.dormant {
            reasons.push(ChangeReason::DormantReactivation);
        }
        if sample.leverage_milli > 4_000 {
            reasons.push(ChangeReason::LeverageEscalation);
        }
        if sample.skill_bps < self.last_mean / 1_000 {
            reasons.push(ChangeReason::SkillDecay);
        }
        if observation + 300_000 < self.last_mean {
            reasons.push(ChangeReason::MakerRatioShift);
        }
        if self.cusum >= self.threshold && self.run >= self.min_evidence {
            if let Some(current) = self.regimes.last_mut() {
                current.ended_at = Some(sample.protocol_time);
            }
            let regime = BehaviorRegime {
                regime_id: self.next_regime,
                started_at: sample.protocol_time,
                ended_at: None,
                reasons: if reasons.is_empty() {
                    vec![ChangeReason::TurnoverShift]
                } else {
                    reasons
                },
            };
            self.next_regime += 1;
            self.cusum = 0;
            self.run = 0;
            self.last_mean = observation;
            self.regimes.push(regime.clone());
            return Ok(Some(regime));
        }
        Ok(None)
    }

    #[must_use]
    pub fn regimes(&self) -> &[BehaviorRegime] {
        &self.regimes
    }
}
