use domain_types::{
    BlockHeight, CohortId, EntityId, KnownTime, Leverage, ProbabilityPpm, ProtocolTime, RegimeId,
};
use serde::{Deserialize, Serialize};
use wallet_intelligence::{IntentClass, StyleClass};

use crate::{MarketError, hash::digest, hash::require_non_empty};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CohortPredicate {
    SkillProbabilityAtLeast(ProbabilityPpm),
    StyleProbabilityAtLeast {
        style: StyleClass,
        value: ProbabilityPpm,
    },
    IntentProbabilityAtLeast {
        intent: IntentClass,
        value: ProbabilityPpm,
    },
    EquityPercentileAtLeast(ProbabilityPpm),
    LeverageAtLeast(Leverage),
    BehaviorRegime(RegimeId),
    And(Vec<CohortPredicate>),
    Or(Vec<CohortPredicate>),
    Not(Box<CohortPredicate>),
}

impl CohortPredicate {
    pub fn matches(&self, member: &CohortMember) -> Result<bool, MarketError> {
        match self {
            Self::SkillProbabilityAtLeast(threshold) => match member.skill_probability {
                Some(value) => Ok(value.ppm() >= threshold.ppm()),
                None => Err(MarketError::MissingInput {
                    name: "skill_probability",
                }),
            },
            Self::StyleProbabilityAtLeast { style, value } => match member.style {
                Some((observed, probability)) => {
                    Ok(observed == *style && probability.ppm() >= value.ppm())
                }
                None => Err(MarketError::MissingInput { name: "style" }),
            },
            Self::IntentProbabilityAtLeast { intent, value } => match member.intent {
                Some((observed, probability)) => {
                    Ok(observed == *intent && probability.ppm() >= value.ppm())
                }
                None => Err(MarketError::MissingInput { name: "intent" }),
            },
            Self::EquityPercentileAtLeast(threshold) => match member.equity_percentile {
                Some(value) => Ok(value.ppm() >= threshold.ppm()),
                None => Err(MarketError::MissingInput {
                    name: "equity_percentile",
                }),
            },
            Self::LeverageAtLeast(threshold) => match member.leverage {
                Some(value) => Ok(value >= *threshold),
                None => Err(MarketError::MissingInput { name: "leverage" }),
            },
            Self::BehaviorRegime(regime) => match &member.regime {
                Some(value) => Ok(value == regime),
                None => Err(MarketError::MissingInput { name: "regime" }),
            },
            Self::And(parts) => {
                if parts.is_empty() {
                    return Err(MarketError::Malformed {
                        what: "cohort_predicate",
                        reason: "empty And",
                    });
                }
                for part in parts {
                    if !part.matches(member)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            Self::Or(parts) => {
                if parts.is_empty() {
                    return Err(MarketError::Malformed {
                        what: "cohort_predicate",
                        reason: "empty Or",
                    });
                }
                let mut missing = false;
                for part in parts {
                    match part.matches(member) {
                        Ok(true) => return Ok(true),
                        Ok(false) => {}
                        Err(MarketError::MissingInput { .. }) => missing = true,
                        Err(error) => return Err(error),
                    }
                }
                if missing {
                    Err(MarketError::MissingInput {
                        name: "cohort_predicate",
                    })
                } else {
                    Ok(false)
                }
            }
            Self::Not(inner) => Ok(!inner.matches(member)?),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CohortDefinition {
    pub cohort_id: CohortId,
    pub version: u32,
    pub predicate: CohortPredicate,
    pub exclusions: Vec<String>,
    pub definition_hash: [u8; 32],
}

impl CohortDefinition {
    pub fn try_new(
        cohort_id: CohortId,
        version: u32,
        predicate: CohortPredicate,
        exclusions: Vec<String>,
    ) -> Result<Self, MarketError> {
        if version == 0 {
            return Err(MarketError::Malformed {
                what: "cohort_definition",
                reason: "version must be >= 1",
            });
        }
        for exclusion in &exclusions {
            require_non_empty(exclusion, "exclusions")?;
        }
        let mut definition = Self {
            cohort_id,
            version,
            predicate,
            exclusions,
            definition_hash: [0_u8; 32],
        };
        definition.definition_hash = definition.compute_hash();
        Ok(definition)
    }

    #[must_use]
    pub fn compute_hash(&self) -> [u8; 32] {
        digest(&[
            self.cohort_id.as_str().as_bytes(),
            &self.version.to_le_bytes(),
            predicate_bytes(&self.predicate).as_slice(),
            self.exclusions.join("\0").as_bytes(),
        ])
    }
}

fn predicate_bytes(predicate: &CohortPredicate) -> Vec<u8> {
    match predicate {
        CohortPredicate::SkillProbabilityAtLeast(value) => {
            let mut out = vec![0];
            out.extend_from_slice(&value.ppm().to_le_bytes());
            out
        }
        CohortPredicate::StyleProbabilityAtLeast { style, value } => {
            let mut out = vec![1];
            out.extend_from_slice(style.as_wire_name().as_bytes());
            out.push(0);
            out.extend_from_slice(&value.ppm().to_le_bytes());
            out
        }
        CohortPredicate::IntentProbabilityAtLeast { intent, value } => {
            let mut out = vec![2];
            out.extend_from_slice(intent.as_wire_name().as_bytes());
            out.push(0);
            out.extend_from_slice(&value.ppm().to_le_bytes());
            out
        }
        CohortPredicate::EquityPercentileAtLeast(value) => {
            let mut out = vec![3];
            out.extend_from_slice(&value.ppm().to_le_bytes());
            out
        }
        CohortPredicate::LeverageAtLeast(value) => {
            let mut out = vec![4];
            out.extend_from_slice(&value.raw().to_le_bytes());
            out.push(value.scale());
            out
        }
        CohortPredicate::BehaviorRegime(regime) => {
            let mut out = vec![5];
            out.extend_from_slice(regime.as_str().as_bytes());
            out
        }
        CohortPredicate::And(parts) => {
            let mut out = vec![6];
            for part in parts {
                let nested = predicate_bytes(part);
                out.extend_from_slice(
                    &(u32::try_from(nested.len()).unwrap_or(u32::MAX)).to_le_bytes(),
                );
                out.extend_from_slice(&nested);
            }
            out
        }
        CohortPredicate::Or(parts) => {
            let mut out = vec![7];
            for part in parts {
                let nested = predicate_bytes(part);
                out.extend_from_slice(
                    &(u32::try_from(nested.len()).unwrap_or(u32::MAX)).to_le_bytes(),
                );
                out.extend_from_slice(&nested);
            }
            out
        }
        CohortPredicate::Not(inner) => {
            let mut out = vec![8];
            out.extend(predicate_bytes(inner));
            out
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CohortMember {
    pub entity_id: EntityId,
    pub independence_weight: ProbabilityPpm,
    pub skill_probability: Option<ProbabilityPpm>,
    pub style: Option<(StyleClass, ProbabilityPpm)>,
    pub intent: Option<(IntentClass, ProbabilityPpm)>,
    pub equity_percentile: Option<ProbabilityPpm>,
    pub leverage: Option<Leverage>,
    pub regime: Option<RegimeId>,
    pub known_at: KnownTime,
    pub effective_at: ProtocolTime,
}

impl CohortMember {
    pub fn visible_at(&self, effective_at: ProtocolTime, known_at: KnownTime) -> bool {
        self.effective_at <= effective_at && self.known_at <= known_at
    }
}

pub fn select_members<'a>(
    definition: &CohortDefinition,
    members: &'a [CohortMember],
    effective_at: ProtocolTime,
    known_at: KnownTime,
    as_of_block: BlockHeight,
) -> Result<Vec<&'a CohortMember>, MarketError> {
    let _ = as_of_block;
    let mut selected = Vec::new();
    for member in members {
        if !member.visible_at(effective_at, known_at) {
            continue;
        }
        if definition
            .exclusions
            .iter()
            .any(|exclusion| exclusion == member.entity_id.as_str())
        {
            continue;
        }
        if definition.predicate.matches(member)? {
            selected.push(member);
        }
    }
    Ok(selected)
}
