use std::collections::BTreeMap;

use domain_types::KnownTime;
use hl_protocol::info::{
    InfoRegistry, TimePageCoverage, TimePageCursor, TimePageOutcome, TimePageRecord, TimeRangeGap,
};
use serde_json::Value;

use crate::egress::{EgressError, InfoTransport, fetch_info};
use crate::request_budget::{
    BudgetError, BudgetLease, RequestBudget, RequestCost, SchedulePriority,
};

const MAX_TIME_PAGES: usize = 1_024;
const REFILL_PERIOD_MILLIS: u64 = 60_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoJob {
    id: String,
    priority: SchedulePriority,
    deadline_millis: u64,
    risk_score: u32,
    last_success_millis: u64,
    estimated_cost: u32,
    capability_id: String,
}

impl InfoJob {
    pub fn try_new(
        id: impl Into<String>,
        priority: SchedulePriority,
        deadline_millis: u64,
        risk_score: u32,
        last_success_millis: u64,
        estimated_cost: u32,
        capability_id: impl Into<String>,
    ) -> Result<Self, SchedulerError> {
        let id = id.into();
        let capability_id = capability_id.into();
        if id.is_empty()
            || id.trim() != id
            || capability_id.is_empty()
            || capability_id.trim() != capability_id
            || estimated_cost == 0
        {
            return Err(SchedulerError::InvalidJob);
        }
        Ok(Self {
            id,
            priority,
            deadline_millis,
            risk_score,
            last_success_millis,
            estimated_cost,
            capability_id,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub const fn priority(&self) -> SchedulePriority {
        self.priority
    }

    #[must_use]
    pub const fn estimated_cost(&self) -> u32 {
        self.estimated_cost
    }

    #[must_use]
    pub fn capability_id(&self) -> &str {
        &self.capability_id
    }

    fn key(&self) -> ScheduleKey {
        ScheduleKey {
            priority: self.priority,
            deadline_millis: self.deadline_millis,
            risk_rank: u32::MAX - self.risk_score,
            last_success_millis: self.last_success_millis,
            estimated_cost: self.estimated_cost,
            job_id: self.id.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ScheduleKey {
    priority: SchedulePriority,
    deadline_millis: u64,
    risk_rank: u32,
    last_success_millis: u64,
    estimated_cost: u32,
    job_id: String,
}

#[derive(Debug)]
pub struct InFlight {
    job: InfoJob,
    lease: BudgetLease,
}

impl InFlight {
    #[must_use]
    pub fn job(&self) -> &InfoJob {
        &self.job
    }

    #[must_use]
    pub fn lease(&self) -> &BudgetLease {
        &self.lease
    }
}

#[derive(Debug, Default)]
pub struct InfoScheduler {
    jobs: BTreeMap<ScheduleKey, InfoJob>,
}

impl InfoScheduler {
    #[must_use]
    pub fn new() -> Self {
        Self {
            jobs: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.jobs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
    }

    pub fn enqueue(&mut self, job: InfoJob) -> Result<(), SchedulerError> {
        if self.jobs.values().any(|queued| queued.id == job.id) {
            return Err(SchedulerError::DuplicateJob);
        }
        self.jobs.insert(job.key(), job);
        Ok(())
    }

    pub fn dispatch(
        &mut self,
        budget: &mut RequestBudget,
        now_millis: u64,
    ) -> Result<Option<InFlight>, SchedulerError> {
        let keys: Vec<ScheduleKey> = self.jobs.keys().cloned().collect();
        for key in keys {
            let job = self.jobs.get(&key).ok_or(SchedulerError::InvalidJob)?;
            match budget.reserve(now_millis, &job.id, job.priority, job.estimated_cost) {
                Ok(lease) => {
                    let job = self.jobs.remove(&key).ok_or(SchedulerError::InvalidJob)?;
                    return Ok(Some(InFlight { job, lease }));
                }
                Err(BudgetError::Insufficient) => {}
                Err(BudgetError::CircuitOpen) => return Err(SchedulerError::CircuitOpen),
                Err(error) => return Err(SchedulerError::Budget(error)),
            }
        }
        Ok(None)
    }

    pub fn complete(
        &mut self,
        budget: &mut RequestBudget,
        inflight: InFlight,
        actual_cost: u32,
        now_millis: u64,
    ) -> Result<InfoJob, SchedulerError> {
        budget
            .commit(now_millis, inflight.lease, actual_cost)
            .map_err(SchedulerError::Budget)?;
        Ok(inflight.job)
    }

    pub fn cancel(
        &mut self,
        budget: &mut RequestBudget,
        inflight: InFlight,
        now_millis: u64,
    ) -> Result<InfoJob, SchedulerError> {
        budget
            .release(now_millis, inflight.lease)
            .map_err(SchedulerError::Budget)?;
        Ok(inflight.job)
    }

    pub fn shutdown(
        &mut self,
        budget: &mut RequestBudget,
        inflight: Vec<InFlight>,
        now_millis: u64,
    ) -> Result<(), SchedulerError> {
        for item in inflight {
            budget
                .release(now_millis, item.lease)
                .map_err(SchedulerError::Budget)?;
        }
        self.jobs.clear();
        Ok(())
    }

    pub fn on_429(
        &mut self,
        budget: &mut RequestBudget,
        inflight: InFlight,
        now_millis: u64,
    ) -> Result<u64, SchedulerError> {
        budget
            .on_429(now_millis, inflight.lease)
            .map_err(SchedulerError::Budget)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimePageStopReason {
    Exhausted,
    EmptyVenue,
    EndOfStream,
    SameMillisecondBurst,
    Truncated,
    BudgetExhausted,
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedPageRecord {
    time_millis: i64,
    stable_id: Option<String>,
    payload: Vec<u8>,
}

impl OwnedPageRecord {
    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub fn stable_id(&self) -> Option<&str> {
        self.stable_id.as_deref()
    }

    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TimePageCrawl {
    records: Vec<OwnedPageRecord>,
    cursor: TimePageCursor,
    coverage: TimePageCoverage,
    stop: TimePageStopReason,
}

impl TimePageCrawl {
    #[must_use]
    pub fn records(&self) -> &[OwnedPageRecord] {
        &self.records
    }

    #[must_use]
    pub const fn cursor(&self) -> &TimePageCursor {
        &self.cursor
    }

    #[must_use]
    pub const fn coverage(&self) -> &TimePageCoverage {
        &self.coverage
    }

    #[must_use]
    pub const fn stop(&self) -> TimePageStopReason {
        self.stop
    }
}

pub struct TimePageCrawlRequest<'a> {
    capability_id: &'a str,
    extra_params: &'a BTreeMap<String, Value>,
    cursor: TimePageCursor,
    page_limit: usize,
    job_id: &'a str,
    priority: SchedulePriority,
    now_millis: u64,
    received_at: KnownTime,
    archive_ref: &'a str,
    cost: RequestCost,
}

impl<'a> TimePageCrawlRequest<'a> {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        capability_id: &'a str,
        extra_params: &'a BTreeMap<String, Value>,
        cursor: TimePageCursor,
        page_limit: usize,
        job_id: &'a str,
        priority: SchedulePriority,
        now_millis: u64,
        received_at: KnownTime,
        archive_ref: &'a str,
        cost: RequestCost,
    ) -> Self {
        Self {
            capability_id,
            extra_params,
            cursor,
            page_limit,
            job_id,
            priority,
            now_millis,
            received_at,
            archive_ref,
            cost,
        }
    }
}

pub fn crawl_time_pages<T: InfoTransport>(
    transport: &mut T,
    budget: &mut RequestBudget,
    registry: InfoRegistry,
    request: TimePageCrawlRequest<'_>,
) -> Result<TimePageCrawl, SchedulerError> {
    if request.page_limit == 0 {
        return Err(SchedulerError::InvalidCursor);
    }
    let mut collected = Vec::new();
    let mut cursor = request.cursor.clone();
    let mut pages = 0_usize;
    let mut now = request.now_millis;
    loop {
        if pages >= MAX_TIME_PAGES {
            return finish_crawl(
                collected,
                cursor,
                true,
                Vec::new(),
                TimePageStopReason::Truncated,
            );
        }
        pages += 1;
        let start = cursor.next_query_start_millis();
        if let Some(last) = cursor.last_time_millis()
            && start == last.saturating_add(1)
        {
            return Err(SchedulerError::AdvancedByOneMillisecond);
        }
        let mut params = request.extra_params.clone();
        params.insert("startTime".to_owned(), Value::from(start));
        let lease = match reserve_page(budget, &request, &mut now) {
            Ok(lease) => lease,
            Err(SchedulerError::CircuitOpen) => {
                return take_progress(
                    collected,
                    cursor,
                    TimePageStopReason::Incomplete,
                    SchedulerError::CircuitOpen,
                );
            }
            Err(error) => {
                return take_progress(
                    collected,
                    cursor,
                    TimePageStopReason::BudgetExhausted,
                    error,
                );
            }
        };
        let fetched = match fetch_info(
            transport,
            registry,
            request.capability_id,
            &params,
            request.received_at,
            request.archive_ref,
        ) {
            Ok(fetched) => fetched,
            Err(EgressError::RateLimited) => {
                budget.on_429(now, lease).map_err(SchedulerError::Budget)?;
                return take_progress(
                    collected,
                    cursor,
                    TimePageStopReason::Incomplete,
                    SchedulerError::RateLimited,
                );
            }
            Err(error) => {
                budget.release(now, lease).map_err(SchedulerError::Budget)?;
                return take_progress(
                    collected,
                    cursor,
                    TimePageStopReason::Incomplete,
                    SchedulerError::Egress(error),
                );
            }
        };
        let page = match owned_records_from_value(fetched.parsed().value()) {
            Ok(page) => page,
            Err(error) => {
                budget.release(now, lease).map_err(SchedulerError::Budget)?;
                return take_progress(collected, cursor, TimePageStopReason::Incomplete, error);
            }
        };
        let actual = request
            .cost
            .actual_weight(u32::try_from(page.len()).unwrap_or(u32::MAX));
        let extra_room = remaining_weight(budget, now, request.priority);
        let payable = actual.min(lease.reserved().saturating_add(extra_room));
        if let Err(error) = budget.commit(now, lease, payable) {
            return take_progress(
                collected,
                cursor,
                TimePageStopReason::BudgetExhausted,
                SchedulerError::Budget(error),
            );
        }
        if page.is_empty() {
            return finish_crawl(
                collected,
                cursor,
                false,
                Vec::new(),
                TimePageStopReason::EmptyVenue,
            );
        }
        let view = match page_view(&page) {
            Ok(view) => view,
            Err(error) => {
                return take_progress(collected, cursor, TimePageStopReason::Incomplete, error);
            }
        };
        match cursor.apply_page(&view, request.page_limit) {
            Ok(TimePageOutcome::Next {
                cursor: next,
                records,
            }) => {
                append_kept(&mut collected, &page, &records);
                cursor = next;
            }
            Ok(TimePageOutcome::Exhausted {
                cursor: next,
                records,
            }) => {
                append_kept(&mut collected, &page, &records);
                return finish_crawl(
                    collected,
                    next,
                    false,
                    Vec::new(),
                    TimePageStopReason::Exhausted,
                );
            }
            Ok(TimePageOutcome::NoProgress) => {
                if page.len() < request.page_limit {
                    return finish_crawl(
                        collected,
                        cursor,
                        false,
                        Vec::new(),
                        TimePageStopReason::EndOfStream,
                    );
                }
                let last = cursor
                    .last_time_millis()
                    .unwrap_or(cursor.start_time_millis());
                let gap =
                    TimeRangeGap::new(last, last).map_err(|_| SchedulerError::InvalidCursor)?;
                return finish_crawl(
                    collected,
                    cursor,
                    true,
                    vec![gap],
                    TimePageStopReason::SameMillisecondBurst,
                );
            }
            Err(_) => {
                return take_progress(
                    collected,
                    cursor,
                    TimePageStopReason::Incomplete,
                    SchedulerError::InvalidCursor,
                );
            }
        }
    }
}

fn remaining_weight(
    budget: &mut RequestBudget,
    now_millis: u64,
    priority: SchedulePriority,
) -> u32 {
    let snap = budget.snapshot(now_millis);
    if priority.is_protected() {
        snap.available_total()
    } else {
        snap.available_general()
    }
}

fn negotiated_row_estimate(cost: RequestCost, page_limit: u32, remaining: u32) -> Option<u32> {
    if remaining < cost.base() {
        return None;
    }
    if cost.variable().is_none() {
        return Some(0);
    }
    // ponytail: extra weight is +1/row. Official candle/history coefficients
    // are not in-tree. Cap the reserved rows so one page cannot exceed the
    // remaining envelope. Swap the table when T09 snapshots it.
    Some(page_limit.min(remaining - cost.base()))
}

fn reserve_page(
    budget: &mut RequestBudget,
    request: &TimePageCrawlRequest<'_>,
    now: &mut u64,
) -> Result<BudgetLease, SchedulerError> {
    let page_limit = u32::try_from(request.page_limit).unwrap_or(u32::MAX);
    for attempt in 0..2_u8 {
        if attempt == 1 {
            *now = now.saturating_add(REFILL_PERIOD_MILLIS);
        }
        let remaining = remaining_weight(budget, *now, request.priority);
        let Some(rows) = negotiated_row_estimate(request.cost, page_limit, remaining) else {
            continue;
        };
        let reserved = request.cost.estimated_weight(rows);
        match budget.reserve(*now, request.job_id, request.priority, reserved) {
            Ok(lease) => return Ok(lease),
            Err(BudgetError::Insufficient) => {}
            Err(BudgetError::CircuitOpen) => return Err(SchedulerError::CircuitOpen),
            Err(error) => return Err(SchedulerError::Budget(error)),
        }
    }
    Err(SchedulerError::Budget(BudgetError::Insufficient))
}

fn take_progress(
    collected: Vec<OwnedPageRecord>,
    cursor: TimePageCursor,
    stop: TimePageStopReason,
    error: SchedulerError,
) -> Result<TimePageCrawl, SchedulerError> {
    if collected.is_empty() {
        Err(error)
    } else {
        finish_crawl(collected, cursor, true, Vec::new(), stop)
    }
}

fn finish_crawl(
    records: Vec<OwnedPageRecord>,
    cursor: TimePageCursor,
    truncated: bool,
    gaps: Vec<TimeRangeGap>,
    stop: TimePageStopReason,
) -> Result<TimePageCrawl, SchedulerError> {
    let earliest = if truncated {
        records.iter().map(OwnedPageRecord::time_millis).min()
    } else {
        None
    };
    Ok(TimePageCrawl {
        records,
        coverage: TimePageCoverage::new(truncated, earliest, gaps)
            .map_err(|_| SchedulerError::InvalidCursor)?,
        cursor,
        stop,
    })
}

fn page_view(page: &[OwnedPageRecord]) -> Result<Vec<TimePageRecord<'_>>, SchedulerError> {
    page.iter()
        .map(|record| {
            TimePageRecord::new(
                record.time_millis,
                record.stable_id.as_deref(),
                &record.payload,
            )
            .map_err(|_| SchedulerError::InvalidCursor)
        })
        .collect()
}

fn append_kept(into: &mut Vec<OwnedPageRecord>, page: &[OwnedPageRecord], indices: &[usize]) {
    for index in indices {
        if let Some(record) = page.get(*index) {
            into.push(record.clone());
        }
    }
}

pub fn owned_records_from_value(value: &Value) -> Result<Vec<OwnedPageRecord>, SchedulerError> {
    let items = value.as_array().ok_or(SchedulerError::NotAPage)?;
    items.iter().map(owned_record_from_item).collect()
}

fn owned_record_from_item(item: &Value) -> Result<OwnedPageRecord, SchedulerError> {
    let time = item
        .get("time")
        .or_else(|| item.get("t"))
        .and_then(Value::as_i64)
        .ok_or(SchedulerError::NotAPage)?;
    if time < 0 {
        return Err(SchedulerError::NotAPage);
    }
    let stable_id = item
        .get("id")
        .or_else(|| item.get("oid"))
        .or_else(|| item.get("tid"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let payload = serde_json::to_vec(item).map_err(|_| SchedulerError::NotAPage)?;
    if payload.is_empty() {
        return Err(SchedulerError::NotAPage);
    }
    Ok(OwnedPageRecord {
        time_millis: time,
        stable_id,
        payload,
    })
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum SchedulerError {
    #[error("info scheduler job is invalid")]
    InvalidJob,
    #[error("info scheduler job id is duplicated")]
    DuplicateJob,
    #[error("info request budget circuit is open after 429")]
    CircuitOpen,
    #[error("info request was rate-limited")]
    RateLimited,
    #[error("time-page crawl would have used last_timestamp+1")]
    AdvancedByOneMillisecond,
    #[error("time-page cursor is invalid")]
    InvalidCursor,
    #[error("info response is not a time page")]
    NotAPage,
    #[error("info request budget error")]
    Budget(BudgetError),
    #[error("info egress error")]
    Egress(EgressError),
}

impl SchedulerError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::InvalidJob => "capture_info.invalid_job",
            Self::DuplicateJob => "capture_info.duplicate_job",
            Self::CircuitOpen => "capture_info.circuit_open",
            Self::RateLimited => "capture_info.rate_limited",
            Self::AdvancedByOneMillisecond => "capture_info.advanced_by_one_ms",
            Self::InvalidCursor => "capture_info.invalid_cursor",
            Self::NotAPage => "capture_info.not_a_page",
            Self::Budget(error) => error.reason_code(),
            Self::Egress(error) => error.reason_code(),
        }
    }
}
