use serde::{Deserialize, Serialize};

use crate::{SignalError, signal::SignalLifecycleState};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AlertDecision {
    Emit,
    CooldownSuppressed,
    BudgetExhausted,
    AlwaysEmitRisk,
}

impl AlertDecision {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Emit => "emit",
            Self::CooldownSuppressed => "cooldown_suppressed",
            Self::BudgetExhausted => "budget_exhausted",
            Self::AlwaysEmitRisk => "always_emit_risk",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertPolicy {
    pub cooldown_micros: u64,
    pub daily_budget: u32,
}

impl AlertPolicy {
    pub fn from_toml(text: &str) -> Result<Self, SignalError> {
        let policy: Self = toml::from_str(text)
            .map_err(|_| SignalError::ContractViolation("alert policy toml"))?;
        if policy.cooldown_micros == 0 || policy.daily_budget == 0 {
            return Err(SignalError::ContractViolation(
                "cooldown and budget must be positive",
            ));
        }
        Ok(policy)
    }

    pub fn decide(
        &self,
        next_state: SignalLifecycleState,
        material: bool,
        elapsed_since_last_alert_micros: u64,
        emitted_today: u32,
        risk_escalation: bool,
    ) -> AlertDecision {
        if matches!(
            next_state,
            SignalLifecycleState::Invalidated | SignalLifecycleState::Expired
        ) || risk_escalation
        {
            return AlertDecision::AlwaysEmitRisk;
        }
        if !material && elapsed_since_last_alert_micros < self.cooldown_micros {
            return AlertDecision::CooldownSuppressed;
        }
        if emitted_today >= self.daily_budget && !material {
            return AlertDecision::BudgetExhausted;
        }
        AlertDecision::Emit
    }
}
