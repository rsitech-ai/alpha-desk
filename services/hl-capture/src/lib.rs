#![forbid(unsafe_code)]

pub mod adapters;
mod app;
pub mod bus;
mod config;
pub mod coordinator;
mod fixture;
pub mod progress;
mod quarantine;
mod secret;
mod sequencer;
mod service;
mod shutdown;
pub mod spool;
mod status;

pub use app::{CaptureRuntime, CaptureRuntimeConfig, CaptureRuntimeError};
pub use config::*;
pub use fixture::{FixtureError, synthetic_fixture_block};
pub use quarantine::*;
pub use sequencer::*;
pub use service::{ConnectedCapture, RuntimeConnectError, connect_capture};
pub use shutdown::{AppError, OwnedTask, run_owned_tasks};
pub use status::{CaptureHealth, CaptureStatus, StatusError, StatusWriter, read_status};
