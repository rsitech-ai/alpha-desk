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
    AUXILIARY_SOURCE_HEALTH, AUXILIARY_SOURCE_QUALIFICATION, CAPTURE_SOURCE_HEALTH,
    CAPTURE_STATUS_SCHEMA_IDS, COMMITTED_SOURCE_CLASSES, CORE_DEADLETTER_REASON_CODES,
    HEALTH_JSON_FIELDS, LAST_HEARTBEAT_THROUGHPUT_FIELDS, LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
    READYZ_200_DESCRIPTION, READYZ_503_DESCRIPTION, READYZ_GET_DESCRIPTION, RESTART_RECONSTRUCTION,
    ROUTER_PATHS, SNAPSHOT_UNAVAILABLE_REASON_CODES,
    auxiliary_source_cursor_epoch_is_optional_string, auxiliary_source_health_openapi_enum,
    auxiliary_source_id_is_required_string, auxiliary_source_partial_line_is_required_bool,
    auxiliary_source_qualification_openapi_enum, auxiliary_source_spool_records_is_required_u64,
    auxiliary_source_unarchived_records_is_required_u64, capture_source_health_openapi_enum,
    committed_source_class_openapi_enum, core_deadletter_reason_openapi_enum,
    health_503_response_ref, health_503_schema_ref, health_reason_code_is_unrestricted_string,
    independent_source_health_openapi_enum, is_core_deadletter_reason,
    is_ledger_unsupported_event_reason, ledger_unsupported_event_reason_openapi_enum, openapi_yaml,
    readyz_200_description, readyz_200_schema_ref, readyz_503_description, readyz_503_schema_ref,
    readyz_get_description, restart_reconstruction_openapi_enum, unavailable_response_schema_ref,
};
