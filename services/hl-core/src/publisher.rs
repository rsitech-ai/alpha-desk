use canonical_ledger::StateDelta;
use domain_types::ChainId;

use crate::{health::FeatureHealth, publication::BlockMarkerError};

pub const STATE_ACCOUNT_DELTA_SUBJECT: &str = "hl.v1.state.account_delta";
pub const STATE_BOOK_DELTA_SUBJECT: &str = "hl.v1.state.book_delta";
pub const STATE_DELTA_SCHEMA_V1: &str = "hyperliquid-alpha-desk/state-delta-publication/v1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishDisposition {
    Published,
    Suppressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PublishError {
    #[error("state-delta payload is invalid")]
    Payload,
    #[error("state-delta sink failed")]
    Sink,
}

impl PublishError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::Payload => "core.publish.payload",
            Self::Sink => "core.publish.sink",
        }
    }
}

pub trait StateDeltaSink {
    fn publish(&mut self, subject: &str, payload: &[u8]) -> Result<(), PublishError>;
}

#[derive(Debug, Default)]
pub struct InMemoryDeltaSink {
    published: Vec<(String, Vec<u8>)>,
}

impl InMemoryDeltaSink {
    #[must_use]
    pub fn published(&self) -> &[(String, Vec<u8>)] {
        &self.published
    }
}

impl StateDeltaSink for InMemoryDeltaSink {
    fn publish(&mut self, subject: &str, payload: &[u8]) -> Result<(), PublishError> {
        if payload.is_empty() {
            return Err(PublishError::Payload);
        }
        self.published.push((subject.to_owned(), payload.to_vec()));
        Ok(())
    }
}

pub fn encode_state_delta(
    chain_id: &ChainId,
    delta: &StateDelta,
) -> Result<Vec<u8>, BlockMarkerError> {
    let checkpoint = delta.checkpoint();
    let mut output = Vec::new();
    push_bytes(&mut output, STATE_DELTA_SCHEMA_V1.as_bytes())?;
    push_bytes(&mut output, chain_id.as_str().as_bytes())?;
    output.extend_from_slice(&checkpoint.block_height().get().to_be_bytes());
    output.extend_from_slice(&checkpoint.canonical_block_hash());
    output.extend_from_slice(&delta.before_state_hash());
    output.extend_from_slice(&checkpoint.state_hash());
    output.extend_from_slice(&delta.event_count().to_be_bytes());
    let mutation_count =
        u64::try_from(delta.mutations().len()).map_err(|_| BlockMarkerError::Malformed)?;
    output.extend_from_slice(&mutation_count.to_be_bytes());
    Ok(output)
}

pub fn publish_state_delta<S: StateDeltaSink>(
    sink: &mut S,
    health: &FeatureHealth,
    chain_id: &ChainId,
    delta: &StateDelta,
) -> Result<PublishDisposition, PublishError> {
    if health.state().suppresses_publication() {
        return Ok(PublishDisposition::Suppressed);
    }
    let payload = encode_state_delta(chain_id, delta).map_err(|_| PublishError::Payload)?;
    sink.publish(STATE_ACCOUNT_DELTA_SUBJECT, &payload)?;
    Ok(PublishDisposition::Published)
}

fn push_bytes(output: &mut Vec<u8>, value: &[u8]) -> Result<(), BlockMarkerError> {
    let len = u64::try_from(value.len()).map_err(|_| BlockMarkerError::Malformed)?;
    if value.is_empty() {
        return Err(BlockMarkerError::InvalidIdentity);
    }
    output.extend_from_slice(&len.to_be_bytes());
    output.extend_from_slice(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::health::HealthState;

    #[test]
    fn red_health_is_the_only_publication_gate() {
        assert!(!HealthState::Green.suppresses_publication());
        assert!(!HealthState::Amber.suppresses_publication());
        assert!(HealthState::Red.suppresses_publication());
    }
}
