use crate::{SignalError, signal::Signal};

pub fn canonical_utility(signal: &Signal) -> Result<i64, SignalError> {
    let net = signal.net_edge_bps.raw();
    let confidence = i128::from(signal.confidence.ppm());
    let freshness = match signal.data_health.state {
        feature_core::HealthState::Green => 1_000_000_i128,
        feature_core::HealthState::Amber => 500_000,
        feature_core::HealthState::Red => 0,
    };
    let capacity_fit = if signal.capacity.raw() <= 0 {
        0
    } else {
        1_000_000
    };
    let crowding_penalty = i128::from(signal.crowding.ppm());
    let tail_penalty = signal.tail_risk_bps.raw().abs() * 1_000;
    let uncertainty = match signal.confirmation_class {
        crate::signal::SignalConfirmationClass::CommittedPrimary => 0,
        crate::signal::SignalConfirmationClass::CommittedIndependent => 50_000,
        crate::signal::SignalConfirmationClass::ProvisionalSource => 250_000,
        crate::signal::SignalConfirmationClass::SyntheticUnqualified => 400_000,
    };
    let weighted = net
        .checked_mul(confidence)
        .and_then(|value| value.checked_mul(capacity_fit))
        .and_then(|value| value.checked_div(1_000_000))
        .and_then(|value| value.checked_mul(freshness))
        .and_then(|value| value.checked_div(1_000_000))
        .ok_or(SignalError::Overflow)?;
    let score = weighted
        .checked_sub(crowding_penalty)
        .and_then(|value| value.checked_sub(tail_penalty))
        .and_then(|value| value.checked_sub(uncertainty))
        .ok_or(SignalError::Overflow)?;
    i64::try_from(score).map_err(|_| SignalError::Overflow)
}
