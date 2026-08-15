use domain_types::{Direction, ProtocolTime, UsdAmount};

use crate::clock::add_protocol;
use crate::error::SimError;
use crate::fees::FundingSchedule;
use crate::math::{apply_funding, zero_usd};

pub fn funding_over_hold(
    schedule: &FundingSchedule,
    direction: Direction,
    entry_notional: UsdAmount,
    opened_at: ProtocolTime,
    closed_at: ProtocolTime,
) -> Result<UsdAmount, SimError> {
    if closed_at < opened_at {
        return Err(SimError::InvalidRequest {
            field: "funding.hold_interval",
        });
    }
    let hold = u64::try_from(closed_at.unix_micros() - opened_at.unix_micros())
        .map_err(|_| SimError::InvalidAmount)?;
    if hold > 0 && schedule.interval_micros() == 0 {
        return Err(SimError::UnmodeledFunding);
    }
    let mut cursor = opened_at;
    let mut total = zero_usd()?;
    while add_protocol(cursor, schedule.interval_micros())? <= closed_at {
        cursor = add_protocol(cursor, schedule.interval_micros())?;
        let payment = apply_funding(entry_notional, schedule.rate())?;
        total = match direction {
            Direction::Long => total.checked_add(payment)?,
            Direction::Short => {
                if payment.raw() == 0 {
                    total
                } else {
                    // Positive funding is paid by longs and received by shorts.
                    total.checked_sub(payment)?
                }
            }
            Direction::Flat => {
                return Err(SimError::InvalidRequest {
                    field: "funding.flat_position",
                });
            }
        };
    }
    Ok(total)
}
