#![forbid(unsafe_code)]

mod book;
mod clock;
mod cost;
mod error;
mod exit;
mod failure;
mod fees;
mod fill;
mod funding;
mod impact;
mod latency;
mod math;
mod order;
mod portfolio;
mod signal;
mod simulator;

pub use book::{BookLevel, BookSnapshot};
pub use clock::SimClock;
pub use cost::CostModel;
pub use error::SimError;
pub use exit::ExitPolicy;
pub use failure::FailureInjection;
pub use fees::{FeeSchedule, FundingSchedule};
pub use impact::{ImpactModel, SlippageModel};
pub use latency::{LatencyAssumptions, LatencyModel};
pub use order::{OrderPolicy, OrderType};
pub use portfolio::PortfolioLimits;
pub use signal::SignalSnapshot;
pub use simulator::{SimulationEvent, SimulationRequest, SimulationResult, run};
