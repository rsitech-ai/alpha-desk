use std::collections::BTreeSet;

use domain_types::ProbabilityPpm;

use crate::{
    MarketError,
    memory::{
        AnalogueMatch, AnalogueSet, MemoryEntry, MemoryQuery, MemorySupport, VectorManifest,
        contributing_dimensions, squared_distance,
    },
};

pub trait VectorIndex {
    fn insert(&mut self, entry: MemoryEntry) -> Result<(), MarketError>;
    fn query(&self, query: &MemoryQuery) -> Result<AnalogueSet, MarketError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExactVectorIndex {
    manifest: VectorManifest,
    entries: Vec<MemoryEntry>,
}

impl ExactVectorIndex {
    #[must_use]
    pub fn new(manifest: VectorManifest) -> Self {
        Self {
            manifest,
            entries: Vec::new(),
        }
    }
}

impl VectorIndex for ExactVectorIndex {
    fn insert(&mut self, entry: MemoryEntry) -> Result<(), MarketError> {
        self.entries.push(entry);
        Ok(())
    }

    fn query(&self, query: &MemoryQuery) -> Result<AnalogueSet, MarketError> {
        if query.limit == 0 {
            return Err(MarketError::Malformed {
                what: "memory_query",
                reason: "limit must be positive",
            });
        }
        if query.known_at.unix_micros() < query.effective_at.unix_micros() {
            return Err(MarketError::Malformed {
                what: "memory_query",
                reason: "known_at precedes effective_at",
            });
        }
        let cutoff = query
            .effective_at
            .unix_micros()
            .checked_sub(
                i64::try_from(query.horizon.as_micros()).map_err(|_| MarketError::Overflow)?,
            )
            .ok_or(MarketError::Overflow)?;
        let mut ranked = Vec::new();
        for entry in &self.entries {
            if entry.episode_id == query.episode_id {
                continue;
            }
            if entry.known_at > query.known_at {
                continue;
            }
            if entry.effective_at.unix_micros() >= cutoff {
                continue;
            }
            let distance = squared_distance(&query.values_milli, &entry.values_milli)?;
            ranked.push((distance, entry));
        }
        ranked.sort_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then(left.1.episode_id.cmp(&right.1.episode_id))
                .then(left.1.effective_at.cmp(&right.1.effective_at))
        });
        let mut seen_episodes = BTreeSet::new();
        let mut matches = Vec::new();
        for (distance, entry) in ranked {
            if !seen_episodes.insert(entry.episode_id.clone()) {
                continue;
            }
            matches.push(AnalogueMatch {
                episode_id: entry.episode_id.clone(),
                distance_milli: distance,
                contributing_dimensions: contributing_dimensions(
                    &query.values_milli,
                    &entry.values_milli,
                )?,
                outcome_bps: entry.outcome_bps,
            });
            if matches.len() == query.limit {
                break;
            }
        }
        let independent_episode_count =
            u32::try_from(seen_episodes.len()).map_err(|_| MarketError::Overflow)?;
        let nearest = matches.first().map(|item| item.distance_milli);
        let support = match nearest {
            Some(distance) if distance <= query.support_distance_milli => MemorySupport::InSupport,
            Some(_) => MemorySupport::OutsideSupport {
                reason: "nearest analogue exceeds support radius".to_owned(),
            },
            None => MemorySupport::OutsideSupport {
                reason: "no eligible historical episode".to_owned(),
            },
        };
        let historical_support = match support {
            MemorySupport::InSupport => ProbabilityPpm::from_ppm(800_000)?,
            MemorySupport::OutsideSupport { .. } => ProbabilityPpm::ZERO,
        };
        Ok(AnalogueSet {
            matches,
            independent_episode_count,
            support,
            historical_support,
            manifest_hash: self.manifest.manifest_hash,
        })
    }
}
