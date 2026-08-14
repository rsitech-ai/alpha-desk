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
    CAPTURE_STATUS_SCHEMA_IDS, COMMITTED_SOURCE_CLASSES, CORE_DEADLETTER_REASON_CODES,
    HEALTH_JSON_FIELDS, LAST_HEARTBEAT_THROUGHPUT_FIELDS, LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
    READYZ_200_DESCRIPTION, READYZ_503_DESCRIPTION, READYZ_GET_DESCRIPTION, ROUTER_PATHS,
    SNAPSHOT_UNAVAILABLE_REASON_CODES, committed_source_class_openapi_enum,
    core_deadletter_reason_openapi_enum, health_503_response_ref, health_503_schema_ref,
    health_reason_code_is_unrestricted_string, is_core_deadletter_reason,
    is_ledger_unsupported_event_reason, ledger_unsupported_event_reason_openapi_enum, openapi_yaml,
    readyz_200_description, readyz_200_schema_ref, readyz_503_description, readyz_503_schema_ref,
    readyz_get_description, unavailable_response_schema_ref,
};
