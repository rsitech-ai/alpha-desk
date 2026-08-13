use domain_types::{KnownTime, ProtocolTime};

use crate::FeatureSnapshot;

pub trait Bitemporal {
    fn effective_at(&self) -> ProtocolTime;
    fn known_at(&self) -> KnownTime;
    fn superseded_at(&self) -> Option<KnownTime>;
    fn revision(&self) -> u32;
}

impl Bitemporal for FeatureSnapshot {
    fn effective_at(&self) -> ProtocolTime {
        self.effective_at
    }

    fn known_at(&self) -> KnownTime {
        self.known_at
    }

    fn superseded_at(&self) -> Option<KnownTime> {
        self.superseded_at
    }

    fn revision(&self) -> u32 {
        self.revision
    }
}

/// Returns the latest row visible at both the requested effective time and
/// knowledge time.
///
/// A fact with `effective_at <= query_effective` is still withheld when
/// `known_at` is after the query knowledge cutoff. Superseded rows are hidden
/// once `superseded_at <= query_known`.
#[must_use]
pub fn asof<T: Bitemporal>(
    rows: &[T],
    effective_at: ProtocolTime,
    known_at: KnownTime,
) -> Option<&T> {
    rows.iter()
        .filter(|row| {
            row.effective_at() <= effective_at
                && row.known_at() <= known_at
                && row
                    .superseded_at()
                    .is_none_or(|superseded_at| superseded_at > known_at)
        })
        .max_by_key(|row| (row.effective_at(), row.known_at(), row.revision()))
}
