use std::collections::BTreeSet;

use domain_types::{
    AccountId, BasisPoints, LatencyDistribution, MarketId, Price, ProbabilityPpm, ProtocolTime,
};
use serde::{Deserialize, Serialize};

use crate::GraphError;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum RelationshipClass {
    Originator,
    IndependentConfirmer,
    FastFollower,
    SlowFollower,
    CopyBot,
    ContrarianResponder,
    NoStableRelation,
}

impl RelationshipClass {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Originator => "originator",
            Self::IndependentConfirmer => "independent_confirmer",
            Self::FastFollower => "fast_follower",
            Self::SlowFollower => "slow_follower",
            Self::CopyBot => "copy_bot",
            Self::ContrarianResponder => "contrarian_responder",
            Self::NoStableRelation => "no_stable_relation",
        }
    }

    /// Leader-side class implied by a follower-side classification.
    ///
    /// `classify_pair` never emits `Originator` on the follower side. That
    /// variant is retained so a later caller-supplied class stays exhaustive.
    #[must_use]
    pub const fn leader_class(self) -> Self {
        match self {
            Self::FastFollower | Self::SlowFollower | Self::CopyBot | Self::ContrarianResponder => {
                Self::Originator
            }
            Self::IndependentConfirmer => Self::IndependentConfirmer,
            Self::NoStableRelation => Self::NoStableRelation,
            Self::Originator => Self::Originator,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionDirection {
    Buy,
    Sell,
}

impl ActionDirection {
    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::Buy => "buy",
            Self::Sell => "sell",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionEvent {
    pub account: AccountId,
    pub market: MarketId,
    pub direction: ActionDirection,
    pub protocol_time: ProtocolTime,
    pub size: u64,
    pub market_move_bps: i64,
    /// Observed executable or fill price. Absent prices withhold degradation.
    pub entry_price: Option<Price>,
    /// Observed forward markout after the action. Absent values withhold edge decay
    /// and independent predictive-value claims.
    pub forward_markout_bps: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeRelationship {
    pub median_leader_size: u64,
    pub median_follower_size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipEdge {
    pub leader: AccountId,
    pub follower: AccountId,
    pub class: RelationshipClass,
    pub leader_class: RelationshipClass,
    pub follower_probability: ProbabilityPpm,
    pub sample_size: u32,
    pub median_lag_micros: i64,
    pub lag_distribution: Option<LatencyDistribution>,
    pub size_relationship: Option<SizeRelationship>,
    pub market_overlap_ppm: ProbabilityPpm,
    pub entry_degradation_bps: Option<BasisPoints>,
    pub follower_adds_independent_value: Option<bool>,
    pub edge_decay_bps: Option<BasisPoints>,
}

struct PairMatch<'a> {
    leader: &'a ActionEvent,
    follower: &'a ActionEvent,
    lag: i64,
    similar: bool,
    similar_without_market: bool,
    contrarian: bool,
}

pub fn classify_pair(
    leader_events: &[ActionEvent],
    follower_events: &[ActionEvent],
    fast_lag_micros: i64,
    slow_lag_micros: i64,
) -> Result<RelationshipEdge, GraphError> {
    if leader_events.is_empty() || follower_events.is_empty() {
        return Err(GraphError::Malformed {
            what: "leader_follower",
            reason: "empty event history",
        });
    }
    if fast_lag_micros <= 0 || slow_lag_micros <= fast_lag_micros {
        return Err(GraphError::Malformed {
            what: "leader_follower",
            reason: "lag bounds invalid",
        });
    }
    let leader = require_single_account(leader_events, "mixed leader accounts")?;
    let follower = require_single_account(follower_events, "mixed follower accounts")?;
    let matches = collect_matches(leader_events, follower_events, slow_lag_micros)?;
    let sample = u32::try_from(leader_events.len()).map_err(|_| GraphError::Overflow)?;
    let similar = u32::try_from(matches.iter().filter(|item| item.similar).count())
        .map_err(|_| GraphError::Overflow)?;
    let similar_without_market = u32::try_from(
        matches
            .iter()
            .filter(|item| item.similar_without_market)
            .count(),
    )
    .map_err(|_| GraphError::Overflow)?;
    let contrarian = u32::try_from(matches.iter().filter(|item| item.contrarian).count())
        .map_err(|_| GraphError::Overflow)?;
    let follower_probability = ProbabilityPpm::from_ppm(
        u64::from(similar)
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_div(u64::from(sample.max(1))))
            .ok_or(GraphError::Overflow)?
            .try_into()
            .map_err(|_| GraphError::Overflow)?,
    )?;
    let market_controlled = similar > 0 && similar_without_market * 2 < similar;
    let lags: Vec<i64> = matches.iter().map(|item| item.lag).collect();
    let median_lag = median_i64(&lags);
    let class = follower_class(
        market_controlled,
        similar,
        contrarian,
        sample,
        follower_probability,
        median_lag,
        fast_lag_micros,
    );
    Ok(RelationshipEdge {
        leader,
        follower,
        class,
        leader_class: class.leader_class(),
        follower_probability,
        sample_size: sample,
        median_lag_micros: median_lag,
        lag_distribution: lag_distribution(&lags)?,
        size_relationship: size_relationship(&matches),
        market_overlap_ppm: market_overlap_ppm(leader_events, follower_events)?,
        entry_degradation_bps: entry_degradation_bps(&matches)?,
        follower_adds_independent_value: independent_predictive_value(market_controlled, &matches)?,
        edge_decay_bps: edge_decay_bps(&matches)?,
    })
}

fn require_single_account(
    events: &[ActionEvent],
    reason: &'static str,
) -> Result<AccountId, GraphError> {
    let first = events[0].account.clone();
    if events.iter().any(|event| event.account != first) {
        return Err(GraphError::Malformed {
            what: "leader_follower",
            reason,
        });
    }
    Ok(first)
}

fn collect_matches<'a>(
    leader_events: &'a [ActionEvent],
    follower_events: &'a [ActionEvent],
    slow_lag_micros: i64,
) -> Result<Vec<PairMatch<'a>>, GraphError> {
    let mut matches = Vec::new();
    for leader_event in leader_events {
        if let Some(follow) = follower_events.iter().find(|candidate| {
            candidate.market == leader_event.market
                && candidate.protocol_time > leader_event.protocol_time
                && candidate.protocol_time.unix_micros() - leader_event.protocol_time.unix_micros()
                    <= slow_lag_micros
        }) {
            let lag = follow.protocol_time.unix_micros() - leader_event.protocol_time.unix_micros();
            if lag <= 0 {
                return Err(GraphError::Malformed {
                    what: "leader_follower",
                    reason: "non-positive lag",
                });
            }
            let similar = follow.direction == leader_event.direction;
            matches.push(PairMatch {
                leader: leader_event,
                follower: follow,
                lag,
                similar,
                similar_without_market: similar && leader_event.market_move_bps.abs() < 5,
                contrarian: !similar,
            });
        }
    }
    Ok(matches)
}

fn follower_class(
    market_controlled: bool,
    similar: u32,
    contrarian: u32,
    sample: u32,
    follower_probability: ProbabilityPpm,
    median_lag: i64,
    fast_lag_micros: i64,
) -> RelationshipClass {
    if market_controlled {
        RelationshipClass::IndependentConfirmer
    } else if similar == 0 && contrarian * 2 >= sample {
        RelationshipClass::ContrarianResponder
    } else if follower_probability.ppm() >= 800_000 && median_lag <= fast_lag_micros / 4 {
        RelationshipClass::CopyBot
    } else if follower_probability.ppm() >= 600_000 && median_lag <= fast_lag_micros {
        RelationshipClass::FastFollower
    } else if follower_probability.ppm() >= 600_000 {
        RelationshipClass::SlowFollower
    } else if follower_probability.ppm() >= 400_000 {
        RelationshipClass::IndependentConfirmer
    } else {
        RelationshipClass::NoStableRelation
    }
}

fn size_relationship(matches: &[PairMatch<'_>]) -> Option<SizeRelationship> {
    if matches.is_empty() {
        return None;
    }
    let leader_sizes: Vec<u64> = matches.iter().map(|item| item.leader.size).collect();
    let follower_sizes: Vec<u64> = matches.iter().map(|item| item.follower.size).collect();
    Some(SizeRelationship {
        median_leader_size: median_u64(&leader_sizes),
        median_follower_size: median_u64(&follower_sizes),
    })
}

fn market_overlap_ppm(
    leader_events: &[ActionEvent],
    follower_events: &[ActionEvent],
) -> Result<ProbabilityPpm, GraphError> {
    let leaders = unique_markets(leader_events);
    let followers = unique_markets(follower_events);
    let intersection = leaders.intersection(&followers).count();
    let mut union = leaders.clone();
    union.extend(followers);
    if union.is_empty() {
        return Err(GraphError::Malformed {
            what: "leader_follower",
            reason: "empty market set",
        });
    }
    let ppm = u128::try_from(intersection)
        .ok()
        .and_then(|count| count.checked_mul(1_000_000))
        .and_then(|value| value.checked_div(u128::try_from(union.len()).ok()?))
        .ok_or(GraphError::Overflow)?;
    ProbabilityPpm::from_ppm(u32::try_from(ppm).map_err(|_| GraphError::Overflow)?)
        .map_err(Into::into)
}

fn unique_markets(events: &[ActionEvent]) -> BTreeSet<&MarketId> {
    events.iter().map(|event| &event.market).collect()
}

fn entry_degradation_bps(matches: &[PairMatch<'_>]) -> Result<Option<BasisPoints>, GraphError> {
    let mut degradations = Vec::new();
    for item in matches {
        if item.leader.direction != item.follower.direction {
            continue;
        }
        let Some(leader_price) = item.leader.entry_price else {
            continue;
        };
        let Some(follower_price) = item.follower.entry_price else {
            continue;
        };
        if leader_price.raw() == 0 {
            return Err(GraphError::Malformed {
                what: "leader_follower",
                reason: "zero leader entry price",
            });
        }
        if leader_price.scale() != follower_price.scale() {
            return Err(GraphError::Malformed {
                what: "leader_follower",
                reason: "entry price scale mismatch",
            });
        }
        let signed_delta = follower_price
            .raw()
            .checked_sub(leader_price.raw())
            .ok_or(GraphError::Overflow)?
            .checked_mul(degradation_sign(item.leader.direction))
            .ok_or(GraphError::Overflow)?;
        let bps = signed_delta
            .checked_mul(10_000)
            .and_then(|value| value.checked_div(leader_price.raw()))
            .ok_or(GraphError::Overflow)?;
        degradations.push(bps);
    }
    if degradations.is_empty() {
        return Ok(None);
    }
    Ok(Some(BasisPoints::from_raw(median_i128(&degradations), 0)?))
}

const fn degradation_sign(direction: ActionDirection) -> i128 {
    match direction {
        ActionDirection::Buy => 1,
        ActionDirection::Sell => -1,
    }
}

fn independent_predictive_value(
    market_controlled: bool,
    matches: &[PairMatch<'_>],
) -> Result<Option<bool>, GraphError> {
    if market_controlled {
        return Ok(Some(false));
    }
    let mut observed = Vec::new();
    for item in matches {
        if !item.similar_without_market {
            continue;
        }
        if let Some(markout) = item.follower.forward_markout_bps {
            observed.push(markout);
        }
    }
    if observed.is_empty() {
        return Ok(None);
    }
    Ok(Some(median_i64(&observed) > 0))
}

fn edge_decay_bps(matches: &[PairMatch<'_>]) -> Result<Option<BasisPoints>, GraphError> {
    let mut decays = Vec::new();
    for item in matches {
        let Some(leader_markout) = item.leader.forward_markout_bps else {
            continue;
        };
        let Some(follower_markout) = item.follower.forward_markout_bps else {
            continue;
        };
        let decay = leader_markout
            .checked_sub(follower_markout)
            .ok_or(GraphError::Overflow)?;
        decays.push(i128::from(decay));
    }
    if decays.is_empty() {
        return Ok(None);
    }
    Ok(Some(BasisPoints::from_raw(median_i128(&decays), 0)?))
}

fn lag_distribution(lags: &[i64]) -> Result<Option<LatencyDistribution>, GraphError> {
    if lags.is_empty() {
        return Ok(None);
    }
    let mut micros = Vec::with_capacity(lags.len());
    for lag in lags {
        micros.push(u64::try_from(*lag).map_err(|_| GraphError::Malformed {
            what: "leader_follower",
            reason: "negative lag",
        })?);
    }
    micros.sort_unstable();
    Ok(Some(LatencyDistribution::new(
        percentile_u64(&micros, 10),
        percentile_u64(&micros, 50),
        percentile_u64(&micros, 90),
        percentile_u64(&micros, 99),
    )?))
}

fn percentile_u64(sorted: &[u64], pct: u32) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (sorted.len() - 1)
        .saturating_mul(pct as usize)
        .saturating_div(100);
    sorted[rank]
}

fn median_i64(values: &[i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    ordered[ordered.len() / 2]
}

fn median_u64(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    ordered[ordered.len() / 2]
}

fn median_i128(values: &[i128]) -> i128 {
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    ordered[ordered.len() / 2]
}
