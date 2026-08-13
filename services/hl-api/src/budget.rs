use std::fs;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use http::StatusCode;
use serde::Deserialize;

use crate::error::ErrorBody;

const BUDGET_SCHEMA_VERSION: &str = "hl.api.query_budgets.v1";
const MAX_BUDGET_FILE_BYTES: u64 = 16 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    InvalidLimit,
    UnsupportedParameter,
    OffsetForbidden,
    MaxRows,
    Concurrency,
    Timeout,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetLoadError {
    Missing,
    Invalid,
}

impl BudgetError {
    #[must_use]
    pub const fn status(self) -> StatusCode {
        match self {
            Self::InvalidLimit
            | Self::UnsupportedParameter
            | Self::OffsetForbidden
            | Self::MaxRows => StatusCode::BAD_REQUEST,
            Self::Concurrency | Self::Timeout => StatusCode::TOO_MANY_REQUESTS,
        }
    }

    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidLimit | Self::UnsupportedParameter | Self::OffsetForbidden => {
                "invalid_query"
            }
            Self::MaxRows | Self::Concurrency | Self::Timeout => "query_budget_exceeded",
        }
    }

    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidLimit => "query.limit",
            Self::UnsupportedParameter => "query.unsupported_parameter",
            Self::OffsetForbidden => "query.offset_forbidden",
            Self::MaxRows => "query.max_rows",
            Self::Concurrency => "query.concurrency",
            Self::Timeout => "query.timeout",
        }
    }

    #[must_use]
    pub const fn error_body(self) -> ErrorBody {
        ErrorBody::new(self.code(), self.reason_code())
    }
}

#[derive(Debug)]
#[must_use]
pub struct QueryPermit {
    in_flight: Arc<AtomicU32>,
}

impl Drop for QueryPermit {
    fn drop(&mut self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone)]
pub struct QueryBudgets {
    max_rows: u32,
    timeout: Duration,
    max_concurrency: u32,
    in_flight: Arc<AtomicU32>,
}

impl PartialEq for QueryBudgets {
    fn eq(&self, other: &Self) -> bool {
        self.max_rows == other.max_rows
            && self.timeout == other.timeout
            && self.max_concurrency == other.max_concurrency
    }
}

impl Eq for QueryBudgets {}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawQueryBudgets {
    schema_version: String,
    max_rows: u32,
    timeout_ms: u32,
    max_concurrency: u32,
}

impl QueryBudgets {
    pub fn from_path(path: &Path) -> Result<Self, BudgetLoadError> {
        let metadata = fs::metadata(path).map_err(|_| BudgetLoadError::Missing)?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > MAX_BUDGET_FILE_BYTES {
            return Err(BudgetLoadError::Invalid);
        }
        let source = fs::read_to_string(path).map_err(|_| BudgetLoadError::Missing)?;
        Self::from_toml(&source)
    }

    pub fn from_toml(source: &str) -> Result<Self, BudgetLoadError> {
        let raw: RawQueryBudgets = toml::from_str(source).map_err(|_| BudgetLoadError::Invalid)?;
        if raw.schema_version != BUDGET_SCHEMA_VERSION
            || raw.max_rows == 0
            || raw.timeout_ms == 0
            || raw.max_concurrency == 0
        {
            return Err(BudgetLoadError::Invalid);
        }
        Ok(Self {
            max_rows: raw.max_rows,
            timeout: Duration::from_millis(u64::from(raw.timeout_ms)),
            max_concurrency: raw.max_concurrency,
            in_flight: Arc::new(AtomicU32::new(0)),
        })
    }

    #[must_use]
    pub const fn max_rows(&self) -> u32 {
        self.max_rows
    }

    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
    }

    #[must_use]
    pub const fn max_concurrency(&self) -> u32 {
        self.max_concurrency
    }

    pub fn check_query(&self, query: Option<&str>) -> Result<Option<u32>, BudgetError> {
        let requested = requested_rows(query)?;
        if let Some(rows) = requested
            && rows > self.max_rows
        {
            return Err(BudgetError::MaxRows);
        }
        Ok(requested)
    }

    pub fn try_acquire(&self) -> Result<QueryPermit, BudgetError> {
        loop {
            let current = self.in_flight.load(Ordering::SeqCst);
            if current >= self.max_concurrency {
                return Err(BudgetError::Concurrency);
            }
            if self
                .in_flight
                .compare_exchange(current, current + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return Ok(QueryPermit {
                    in_flight: Arc::clone(&self.in_flight),
                });
            }
        }
    }

    pub fn check_and_acquire(&self, query: Option<&str>) -> Result<QueryPermit, BudgetError> {
        self.check_query(query)?;
        self.try_acquire()
    }

    pub async fn execute<F, T>(&self, query: Option<&str>, work: F) -> Result<T, BudgetError>
    where
        F: Future<Output = T>,
    {
        let _permit = self.check_and_acquire(query)?;
        tokio::time::timeout(self.timeout, work)
            .await
            .map_err(|_| BudgetError::Timeout)
    }
}

fn requested_rows(query: Option<&str>) -> Result<Option<u32>, BudgetError> {
    let Some(query) = query.filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let mut limit = None;
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').ok_or(BudgetError::InvalidLimit)?;
        if key.is_empty() {
            return Err(BudgetError::UnsupportedParameter);
        }
        match key {
            "limit" => {
                if limit.is_some() {
                    return Err(BudgetError::InvalidLimit);
                }
                let parsed = value
                    .parse::<u32>()
                    .map_err(|_| BudgetError::InvalidLimit)?;
                if parsed == 0 {
                    return Err(BudgetError::InvalidLimit);
                }
                limit = Some(parsed);
            }
            "offset" => return Err(BudgetError::OffsetForbidden),
            _ => return Err(BudgetError::UnsupportedParameter),
        }
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::{BudgetError, QueryBudgets};

    fn budgets() -> QueryBudgets {
        QueryBudgets::from_toml(
            "schema_version = \"hl.api.query_budgets.v1\"\nmax_rows = 4\ntimeout_ms = 25\nmax_concurrency = 1\n",
        )
        .expect("literal budgets")
    }

    #[test]
    fn oversized_limit_is_query_budget_exceeded() {
        let error = budgets()
            .check_query(Some("limit=5"))
            .expect_err("oversize must fail closed");
        assert_eq!(error, BudgetError::MaxRows);
        assert_eq!(error.status().as_u16(), 400);
        assert_eq!(error.code(), "query_budget_exceeded");
        assert_eq!(error.reason_code(), "query.max_rows");
    }

    #[test]
    fn limit_at_budget_is_accepted() {
        assert_eq!(
            budgets()
                .check_query(Some("limit=4"))
                .expect("within budget"),
            Some(4)
        );
        assert_eq!(budgets().check_query(None).expect("no limit"), None);
    }

    #[test]
    fn offset_and_unknown_parameters_fail_closed() {
        assert_eq!(
            budgets().check_query(Some("offset=1")).expect_err("offset"),
            BudgetError::OffsetForbidden
        );
        assert_eq!(
            budgets()
                .check_query(Some("cursor=abc"))
                .expect_err("unknown"),
            BudgetError::UnsupportedParameter
        );
        assert_eq!(
            budgets().check_query(Some("limit=0")).expect_err("zero"),
            BudgetError::InvalidLimit
        );
        assert_eq!(
            budgets()
                .check_query(Some("limit=1&limit=2"))
                .expect_err("duplicate"),
            BudgetError::InvalidLimit
        );
    }

    #[test]
    fn concurrency_budget_rejects_a_second_permit() {
        let budgets = budgets();
        let _permit = budgets.try_acquire().expect("first permit");
        let error = budgets.try_acquire().expect_err("second permit");
        assert_eq!(error, BudgetError::Concurrency);
        assert_eq!(error.status().as_u16(), 429);
        assert_eq!(error.reason_code(), "query.concurrency");
    }

    #[test]
    fn zero_budgets_fail_closed_at_load() {
        let error = QueryBudgets::from_toml(
            "schema_version = \"hl.api.query_budgets.v1\"\nmax_rows = 0\ntimeout_ms = 25\nmax_concurrency = 1\n",
        )
        .expect_err("zero max_rows");
        assert_eq!(error, super::BudgetLoadError::Invalid);
    }
}
