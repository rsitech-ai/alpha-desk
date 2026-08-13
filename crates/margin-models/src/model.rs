use std::collections::BTreeMap;

use domain_types::{
    Address, BlockHeight, DexId, ExactQuoteNotional, FeeRate, MarginRatio, MarketId,
    PositionQuantity, Price, Quantity, UsdAmount, ValueError,
};
use num_bigint::BigInt;
use serde::{Deserialize, Serialize};

pub const HIP3_RULES_V1: &str = "hip3-margin@1.0.0";
pub const PORTFOLIO_RULES_UNSUPPORTED_EXACT: &str =
    "portfolio margin is not exactly reconstructible from V1 canonical inputs";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", deny_unknown_fields)]
pub enum AccountModeMetadata {
    StandardCross,
    StandardIsolated {
        market_id: MarketId,
    },
    Unified,
    Portfolio {
        rules_version: String,
    },
    Hip3 {
        dex_id: DexId,
        rules_version: String,
    },
    Outcome {
        market_id: MarketId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionState {
    pub market_id: MarketId,
    pub quantity: PositionQuantity,
    pub initial_margin_rate: FeeRate,
    pub maintenance_margin_rate: FeeRate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarginInput {
    pub account_id: Address,
    pub mode: AccountModeMetadata,
    pub collateral_value: UsdAmount,
    pub positions: Vec<PositionState>,
    pub oracle_prices: BTreeMap<MarketId, Price>,
    pub metadata_block: BlockHeight,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum LiquidationEstimate {
    Exact {
        trigger_price: Price,
    },
    Range {
        lower: Price,
        upper: Price,
        reason: String,
    },
    NotApplicable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CalculationConfidence {
    Exact,
    Bounded,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarginAssessment {
    pub initial_margin: UsdAmount,
    pub maintenance_margin: UsdAmount,
    pub margin_ratio: MarginRatio,
    pub liquidation: LiquidationEstimate,
    pub confidence: CalculationConfidence,
    pub reconciliation_fields: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum MarginError {
    #[error("unsupported account or margin metadata version")]
    UnsupportedVersion,
    #[error("required market or oracle input is missing: {0}")]
    MissingInput(String),
    #[error("fixed-point calculation failed: {0}")]
    Calculation(String),
}

pub trait MarginModel: Send + Sync {
    fn model_id(&self) -> &'static str;
    fn supports(&self, metadata: &AccountModeMetadata) -> bool;
    fn evaluate(&self, input: &MarginInput) -> Result<MarginAssessment, MarginError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PositionSide {
    Long,
    Short,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PositionRequirement {
    pub market_id: MarketId,
    pub side: PositionSide,
    pub oracle: Price,
    pub abs_quantity: Quantity,
    pub initial_margin: UsdAmount,
    pub maintenance_margin: UsdAmount,
}

pub(crate) fn require_supported(
    model: &impl MarginModel,
    input: &MarginInput,
) -> Result<(), MarginError> {
    if model.supports(&input.mode) {
        Ok(())
    } else {
        Err(MarginError::Calculation(format!(
            "{} does not support the supplied account mode",
            model.model_id()
        )))
    }
}

pub(crate) fn requirements(input: &MarginInput) -> Result<Vec<PositionRequirement>, MarginError> {
    let usd_scale = input.collateral_value.scale();
    let mut requirements = Vec::new();
    for position in &input.positions {
        if position.quantity.raw() == 0 {
            continue;
        }
        if position.initial_margin_rate.raw() < 0 || position.maintenance_margin_rate.raw() < 0 {
            return Err(MarginError::Calculation(
                "margin rates must be nonnegative".to_owned(),
            ));
        }
        let oracle = *input
            .oracle_prices
            .get(&position.market_id)
            .ok_or_else(|| {
                MarginError::MissingInput(format!("oracle:{}", position.market_id.as_str()))
            })?;
        if oracle.raw() <= 0 {
            return Err(MarginError::Calculation(
                "oracle price must be positive".to_owned(),
            ));
        }
        let abs_quantity = abs_quantity(position.quantity)?;
        let notional =
            ExactQuoteNotional::checked_product(oracle, abs_quantity).map_err(map_value_error)?;
        let side = if position.quantity.raw() > 0 {
            PositionSide::Long
        } else {
            PositionSide::Short
        };
        requirements.push(PositionRequirement {
            market_id: position.market_id.clone(),
            side,
            oracle,
            abs_quantity,
            initial_margin: usd_from_notional_rate(
                &notional,
                position.initial_margin_rate,
                usd_scale,
            )?,
            maintenance_margin: usd_from_notional_rate(
                &notional,
                position.maintenance_margin_rate,
                usd_scale,
            )?,
        });
    }
    Ok(requirements)
}

pub(crate) fn sum_usd(
    values: impl IntoIterator<Item = UsdAmount>,
    scale: u8,
) -> Result<UsdAmount, MarginError> {
    let mut total = UsdAmount::from_raw(0, scale).map_err(map_value_error)?;
    for value in values {
        total = total.checked_add(value).map_err(map_value_error)?;
    }
    Ok(total)
}

pub(crate) fn max_usd(left: UsdAmount, right: UsdAmount) -> Result<UsdAmount, MarginError> {
    if left.scale() != right.scale() {
        return Err(MarginError::Calculation(
            "usd amounts must share a scale".to_owned(),
        ));
    }
    if left.raw() >= right.raw() {
        Ok(left)
    } else {
        Ok(right)
    }
}

pub(crate) fn assessment(
    input: &MarginInput,
    initial_margin: UsdAmount,
    maintenance_margin: UsdAmount,
    liquidation: LiquidationEstimate,
    confidence: CalculationConfidence,
    extra: BTreeMap<String, String>,
) -> Result<MarginAssessment, MarginError> {
    if matches!(liquidation, LiquidationEstimate::Exact { .. })
        && confidence != CalculationConfidence::Exact
    {
        return Err(MarginError::Calculation(
            "exact liquidation requires exact confidence".to_owned(),
        ));
    }
    let mut reconciliation_fields = extra;
    reconciliation_fields.insert("account_id".to_owned(), input.account_id.to_api_string());
    reconciliation_fields.insert(
        "metadata_block".to_owned(),
        input.metadata_block.get().to_string(),
    );
    Ok(MarginAssessment {
        initial_margin,
        maintenance_margin,
        margin_ratio: margin_ratio(input.collateral_value, maintenance_margin)?,
        liquidation,
        confidence,
        reconciliation_fields,
    })
}

pub(crate) fn single_market_liquidation(
    requirement: &PositionRequirement,
    collateral: UsdAmount,
) -> Result<LiquidationEstimate, MarginError> {
    if requirement.maintenance_margin.raw() == 0 {
        return Ok(LiquidationEstimate::NotApplicable);
    }
    if collateral == requirement.maintenance_margin {
        return Ok(LiquidationEstimate::Exact {
            trigger_price: requirement.oracle,
        });
    }
    exact_trigger_price(requirement, collateral)
}

fn exact_trigger_price(
    requirement: &PositionRequirement,
    collateral: UsdAmount,
) -> Result<LiquidationEstimate, MarginError> {
    let rate = requirement.maintenance_margin;
    // p_liq = collateral * oracle / maintenance, exact only when divisible.
    if rate.raw() == 0 {
        return Ok(LiquidationEstimate::NotApplicable);
    }
    let numerator = BigInt::from(collateral.raw()) * BigInt::from(requirement.oracle.raw());
    let denominator = BigInt::from(rate.raw());
    let (quotient, remainder) = div_rem(&numerator, &denominator)?;
    if remainder != BigInt::from(0) {
        return Ok(LiquidationEstimate::Range {
            lower: requirement.oracle,
            upper: requirement.oracle,
            reason: "liquidation trigger is not an exact fixed-point price".to_owned(),
        });
    }
    let trigger = i128::try_from(&quotient)
        .map_err(|_| MarginError::Calculation("liquidation trigger overflowed i128".to_owned()))?;
    let trigger_price =
        Price::from_raw(trigger, requirement.oracle.scale()).map_err(map_value_error)?;
    if trigger_price.raw() <= 0 {
        return Ok(LiquidationEstimate::NotApplicable);
    }
    Ok(LiquidationEstimate::Exact { trigger_price })
}

fn margin_ratio(collateral: UsdAmount, maintenance: UsdAmount) -> Result<MarginRatio, MarginError> {
    if collateral.scale() != maintenance.scale() {
        return Err(MarginError::Calculation(
            "ratio operands must share a scale".to_owned(),
        ));
    }
    if maintenance.raw() == 0 {
        if collateral.raw() < 0 {
            return Err(MarginError::Calculation(
                "negative collateral with zero maintenance is undefined".to_owned(),
            ));
        }
        return MarginRatio::from_raw(0, collateral.scale()).map_err(map_value_error);
    }
    // ratio = collateral / maintenance at the shared scale: (c * 10^scale) / m
    let numerator = BigInt::from(collateral.raw()) * pow10(u32::from(collateral.scale()))?;
    let (quotient, remainder) = div_rem(&numerator, &BigInt::from(maintenance.raw()))?;
    if remainder != BigInt::from(0) {
        return Err(MarginError::Calculation(
            "margin ratio is not an exact fixed-point value".to_owned(),
        ));
    }
    let raw = i128::try_from(&quotient)
        .map_err(|_| MarginError::Calculation("margin ratio overflowed i128".to_owned()))?;
    MarginRatio::from_raw(raw, collateral.scale()).map_err(map_value_error)
}

fn abs_quantity(quantity: PositionQuantity) -> Result<Quantity, MarginError> {
    let raw = quantity.raw();
    if raw == i128::MIN {
        return Err(MarginError::Calculation(
            "position quantity absolute value overflowed".to_owned(),
        ));
    }
    Quantity::from_raw(raw.abs(), quantity.scale()).map_err(map_value_error)
}

fn usd_from_notional_rate(
    notional: &ExactQuoteNotional,
    rate: FeeRate,
    usd_scale: u8,
) -> Result<UsdAmount, MarginError> {
    let coefficient = notional.coefficient() * BigInt::from(rate.raw());
    let scale = u32::from(notional.scale())
        .checked_add(u32::from(rate.scale()))
        .ok_or_else(|| MarginError::Calculation("notional rate scale overflowed".to_owned()))?;
    let scaled = rescale_exact(coefficient, scale, u32::from(usd_scale))?;
    let raw = i128::try_from(&scaled)
        .map_err(|_| MarginError::Calculation("usd amount overflowed i128".to_owned()))?;
    UsdAmount::from_raw(raw, usd_scale).map_err(map_value_error)
}

fn rescale_exact(
    mut coefficient: BigInt,
    from_scale: u32,
    to_scale: u32,
) -> Result<BigInt, MarginError> {
    if from_scale == to_scale {
        return Ok(coefficient);
    }
    if from_scale < to_scale {
        coefficient *= pow10(to_scale - from_scale)?;
        return Ok(coefficient);
    }
    let divisor = pow10(from_scale - to_scale)?;
    let (quotient, remainder) = div_rem(&coefficient, &divisor)?;
    if remainder != BigInt::from(0) {
        return Err(MarginError::Calculation(
            "usd conversion is not exact at the collateral scale".to_owned(),
        ));
    }
    Ok(quotient)
}

fn pow10(exponent: u32) -> Result<BigInt, MarginError> {
    Ok(BigInt::from(10_u8).pow(exponent))
}

fn div_rem(numerator: &BigInt, denominator: &BigInt) -> Result<(BigInt, BigInt), MarginError> {
    if *denominator == BigInt::from(0) {
        return Err(MarginError::Calculation("division by zero".to_owned()));
    }
    let quotient = numerator / denominator;
    let remainder = numerator - &quotient * denominator;
    Ok((quotient, remainder))
}

fn map_value_error(error: ValueError) -> MarginError {
    MarginError::Calculation(error.to_string())
}
