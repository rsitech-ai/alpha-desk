use domain_types::{ProbabilityPpm, ValueError};

use crate::IntelligenceError;

pub const PPM_ONE: u32 = 1_000_000;

pub fn allocate_ppm(weights: &[u128]) -> Result<Vec<ProbabilityPpm>, IntelligenceError> {
    if weights.is_empty() {
        return Err(IntelligenceError::Malformed {
            what: "ppm_allocate",
            reason: "empty weights",
        });
    }
    let total: u128 = weights.iter().sum();
    if total == 0 {
        return Err(IntelligenceError::Malformed {
            what: "ppm_allocate",
            reason: "zero weight total",
        });
    }
    let mut floors = Vec::with_capacity(weights.len());
    let mut remainders = Vec::with_capacity(weights.len());
    let mut assigned = 0_u32;
    for (index, weight) in weights.iter().enumerate() {
        let scaled = weight
            .checked_mul(u128::from(PPM_ONE))
            .ok_or(IntelligenceError::Overflow)?
            / total;
        let floor = u32::try_from(scaled).map_err(|_| IntelligenceError::Overflow)?;
        let remainder = weight
            .checked_mul(u128::from(PPM_ONE))
            .ok_or(IntelligenceError::Overflow)?
            % total;
        assigned = assigned
            .checked_add(floor)
            .ok_or(IntelligenceError::Overflow)?;
        floors.push(floor);
        remainders.push((remainder, index));
    }
    remainders.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    let mut leftover = PPM_ONE
        .checked_sub(assigned)
        .ok_or(IntelligenceError::Overflow)?;
    for (_, index) in remainders {
        if leftover == 0 {
            break;
        }
        floors[index] = floors[index]
            .checked_add(1)
            .ok_or(IntelligenceError::Overflow)?;
        leftover -= 1;
    }
    floors
        .into_iter()
        .map(|ppm| ProbabilityPpm::from_ppm(ppm).map_err(IntelligenceError::from))
        .collect()
}

#[must_use]
pub fn integer_sqrt(value: u128) -> u128 {
    if value < 2 {
        return value;
    }
    let mut x0 = value;
    let mut x1 = value.div_ceil(2);
    while x1 < x0 {
        x0 = x1;
        x1 = (x0 + value / x0) / 2;
    }
    x0
}

pub fn freshness_ppm(
    age_micros: u64,
    half_life_micros: u64,
) -> Result<ProbabilityPpm, IntelligenceError> {
    if half_life_micros == 0 {
        return Err(IntelligenceError::Malformed {
            what: "freshness",
            reason: "half-life must be positive",
        });
    }
    let whole = age_micros / half_life_micros;
    if whole >= 20 {
        return ProbabilityPpm::from_ppm(0).map_err(Into::into);
    }
    let mut value = PPM_ONE;
    for _ in 0..whole {
        value /= 2;
    }
    let frac = age_micros % half_life_micros;
    let reduction = u64::from(value)
        .checked_mul(frac)
        .and_then(|product| product.checked_div(2 * half_life_micros))
        .ok_or(IntelligenceError::Overflow)?;
    let ppm = value
        .checked_sub(u32::try_from(reduction).map_err(|_| IntelligenceError::Overflow)?)
        .ok_or(IntelligenceError::Overflow)?;
    ProbabilityPpm::from_ppm(ppm).map_err(Into::into)
}

/// Logistic Φ-style map from a signed milli-z score into ppm.
///
/// Uses an integer logistic with slope 1_700/1_000 so outputs are monotone and
/// fail closed on overflow rather than using binary floating-point accounting.
pub fn logistic_ppm(z_milli: i64) -> Result<ProbabilityPpm, IntelligenceError> {
    let scaled = z_milli
        .checked_mul(1_700)
        .ok_or(IntelligenceError::Overflow)?;
    let exp_neg = integer_exp_neg_milli(scaled)?;
    let denominator = 1_000_000_u128
        .checked_add(u128::from(exp_neg))
        .ok_or(IntelligenceError::Overflow)?;
    let ppm = 1_000_000_u128
        .checked_mul(1_000_000)
        .and_then(|numerator| numerator.checked_div(denominator))
        .ok_or(IntelligenceError::Overflow)?;
    ProbabilityPpm::from_ppm(u32::try_from(ppm).map_err(|_| IntelligenceError::Overflow)?)
        .map_err(Into::into)
}

fn integer_exp_neg_milli(milli: i64) -> Result<u32, IntelligenceError> {
    if milli >= 20_000 {
        return Ok(0);
    }
    if milli <= -20_000 {
        return Ok(u32::MAX);
    }
    let abs = milli.unsigned_abs();
    let x_num = u128::from(abs);
    let mut term = 1_000_000_u128;
    let mut sum = 1_000_000_u128;
    for k in 1..=8_u32 {
        term = term
            .checked_mul(x_num)
            .and_then(|value| value.checked_div(1_000 * u128::from(k)))
            .ok_or(IntelligenceError::Overflow)?;
        sum = sum.checked_add(term).ok_or(IntelligenceError::Overflow)?;
    }
    if milli >= 0 {
        let ppm = 1_000_000_u128
            .checked_mul(1_000_000)
            .and_then(|numerator| numerator.checked_div(sum.max(1)))
            .ok_or(IntelligenceError::Overflow)?;
        u32::try_from(ppm.min(u128::from(u32::MAX))).map_err(|_| IntelligenceError::Overflow)
    } else {
        u32::try_from(sum.min(u128::from(u32::MAX))).map_err(|_| IntelligenceError::Overflow)
    }
}

pub fn require_ppm(value: u32) -> Result<ProbabilityPpm, IntelligenceError> {
    ProbabilityPpm::from_ppm(value).map_err(|error: ValueError| error.into())
}
