use domain_types::ValueError;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SimError {
    #[error("cost model omits or invalidates a required executable cost: {component}")]
    UnmodeledCost { component: &'static str },
    #[error("input contains information known after the evaluation cutoff")]
    FutureData { field: &'static str },
    #[error("simulation request is invalid: {field}")]
    InvalidRequest { field: &'static str },
    #[error("fixed-point amount is invalid")]
    InvalidAmount,
    #[error("no two-sided book is known at the order arrival time")]
    MissingArrivalBook,
    #[error("arrival-time book is stale relative to the modeled latency")]
    StaleBook,
    #[error("order was rejected by the simulated venue")]
    OrderRejected { reason: &'static str },
    #[error("position cannot close because no modeled exit triggered")]
    UnmodeledExit,
    #[error("funding schedule does not cover the open holding interval")]
    UnmodeledFunding,
    #[error("execution-sim cannot sign or place live orders")]
    TradingSignerForbidden,
    #[error("execution-sim cannot claim invented fills")]
    FillsInventedForbidden,
    #[error("execution-sim cannot claim live execution")]
    LiveExecutionForbidden,
    #[error("execution-sim fills are synthetic and are not venue fills")]
    VenueFillForbidden,
}

impl SimError {
    #[must_use]
    pub const fn reason_code(&self) -> &'static str {
        match self {
            Self::UnmodeledCost { .. } => "execution_sim.unmodeled_cost",
            Self::FutureData { .. } => "execution_sim.future_data",
            Self::InvalidRequest { .. } => "execution_sim.invalid_request",
            Self::InvalidAmount => "execution_sim.invalid_amount",
            Self::MissingArrivalBook => "execution_sim.missing_arrival_book",
            Self::StaleBook => "execution_sim.stale_book",
            Self::OrderRejected { .. } => "execution_sim.order_rejected",
            Self::UnmodeledExit => "execution_sim.unmodeled_exit",
            Self::UnmodeledFunding => "execution_sim.unmodeled_funding",
            Self::TradingSignerForbidden => "execution_sim.trading_signer_forbidden",
            Self::FillsInventedForbidden => "execution_sim.fills_invented",
            Self::LiveExecutionForbidden => "execution_sim.live_execution",
            Self::VenueFillForbidden => "execution_sim.venue_fill",
        }
    }
}

impl From<ValueError> for SimError {
    fn from(_error: ValueError) -> Self {
        Self::InvalidAmount
    }
}
