use crate::IntelligenceError;

/// Block-bootstrap-style effective sample size using lag-1 autocorrelation.
///
/// `n_eff = n * (1 - rho) / (1 + rho)` in milli-observations. A constant
/// series is treated as one independent observation rather than as infinite
/// evidence.
pub fn effective_sample_size_milli(values: &[i64]) -> Result<u64, IntelligenceError> {
    let n = u64::try_from(values.len()).map_err(|_| IntelligenceError::Overflow)?;
    if n == 0 {
        return Err(IntelligenceError::InsufficientHistory {
            what: "effective_sample_size",
        });
    }
    if n == 1 {
        return Ok(1_000);
    }
    let mean = values
        .iter()
        .try_fold(0_i128, |acc, value| acc.checked_add(i128::from(*value)))
        .ok_or(IntelligenceError::Overflow)?
        / i128::from(n);
    let mut var = 0_i128;
    let mut lag = 0_i128;
    for value in values {
        let delta = i128::from(*value)
            .checked_sub(mean)
            .ok_or(IntelligenceError::Overflow)?;
        var = var
            .checked_add(
                delta
                    .checked_mul(delta)
                    .ok_or(IntelligenceError::Overflow)?,
            )
            .ok_or(IntelligenceError::Overflow)?;
    }
    for window in values.windows(2) {
        let left = i128::from(window[0])
            .checked_sub(mean)
            .ok_or(IntelligenceError::Overflow)?;
        let right = i128::from(window[1])
            .checked_sub(mean)
            .ok_or(IntelligenceError::Overflow)?;
        lag = lag
            .checked_add(left.checked_mul(right).ok_or(IntelligenceError::Overflow)?)
            .ok_or(IntelligenceError::Overflow)?;
    }
    if var == 0 {
        return Ok(n.saturating_mul(1_000));
    }
    let rho_ppm = lag
        .checked_mul(1_000_000)
        .and_then(|value| value.checked_div(var))
        .ok_or(IntelligenceError::Overflow)?;
    let rho_ppm = rho_ppm.clamp(0, 990_000);
    let numer = 1_000_000_i128
        .checked_sub(rho_ppm)
        .ok_or(IntelligenceError::Overflow)?;
    let denom = 1_000_000_i128
        .checked_add(rho_ppm)
        .ok_or(IntelligenceError::Overflow)?;
    let ess = i128::from(n)
        .checked_mul(1_000)
        .and_then(|value| value.checked_mul(numer))
        .and_then(|value| value.checked_div(denom))
        .ok_or(IntelligenceError::Overflow)?;
    let ess = u64::try_from(ess.max(1_000)).map_err(|_| IntelligenceError::Overflow)?;
    Ok(ess.min(n.saturating_mul(1_000)))
}
