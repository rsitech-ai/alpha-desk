use canonical_events::CanonicalEventEnvelope;

use crate::{ApplyContext, EventReducer, ReducerError, StateMutation, StateView};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CanonicalBorrowLendReducerV1;

impl CanonicalBorrowLendReducerV1 {
    pub const VERSION: &'static str = "hyperliquid-alpha-desk-canonical-borrow-lend@1.0.0";
}

impl EventReducer for CanonicalBorrowLendReducerV1 {
    fn reducer_set_version(&self) -> &str {
        Self::VERSION
    }

    fn supports(&self, event: &CanonicalEventEnvelope) -> bool {
        let _ = event;
        false
    }

    fn reduce(
        &self,
        _state: &StateView<'_>,
        _event: &CanonicalEventEnvelope,
        _context: &ApplyContext<'_>,
    ) -> Result<Vec<StateMutation>, ReducerError> {
        Err(ReducerError::from_static(
            "borrow_lend_state.unsupported_event",
            "borrow/lend kinds were not shipped in catalog 1.1.0",
        ))
    }
}

#[cfg(test)]
mod tests {
    use canonical_events::EventKind;

    use super::*;

    #[test]
    fn catalog_1_1_has_no_borrow_lend_kinds() {
        assert!(EventKind::ALL.iter().all(|kind| {
            let name = kind.as_wire_name();
            !name.contains("Borrow") && !name.contains("Lend")
        }));
        assert_eq!(
            CanonicalBorrowLendReducerV1::VERSION,
            "hyperliquid-alpha-desk-canonical-borrow-lend@1.0.0"
        );
    }
}
