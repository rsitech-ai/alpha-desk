use domain_types::{AccountId, Direction, MarketId, ProbabilityPpm, ScenarioId, UsdAmount};
use feature_core::{FeatureValue, HealthAssessment, HealthState, MissingReason};
use serde::{Deserialize, Serialize};

use crate::{
    MarketError, ObservationStatus,
    fragility::{apply_collateral_contagion, forced_impact_bps, liquidate_accounts, path_variant},
    hash::digest,
    market_feature_key,
    math::USD_SCALE,
    sentiment::MarketFeatureSnapshot,
};

pub const DEFAULT_SHOCKS_BPS: [i64; 12] =
    [-25, -50, -100, -200, -300, -500, 25, 50, 100, 200, 300, 500];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SimulatedMarginMode {
    IsolatedExact,
    PortfolioUncertain { band_bps: u32 },
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatedAccount {
    pub account_id: AccountId,
    pub market_id: MarketId,
    pub side: Direction,
    pub notional: UsdAmount,
    pub distance_to_maintenance_bps: i64,
    pub margin_mode: SimulatedMarginMode,
    pub collateral_group: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulatedBook {
    pub executable_depth: UsdAmount,
    pub health: HealthAssessment,
    pub observation: ObservationStatus,
}

impl SimulatedBook {
    #[must_use]
    pub fn observed(executable_depth: UsdAmount, health: HealthAssessment) -> Self {
        Self {
            executable_depth,
            health,
            observation: ObservationStatus::Observed,
        }
    }

    pub fn missing(reason: MissingReason) -> Result<Self, MarketError> {
        Ok(Self {
            executable_depth: UsdAmount::from_raw(0, USD_SCALE)?,
            health: HealthAssessment::try_new("book", HealthState::Amber, reason.as_wire_name())?,
            observation: ObservationStatus::Missing(reason),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragilityScenario {
    pub scenario_id: ScenarioId,
    pub shocks_bps: Vec<i64>,
    pub max_iterations: u32,
    pub max_total_impact_bps: i64,
    pub liquidation_participation: ProbabilityPpm,
    pub book_stress_multiplier: ProbabilityPpm,
}

impl FragilityScenario {
    pub fn from_toml(text: &str) -> Result<Self, MarketError> {
        let raw: RawScenario = toml::from_str(text).map_err(|_| MarketError::Malformed {
            what: "fragility_scenario",
            reason: "toml parse failed",
        })?;
        if raw.max_iterations == 0 || raw.shocks_bps.is_empty() {
            return Err(MarketError::Malformed {
                what: "fragility_scenario",
                reason: "shocks and iterations required",
            });
        }
        Ok(Self {
            scenario_id: ScenarioId::new(raw.scenario_id)?,
            shocks_bps: raw.shocks_bps,
            max_iterations: raw.max_iterations,
            max_total_impact_bps: raw.max_total_impact_bps,
            liquidation_participation: ProbabilityPpm::from_ppm(raw.liquidation_participation_ppm)?,
            book_stress_multiplier: ProbabilityPpm::from_ppm(raw.book_stress_multiplier_ppm)?,
        })
    }

    #[must_use]
    pub fn default_grid(scenario_id: ScenarioId) -> Self {
        Self {
            scenario_id,
            shocks_bps: DEFAULT_SHOCKS_BPS.to_vec(),
            max_iterations: 8,
            max_total_impact_bps: 1_500,
            liquidation_participation: ProbabilityPpm::ONE,
            book_stress_multiplier: ProbabilityPpm::ONE,
        }
    }
}

#[derive(Deserialize)]
struct RawScenario {
    scenario_id: String,
    shocks_bps: Vec<i64>,
    max_iterations: u32,
    max_total_impact_bps: i64,
    liquidation_participation_ppm: u32,
    book_stress_multiplier_ppm: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScenarioPathResult {
    pub terminal_price_change_bps: i64,
    pub waves: Vec<crate::fragility::LiquidationWave>,
    pub total_forced_notional: UsdAmount,
    pub absorbed_notional: UsdAmount,
    pub vulnerable_notional_remaining: UsdAmount,
    pub iteration_limit_reached: bool,
    pub health: HealthAssessment,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FragilityResult {
    pub low: ScenarioPathResult,
    pub base: ScenarioPathResult,
    pub high: ScenarioPathResult,
    pub confidence: ProbabilityPpm,
    pub missing_inputs: Vec<String>,
    pub provenance_hash: [u8; 32],
}

pub fn simulate_fragility(
    scenario: &FragilityScenario,
    accounts: &[SimulatedAccount],
    book: &SimulatedBook,
    shock_bps: i64,
) -> Result<FragilityResult, MarketError> {
    let mut missing = Vec::new();
    if matches!(book.observation, ObservationStatus::Missing(_))
        || book.health.state == HealthState::Red
    {
        missing.push("book".to_owned());
    }
    if accounts
        .iter()
        .any(|account| matches!(account.margin_mode, SimulatedMarginMode::Unsupported))
    {
        missing.push("margin_model".to_owned());
    }
    let low = simulate_path(scenario, accounts, book, shock_bps, -1)?;
    let base = simulate_path(scenario, accounts, book, shock_bps, 0)?;
    let high = simulate_path(scenario, accounts, book, shock_bps, 1)?;
    let confidence = if missing.is_empty() {
        ProbabilityPpm::from_ppm(800_000)?
    } else {
        ProbabilityPpm::ZERO
    };
    let provenance_hash = digest(&[
        scenario.scenario_id.as_str().as_bytes(),
        &shock_bps.to_le_bytes(),
        &base.terminal_price_change_bps.to_le_bytes(),
        &base.total_forced_notional.raw().to_le_bytes(),
        &u32::from(base.iteration_limit_reached).to_le_bytes(),
    ]);
    Ok(FragilityResult {
        low,
        base,
        high,
        confidence,
        missing_inputs: missing,
        provenance_hash,
    })
}

pub fn simulate_fragility_from_snapshot(
    snapshot: &MarketFeatureSnapshot,
    scenario: &FragilityScenario,
    accounts: &[SimulatedAccount],
    shock_bps: i64,
) -> Result<FragilityResult, MarketError> {
    snapshot.require_observed_book_and_fills()?;
    let book_key = market_feature_key("book")?;
    let executable_depth = match snapshot.values.get(&book_key) {
        Some(FeatureValue::Decimal { raw, scale }) => {
            let scale = u8::try_from(*scale).map_err(|_| MarketError::OutOfRange)?;
            UsdAmount::from_raw(*raw, scale)?
        }
        Some(FeatureValue::Missing(_)) | None => {
            return Err(MarketError::MissingInput { name: "book" });
        }
        Some(FeatureValue::SignedInteger(_))
        | Some(FeatureValue::UnsignedInteger(_))
        | Some(FeatureValue::ProbabilityPpm(_))
        | Some(FeatureValue::Category(_))
        | Some(FeatureValue::Boolean(_)) => {
            return Err(MarketError::Malformed {
                what: "book",
                reason: "observed book must be decimal executable depth",
            });
        }
    };
    let book = SimulatedBook::observed(executable_depth, snapshot.health.clone());
    simulate_fragility(scenario, accounts, &book, shock_bps)
}

pub fn simulate_path(
    scenario: &FragilityScenario,
    accounts: &[SimulatedAccount],
    book: &SimulatedBook,
    shock_bps: i64,
    bound_sign: i32,
) -> Result<ScenarioPathResult, MarketError> {
    match book.observation {
        ObservationStatus::Missing(_) => {
            if book.executable_depth.raw() != 0 {
                return Err(MarketError::Malformed {
                    what: "book",
                    reason: "missing book cannot carry executable depth",
                });
            }
            return red_path(accounts, &book.health);
        }
        ObservationStatus::Observed => {}
    }
    if book.health.state == HealthState::Red {
        return red_path(accounts, &book.health);
    }
    if accounts.is_empty() {
        return Err(MarketError::InsufficientHistory { what: "fragility" });
    }
    let scale = accounts[0].notional.scale();
    let mut remaining: Vec<SimulatedAccount> = accounts
        .iter()
        .cloned()
        .map(|account| path_variant(account, bound_sign))
        .collect::<Result<_, _>>()?;
    if remaining
        .iter()
        .any(|account| matches!(account.margin_mode, SimulatedMarginMode::Unsupported))
    {
        return red_path(accounts, &book.health);
    }
    apply_price_move(&mut remaining, shock_bps)?;
    let mut waves = Vec::new();
    let mut total_forced = UsdAmount::from_raw(0, scale)?;
    let mut cumulative = shock_bps;
    let mut iteration_limit_reached = false;
    for iteration in 1..=scenario.max_iterations {
        let (liquidated, survivors) = liquidate_accounts(&remaining)?;
        remaining = survivors;
        if liquidated.is_empty() {
            break;
        }
        let forced = liquidated
            .iter()
            .try_fold(UsdAmount::from_raw(0, scale)?, |acc, account| {
                acc.checked_add(account.notional)
            })?;
        let signed_impact = wave_signed_impact(
            &liquidated,
            book,
            scenario.liquidation_participation,
            scenario.book_stress_multiplier,
        )?;
        remaining = apply_collateral_contagion(remaining, 10)?;
        apply_price_move(&mut remaining, signed_impact)?;
        cumulative = cumulative
            .checked_add(signed_impact)
            .ok_or(MarketError::Overflow)?;
        waves.push(crate::fragility::LiquidationWave {
            iteration,
            liquidated_accounts: liquidated
                .iter()
                .map(|account| account.account_id.clone())
                .collect(),
            forced_notional: forced,
            estimated_impact_bps: signed_impact,
        });
        total_forced = total_forced.checked_add(forced)?;
        if cumulative.abs() > scenario.max_total_impact_bps.abs() {
            iteration_limit_reached = true;
            break;
        }
        if iteration == scenario.max_iterations {
            let (more, _) = liquidate_accounts(&remaining)?;
            if !more.is_empty() {
                iteration_limit_reached = true;
            }
        }
    }
    let vulnerable =
        remaining
            .iter()
            .try_fold(UsdAmount::from_raw(0, scale)?, |acc, account| {
                if account.distance_to_maintenance_bps <= 50 {
                    acc.checked_add(account.notional)
                } else {
                    Ok(acc)
                }
            })?;
    let absorbed = if book.executable_depth.raw() < total_forced.raw() {
        book.executable_depth
    } else {
        total_forced
    };
    Ok(ScenarioPathResult {
        terminal_price_change_bps: cumulative,
        waves,
        total_forced_notional: total_forced,
        absorbed_notional: absorbed,
        vulnerable_notional_remaining: vulnerable,
        iteration_limit_reached,
        health: book.health.clone(),
    })
}

fn wave_signed_impact(
    liquidated: &[SimulatedAccount],
    book: &SimulatedBook,
    participation: ProbabilityPpm,
    stress: ProbabilityPpm,
) -> Result<i64, MarketError> {
    let scale = liquidated[0].notional.scale();
    let mut long = UsdAmount::from_raw(0, scale)?;
    let mut short = UsdAmount::from_raw(0, scale)?;
    for account in liquidated {
        match account.side {
            Direction::Long => long = long.checked_add(account.notional)?,
            Direction::Short => short = short.checked_add(account.notional)?,
            Direction::Flat => {
                return Err(MarketError::Malformed {
                    what: "fragility",
                    reason: "flat accounts cannot liquidate",
                });
            }
        }
    }
    let net = long.raw() - short.raw();
    let magnitude = UsdAmount::from_raw(net.abs(), scale)?;
    let impact = forced_impact_bps(magnitude, book.executable_depth, participation, stress)?;
    if net >= 0 { Ok(-impact) } else { Ok(impact) }
}

fn apply_price_move(accounts: &mut [SimulatedAccount], delta_bps: i64) -> Result<(), MarketError> {
    for account in accounts {
        let against = match account.side {
            Direction::Long => -delta_bps,
            Direction::Short => delta_bps,
            Direction::Flat => {
                return Err(MarketError::Malformed {
                    what: "fragility",
                    reason: "flat accounts are not simulated",
                });
            }
        };
        account.distance_to_maintenance_bps = account
            .distance_to_maintenance_bps
            .checked_sub(against)
            .ok_or(MarketError::Overflow)?;
    }
    Ok(())
}

fn red_path(
    accounts: &[SimulatedAccount],
    health: &HealthAssessment,
) -> Result<ScenarioPathResult, MarketError> {
    let scale = accounts
        .first()
        .map(|account| account.notional.scale())
        .unwrap_or(8);
    Ok(ScenarioPathResult {
        terminal_price_change_bps: 0,
        waves: Vec::new(),
        total_forced_notional: UsdAmount::from_raw(0, scale)?,
        absorbed_notional: UsdAmount::from_raw(0, scale)?,
        vulnerable_notional_remaining: UsdAmount::from_raw(0, scale)?,
        iteration_limit_reached: false,
        health: HealthAssessment::try_new(
            health.scope.clone(),
            HealthState::Red,
            "unsupported_or_red_dependency",
        )?,
    })
}
