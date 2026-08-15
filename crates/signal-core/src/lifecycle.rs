use domain_types::{KnownTime, ProtocolTime, SignalId};
use feature_core::HealthState;
use serde::{Deserialize, Serialize};

use crate::{
    SignalError,
    evidence::EvidenceBundle,
    signal::{SignalActor, SignalLifecycleState, SignalType},
};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalLifecycleEvent {
    pub signal_id: SignalId,
    pub previous: Option<SignalLifecycleState>,
    pub next: SignalLifecycleState,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub reason_code: String,
    pub evidence_bundle_hash: [u8; 32],
    pub build_commit: String,
    pub actor: SignalActor,
}

impl SignalLifecycleEvent {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        signal_id: SignalId,
        previous: Option<SignalLifecycleState>,
        next: SignalLifecycleState,
        effective_at: ProtocolTime,
        known_at: KnownTime,
        reason_code: String,
        evidence_bundle_hash: [u8; 32],
        build_commit: String,
        actor: SignalActor,
    ) -> Result<Self, SignalError> {
        if reason_code.trim().is_empty() || build_commit.trim().is_empty() {
            return Err(SignalError::EmptyIdentifier {
                field: "lifecycle_event",
            });
        }
        if evidence_bundle_hash.iter().all(|byte| *byte == 0) {
            return Err(SignalError::IncompleteEvidence(vec![
                "lifecycle_evidence_hash".to_owned(),
            ]));
        }
        if known_at.unix_micros() < effective_at.unix_micros() {
            return Err(SignalError::ContractViolation(
                "lifecycle known_at precedes effective_at",
            ));
        }
        Ok(Self {
            signal_id,
            previous,
            next,
            effective_at,
            known_at,
            reason_code,
            evidence_bundle_hash,
            build_commit,
            actor,
        })
    }
}

pub fn transition_allowed(
    from: Option<SignalLifecycleState>,
    to: SignalLifecycleState,
    signal_type: &SignalType,
    evidence: &EvidenceBundle,
    health: HealthState,
    confirmation_live_ok: bool,
) -> Result<(), SignalError> {
    if requires_evidence_admission(to) {
        let missing = evidence.missing_for_admission();
        if !missing.is_empty() {
            return Err(SignalError::IncompleteEvidence(missing));
        }
        if let Some((what, reason)) = evidence.malformed_for_admission() {
            return Err(SignalError::Malformed { what, reason });
        }
        if health == HealthState::Red {
            return Err(SignalError::UnsupportedHealth);
        }
    }
    if to == SignalLifecycleState::Live {
        if !signal_type.can_enter_live() {
            return Err(SignalError::ResearchOnlyCannotGoLive);
        }
        if !confirmation_live_ok {
            return Err(SignalError::ContractViolation(
                "confirmation class cannot enter live",
            ));
        }
        if health != HealthState::Green {
            return Err(SignalError::UnsupportedHealth);
        }
    }
    match (from, to) {
        (None, SignalLifecycleState::Candidate) => Ok(()),
        (Some(SignalLifecycleState::Candidate), SignalLifecycleState::Validated)
        | (Some(SignalLifecycleState::Candidate), SignalLifecycleState::Invalidated)
        | (Some(SignalLifecycleState::Candidate), SignalLifecycleState::Expired)
        | (Some(SignalLifecycleState::Validated), SignalLifecycleState::Live)
        | (Some(SignalLifecycleState::Validated), SignalLifecycleState::Invalidated)
        | (Some(SignalLifecycleState::Validated), SignalLifecycleState::Expired)
        | (Some(SignalLifecycleState::Live), SignalLifecycleState::Decaying)
        | (Some(SignalLifecycleState::Live), SignalLifecycleState::Invalidated)
        | (Some(SignalLifecycleState::Live), SignalLifecycleState::Expired)
        | (Some(SignalLifecycleState::Decaying), SignalLifecycleState::Invalidated)
        | (Some(SignalLifecycleState::Decaying), SignalLifecycleState::Expired)
        | (Some(SignalLifecycleState::Decaying), SignalLifecycleState::Resolved)
        | (Some(SignalLifecycleState::Invalidated), SignalLifecycleState::Resolved)
        | (Some(SignalLifecycleState::Expired), SignalLifecycleState::Resolved) => Ok(()),
        (Some(from), to) => Err(SignalError::InvalidTransition { from, to }),
        (None, to) => Err(SignalError::InvalidTransition {
            from: SignalLifecycleState::Candidate,
            to,
        }),
    }
}

const fn requires_evidence_admission(to: SignalLifecycleState) -> bool {
    match to {
        SignalLifecycleState::Validated | SignalLifecycleState::Live => true,
        SignalLifecycleState::Candidate
        | SignalLifecycleState::Decaying
        | SignalLifecycleState::Invalidated
        | SignalLifecycleState::Expired
        | SignalLifecycleState::Resolved => false,
    }
}

pub fn fold_lifecycle(
    events: &[SignalLifecycleEvent],
) -> Result<SignalLifecycleState, SignalError> {
    if events.is_empty() {
        return Err(SignalError::ContractViolation("empty lifecycle"));
    }
    let mut state: Option<SignalLifecycleState> = None;
    let signal_id = &events[0].signal_id;
    for event in events {
        if event.signal_id != *signal_id {
            return Err(SignalError::ContractViolation("mixed signal ids"));
        }
        if event.previous != state {
            return Err(SignalError::ContractViolation(
                "lifecycle is not an append-only fold",
            ));
        }
        state = Some(event.next);
    }
    state.ok_or(SignalError::ContractViolation("empty fold"))
}

pub fn append_event(
    events: &[SignalLifecycleEvent],
    next: SignalLifecycleEvent,
    signal_type: &SignalType,
    evidence: &EvidenceBundle,
    health: HealthState,
    confirmation_live_ok: bool,
) -> Result<Vec<SignalLifecycleEvent>, SignalError> {
    let from = if events.is_empty() {
        None
    } else {
        Some(fold_lifecycle(events)?)
    };
    transition_allowed(
        from,
        next.next,
        signal_type,
        evidence,
        health,
        confirmation_live_ok,
    )?;
    if next.previous != from {
        return Err(SignalError::ContractViolation(
            "event previous does not match fold",
        ));
    }
    if next.evidence_bundle_hash != evidence.content_hash {
        return Err(SignalError::ContractViolation(
            "lifecycle event hash mismatch",
        ));
    }
    let mut out = events.to_vec();
    out.push(next);
    Ok(out)
}
