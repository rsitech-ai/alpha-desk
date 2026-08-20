use crate::model::{
    AccountModeMetadata, CalculationConfidence, LiquidationEstimate, MarginAssessment, MarginError,
    MarginInput, MarginModel, assessment, require_supported, requirements, sum_usd,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutcomeMarginModel;

impl MarginModel for OutcomeMarginModel {
    fn model_id(&self) -> &'static str {
        "hyperliquid-alpha-desk-margin-outcome@1.0.0"
    }

    fn supports(&self, metadata: &AccountModeMetadata) -> bool {
        matches!(metadata, AccountModeMetadata::Outcome { .. })
    }

    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError> {
        require_supported(self, input)?;
        let AccountModeMetadata::Outcome { market_id } = &input.mode else {
            return Err(MarginError::UnsupportedVersion);
        };
        let required: Vec<_> = requirements(input)?
            .into_iter()
            .filter(|item| item.market_id == *market_id)
            .collect();
        if required.is_empty() {
            return Err(MarginError::MissingInput(format!(
                "outcome position:{}",
                market_id.as_str()
            )));
        }
        let scale = input.collateral_value.scale();
        let initial = sum_usd(required.iter().map(|item| item.initial_margin), scale)?;
        let maintenance = sum_usd(required.iter().map(|item| item.maintenance_margin), scale)?;
        assessment(
            input,
            initial,
            maintenance,
            LiquidationEstimate::NotApplicable,
            CalculationConfidence::Exact,
            [
                ("model_id".to_owned(), self.model_id().to_owned()),
                ("outcome_market".to_owned(), market_id.as_str().to_owned()),
            ]
            .into_iter()
            .collect(),
        )
    }
}
