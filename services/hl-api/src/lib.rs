#![forbid(unsafe_code)]

mod auth;
mod budget;
mod config;
mod error;
mod http;
mod openapi;
mod snapshot;

pub use budget::{BudgetError, QueryBudgets, QueryPermit};
pub use config::{ApiConfig, AuthMode, ConfigError};
pub use error::{ERROR_SCHEMA_VERSION, ErrorBody};
pub use http::{ApiHandle, AppState, serve, spawn_local, spawn_state};
pub use openapi::{
    CAPTURE_STATUS_SCHEMA_IDS, CORE_DEADLETTER_REASON_CODES, HEALTH_JSON_FIELDS,
    LAST_HEARTBEAT_THROUGHPUT_FIELDS, ROUTER_PATHS, SNAPSHOT_UNAVAILABLE_REASON_CODES,
    core_deadletter_reason_openapi_enum, health_reason_code_is_unrestricted_string,
    is_core_deadletter_reason, openapi_yaml,
};
