use serde::Serialize;

pub const ERROR_SCHEMA_VERSION: &str = "hl.api.error.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ErrorBody {
    pub schema_version: &'static str,
    pub code: &'static str,
    pub reason_code: &'static str,
}

impl ErrorBody {
    #[must_use]
    pub const fn new(code: &'static str, reason_code: &'static str) -> Self {
        Self {
            schema_version: ERROR_SCHEMA_VERSION,
            code,
            reason_code,
        }
    }
}
