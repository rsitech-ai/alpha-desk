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
pub use http::{ApiHandle, AppState, serve, spawn_local};
pub use openapi::{HEALTH_JSON_FIELDS, ROUTER_PATHS, openapi_yaml};
