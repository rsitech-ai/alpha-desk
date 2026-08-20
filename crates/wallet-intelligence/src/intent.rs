use domain_types::{ProbabilityPpm, ProtocolTime};
use serde::{Deserialize, Serialize};

use crate::{IntelligenceError, math::allocate_ppm};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IntentClass {
    OpenDirectional,
    AddDirectional,
    ReduceRisk,
    CloseDirectional,
    HedgeExistingExposure,
    CarryOrBasis,
    MarketMakerInventory,
    LiquidationOrForced,
    TransferOrAccountRebalance,
    Unknown,
}

impl IntentClass {
    pub const ALL: [Self; 10] = [
        Self::OpenDirectional,
        Self::AddDirectional,
        Self::ReduceRisk,
        Self::CloseDirectional,
        Self::HedgeExistingExposure,
        Self::CarryOrBasis,
        Self::MarketMakerInventory,
        Self::LiquidationOrForced,
        Self::TransferOrAccountRebalance,
        Self::Unknown,
    ];

    #[must_use]
    pub const fn as_wire_name(self) -> &'static str {
        match self {
            Self::OpenDirectional => "open_directional",
            Self::AddDirectional => "add_directional",
            Self::ReduceRisk => "reduce_risk",
            Self::CloseDirectional => "close_directional",
            Self::HedgeExistingExposure => "hedge_existing_exposure",
            Self::CarryOrBasis => "carry_or_basis",
            Self::MarketMakerInventory => "market_maker_inventory",
            Self::LiquidationOrForced => "liquidation_or_forced",
            Self::TransferOrAccountRebalance => "transfer_or_account_rebalance",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentFeatures {
    pub position_was_flat: Option<bool>,
    pub size_increased: Option<bool>,
    pub size_decreased: Option<bool>,
    pub leverage_decreased: Option<bool>,
    pub maker_inventory: Option<bool>,
    pub carry_or_basis: Option<bool>,
    pub liquidation: Option<bool>,
    pub transfer: Option<bool>,
    pub hedge_evidence: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntentSnapshot {
    pub effective_at: ProtocolTime,
    pub probabilities: Vec<(IntentClass, ProbabilityPpm)>,
    pub missing_critical_inputs: bool,
}

pub fn classify_intent(
    features: &IntentFeatures,
    effective_at: ProtocolTime,
) -> Result<IntentSnapshot, IntelligenceError> {
    let missing = features.position_was_flat.is_none() && features.size_increased.is_none();
    let mut weights = [1_u128; 10];
    if missing {
        weights[intent_index(IntentClass::Unknown)] = 70;
    }
    match (
        features.position_was_flat,
        features.size_increased,
        features.size_decreased,
    ) {
        (Some(true), Some(true), _) => bump(&mut weights, IntentClass::OpenDirectional, 40),
        (Some(false), Some(true), _) => bump(&mut weights, IntentClass::AddDirectional, 40),
        (_, _, Some(true)) if features.leverage_decreased == Some(true) => {
            bump(&mut weights, IntentClass::ReduceRisk, 40);
        }
        (Some(false), _, Some(true)) => bump(&mut weights, IntentClass::CloseDirectional, 30),
        _ => {}
    }
    if features.hedge_evidence == Some(true) {
        bump(&mut weights, IntentClass::HedgeExistingExposure, 35);
    }
    if features.carry_or_basis == Some(true) {
        bump(&mut weights, IntentClass::CarryOrBasis, 35);
    }
    if features.maker_inventory == Some(true) {
        bump(&mut weights, IntentClass::MarketMakerInventory, 35);
    }
    if features.liquidation == Some(true) {
        bump(&mut weights, IntentClass::LiquidationOrForced, 50);
    }
    if features.transfer == Some(true) {
        bump(&mut weights, IntentClass::TransferOrAccountRebalance, 50);
    }
    let allocated = allocate_ppm(&weights)?;
    let probabilities = IntentClass::ALL
        .into_iter()
        .zip(allocated)
        .collect::<Vec<_>>();
    let total: u32 = probabilities.iter().map(|(_, ppm)| ppm.ppm()).sum();
    if total != 1_000_000 {
        return Err(IntelligenceError::Malformed {
            what: "intent",
            reason: "probabilities must sum to 1_000_000 ppm",
        });
    }
    Ok(IntentSnapshot {
        effective_at,
        probabilities,
        missing_critical_inputs: missing,
    })
}

fn bump(weights: &mut [u128; 10], class: IntentClass, amount: u128) {
    weights[intent_index(class)] = weights[intent_index(class)].saturating_add(amount);
}

fn intent_index(class: IntentClass) -> usize {
    match class {
        IntentClass::OpenDirectional => 0,
        IntentClass::AddDirectional => 1,
        IntentClass::ReduceRisk => 2,
        IntentClass::CloseDirectional => 3,
        IntentClass::HedgeExistingExposure => 4,
        IntentClass::CarryOrBasis => 5,
        IntentClass::MarketMakerInventory => 6,
        IntentClass::LiquidationOrForced => 7,
        IntentClass::TransferOrAccountRebalance => 8,
        IntentClass::Unknown => 9,
    }
}
