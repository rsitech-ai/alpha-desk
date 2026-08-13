#![forbid(unsafe_code)]

mod fixture;
mod hip3;
mod isolated;
mod model;
mod outcome;
mod portfolio;
mod standard;
mod unified;

pub use fixture::{
    MARGIN_FIXTURE_SCHEMA, MarginFixture, MarginFixtureError, assert_margin_fixture,
    parse_margin_fixture,
};
pub use hip3::Hip3MarginModel;
pub use isolated::IsolatedMarginModel;
pub use model::{
    AccountModeMetadata, CalculationConfidence, HIP3_RULES_V1, LiquidationEstimate,
    MarginAssessment, MarginError, MarginInput, MarginModel, PORTFOLIO_RULES_UNSUPPORTED_EXACT,
    PositionState,
};
pub use outcome::OutcomeMarginModel;
pub use portfolio::PortfolioMarginModel;
pub use standard::StandardCrossMarginModel;
pub use unified::UnifiedMarginModel;

pub fn evaluate(input: &MarginInput) -> Result<MarginAssessment, MarginError> {
    match &input.mode {
        AccountModeMetadata::StandardCross => StandardCrossMarginModel.evaluate(input),
        AccountModeMetadata::StandardIsolated { .. } => IsolatedMarginModel.evaluate(input),
        AccountModeMetadata::Unified => UnifiedMarginModel.evaluate(input),
        AccountModeMetadata::Portfolio { .. } => PortfolioMarginModel.evaluate(input),
        AccountModeMetadata::Hip3 { .. } => Hip3MarginModel.evaluate(input),
        AccountModeMetadata::Outcome { .. } => OutcomeMarginModel.evaluate(input),
    }
}

pub const CRATE_BOOTSTRAPPED: bool = false;
