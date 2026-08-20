use crate::model::{
    AccountModeMetadata, CalculationConfidence, MarginAssessment, MarginError, MarginInput,
    MarginModel, assessment, require_supported, requirements, single_market_liquidation, sum_usd,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StandardCrossMarginModel;

impl MarginModel for StandardCrossMarginModel {
    fn model_id(&self) -> &'static str {
        "hyperliquid-alpha-desk-margin-standard-cross@1.0.0"
    }

    fn supports(&self, metadata: &AccountModeMetadata) -> bool {
        matches!(metadata, AccountModeMetadata::StandardCross)
    }

    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError> {
        require_supported(self, input)?;
        let required = requirements(input)?;
        let scale = input.collateral_value.scale();
        let initial = sum_usd(required.iter().map(|item| item.initial_margin), scale)?;
        let maintenance = sum_usd(required.iter().map(|item| item.maintenance_margin), scale)?;
        let liquidation = match required.as_slice() {
            [] => crate::model::LiquidationEstimate::NotApplicable,
            [single] => single_market_liquidation(single, input.collateral_value)?,
            _ => crate::model::LiquidationEstimate::Range {
                lower: required
                    .iter()
                    .map(|item| item.oracle)
                    .min()
                    .expect("non-empty"),
                upper: required
                    .iter()
                    .map(|item| item.oracle)
                    .max()
                    .expect("non-empty"),
                reason: "cross liquidation is not a single-market exact price".to_owned(),
            },
        };
        assessment(
            input,
            initial,
            maintenance,
            liquidation,
            CalculationConfidence::Exact,
            [("model_id".to_owned(), self.model_id().to_owned())]
                .into_iter()
                .collect(),
        )
    }
}
