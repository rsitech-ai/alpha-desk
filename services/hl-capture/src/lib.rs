#![forbid(unsafe_code)]

pub mod adapters;
mod app;
mod auxiliary_checkpoint;
mod backlog;
pub mod bus;
mod committed_pipeline;
mod config;
pub mod coordinator;
mod disk;
mod egress;
mod failover;
mod fixture;
mod info_scheduler;
mod operator;
pub mod progress;
mod quarantine;
mod raw_archive;
mod request_budget;
mod secret;
mod sequencer;
mod service;
mod shutdown;
mod source_runtime;
pub mod spool;
mod status;

pub use app::{CaptureRuntime, CaptureRuntimeConfig, CaptureRuntimeError};
pub use backlog::*;
pub use committed_pipeline::*;
pub use config::*;
pub use disk::*;
pub use egress::*;
pub use failover::*;
pub use fixture::{FixtureError, synthetic_fixture_block, synthetic_independent_fixture_block};
pub use info_scheduler::*;
pub use operator::{
    OperatorError, accept_operator_status, encode_info_budget_status, serve_operator_status,
};
pub use quarantine::*;
pub use raw_archive::*;
pub use request_budget::*;
pub use sequencer::*;
pub use service::{ConnectedCapture, RuntimeConnectError, connect_capture};
pub use shutdown::{AppError, OwnedTask, run_owned_tasks};
pub use status::{
    AuxiliaryQualificationState, AuxiliarySourceHealth, AuxiliarySourceStatus, CaptureHealth,
    CaptureMaintenanceStatus, CaptureSourceHealth, CaptureStatus, CommittedSourceClass,
    RestartReconstruction, StatusError, StatusWriter, read_status,
};
