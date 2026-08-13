#![forbid(unsafe_code)]

mod error;
mod experiment;
mod report;
mod synthetic;

pub use error::ResearchError;
pub use experiment::{ExperimentManifest, ExperimentRecord, ExperimentRegistry, ExperimentStatus};
pub use report::{ResearchReport, ResearchStatus};
pub use synthetic::{run_synthetic_bytes, run_synthetic_fixture};
