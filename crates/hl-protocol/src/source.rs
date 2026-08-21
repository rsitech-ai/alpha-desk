use async_trait::async_trait;
use domain_types::{AccountId, MarketId, SourceId};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use crate::{ObservationClass, SourceCursor, SourceError, SourceObservation};

#[must_use]
pub const fn observation_qualifies_committed_source(class: ObservationClass) -> bool {
    matches!(class, ObservationClass::CommittedBlock)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SnapshotTarget {
    Account(AccountId),
    Market(MarketId),
}

#[derive(Debug, Clone)]
pub struct SourceRequestContext {
    cancellation: CancellationToken,
    backpressure_deadline: Instant,
}

impl SourceRequestContext {
    #[must_use]
    pub const fn new(cancellation: CancellationToken, backpressure_deadline: Instant) -> Self {
        Self {
            cancellation,
            backpressure_deadline,
        }
    }

    pub fn check(&self) -> Result<(), SourceError> {
        if self.cancellation.is_cancelled() {
            return Err(SourceError::Cancelled);
        }
        if Instant::now() >= self.backpressure_deadline {
            return Err(SourceError::BackpressureTimeout);
        }
        Ok(())
    }

    #[must_use]
    pub fn cancellation(&self) -> &CancellationToken {
        &self.cancellation
    }

    #[must_use]
    pub const fn backpressure_deadline(&self) -> Instant {
        self.backpressure_deadline
    }
}

#[async_trait]
pub trait BlockSource: Send {
    async fn next_observation(
        &mut self,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError>;

    fn source_id(&self) -> &SourceId;

    fn committed_cursor(&self) -> Option<&SourceCursor>;
}

#[async_trait]
pub trait HistoricalRangeSource: Send + Sync {
    async fn fetch_range(
        &self,
        start: u64,
        end_inclusive: u64,
        context: &SourceRequestContext,
    ) -> Result<Vec<SourceObservation>, SourceError>;
}

#[async_trait]
pub trait SnapshotSource: Send + Sync {
    async fn fetch_snapshot(
        &self,
        target: &SnapshotTarget,
        context: &SourceRequestContext,
    ) -> Result<SourceObservation, SourceError>;
}
