use crate::model::{
    AccountModeMetadata, CalculationConfidence, LiquidationEstimate, MarginAssessment, MarginError,
    MarginInput, MarginModel, PositionSide, assessment, max_usd, require_supported, requirements,
    sum_usd,
};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct UnifiedMarginModel;

impl MarginModel for UnifiedMarginModel {
    fn model_id(&self) -> &'static str {
        "hyperliquid-alpha-desk-margin-unified@1.0.0"
    }

    fn supports(&self, metadata: &AccountModeMetadata) -> bool {
        matches!(metadata, AccountModeMetadata::Unified)
    }

    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError> {
        require_supported(self, input)?;
        let required = requirements(input)?;
        let scale = input.collateral_value.scale();
        let initial = sum_usd(required.iter().map(|item| item.initial_margin), scale)?;
        let long_mm = sum_usd(
            required
                .iter()
                .filter(|item| item.side == PositionSide::Long)
                .map(|item| item.maintenance_margin),
            scale,
        )?;
        let short_mm = sum_usd(
            required
                .iter()
                .filter(|item| item.side == PositionSide::Short)
                .map(|item| item.maintenance_margin),
            scale,
        )?;
        let maintenance = max_usd(long_mm, short_mm)?;
        let liquidation = if required.is_empty() {
            LiquidationEstimate::NotApplicable
        } else {
            LiquidationEstimate::Range {
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
                reason: "unified netting does not yield a single exact liquidation price"
                    .to_owned(),
            }
        };
        assessment(
            input,
            initial,
            maintenance,
            liquidation,
            CalculationConfidence::Exact,
            [
                ("model_id".to_owned(), self.model_id().to_owned()),
                ("long_maintenance".to_owned(), long_mm.to_string()),
                ("short_maintenance".to_owned(), short_mm.to_string()),
            ]
            .into_iter()
            .collect(),
        )
    }
}
