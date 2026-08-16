use domain_types::{
    AssetId, BasisPoints, BlockHeight, Decimal, DexId, FeatureSetVersion, KnownTime, MarketId,
    ProbabilityPpm, ProtocolTime, RegimeId, RoundingMode, UsdAmount,
};
use feature_core::HealthState;
use serde::{Deserialize, Serialize};

use crate::{
    Applicability, ApplicabilitySupport, ExternalCashFlow, IntelligenceError, IntelligenceSubject,
    math::integer_sqrt,
};

pub const DEFAULT_USD_SCALE: u8 = 8;
pub const DEFAULT_RETURN_SCALE: u8 = 12;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EquityObservation {
    pub protocol_time: ProtocolTime,
    pub equity: UsdAmount,
    pub realized_pnl: UsdAmount,
    pub unrealized_pnl: UsdAmount,
    pub fees: UsdAmount,
    pub funding: UsdAmount,
    pub gross_exposure: UsdAmount,
    pub capital_at_risk: UsdAmount,
}

impl EquityObservation {
    #[allow(clippy::too_many_arguments)]
    pub fn try_new(
        protocol_time: ProtocolTime,
        equity: UsdAmount,
        realized_pnl: UsdAmount,
        unrealized_pnl: UsdAmount,
        fees: UsdAmount,
        funding: UsdAmount,
        gross_exposure: UsdAmount,
        capital_at_risk: UsdAmount,
    ) -> Result<Self, IntelligenceError> {
        let scale = equity.scale();
        for amount in [
            realized_pnl,
            unrealized_pnl,
            fees,
            funding,
            gross_exposure,
            capital_at_risk,
        ] {
            if amount.scale() != scale {
                return Err(IntelligenceError::ScaleMismatch);
            }
        }
        if gross_exposure.raw() < 0 || capital_at_risk.raw() < 0 {
            return Err(IntelligenceError::Malformed {
                what: "equity_observation",
                reason: "exposures must be non-negative",
            });
        }
        Ok(Self {
            protocol_time,
            equity,
            realized_pnl,
            unrealized_pnl,
            fees,
            funding,
            gross_exposure,
            capital_at_risk,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcentrationInput {
    pub asset_pnl: Vec<(AssetId, UsdAmount)>,
    pub dex_pnl: Vec<(DexId, UsdAmount)>,
    pub collateral_pnl: Vec<(AssetId, UsdAmount)>,
    pub regime_pnl: Vec<(RegimeId, UsdAmount)>,
    pub trade_pnl: Vec<UsdAmount>,
    pub month_pnl: Vec<UsdAmount>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConcentrationBreakdown {
    pub asset_hhi_ppm: ProbabilityPpm,
    pub dex_hhi_ppm: ProbabilityPpm,
    pub collateral_hhi_ppm: Option<ProbabilityPpm>,
    pub regime_hhi_ppm: Option<ProbabilityPpm>,
    pub best_trade_share: ProbabilityPpm,
    pub best_month_share: ProbabilityPpm,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PerformanceSnapshot {
    pub subject: IntelligenceSubject,
    pub feature_set_version: FeatureSetVersion,
    pub effective_at: ProtocolTime,
    pub known_at: KnownTime,
    pub input_watermark: BlockHeight,
    pub starting_equity: UsdAmount,
    pub ending_equity: UsdAmount,
    pub net_external_cash_flow: UsdAmount,
    pub trading_gain: UsdAmount,
    pub realized_pnl: UsdAmount,
    pub unrealized_pnl: UsdAmount,
    pub fees: UsdAmount,
    pub funding: UsdAmount,
    pub time_weighted_return: Decimal,
    pub money_weighted_return: Option<Decimal>,
    pub max_drawdown: Decimal,
    pub recovery_duration_micros: Option<i64>,
    pub expected_shortfall: Option<Decimal>,
    pub downside_deviation: Option<Decimal>,
    pub turnover: Decimal,
    pub utilization: Decimal,
    pub observation_count: u64,
    pub data_health: HealthState,
    pub applicability: Applicability,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerformanceLedger {
    subject: IntelligenceSubject,
    usd_scale: u8,
    return_scale: u8,
    events: Vec<LedgerEvent>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum LedgerEvent {
    Observation(EquityObservation),
    CashFlow(ExternalCashFlow),
}

impl PerformanceLedger {
    pub fn try_new(
        subject: IntelligenceSubject,
        usd_scale: u8,
        return_scale: u8,
    ) -> Result<Self, IntelligenceError> {
        if usd_scale == 0 || return_scale == 0 {
            return Err(IntelligenceError::Malformed {
                what: "performance_ledger",
                reason: "scales must be positive",
            });
        }
        Ok(Self {
            subject,
            usd_scale,
            return_scale,
            events: Vec::new(),
        })
    }

    pub fn observe(&mut self, observation: EquityObservation) -> Result<(), IntelligenceError> {
        self.require_scale(observation.equity)?;
        self.require_monotonic(observation.protocol_time)?;
        self.events.push(LedgerEvent::Observation(observation));
        Ok(())
    }

    pub fn apply_cash_flow(&mut self, flow: ExternalCashFlow) -> Result<(), IntelligenceError> {
        self.require_scale(flow.amount)?;
        self.require_monotonic(flow.protocol_time)?;
        self.events.push(LedgerEvent::CashFlow(flow));
        Ok(())
    }

    pub fn snapshot(
        &self,
        feature_set_version: FeatureSetVersion,
        known_at: KnownTime,
        input_watermark: BlockHeight,
        as_of: Option<ProtocolTime>,
    ) -> Result<PerformanceSnapshot, IntelligenceError> {
        let replay = self.replay(as_of)?;
        Ok(PerformanceSnapshot {
            subject: self.subject.clone(),
            feature_set_version,
            effective_at: replay.effective_at,
            known_at,
            input_watermark,
            starting_equity: replay.starting_equity,
            ending_equity: replay.ending_equity,
            net_external_cash_flow: replay.net_external_cash_flow,
            trading_gain: replay.trading_gain,
            realized_pnl: replay.realized_pnl,
            unrealized_pnl: replay.unrealized_pnl,
            fees: replay.fees,
            funding: replay.funding,
            time_weighted_return: replay.time_weighted_return,
            money_weighted_return: replay.money_weighted_return,
            max_drawdown: replay.max_drawdown,
            recovery_duration_micros: replay.recovery_duration_micros,
            expected_shortfall: replay.expected_shortfall,
            downside_deviation: replay.downside_deviation,
            turnover: replay.turnover,
            utilization: replay.utilization,
            observation_count: replay.observation_count,
            data_health: HealthState::Green,
            applicability: Applicability::try_new(
                Vec::new(),
                Vec::new(),
                Vec::new(),
                ApplicabilitySupport::Supported,
                Vec::new(),
            )?,
        })
    }

    fn replay(&self, as_of: Option<ProtocolTime>) -> Result<Replay, IntelligenceError> {
        let mut points = Vec::new();
        for event in &self.events {
            match event {
                LedgerEvent::Observation(observation)
                    if as_of.is_none_or(|cutoff| observation.protocol_time <= cutoff) =>
                {
                    points.push(Point::Observation(observation));
                }
                LedgerEvent::CashFlow(flow)
                    if as_of.is_none_or(|cutoff| flow.protocol_time <= cutoff) =>
                {
                    points.push(Point::CashFlow(flow));
                }
                LedgerEvent::Observation(_) | LedgerEvent::CashFlow(_) => {}
            }
        }
        points.sort_by_key(|point| {
            let (time, rank) = match point {
                Point::Observation(observation) => (observation.protocol_time, 0_u8),
                Point::CashFlow(flow) => (flow.protocol_time, 1_u8),
            };
            (time.unix_micros(), rank)
        });
        let first = points.iter().find_map(|point| match point {
            Point::Observation(observation) => Some(*observation),
            Point::CashFlow(_) => None,
        });
        let Some(first) = first else {
            return Err(IntelligenceError::InsufficientHistory {
                what: "performance_ledger",
            });
        };
        if matches!(points.first(), Some(Point::CashFlow(_))) {
            return Err(IntelligenceError::Malformed {
                what: "performance_ledger",
                reason: "cash flow before opening equity observation",
            });
        }

        let mut current_equity = first.equity;
        let mut subperiod_start = first.equity;
        let mut wealth_num = 1_i128;
        let mut wealth_den = 1_i128;
        let mut peak_wealth = one(self.return_scale)?;
        let mut max_drawdown = Decimal::from_raw(0, self.return_scale)?;
        let mut peak_time = first.protocol_time;
        let mut recovery = None;
        let mut in_drawdown = false;
        let mut net_cf = UsdAmount::from_raw(0, self.usd_scale)?;
        let mut last_obs = first.clone();
        let mut interval_returns = Vec::new();
        let mut abs_exposure_delta = UsdAmount::from_raw(0, self.usd_scale)?;
        let mut utilization_num = first.capital_at_risk.raw();
        let mut last_exposure = first.gross_exposure;
        let mut weighted_cf = 0_i128;
        let mut observation_count = 0_u64;
        let start_time = first.protocol_time;
        let end_time = match points.last() {
            Some(Point::Observation(observation)) => observation.protocol_time,
            Some(Point::CashFlow(flow)) => flow.protocol_time,
            None => first.protocol_time,
        };
        let span = end_time
            .unix_micros()
            .checked_sub(start_time.unix_micros())
            .ok_or(IntelligenceError::Overflow)?;

        for point in points {
            match point {
                Point::Observation(observation) => {
                    observation_count = observation_count
                        .checked_add(1)
                        .ok_or(IntelligenceError::Overflow)?;
                    if observation_count > 1 && current_equity.raw() != 0 {
                        let interval =
                            ratio(observation.equity, current_equity, self.return_scale)?;
                        interval_returns.push(interval.checked_sub(one(self.return_scale)?)?);
                    }
                    abs_exposure_delta = abs_exposure_delta
                        .checked_add(abs_diff(observation.gross_exposure, last_exposure)?)?;
                    last_exposure = observation.gross_exposure;
                    if observation_count > 1 {
                        utilization_num = utilization_num
                            .checked_add(observation.capital_at_risk.raw())
                            .ok_or(IntelligenceError::Overflow)?;
                    }
                    current_equity = observation.equity;
                    last_obs = observation.clone();
                    note_drawdown(
                        wealth_decimal(
                            wealth_num,
                            wealth_den,
                            current_equity,
                            subperiod_start,
                            self.return_scale,
                        )?,
                        &mut peak_wealth,
                        &mut max_drawdown,
                        &mut peak_time,
                        &mut recovery,
                        &mut in_drawdown,
                        observation.protocol_time,
                        self.return_scale,
                    )?;
                }
                Point::CashFlow(flow) => {
                    if current_equity.raw() <= 0 {
                        return Err(IntelligenceError::Malformed {
                            what: "performance_ledger",
                            reason: "cash flow against non-positive equity",
                        });
                    }
                    mul_ratio(
                        &mut wealth_num,
                        &mut wealth_den,
                        current_equity.raw(),
                        subperiod_start.raw(),
                    )?;
                    let signed = flow.signed_amount()?;
                    net_cf = net_cf.checked_add(signed)?;
                    current_equity = current_equity.checked_add(signed)?;
                    subperiod_start = current_equity;
                    if span > 0 {
                        let remaining = end_time
                            .unix_micros()
                            .checked_sub(flow.protocol_time.unix_micros())
                            .ok_or(IntelligenceError::Overflow)?;
                        weighted_cf = weighted_cf
                            .checked_add(
                                signed
                                    .raw()
                                    .checked_mul(i128::from(remaining))
                                    .and_then(|value| value.checked_div(i128::from(span)))
                                    .ok_or(IntelligenceError::Overflow)?,
                            )
                            .ok_or(IntelligenceError::Overflow)?;
                    }
                }
            }
        }
        mul_ratio(
            &mut wealth_num,
            &mut wealth_den,
            current_equity.raw(),
            subperiod_start.raw(),
        )?;
        let wealth_base = fraction_to_decimal(wealth_num, wealth_den, self.return_scale)?;
        note_drawdown(
            wealth_base,
            &mut peak_wealth,
            &mut max_drawdown,
            &mut peak_time,
            &mut recovery,
            &mut in_drawdown,
            end_time,
            self.return_scale,
        )?;

        let trading_gain = current_equity
            .checked_sub(first.equity)?
            .checked_sub(net_cf)?;
        let twr = wealth_base.checked_sub(one(self.return_scale)?)?;
        let capital = first
            .equity
            .checked_add(UsdAmount::from_raw(weighted_cf, self.usd_scale)?)?;
        let money_weighted_return = if capital.raw() <= 0 {
            None
        } else {
            Some(usd_div(trading_gain, capital, self.return_scale)?)
        };
        let n = i128::from(observation_count.max(1));
        let turnover = usd_div(abs_exposure_delta, first.equity, self.return_scale)?;
        let utilization = Decimal::from_raw(utilization_num, self.usd_scale)?.checked_div(
            Decimal::from_raw(
                last_obs
                    .equity
                    .raw()
                    .checked_mul(n)
                    .ok_or(IntelligenceError::Overflow)?,
                self.usd_scale,
            )?,
            self.return_scale,
            RoundingMode::NearestTiesToEven,
        )?;
        Ok(Replay {
            effective_at: end_time,
            starting_equity: first.equity,
            ending_equity: current_equity,
            net_external_cash_flow: net_cf,
            trading_gain,
            realized_pnl: last_obs.realized_pnl,
            unrealized_pnl: last_obs.unrealized_pnl,
            fees: last_obs.fees,
            funding: last_obs.funding,
            time_weighted_return: twr,
            money_weighted_return,
            max_drawdown,
            recovery_duration_micros: recovery,
            expected_shortfall: expected_shortfall(&interval_returns, self.return_scale)?,
            downside_deviation: downside_deviation(&interval_returns, self.return_scale)?,
            turnover,
            utilization,
            observation_count,
        })
    }

    fn require_scale(&self, amount: UsdAmount) -> Result<(), IntelligenceError> {
        if amount.scale() == self.usd_scale {
            Ok(())
        } else {
            Err(IntelligenceError::ScaleMismatch)
        }
    }

    fn require_monotonic(&self, time: ProtocolTime) -> Result<(), IntelligenceError> {
        let last = self.events.last().map(event_time);
        if last.is_some_and(|previous| previous.unix_micros() > time.unix_micros()) {
            Err(IntelligenceError::Malformed {
                what: "performance_ledger",
                reason: "events must be non-decreasing in protocol time",
            })
        } else {
            Ok(())
        }
    }
}

enum Point<'a> {
    Observation(&'a EquityObservation),
    CashFlow(&'a ExternalCashFlow),
}

struct Replay {
    effective_at: ProtocolTime,
    starting_equity: UsdAmount,
    ending_equity: UsdAmount,
    net_external_cash_flow: UsdAmount,
    trading_gain: UsdAmount,
    realized_pnl: UsdAmount,
    unrealized_pnl: UsdAmount,
    fees: UsdAmount,
    funding: UsdAmount,
    time_weighted_return: Decimal,
    money_weighted_return: Option<Decimal>,
    max_drawdown: Decimal,
    recovery_duration_micros: Option<i64>,
    expected_shortfall: Option<Decimal>,
    downside_deviation: Option<Decimal>,
    turnover: Decimal,
    utilization: Decimal,
    observation_count: u64,
}

fn event_time(event: &LedgerEvent) -> ProtocolTime {
    match event {
        LedgerEvent::Observation(observation) => observation.protocol_time,
        LedgerEvent::CashFlow(flow) => flow.protocol_time,
    }
}

fn one(scale: u8) -> Result<Decimal, IntelligenceError> {
    Decimal::from_raw(10_i128.pow(u32::from(scale)), scale).map_err(Into::into)
}

fn ratio(
    numerator: UsdAmount,
    denominator: UsdAmount,
    scale: u8,
) -> Result<Decimal, IntelligenceError> {
    if denominator.raw() == 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    Decimal::from_raw(numerator.raw(), numerator.scale())?
        .checked_div(
            Decimal::from_raw(denominator.raw(), denominator.scale())?,
            scale,
            RoundingMode::NearestTiesToEven,
        )
        .map_err(Into::into)
}

fn usd_div(
    numerator: UsdAmount,
    denominator: UsdAmount,
    scale: u8,
) -> Result<Decimal, IntelligenceError> {
    Decimal::from_raw(numerator.raw(), numerator.scale())?
        .checked_div(
            Decimal::from_raw(denominator.raw(), denominator.scale())?,
            scale,
            RoundingMode::NearestTiesToEven,
        )
        .map_err(Into::into)
}

fn mul_ratio(
    numerator: &mut i128,
    denominator: &mut i128,
    factor_num: i128,
    factor_den: i128,
) -> Result<(), IntelligenceError> {
    if factor_den == 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    *numerator = numerator
        .checked_mul(factor_num)
        .ok_or(IntelligenceError::Overflow)?;
    *denominator = denominator
        .checked_mul(factor_den)
        .ok_or(IntelligenceError::Overflow)?;
    reduce_ratio(numerator, denominator);
    Ok(())
}

fn reduce_ratio(numerator: &mut i128, denominator: &mut i128) {
    let gcd = gcd_i128(numerator.unsigned_abs(), denominator.unsigned_abs());
    if let Ok(divisor) = i128::try_from(gcd)
        && divisor > 1
    {
        *numerator /= divisor;
        *denominator /= divisor;
    }
}

fn gcd_i128(mut left: u128, mut right: u128) -> u128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

fn fraction_to_decimal(
    numerator: i128,
    denominator: i128,
    scale: u8,
) -> Result<Decimal, IntelligenceError> {
    Decimal::from_raw(numerator, 0)?
        .checked_div(
            Decimal::from_raw(denominator, 0)?,
            scale,
            RoundingMode::NearestTiesToEven,
        )
        .map_err(Into::into)
}

fn wealth_decimal(
    wealth_num: i128,
    wealth_den: i128,
    current_equity: UsdAmount,
    subperiod_start: UsdAmount,
    scale: u8,
) -> Result<Decimal, IntelligenceError> {
    let mut num = wealth_num;
    let mut den = wealth_den;
    mul_ratio(
        &mut num,
        &mut den,
        current_equity.raw(),
        subperiod_start.raw(),
    )?;
    fraction_to_decimal(num, den, scale)
}

fn abs_diff(left: UsdAmount, right: UsdAmount) -> Result<UsdAmount, IntelligenceError> {
    if left >= right {
        left.checked_sub(right).map_err(Into::into)
    } else {
        right.checked_sub(left).map_err(Into::into)
    }
}

#[allow(clippy::too_many_arguments)]
fn note_drawdown(
    current_wealth: Decimal,
    peak_wealth: &mut Decimal,
    max_drawdown: &mut Decimal,
    peak_time: &mut ProtocolTime,
    recovery: &mut Option<i64>,
    in_drawdown: &mut bool,
    time: ProtocolTime,
    scale: u8,
) -> Result<(), IntelligenceError> {
    if current_wealth > *peak_wealth {
        if *in_drawdown {
            *recovery = Some(
                time.unix_micros()
                    .checked_sub(peak_time.unix_micros())
                    .ok_or(IntelligenceError::Overflow)?,
            );
        }
        *peak_wealth = current_wealth;
        *peak_time = time;
        *in_drawdown = false;
        return Ok(());
    }
    if peak_wealth.raw() == 0 {
        return Ok(());
    }
    let drawdown = peak_wealth.checked_sub(current_wealth)?.checked_div(
        *peak_wealth,
        scale,
        RoundingMode::NearestTiesToEven,
    )?;
    if drawdown > *max_drawdown {
        *max_drawdown = drawdown;
    }
    *in_drawdown = true;
    Ok(())
}

fn expected_shortfall(
    returns: &[Decimal],
    scale: u8,
) -> Result<Option<Decimal>, IntelligenceError> {
    if returns.len() < 5 {
        return Ok(None);
    }
    let mut ordered = returns.to_vec();
    ordered.sort();
    let tail = ordered.len().div_ceil(20).max(1);
    let mut sum = Decimal::from_raw(0, scale)?;
    for value in ordered.iter().take(tail) {
        sum = sum.checked_add(*value)?;
    }
    let count = Decimal::from_raw(
        i128::try_from(tail).map_err(|_| IntelligenceError::Overflow)?,
        0,
    )?;
    Ok(Some(sum.checked_div(
        count,
        scale,
        RoundingMode::NearestTiesToEven,
    )?))
}

fn downside_deviation(
    returns: &[Decimal],
    scale: u8,
) -> Result<Option<Decimal>, IntelligenceError> {
    if returns.is_empty() {
        return Ok(None);
    }
    let zero = Decimal::from_raw(0, scale)?;
    let mut sum_sq = 0_i128;
    let mut count = 0_i128;
    for value in returns {
        if *value < zero {
            let raw = value.raw();
            sum_sq = sum_sq
                .checked_add(raw.checked_mul(raw).ok_or(IntelligenceError::Overflow)?)
                .ok_or(IntelligenceError::Overflow)?;
            count += 1;
        }
    }
    if count == 0 {
        return Ok(Some(zero));
    }
    let mean_sq = sum_sq
        .checked_div(count)
        .ok_or(IntelligenceError::DivisionByZero)?;
    let sqrt =
        integer_sqrt(u128::try_from(mean_sq.max(0)).map_err(|_| IntelligenceError::Overflow)?);
    Ok(Some(Decimal::from_raw(
        i128::try_from(sqrt).map_err(|_| IntelligenceError::Overflow)?,
        scale,
    )?))
}

pub fn concentration_breakdown(
    input: &ConcentrationInput,
) -> Result<ConcentrationBreakdown, IntelligenceError> {
    Ok(ConcentrationBreakdown {
        asset_hhi_ppm: herfindahl(&input.asset_pnl)?,
        dex_hhi_ppm: herfindahl(&input.dex_pnl)?,
        collateral_hhi_ppm: optional_herfindahl(&input.collateral_pnl)?,
        regime_hhi_ppm: optional_herfindahl(&input.regime_pnl)?,
        best_trade_share: best_share(&input.trade_pnl)?,
        best_month_share: best_share(&input.month_pnl)?,
    })
}

fn optional_herfindahl<T>(
    items: &[(T, UsdAmount)],
) -> Result<Option<ProbabilityPpm>, IntelligenceError> {
    if items.is_empty() {
        Ok(None)
    } else {
        Ok(Some(herfindahl(items)?))
    }
}

fn herfindahl<T>(items: &[(T, UsdAmount)]) -> Result<ProbabilityPpm, IntelligenceError> {
    share_metric(
        &items.iter().map(|(_, amount)| *amount).collect::<Vec<_>>(),
        true,
    )
}

fn best_share(values: &[UsdAmount]) -> Result<ProbabilityPpm, IntelligenceError> {
    share_metric(values, false)
}

fn share_metric(values: &[UsdAmount], squared: bool) -> Result<ProbabilityPpm, IntelligenceError> {
    if values.is_empty() {
        return Err(IntelligenceError::InsufficientHistory {
            what: "concentration",
        });
    }
    let mut abs_values = Vec::new();
    let mut total = 0_i128;
    for value in values {
        let abs = value
            .raw()
            .checked_abs()
            .ok_or(IntelligenceError::Overflow)?;
        total = total.checked_add(abs).ok_or(IntelligenceError::Overflow)?;
        abs_values.push(abs);
    }
    if total == 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    if squared {
        let mut hhi = 0_i128;
        for value in abs_values {
            let share = value
                .checked_mul(1_000_000)
                .and_then(|product| product.checked_div(total))
                .ok_or(IntelligenceError::Overflow)?;
            hhi = hhi
                .checked_add(
                    share
                        .checked_mul(share)
                        .and_then(|product| product.checked_div(1_000_000))
                        .ok_or(IntelligenceError::Overflow)?,
                )
                .ok_or(IntelligenceError::Overflow)?;
        }
        return ProbabilityPpm::from_ppm(
            u32::try_from(hhi).map_err(|_| IntelligenceError::Overflow)?,
        )
        .map_err(Into::into);
    }
    let best = abs_values.into_iter().max().unwrap_or(0);
    let share = best
        .checked_mul(1_000_000)
        .and_then(|product| product.checked_div(total))
        .ok_or(IntelligenceError::Overflow)?;
    ProbabilityPpm::from_ppm(u32::try_from(share).map_err(|_| IntelligenceError::Overflow)?)
        .map_err(Into::into)
}

pub fn maker_taker_mix(
    maker_notional: UsdAmount,
    taker_notional: UsdAmount,
) -> Result<ProbabilityPpm, IntelligenceError> {
    if maker_notional.scale() != taker_notional.scale() {
        return Err(IntelligenceError::ScaleMismatch);
    }
    if maker_notional.raw() < 0 || taker_notional.raw() < 0 {
        return Err(IntelligenceError::Malformed {
            what: "maker_taker_mix",
            reason: "notionals must be non-negative",
        });
    }
    let total = maker_notional
        .raw()
        .checked_add(taker_notional.raw())
        .ok_or(IntelligenceError::Overflow)?;
    if total == 0 {
        return Err(IntelligenceError::DivisionByZero);
    }
    let ppm = maker_notional
        .raw()
        .checked_mul(1_000_000)
        .and_then(|product| product.checked_div(total))
        .ok_or(IntelligenceError::Overflow)?;
    ProbabilityPpm::from_ppm(u32::try_from(ppm).map_err(|_| IntelligenceError::Overflow)?)
        .map_err(Into::into)
}

pub fn long_short_beta(
    long_pnl: UsdAmount,
    short_pnl: UsdAmount,
    market_return: BasisPoints,
) -> Result<(Decimal, Decimal), IntelligenceError> {
    if market_return.raw() == 0 {
        return Err(IntelligenceError::Unsupported {
            what: "beta_with_zero_market_return",
        });
    }
    let scale = 8_u8;
    let market = Decimal::from_raw(market_return.raw(), market_return.scale())?;
    let long = Decimal::from_raw(long_pnl.raw(), long_pnl.scale())?.checked_div(
        market,
        scale,
        RoundingMode::NearestTiesToEven,
    )?;
    let short = Decimal::from_raw(short_pnl.raw(), short_pnl.scale())?.checked_div(
        market,
        scale,
        RoundingMode::NearestTiesToEven,
    )?;
    Ok((long, short))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketBetaObservation {
    pub market_id: MarketId,
    pub long_pnl: UsdAmount,
    pub short_pnl: UsdAmount,
    pub market_return: Option<BasisPoints>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketBeta {
    pub market_id: MarketId,
    pub long_beta: Decimal,
    pub short_beta: Decimal,
}

pub fn long_short_beta_by_market(
    observations: &[MarketBetaObservation],
) -> Result<Option<Vec<MarketBeta>>, IntelligenceError> {
    if observations.is_empty() {
        return Ok(None);
    }
    let mut betas = Vec::new();
    for (index, observation) in observations.iter().enumerate() {
        if observations[..index]
            .iter()
            .any(|prior| prior.market_id == observation.market_id)
        {
            return Err(IntelligenceError::Malformed {
                what: "beta",
                reason: "duplicate market",
            });
        }
        let Some(market_return) = observation.market_return else {
            continue;
        };
        let (long_beta, short_beta) =
            long_short_beta(observation.long_pnl, observation.short_pnl, market_return)?;
        betas.push(MarketBeta {
            market_id: observation.market_id.clone(),
            long_beta,
            short_beta,
        });
    }
    if betas.is_empty() {
        return Ok(None);
    }
    betas.sort_by(|left, right| left.market_id.as_str().cmp(right.market_id.as_str()));
    Ok(Some(betas))
}

pub fn performance_before_after_capital_change(
    before: &PerformanceSnapshot,
    after: &PerformanceSnapshot,
) -> Result<(Decimal, Decimal), IntelligenceError> {
    if before.subject != after.subject {
        return Err(IntelligenceError::Malformed {
            what: "capital_split",
            reason: "subject mismatch",
        });
    }
    Ok((before.time_weighted_return, after.time_weighted_return))
}
