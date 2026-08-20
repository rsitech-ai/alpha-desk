use domain_types::{KnownTime, ProbabilityPpm, ProtocolTime};

use crate::{IntelligenceError, math::freshness_ppm, skill::SkillPrior};

pub fn current_freshness(
    last_observation_at: ProtocolTime,
    known_at: KnownTime,
    prior: &SkillPrior,
) -> Result<ProbabilityPpm, IntelligenceError> {
    let age = known_at
        .unix_micros()
        .checked_sub(last_observation_at.unix_micros())
        .ok_or(IntelligenceError::Malformed {
            what: "freshness",
            reason: "known_at precedes last observation",
        })?;
    freshness_ppm(
        u64::try_from(age).map_err(|_| IntelligenceError::Overflow)?,
        prior.half_life_micros,
    )
}
