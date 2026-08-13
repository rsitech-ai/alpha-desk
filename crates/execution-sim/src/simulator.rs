use domain_types::{Direction, KnownTime, Price, ProtocolTime, Quantity, SignalId, UsdAmount};
use serde::{Deserialize, Serialize};

use crate::book::{BookSnapshot, select_book};
use crate::clock::{SimClock, add_protocol};
use crate::cost::CostModel;
use crate::error::SimError;
use crate::exit::ExitPolicy;
use crate::failure::FailureInjection;
use crate::fees::FundingSchedule;
use crate::fill::{CostedFill, FillCostParams, costed_walk};
use crate::funding::funding_over_hold;
use crate::math::{align_qty, qty_is_zero, quote_notional, zero_usd};
use crate::order::{OrderPolicy, OrderType, entry_is_buy};
use crate::portfolio::PortfolioLimits;
use crate::signal::SignalSnapshot;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimulationRequest {
    evaluation_known_at: KnownTime,
    signal: SignalSnapshot,
    books: Vec<BookSnapshot>,
    cost_model: CostModel,
    order_policy: OrderPolicy,
    exit_policy: ExitPolicy,
    portfolio: PortfolioLimits,
    failure: FailureInjection,
    #[serde(default)]
    invalidate_at: Option<ProtocolTime>,
    seed: u64,
}

impl SimulationRequest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        evaluation_known_at: KnownTime,
        signal: SignalSnapshot,
        books: Vec<BookSnapshot>,
        cost_model: CostModel,
        order_policy: OrderPolicy,
        exit_policy: ExitPolicy,
        portfolio: PortfolioLimits,
        failure: FailureInjection,
        seed: u64,
    ) -> Result<Self, SimError> {
        Self::new_with_invalidation(
            evaluation_known_at,
            signal,
            books,
            cost_model,
            order_policy,
            exit_policy,
            portfolio,
            failure,
            None,
            seed,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_invalidation(
        evaluation_known_at: KnownTime,
        signal: SignalSnapshot,
        books: Vec<BookSnapshot>,
        cost_model: CostModel,
        order_policy: OrderPolicy,
        exit_policy: ExitPolicy,
        portfolio: PortfolioLimits,
        failure: FailureInjection,
        invalidate_at: Option<ProtocolTime>,
        seed: u64,
    ) -> Result<Self, SimError> {
        let request = Self {
            evaluation_known_at,
            signal,
            books,
            cost_model,
            order_policy,
            exit_policy,
            portfolio,
            failure,
            invalidate_at,
            seed,
        };
        request.validate()?;
        Ok(request)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, SimError> {
        let request: Self =
            serde_json::from_slice(bytes).map_err(|_| SimError::InvalidRequest {
                field: "fixture.json",
            })?;
        request.validate()?;
        Ok(request)
    }

    pub fn validate(&self) -> Result<(), SimError> {
        self.cost_model.validate()?;
        self.order_policy.validate()?;
        self.signal.refuse_future(self.evaluation_known_at)?;
        if self.books.is_empty() {
            return Err(SimError::UnmodeledCost {
                component: "arrival_book",
            });
        }
        for book in &self.books {
            let (bids, asks) = book.levels_for_rebuild();
            BookSnapshot::new(book.effective_at(), book.known_at(), bids, asks)?;
            if book.known_at() > self.evaluation_known_at {
                return Err(SimError::FutureData {
                    field: "book.known_at",
                });
            }
        }
        Ok(())
    }

    #[must_use]
    pub const fn evaluation_known_at(&self) -> KnownTime {
        self.evaluation_known_at
    }

    #[must_use]
    pub const fn signal(&self) -> &SignalSnapshot {
        &self.signal
    }

    #[must_use]
    pub const fn cost_model(&self) -> &CostModel {
        &self.cost_model
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SimulationEvent {
    SignalObserved {
        at: ProtocolTime,
        signal_id: SignalId,
    },
    OrderSubmitted {
        at: ProtocolTime,
        order_type: OrderType,
    },
    OrderRested {
        at: ProtocolTime,
    },
    PartialFill {
        at: ProtocolTime,
        quantity: Quantity,
        price: Price,
    },
    Fill {
        at: ProtocolTime,
        quantity: Quantity,
        price: Price,
    },
    Cancelled {
        at: ProtocolTime,
    },
    FundingApplied {
        at: ProtocolTime,
        amount: UsdAmount,
    },
    ExitSubmitted {
        at: ProtocolTime,
        reason: &'static str,
    },
    PositionClosed {
        at: ProtocolTime,
    },
    Rejected {
        at: ProtocolTime,
        reason: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SimulationResult {
    events: Vec<SimulationEvent>,
    filled_quantity: Quantity,
    missed_quantity: Quantity,
    entry_vwap: Option<Price>,
    exit_vwap: Option<Price>,
    entry_fees: UsdAmount,
    exit_fees: UsdAmount,
    funding: UsdAmount,
    slippage: UsdAmount,
    impact: UsdAmount,
    spread_cost: UsdAmount,
    net_pnl: UsdAmount,
    trace_hash: [u8; 32],
}

impl SimulationResult {
    #[must_use]
    pub fn events(&self) -> &[SimulationEvent] {
        &self.events
    }

    #[must_use]
    pub const fn filled_quantity(&self) -> Quantity {
        self.filled_quantity
    }

    #[must_use]
    pub const fn missed_quantity(&self) -> Quantity {
        self.missed_quantity
    }

    #[must_use]
    pub const fn entry_vwap(&self) -> Option<Price> {
        self.entry_vwap
    }

    #[must_use]
    pub const fn exit_vwap(&self) -> Option<Price> {
        self.exit_vwap
    }

    #[must_use]
    pub const fn entry_fees(&self) -> UsdAmount {
        self.entry_fees
    }

    #[must_use]
    pub const fn exit_fees(&self) -> UsdAmount {
        self.exit_fees
    }

    #[must_use]
    pub const fn funding(&self) -> UsdAmount {
        self.funding
    }

    #[must_use]
    pub const fn slippage(&self) -> UsdAmount {
        self.slippage
    }

    #[must_use]
    pub const fn impact(&self) -> UsdAmount {
        self.impact
    }

    #[must_use]
    pub const fn spread_cost(&self) -> UsdAmount {
        self.spread_cost
    }

    #[must_use]
    pub const fn net_pnl(&self) -> UsdAmount {
        self.net_pnl
    }

    #[must_use]
    pub const fn trace_hash(&self) -> [u8; 32] {
        self.trace_hash
    }
}

pub fn run(request: &SimulationRequest) -> Result<SimulationResult, SimError> {
    request.validate()?;
    let mut events = Vec::new();
    let signal = request.signal();
    events.push(SimulationEvent::SignalObserved {
        at: signal.detected_at(),
        signal_id: signal.signal_id().clone(),
    });

    let mut clock = SimClock::new(signal.detected_at(), signal.known_at())?;
    let latency = request
        .cost_model
        .latency()
        .total_delay_micros(request.seed)?;
    let extra = request.failure.extra_delay_micros();
    clock.advance(latency.checked_add(extra).ok_or(SimError::InvalidAmount)?)?;

    match request.failure {
        FailureInjection::RejectOrder => {
            events.push(SimulationEvent::Rejected {
                at: clock.protocol_time(),
                reason: "injected_reject",
            });
            return Err(SimError::OrderRejected {
                reason: "injected_reject",
            });
        }
        FailureInjection::MarkBookStale => return Err(SimError::StaleBook),
        FailureInjection::RemoveBookLiquidity => {
            return Err(SimError::OrderRejected {
                reason: "injected_empty_book",
            });
        }
        FailureInjection::None | FailureInjection::DelaySubmission { .. } => {}
    }

    events.push(SimulationEvent::OrderSubmitted {
        at: clock.protocol_time(),
        order_type: request.order_policy.order_type(),
    });

    let arrival = clock.protocol_time();
    let book = select_book(&request.books, arrival, request.evaluation_known_at)?;
    let stale = u64::try_from(arrival.unix_micros() - book.effective_at().unix_micros())
        .map_err(|_| SimError::InvalidAmount)?;
    if stale > request.cost_model.latency().max_book_staleness_micros() {
        return Err(SimError::StaleBook);
    }

    let buy = entry_is_buy(signal.direction());
    if signal.direction() == Direction::Flat {
        return Err(SimError::InvalidRequest {
            field: "signal.flat_direction",
        });
    }

    if request.order_policy.order_type() == OrderType::Alo {
        let limit = request
            .order_policy
            .limit_price()
            .ok_or(SimError::UnmodeledCost {
                component: "alo_limit",
            })?;
        let crosses = if buy {
            limit >= book.best_ask()
        } else {
            limit <= book.best_bid()
        };
        if crosses {
            events.push(SimulationEvent::Rejected {
                at: arrival,
                reason: "alo_would_take",
            });
            return Err(SimError::OrderRejected {
                reason: "alo_would_take",
            });
        }
    }

    let original = align_qty(signal.requested_quantity())?;
    let depth = book.opposite_depth(buy)?;
    let walk_qty = request.portfolio.cap_quantity(original, depth)?;
    let worst_notional = match book.opposite_levels(buy).last() {
        Some(level) => quote_notional(level.price(), walk_qty)?,
        None => return Err(SimError::MissingArrivalBook),
    };
    request.portfolio.admit_notional(worst_notional)?;

    let entry = execute_entry(request, book, buy, walk_qty, original, arrival, &mut events)?;
    let missed = original.checked_sub(entry.filled_quantity)?;
    if qty_is_zero(entry.filled_quantity) {
        events.push(SimulationEvent::Cancelled { at: arrival });
        return finish(FinishInput {
            events,
            entry,
            exit: None,
            missed,
            opened_at: arrival,
            closed_at: arrival,
            direction: signal.direction(),
            funding_schedule: request.cost_model.funding(),
        });
    }

    let opened_at = arrival;
    let (exit_at, reason) = request.exit_policy.choose_exit(
        &request.books,
        opened_at,
        request.evaluation_known_at,
        buy,
        request.invalidate_at,
    )?;
    events.push(SimulationEvent::ExitSubmitted {
        at: exit_at,
        reason,
    });
    let exit_book = select_book(&request.books, exit_at, request.evaluation_known_at)?;
    let exit_stale = u64::try_from(exit_at.unix_micros() - exit_book.effective_at().unix_micros())
        .map_err(|_| SimError::InvalidAmount)?;
    if exit_stale > request.cost_model.latency().max_book_staleness_micros() {
        return Err(SimError::StaleBook);
    }
    let exit_fill = costed_walk(
        exit_book,
        !buy,
        entry.filled_quantity,
        None,
        None,
        fill_costs(request, true),
    )?;
    push_fill_events(&mut events, exit_at, &exit_fill, entry.filled_quantity);
    if qty_is_zero(exit_fill.filled_quantity) {
        return Err(SimError::UnmodeledExit);
    }
    events.push(SimulationEvent::PositionClosed { at: exit_at });
    finish(FinishInput {
        events,
        entry,
        exit: Some(exit_fill),
        missed,
        opened_at,
        closed_at: exit_at,
        direction: signal.direction(),
        funding_schedule: request.cost_model.funding(),
    })
}

fn execute_entry(
    request: &SimulationRequest,
    book: &BookSnapshot,
    buy: bool,
    requested: Quantity,
    original: Quantity,
    arrival: ProtocolTime,
    events: &mut Vec<SimulationEvent>,
) -> Result<CostedFill, SimError> {
    let limit = request.order_policy.limit_price();
    let ppm = match request.order_policy.order_type() {
        OrderType::Gtc => request.order_policy.queue_fill_ppm(request.seed)?,
        OrderType::Market | OrderType::Ioc | OrderType::Alo => None,
    };
    let immediate = costed_walk(book, buy, requested, limit, ppm, fill_costs(request, true))?;
    push_fill_events(events, arrival, &immediate, original);

    match request.order_policy.order_type() {
        OrderType::Market | OrderType::Ioc | OrderType::Alo => Ok(immediate),
        OrderType::Gtc if qty_is_zero(immediate.remaining_quantity) => Ok(immediate),
        OrderType::Gtc => {
            events.push(SimulationEvent::OrderRested { at: arrival });
            rest_gtc(request, buy, immediate, original, arrival, events)
        }
    }
}

fn rest_gtc(
    request: &SimulationRequest,
    buy: bool,
    mut fill: CostedFill,
    original: Quantity,
    arrival: ProtocolTime,
    events: &mut Vec<SimulationEvent>,
) -> Result<CostedFill, SimError> {
    let ppm = request
        .order_policy
        .queue_fill_ppm(request.seed.wrapping_add(1))?;
    let time_exit = add_protocol(arrival, request.exit_policy.time_hold_micros())?;
    for book in &request.books {
        if qty_is_zero(fill.remaining_quantity) {
            break;
        }
        if book.effective_at() <= arrival || book.effective_at() > time_exit {
            continue;
        }
        let extra = costed_walk(
            book,
            buy,
            fill.remaining_quantity,
            request.order_policy.limit_price(),
            ppm,
            fill_costs(request, false),
        )?;
        if qty_is_zero(extra.filled_quantity) {
            continue;
        }
        push_fill_events(events, book.effective_at(), &extra, original);
        fill = merge_fills(fill, extra)?;
    }
    if !qty_is_zero(fill.remaining_quantity) {
        events.push(SimulationEvent::Cancelled { at: time_exit });
    }
    Ok(fill)
}

fn merge_fills(left: CostedFill, right: CostedFill) -> Result<CostedFill, SimError> {
    let filled = left.filled_quantity.checked_add(right.filled_quantity)?;
    let notional = left.quote_notional.checked_add(right.quote_notional)?;
    let vwap = if qty_is_zero(filled) {
        None
    } else {
        let price = domain_types::Decimal::from_raw(notional.raw(), notional.scale())?
            .checked_div(
                domain_types::Decimal::from_raw(filled.raw(), filled.scale())?,
                crate::math::PRICE_SCALE,
                domain_types::RoundingMode::TowardZero,
            )?;
        Some(Price::from_raw(price.raw(), price.scale())?)
    };
    Ok(CostedFill {
        filled_quantity: filled,
        remaining_quantity: right.remaining_quantity,
        vwap,
        quote_notional: notional,
        fee: left.fee.checked_add(right.fee)?,
        slippage: left.slippage.checked_add(right.slippage)?,
        impact: left.impact.checked_add(right.impact)?,
        spread_cost: left.spread_cost.checked_add(right.spread_cost)?,
        is_taker: left.is_taker || right.is_taker,
    })
}

fn push_fill_events(
    events: &mut Vec<SimulationEvent>,
    at: ProtocolTime,
    fill: &CostedFill,
    original_request: Quantity,
) {
    if qty_is_zero(fill.filled_quantity) {
        return;
    }
    let Some(price) = fill.vwap else {
        return;
    };
    if fill.filled_quantity < original_request {
        events.push(SimulationEvent::PartialFill {
            at,
            quantity: fill.filled_quantity,
            price,
        });
    } else {
        events.push(SimulationEvent::Fill {
            at,
            quantity: fill.filled_quantity,
            price,
        });
    }
}

fn fill_costs(request: &SimulationRequest, is_taker: bool) -> FillCostParams {
    FillCostParams {
        is_taker,
        taker_fee_rate: request.cost_model.fees().taker_fee_rate(),
        maker_fee_rate: request.cost_model.fees().maker_fee_rate(),
        slippage: request.cost_model.slippage(),
        impact: request.cost_model.impact(),
    }
}

struct FinishInput<'a> {
    events: Vec<SimulationEvent>,
    entry: CostedFill,
    exit: Option<CostedFill>,
    missed: Quantity,
    opened_at: ProtocolTime,
    closed_at: ProtocolTime,
    direction: Direction,
    funding_schedule: &'a FundingSchedule,
}

fn finish(input: FinishInput<'_>) -> Result<SimulationResult, SimError> {
    let FinishInput {
        mut events,
        entry,
        exit,
        missed,
        opened_at,
        closed_at,
        direction,
        funding_schedule,
    } = input;
    let filled = entry.filled_quantity;
    let (exit_fees, exit_vwap, exit_slip, exit_impact, exit_spread, exit_notional) = match exit {
        Some(fill) => (
            fill.fee,
            fill.vwap,
            fill.slippage,
            fill.impact,
            fill.spread_cost,
            fill.quote_notional,
        ),
        None => (
            zero_usd()?,
            None,
            zero_usd()?,
            zero_usd()?,
            zero_usd()?,
            zero_usd()?,
        ),
    };
    let funding = if qty_is_zero(filled) {
        zero_usd()?
    } else {
        let amount = funding_over_hold(
            funding_schedule,
            direction,
            entry.quote_notional,
            opened_at,
            closed_at,
        )?;
        if amount.raw() != 0 {
            events.push(SimulationEvent::FundingApplied {
                at: closed_at,
                amount,
            });
        }
        amount
    };
    let slippage = entry.slippage.checked_add(exit_slip)?;
    let impact = entry.impact.checked_add(exit_impact)?;
    let spread_cost = entry.spread_cost.checked_add(exit_spread)?;
    let gross = match (entry.vwap, exit_vwap, direction) {
        (Some(_), Some(_), Direction::Long) => exit_notional.checked_sub(entry.quote_notional)?,
        (Some(_), Some(_), Direction::Short) => entry.quote_notional.checked_sub(exit_notional)?,
        (Some(_), Some(_), Direction::Flat) => {
            return Err(SimError::InvalidRequest {
                field: "result.flat",
            });
        }
        _ => zero_usd()?,
    };
    let costs = entry
        .fee
        .checked_add(exit_fees)?
        .checked_add(slippage)?
        .checked_add(impact)?;
    let net_pnl = gross.checked_sub(costs)?.checked_sub(funding)?;
    let trace_hash = hash_events(&events);
    Ok(SimulationResult {
        events,
        filled_quantity: filled,
        missed_quantity: missed,
        entry_vwap: entry.vwap,
        exit_vwap,
        entry_fees: entry.fee,
        exit_fees,
        funding,
        slippage,
        impact,
        spread_cost,
        net_pnl,
        trace_hash,
    })
}

fn hash_events(events: &[SimulationEvent]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"hl.execution-sim.trace.v1");
    if let Ok(encoded) = serde_json::to_vec(events) {
        hasher.update(&encoded);
    }
    *hasher.finalize().as_bytes()
}
