use domain_types::{Decimal, ProbabilityPpm, RoundingMode, UsdAmount};

use crate::MarketError;

pub const PPM_ONE: u32 = 1_000_000;
pub const USD_SCALE: u8 = 8;
pub const RATIO_SCALE: u8 = 8;
pub const COUNT_SCALE: u8 = 6;

pub fn require_ppm(value: u32) -> Result<ProbabilityPpm, MarketError> {
    ProbabilityPpm::from_ppm(value).map_err(Into::into)
}

pub fn product_ppm(factors: &[ProbabilityPpm]) -> Result<ProbabilityPpm, MarketError> {
    if factors.is_empty() {
        return Err(MarketError::Malformed {
            what: "ppm_product",
            reason: "empty factor list",
        });
    }
    let mut acc = 1_u128;
    for (index, factor) in factors.iter().enumerate() {
        acc = acc
            .checked_mul(u128::from(factor.ppm()))
            .ok_or(MarketError::Overflow)?;
        if index > 0 {
            acc /= u128::from(PPM_ONE);
        }
    }
    require_ppm(u32::try_from(acc).map_err(|_| MarketError::Overflow)?)
}

pub fn allocate_ppm(weights: &[u128]) -> Result<Vec<ProbabilityPpm>, MarketError> {
    if weights.is_empty() {
        return Err(MarketError::Malformed {
            what: "ppm_allocate",
            reason: "empty weights",
        });
    }
    let total: u128 = weights.iter().sum();
    if total == 0 {
        return Err(MarketError::Malformed {
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
            .ok_or(MarketError::Overflow)?
            / total;
        let floor = u32::try_from(scaled).map_err(|_| MarketError::Overflow)?;
        let remainder = weight
            .checked_mul(u128::from(PPM_ONE))
            .ok_or(MarketError::Overflow)?
            % total;
        assigned = assigned.checked_add(floor).ok_or(MarketError::Overflow)?;
        floors.push(floor);
        remainders.push((remainder, index));
    }
    remainders.sort_by(|left, right| right.0.cmp(&left.0).then(left.1.cmp(&right.1)));
    let mut leftover = PPM_ONE.checked_sub(assigned).ok_or(MarketError::Overflow)?;
    for (_, index) in remainders {
        if leftover == 0 {
            break;
        }
        floors[index] = floors[index].checked_add(1).ok_or(MarketError::Overflow)?;
        leftover -= 1;
    }
    floors.into_iter().map(require_ppm).collect()
}

pub fn scale_usd_by_ppm(
    amount: UsdAmount,
    weight: ProbabilityPpm,
) -> Result<UsdAmount, MarketError> {
    let raw = weight
        .checked_scale_i128_toward_zero(amount.raw())
        .map_err(MarketError::from)?;
    UsdAmount::from_raw(raw, amount.scale()).map_err(Into::into)
}

pub fn ratio(numerator: Decimal, denominator: Decimal) -> Result<Decimal, MarketError> {
    if denominator.raw() == 0 {
        return Err(MarketError::EmptyDenominator);
    }
    numerator
        .checked_div(denominator, RATIO_SCALE, RoundingMode::TowardZero)
        .map_err(Into::into)
}

pub fn median_i128(values: &mut [i128]) -> Result<i128, MarketError> {
    if values.is_empty() {
        return Err(MarketError::InsufficientHistory { what: "median" });
    }
    values.sort_unstable();
    let mid = values.len() / 2;
    if values.len().is_multiple_of(2) {
        let sum = values[mid - 1]
            .checked_add(values[mid])
            .ok_or(MarketError::Overflow)?;
        Ok(sum / 2)
    } else {
        Ok(values[mid])
    }
}

pub fn mad_i128(values: &[i128], median: i128) -> Result<i128, MarketError> {
    if values.is_empty() {
        return Err(MarketError::InsufficientHistory { what: "mad" });
    }
    let mut deviations = Vec::with_capacity(values.len());
    for value in values {
        let delta = value.checked_sub(median).ok_or(MarketError::Overflow)?;
        let abs = if delta < 0 {
            delta.checked_neg().ok_or(MarketError::Overflow)?
        } else {
            delta
        };
        deviations.push(abs);
    }
    median_i128(&mut deviations)
}

pub fn robust_z_milli(value: i128, sample: &[i128]) -> Result<i64, MarketError> {
    if sample.len() < 3 {
        return Err(MarketError::InsufficientHistory { what: "robust_z" });
    }
    let mut owned = sample.to_vec();
    let median = median_i128(&mut owned)?;
    let mad = mad_i128(sample, median)?;
    if mad == 0 {
        return Err(MarketError::Malformed {
            what: "robust_z",
            reason: "zero median absolute deviation",
        });
    }
    let numerator = value
        .checked_sub(median)
        .and_then(|delta| delta.checked_mul(674_500))
        .ok_or(MarketError::Overflow)?;
    i64::try_from(numerator / mad).map_err(|_| MarketError::Overflow)
}

pub fn require_matching_usd_scale(left: UsdAmount, right: UsdAmount) -> Result<(), MarketError> {
    if left.scale() == right.scale() {
        Ok(())
    } else {
        Err(MarketError::ScaleMismatch)
    }
}
