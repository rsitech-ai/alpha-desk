use canonical_events::CanonicalEventEnvelope;

use crate::{ApplyContext, EventReducer, ReducerError, StateMutation, StateView};

/// Production reducer set for the currently qualified committed source shape.
///
/// The qualified node corpus presently proves only actually empty committed
/// blocks. Such blocks advance the ledger watermark without invoking a reducer.
/// Every action-bearing event remains unsupported and is quarantined by
/// `CanonicalLedger` before `reduce` can run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct WatermarkOnlyReducerV1;

impl WatermarkOnlyReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-watermark-only@1.0.0";
}

impl EventReducer for WatermarkOnlyReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, _event: &CanonicalEventEnvelope) -> bool {
        false
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        Err(ReducerError::from_static(
            "reducer.unqualified_event",
            "watermark-only reducer cannot apply action-bearing events",
        ))
    }
}
