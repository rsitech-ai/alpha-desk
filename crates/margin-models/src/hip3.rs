use crate::model::{
    AccountModeMetadata, CalculationConfidence, HIP3_RULES_V1, LiquidationEstimate,
    MarginAssessment, MarginError, MarginInput, MarginModel, assessment, require_supported,
    requirements, single_market_liquidation, sum_usd,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Hip3MarginModel;

impl MarginModel for Hip3MarginModel {
    fn model_id(&self) -> &'static str {
        "hyperliquid-alpha-desk-margin-hip3@1.0.0"
    }

    fn supports(&self, metadata: &AccountModeMetadata) -> bool {
        matches!(metadata, AccountModeMetadata::Hip3 { .. })
    }

    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError> {
        require_supported(self, input)?;
        let AccountModeMetadata::Hip3 {
            dex_id,
            rules_version,
        } = &input.mode
        else {
            return Err(MarginError::UnsupportedVersion);
        };
        if rules_version != HIP3_RULES_V1 {
            return Err(MarginError::UnsupportedVersion);
        }
        let required = requirements(input)?;
        let scale = input.collateral_value.scale();
        let initial = sum_usd(required.iter().map(|item| item.initial_margin), scale)?;
        let maintenance = sum_usd(required.iter().map(|item| item.maintenance_margin), scale)?;
        let liquidation = match required.as_slice() {
            [] => LiquidationEstimate::NotApplicable,
            [single] => single_market_liquidation(single, input.collateral_value)?,
            _ => LiquidationEstimate::Range {
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
                reason: "hip3 multi-market liquidation is not a single exact price".to_owned(),
            },
        };
        assessment(
            input,
            initial,
            maintenance,
            liquidation,
            CalculationConfidence::Exact,
            [
                ("model_id".to_owned(), self.model_id().to_owned()),
                ("dex_id".to_owned(), dex_id.as_str().to_owned()),
                ("rules_version".to_owned(), rules_version.clone()),
            ]
            .into_iter()
            .collect(),
        )
    }
}
