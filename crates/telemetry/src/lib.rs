#![forbid(unsafe_code)]

mod health;
mod metrics;
mod provenance;
mod telemetry;

pub use health::{HealthAssessment, HealthError, HealthState};
pub use metrics::{FoundationMetrics, encode_registry};
pub use provenance::{BuildProvenance, ProvenanceError};
pub use telemetry::{TelemetryConfig, TelemetryError, TelemetryGuard, init_telemetry};
