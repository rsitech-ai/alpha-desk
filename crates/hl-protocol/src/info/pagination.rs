use std::collections::BTreeSet;

use super::InfoError;

/// Overlap cursor for `/info` `by_time` pages.
///
/// Spec §10.4 sketches `last_stable_id` as a scalar. Same-millisecond
/// records are deduped against `identities_at_last_time` instead. A
/// lexicographic high-water mark drops unseen ids that sort below it
/// (`"100" < "99"`). The set is bounded by how many records share one
/// millisecond.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimePageCursor {
    start_time_millis: i64,
    last_time_millis: Option<i64>,
    identities_at_last_time: BTreeSet<String>,
    overlap_millis: i64,
}

impl TimePageCursor {
    pub fn new(start_time_millis: i64, overlap_millis: i64) -> Result<Self, InfoError> {
        if start_time_millis < 0 || overlap_millis <= 0 {
            return Err(InfoError::InvalidCursor);
        }
        Ok(Self {
            start_time_millis,
            last_time_millis: None,
            identities_at_last_time: BTreeSet::new(),
            overlap_millis,
        })
    }

    #[must_use]
    pub const fn start_time_millis(&self) -> i64 {
        self.start_time_millis
    }

    #[must_use]
    pub const fn last_time_millis(&self) -> Option<i64> {
        self.last_time_millis
    }

    #[must_use]
    pub fn last_stable_id(&self) -> Option<&str> {
        self.identities_at_last_time
            .iter()
            .next_back()
            .map(String::as_str)
    }

    #[must_use]
    pub const fn overlap_millis(&self) -> i64 {
        self.overlap_millis
    }

    #[must_use]
    pub fn next_query_start_millis(&self) -> i64 {
        match self.last_time_millis {
            None => self.start_time_millis,
            Some(last) => last
                .saturating_sub(self.overlap_millis)
                .max(self.start_time_millis),
        }
    }

    pub fn apply_page(
        &self,
        records: &[TimePageRecord<'_>],
        page_limit: usize,
    ) -> Result<TimePageOutcome, InfoError> {
        if page_limit == 0 {
            return Err(InfoError::InvalidCursor);
        }
        let mut ranked: Vec<RankedRecord> = records
            .iter()
            .enumerate()
            .map(|(index, record)| RankedRecord::from_record(index, record))
            .collect();
        ranked.sort_by(|left, right| {
            left.time_millis
                .cmp(&right.time_millis)
                .then_with(|| left.identity.cmp(&right.identity))
        });
        ranked.dedup_by(|left, right| {
            left.time_millis == right.time_millis && left.identity == right.identity
        });

        let kept: Vec<RankedRecord> = ranked
            .into_iter()
            .filter(|record| self.is_after_cursor(record))
            .collect();
        if kept.is_empty() && !records.is_empty() {
            return Ok(TimePageOutcome::NoProgress);
        }

        let mut next = self.clone();
        if let Some(last) = kept.last() {
            let at_last: BTreeSet<String> = kept
                .iter()
                .filter(|record| record.time_millis == last.time_millis)
                .map(|record| record.identity.clone())
                .collect();
            if self.last_time_millis == Some(last.time_millis) {
                next.identities_at_last_time.extend(at_last);
            } else {
                next.identities_at_last_time = at_last;
            }
            next.last_time_millis = Some(last.time_millis);
        }
        let indices = kept.iter().map(|record| record.index).collect();
        if records.len() < page_limit {
            Ok(TimePageOutcome::Exhausted {
                cursor: next,
                records: indices,
            })
        } else {
            Ok(TimePageOutcome::Next {
                cursor: next,
                records: indices,
            })
        }
    }

    fn is_after_cursor(&self, record: &RankedRecord) -> bool {
        let Some(last_time) = self.last_time_millis else {
            return true;
        };
        match record.time_millis.cmp(&last_time) {
            std::cmp::Ordering::Greater => true,
            std::cmp::Ordering::Less => false,
            std::cmp::Ordering::Equal => !self
                .identities_at_last_time
                .contains(record.identity.as_str()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimePageRecord<'a> {
    time_millis: i64,
    stable_id: Option<&'a str>,
    payload: &'a [u8],
}

impl<'a> TimePageRecord<'a> {
    pub fn new(
        time_millis: i64,
        stable_id: Option<&'a str>,
        payload: &'a [u8],
    ) -> Result<Self, InfoError> {
        if time_millis < 0 || payload.is_empty() {
            return Err(InfoError::InvalidCursor);
        }
        if let Some(id) = stable_id
            && (id.is_empty() || id.trim() != id)
        {
            return Err(InfoError::InvalidCursor);
        }
        Ok(Self {
            time_millis,
            stable_id,
            payload,
        })
    }

    #[must_use]
    pub const fn time_millis(self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub const fn stable_id(self) -> Option<&'a str> {
        self.stable_id
    }

    #[must_use]
    pub const fn payload(self) -> &'a [u8] {
        self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimePageOutcome {
    Next {
        cursor: TimePageCursor,
        records: Vec<usize>,
    },
    Exhausted {
        cursor: TimePageCursor,
        records: Vec<usize>,
    },
    NoProgress,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimeRangeGap {
    start_millis: i64,
    end_millis: i64,
}

impl TimeRangeGap {
    pub fn new(start_millis: i64, end_millis: i64) -> Result<Self, InfoError> {
        if start_millis < 0 || end_millis < start_millis {
            return Err(InfoError::InvalidCoverage);
        }
        Ok(Self {
            start_millis,
            end_millis,
        })
    }

    #[must_use]
    pub const fn start_millis(&self) -> i64 {
        self.start_millis
    }

    #[must_use]
    pub const fn end_millis(&self) -> i64 {
        self.end_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimePageCoverage {
    truncated: bool,
    earliest_reliable_millis: Option<i64>,
    known_gaps: Vec<TimeRangeGap>,
}

impl TimePageCoverage {
    pub fn new(
        truncated: bool,
        earliest_reliable_millis: Option<i64>,
        known_gaps: Vec<TimeRangeGap>,
    ) -> Result<Self, InfoError> {
        if let Some(earliest) = earliest_reliable_millis
            && earliest < 0
        {
            return Err(InfoError::InvalidCoverage);
        }
        Ok(Self {
            truncated,
            earliest_reliable_millis,
            known_gaps,
        })
    }

    #[must_use]
    pub const fn truncated(&self) -> bool {
        self.truncated
    }

    #[must_use]
    pub const fn earliest_reliable_millis(&self) -> Option<i64> {
        self.earliest_reliable_millis
    }

    #[must_use]
    pub fn known_gaps(&self) -> &[TimeRangeGap] {
        &self.known_gaps
    }
}

struct RankedRecord {
    index: usize,
    time_millis: i64,
    identity: String,
}

impl RankedRecord {
    fn from_record(index: usize, record: &TimePageRecord<'_>) -> Self {
        let identity = match record.stable_id {
            Some(id) => id.to_owned(),
            None => format!(
                "blake3:{}",
                hex::encode(blake3::hash(record.payload).as_bytes())
            ),
        };
        Self {
            index,
            time_millis: record.time_millis,
            identity,
        }
    }
}
