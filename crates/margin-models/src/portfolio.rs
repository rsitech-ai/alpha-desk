use crate::model::{
    AccountModeMetadata, CalculationConfidence, LiquidationEstimate, MarginAssessment, MarginError,
    MarginInput, MarginModel, PORTFOLIO_RULES_UNSUPPORTED_EXACT, assessment, require_supported,
    requirements, sum_usd,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortfolioMarginModel;

impl MarginModel for PortfolioMarginModel {
    fn model_id(&self) -> &'static str {
        "hyperliquid-alpha-desk-margin-portfolio@1.0.0"
    }

    fn supports(&self, metadata: &AccountModeMetadata) -> bool {
        matches!(metadata, AccountModeMetadata::Portfolio { .. })
    }

    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError> {
        require_supported(self, input)?;
        let AccountModeMetadata::Portfolio { rules_version } = &input.mode else {
            return Err(MarginError::UnsupportedVersion);
        };
        let required = requirements(input)?;
        let scale = input.collateral_value.scale();
        let initial = sum_usd(required.iter().map(|item| item.initial_margin), scale)?;
        let maintenance = sum_usd(required.iter().map(|item| item.maintenance_margin), scale)?;
        let (lower, upper) = match required.as_slice() {
            [] => {
                let zero = domain_types::Price::from_raw(0, 0)
                    .map_err(|error| MarginError::Calculation(error.to_string()))?;
                (zero, zero)
            }
            items => (
                items
                    .iter()
                    .map(|item| item.oracle)
                    .min()
                    .expect("non-empty"),
                items
                    .iter()
                    .map(|item| item.oracle)
                    .max()
                    .expect("non-empty"),
            ),
        };
        assessment(
            input,
            initial,
            maintenance,
            LiquidationEstimate::Range {
                lower,
                upper,
                reason: PORTFOLIO_RULES_UNSUPPORTED_EXACT.to_owned(),
            },
            CalculationConfidence::Bounded,
            [
                ("model_id".to_owned(), self.model_id().to_owned()),
                ("rules_version".to_owned(), rules_version.clone()),
            ]
            .into_iter()
            .collect(),
        )
    }
}
