use crate::model::{
    AccountModeMetadata, CalculationConfidence, MarginAssessment, MarginError, MarginInput,
    MarginModel, assessment, require_supported, requirements, single_market_liquidation, sum_usd,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct IsolatedMarginModel;

impl MarginModel for IsolatedMarginModel {
    fn model_id(&self) -> &'static str {
        "hyperliquid-alpha-desk-margin-isolated@1.0.0"
    }

    fn supports(&self, metadata: &AccountModeMetadata) -> bool {
        matches!(metadata, AccountModeMetadata::StandardIsolated { .. })
    }

    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError> {
        require_supported(self, input)?;
        let AccountModeMetadata::StandardIsolated { market_id } = &input.mode else {
            return Err(MarginError::UnsupportedVersion);
        };
        let required: Vec<_> = requirements(input)?
            .into_iter()
            .filter(|item| item.market_id == *market_id)
            .collect();
        if required.is_empty() {
            return Err(MarginError::MissingInput(format!(
                "isolated position:{}",
                market_id.as_str()
            )));
        }
        if required.len() != 1 {
            return Err(MarginError::Calculation(
                "isolated mode requires exactly one position for the named market".to_owned(),
            ));
        }
        let scale = input.collateral_value.scale();
        let initial = sum_usd(required.iter().map(|item| item.initial_margin), scale)?;
        let maintenance = sum_usd(required.iter().map(|item| item.maintenance_margin), scale)?;
        let liquidation = single_market_liquidation(&required[0], input.collateral_value)?;
        assessment(
            input,
            initial,
            maintenance,
            liquidation,
            CalculationConfidence::Exact,
            [
                ("model_id".to_owned(), self.model_id().to_owned()),
                ("isolated_market".to_owned(), market_id.as_str().to_owned()),
            ]
            .into_iter()
            .collect(),
        )
    }
}
