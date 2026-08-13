mod bounds;
mod contagion;
mod liquidations;
mod scenario;

pub use bounds::path_variant;
pub use contagion::apply_collateral_contagion;
pub use liquidations::{LiquidationWave, forced_impact_bps, liquidate_accounts};
pub use scenario::{
    DEFAULT_SHOCKS_BPS, FragilityResult, FragilityScenario, ScenarioPathResult, SimulatedAccount,
    SimulatedBook, SimulatedMarginMode, simulate_fragility, simulate_path,
};
