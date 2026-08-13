use domain_types::UsdAmount;
use serde::{Deserialize, Serialize};

use crate::MarketError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryBin {
    pub lower_bps: i64,
    pub upper_bps: i64,
    pub mass: UsdAmount,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryHistogram {
    pub bins: Vec<EntryBin>,
    pub total_mass: UsdAmount,
}

pub fn entry_histogram(
    entries: &[(i64, UsdAmount)],
    bin_width_bps: i64,
) -> Result<EntryHistogram, MarketError> {
    if bin_width_bps <= 0 {
        return Err(MarketError::Malformed {
            what: "entry_map",
            reason: "bin width must be positive",
        });
    }
    if entries.is_empty() {
        return Err(MarketError::InsufficientHistory { what: "entry_map" });
    }
    let scale = entries[0].1.scale();
    let mut totals = std::collections::BTreeMap::new();
    let mut mass = UsdAmount::from_raw(0, scale)?;
    for (bps, amount) in entries {
        if amount.scale() != scale {
            return Err(MarketError::ScaleMismatch);
        }
        if amount.raw() < 0 {
            return Err(MarketError::Malformed {
                what: "entry_map",
                reason: "mass must be non-negative",
            });
        }
        let index = bps.div_euclid(bin_width_bps);
        let slot = totals.entry(index).or_insert(0_i128);
        *slot = slot
            .checked_add(amount.raw())
            .ok_or(MarketError::Overflow)?;
        mass = mass.checked_add(*amount)?;
    }
    let bins = totals
        .into_iter()
        .map(|(index, raw)| {
            Ok(EntryBin {
                lower_bps: index
                    .checked_mul(bin_width_bps)
                    .ok_or(MarketError::Overflow)?,
                upper_bps: index
                    .checked_mul(bin_width_bps)
                    .and_then(|value| value.checked_add(bin_width_bps))
                    .ok_or(MarketError::Overflow)?,
                mass: UsdAmount::from_raw(raw, scale)?,
            })
        })
        .collect::<Result<Vec<_>, MarketError>>()?;
    let reconstructed = bins
        .iter()
        .try_fold(UsdAmount::from_raw(0, scale)?, |acc, bin| {
            acc.checked_add(bin.mass)
        })?;
    if reconstructed.raw() != mass.raw() {
        return Err(MarketError::Malformed {
            what: "entry_map",
            reason: "histogram mass mismatch",
        });
    }
    Ok(EntryHistogram {
        bins,
        total_mass: mass,
    })
}

pub fn break_even_bps(entry_bps: i64, fees_bps: i64, funding_bps: i64) -> Result<i64, MarketError> {
    entry_bps
        .checked_add(fees_bps)
        .and_then(|value| value.checked_add(funding_bps))
        .ok_or(MarketError::Overflow)
}
