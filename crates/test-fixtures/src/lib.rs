#![forbid(unsafe_code)]

mod builders;
mod manifest;

pub use builders::{ScenarioBuildError, TradeScenarioBuilder};
pub use manifest::{FixtureEntry, FixtureError, FixtureManifest};

pub const CRATE_BOOTSTRAPPED: bool = true;
