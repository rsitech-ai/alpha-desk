use domain_types::{AccountId, MarketId, ProbabilityPpm, ProtocolTime};
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
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActionDirection {
    Buy,
    Sell,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionEvent {
    pub account: AccountId,
    pub market: MarketId,
    pub direction: ActionDirection,
    pub protocol_time: ProtocolTime,
    pub size: u64,
    pub market_move_bps: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelationshipEdge {
    pub leader: AccountId,
    pub follower: AccountId,
    pub class: RelationshipClass,
    pub follower_probability: ProbabilityPpm,
    pub sample_size: u32,
    pub median_lag_micros: i64,
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
    let leader = leader_events[0].account.clone();
    let follower = follower_events[0].account.clone();
    let mut similar = 0_u32;
    let mut similar_without_market = 0_u32;
    let mut contrarian = 0_u32;
    let mut lags = Vec::new();
    for leader_event in leader_events {
        if let Some(follow) = follower_events.iter().find(|candidate| {
            candidate.market == leader_event.market
                && candidate.protocol_time > leader_event.protocol_time
                && candidate.protocol_time.unix_micros() - leader_event.protocol_time.unix_micros()
                    <= slow_lag_micros
        }) {
            let lag = follow.protocol_time.unix_micros() - leader_event.protocol_time.unix_micros();
            lags.push(lag);
            if follow.direction == leader_event.direction {
                similar += 1;
                if leader_event.market_move_bps.abs() < 5 {
                    similar_without_market += 1;
                }
            } else {
                contrarian += 1;
            }
        }
    }
    let sample = u32::try_from(leader_events.len()).map_err(|_| GraphError::Overflow)?;
    let follower_probability = ProbabilityPpm::from_ppm(
        u64::from(similar)
            .checked_mul(1_000_000)
            .and_then(|value| value.checked_div(u64::from(sample.max(1))))
            .ok_or(GraphError::Overflow)?
            .try_into()
            .map_err(|_| GraphError::Overflow)?,
    )?;
    let market_controlled = similar > 0 && similar_without_market * 2 < similar;
    let median_lag = median_i64(&lags);
    let class = if market_controlled {
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
    };
    Ok(RelationshipEdge {
        leader,
        follower,
        class,
        follower_probability,
        sample_size: sample,
        median_lag_micros: median_lag,
    })
}

fn median_i64(values: &[i64]) -> i64 {
    if values.is_empty() {
        return 0;
    }
    let mut ordered = values.to_vec();
    ordered.sort_unstable();
    ordered[ordered.len() / 2]
}
