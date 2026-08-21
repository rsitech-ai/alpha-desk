const OFFICIAL_REST_WEIGHT_PER_MINUTE: u32 = 1_200;
const WEIGHT_2: &[&str] = &[
    "allMids",
    "clearinghouseState",
    "exchangeStatus",
    "l2Book",
    "orderStatus",
    "spotClearinghouseState",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SchedulePriority {
    P0,
    P1,
    P2,
    P3,
    P4,
    P5,
}

impl SchedulePriority {
    #[must_use]
    pub const fn is_protected(self) -> bool {
        matches!(self, Self::P0 | Self::P1)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VariableCost {
    Window,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestCost {
    base: u32,
    variable: Option<VariableCost>,
}

impl RequestCost {
    #[must_use]
    pub const fn base(self) -> u32 {
        self.base
    }

    #[must_use]
    pub const fn variable(self) -> Option<VariableCost> {
        self.variable
    }

    #[must_use]
    pub fn estimated_weight(self, estimated_rows: u32) -> u32 {
        match self.variable {
            None => self.base,
            Some(VariableCost::Window) => self.base.saturating_add(estimated_rows),
        }
    }

    #[must_use]
    pub fn actual_weight(self, actual_rows: u32) -> u32 {
        self.estimated_weight(actual_rows)
    }
}

pub fn spec_12_1_base_weight(identifier: &str) -> u32 {
    if WEIGHT_2.contains(&identifier) {
        2
    } else if identifier == "userRole" {
        60
    } else {
        20
    }
}

pub fn parse_request_cost(spec: &str) -> Result<RequestCost, BudgetError> {
    let mut parts = spec.split_whitespace();
    let base_part = parts.next().ok_or(BudgetError::InvalidCost)?;
    let digits = base_part
        .strip_prefix("base:")
        .ok_or(BudgetError::InvalidCost)?;
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(BudgetError::InvalidCost);
    }
    let base = digits
        .parse::<u32>()
        .map_err(|_| BudgetError::InvalidCost)?;
    if base == 0 {
        return Err(BudgetError::InvalidCost);
    }
    let variable = match parts.next() {
        None => None,
        Some("variable:window") => Some(VariableCost::Window),
        Some(_) => return Err(BudgetError::InvalidCost),
    };
    if parts.next().is_some() {
        return Err(BudgetError::InvalidCost);
    }
    Ok(RequestCost { base, variable })
}

pub fn request_cost_for_identifier(
    identifier: &str,
    spec: &str,
) -> Result<RequestCost, BudgetError> {
    let cost = parse_request_cost(spec)?;
    if cost.base != spec_12_1_base_weight(identifier) {
        return Err(BudgetError::CostMismatch);
    }
    Ok(cost)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetLease {
    egress_id: String,
    job_id: String,
    reserved: u32,
    from_priority: u32,
    from_general: u32,
    protected: bool,
}

impl BudgetLease {
    #[must_use]
    pub fn egress_id(&self) -> &str {
        &self.egress_id
    }

    #[must_use]
    pub fn job_id(&self) -> &str {
        &self.job_id
    }

    #[must_use]
    pub const fn reserved(&self) -> u32 {
        self.reserved
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EgressBudgetSnapshot {
    egress_id: String,
    ceiling_weight_per_minute: u32,
    envelope_weight_per_minute: u32,
    available_priority: u32,
    available_general: u32,
    circuit_open_until_millis: Option<u64>,
    http_429_count: u64,
    requests_ok: u64,
}

impl EgressBudgetSnapshot {
    #[must_use]
    pub fn egress_id(&self) -> &str {
        &self.egress_id
    }

    #[must_use]
    pub const fn ceiling_weight_per_minute(&self) -> u32 {
        self.ceiling_weight_per_minute
    }

    #[must_use]
    pub const fn envelope_weight_per_minute(&self) -> u32 {
        self.envelope_weight_per_minute
    }

    #[must_use]
    pub const fn available_priority(&self) -> u32 {
        self.available_priority
    }

    #[must_use]
    pub const fn available_general(&self) -> u32 {
        self.available_general
    }

    #[must_use]
    pub const fn available_total(&self) -> u32 {
        self.available_priority
            .saturating_add(self.available_general)
    }

    #[must_use]
    pub const fn circuit_open_until_millis(&self) -> Option<u64> {
        self.circuit_open_until_millis
    }

    #[must_use]
    pub const fn http_429_count(&self) -> u64 {
        self.http_429_count
    }

    #[must_use]
    pub const fn requests_ok(&self) -> u64 {
        self.requests_ok
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestBudget {
    egress_id: String,
    ceiling: u32,
    envelope: u32,
    priority_cap: u32,
    general_cap: u32,
    available_priority: u32,
    available_general: u32,
    priority_remainder: u64,
    general_remainder: u64,
    last_refill_millis: u64,
    circuit_open_until_millis: Option<u64>,
    consecutive_429: u32,
    http_429_count: u64,
    requests_ok: u64,
    seed: u64,
}

impl RequestBudget {
    pub fn try_new(
        egress_id: impl Into<String>,
        ceiling: u32,
        envelope_percent: u8,
        priority_reserve_percent: u8,
        now_millis: u64,
        seed: u64,
    ) -> Result<Self, BudgetError> {
        let egress_id = egress_id.into();
        if egress_id.is_empty() || egress_id.trim() != egress_id {
            return Err(BudgetError::InvalidConfig);
        }
        if ceiling == 0
            || !(70..=80).contains(&envelope_percent)
            || !(1..=90).contains(&priority_reserve_percent)
        {
            return Err(BudgetError::InvalidConfig);
        }
        let envelope = u32::try_from(u64::from(ceiling) * u64::from(envelope_percent) / 100)
            .map_err(|_| BudgetError::InvalidConfig)?;
        if envelope == 0 {
            return Err(BudgetError::InvalidConfig);
        }
        let priority_cap =
            u32::try_from(u64::from(envelope) * u64::from(priority_reserve_percent) / 100)
                .map_err(|_| BudgetError::InvalidConfig)?;
        if priority_cap == 0 || priority_cap >= envelope {
            return Err(BudgetError::InvalidConfig);
        }
        let general_cap = envelope - priority_cap;
        Ok(Self {
            egress_id,
            ceiling,
            envelope,
            priority_cap,
            general_cap,
            available_priority: priority_cap,
            available_general: general_cap,
            priority_remainder: 0,
            general_remainder: 0,
            last_refill_millis: now_millis,
            circuit_open_until_millis: None,
            consecutive_429: 0,
            http_429_count: 0,
            requests_ok: 0,
            seed,
        })
    }

    pub fn official(
        egress_id: impl Into<String>,
        envelope_percent: u8,
        now_millis: u64,
        seed: u64,
    ) -> Result<Self, BudgetError> {
        Self::try_new(
            egress_id,
            OFFICIAL_REST_WEIGHT_PER_MINUTE,
            envelope_percent,
            40,
            now_millis,
            seed,
        )
    }

    #[must_use]
    pub fn egress_id(&self) -> &str {
        &self.egress_id
    }

    #[must_use]
    pub const fn envelope(&self) -> u32 {
        self.envelope
    }

    #[must_use]
    pub const fn ceiling(&self) -> u32 {
        self.ceiling
    }

    pub fn snapshot(&mut self, now_millis: u64) -> EgressBudgetSnapshot {
        self.refill(now_millis);
        EgressBudgetSnapshot {
            egress_id: self.egress_id.clone(),
            ceiling_weight_per_minute: self.ceiling,
            envelope_weight_per_minute: self.envelope,
            available_priority: self.available_priority,
            available_general: self.available_general,
            circuit_open_until_millis: self.circuit_open_until_millis,
            http_429_count: self.http_429_count,
            requests_ok: self.requests_ok,
        }
    }

    pub fn reserve(
        &mut self,
        now_millis: u64,
        job_id: &str,
        priority: SchedulePriority,
        cost: u32,
    ) -> Result<BudgetLease, BudgetError> {
        self.refill(now_millis);
        self.ensure_circuit(now_millis)?;
        if cost == 0 || job_id.is_empty() {
            return Err(BudgetError::InvalidLease);
        }
        let (from_priority, from_general) = if priority.is_protected() {
            let from_priority = cost.min(self.available_priority);
            let from_general = cost - from_priority;
            if from_general > self.available_general {
                return Err(BudgetError::Insufficient);
            }
            (from_priority, from_general)
        } else {
            if cost > self.available_general {
                return Err(BudgetError::Insufficient);
            }
            (0, cost)
        };
        self.available_priority -= from_priority;
        self.available_general -= from_general;
        Ok(BudgetLease {
            egress_id: self.egress_id.clone(),
            job_id: job_id.to_owned(),
            reserved: cost,
            from_priority,
            from_general,
            protected: priority.is_protected(),
        })
    }

    pub fn commit(
        &mut self,
        now_millis: u64,
        lease: BudgetLease,
        actual: u32,
    ) -> Result<(), BudgetError> {
        self.refill(now_millis);
        if lease.egress_id != self.egress_id {
            return Err(BudgetError::LeaseMismatch);
        }
        if actual <= lease.reserved {
            let refund = lease.reserved - actual;
            self.refund(lease.from_priority, lease.from_general, refund);
        } else {
            let extra = actual - lease.reserved;
            self.charge_extra(lease.protected, extra)?;
        }
        self.consecutive_429 = 0;
        self.requests_ok = self.requests_ok.saturating_add(1);
        Ok(())
    }

    pub fn release(&mut self, now_millis: u64, lease: BudgetLease) -> Result<(), BudgetError> {
        self.refill(now_millis);
        if lease.egress_id != self.egress_id {
            return Err(BudgetError::LeaseMismatch);
        }
        self.refund(lease.from_priority, lease.from_general, lease.reserved);
        Ok(())
    }

    pub fn on_429(&mut self, now_millis: u64, lease: BudgetLease) -> Result<u64, BudgetError> {
        self.refill(now_millis);
        if lease.egress_id != self.egress_id {
            return Err(BudgetError::LeaseMismatch);
        }
        self.http_429_count = self.http_429_count.saturating_add(1);
        self.consecutive_429 = self.consecutive_429.saturating_add(1);
        self.available_priority = 0;
        self.available_general = 0;
        self.priority_remainder = 0;
        self.general_remainder = 0;
        let backoff = self.backoff_millis(self.consecutive_429);
        let open_until = now_millis.saturating_add(backoff);
        self.circuit_open_until_millis = Some(open_until);
        Ok(open_until)
    }

    fn refund(&mut self, from_priority: u32, from_general: u32, amount: u32) {
        if amount == 0 {
            return;
        }
        let to_general = amount.min(from_general);
        let to_priority = (amount - to_general).min(from_priority);
        self.available_general = self
            .general_cap
            .min(self.available_general.saturating_add(to_general));
        self.available_priority = self
            .priority_cap
            .min(self.available_priority.saturating_add(to_priority));
    }

    fn charge_extra(&mut self, protected: bool, extra: u32) -> Result<(), BudgetError> {
        if extra == 0 {
            return Ok(());
        }
        if protected {
            let from_priority = extra.min(self.available_priority);
            let from_general = extra - from_priority;
            if from_general > self.available_general {
                return Err(BudgetError::Insufficient);
            }
            self.available_priority -= from_priority;
            self.available_general -= from_general;
            Ok(())
        } else if extra > self.available_general {
            Err(BudgetError::Insufficient)
        } else {
            self.available_general -= extra;
            Ok(())
        }
    }

    fn refill(&mut self, now_millis: u64) {
        if now_millis < self.last_refill_millis {
            self.last_refill_millis = now_millis;
            self.priority_remainder = 0;
            self.general_remainder = 0;
            return;
        }
        let elapsed = now_millis - self.last_refill_millis;
        if elapsed == 0 {
            return;
        }
        self.available_priority = refill_pool(
            self.available_priority,
            self.priority_cap,
            elapsed,
            &mut self.priority_remainder,
        );
        self.available_general = refill_pool(
            self.available_general,
            self.general_cap,
            elapsed,
            &mut self.general_remainder,
        );
        self.last_refill_millis = now_millis;
        if let Some(until) = self.circuit_open_until_millis
            && now_millis >= until
        {
            self.circuit_open_until_millis = None;
        }
    }

    fn ensure_circuit(&self, now_millis: u64) -> Result<(), BudgetError> {
        match self.circuit_open_until_millis {
            Some(until) if now_millis < until => Err(BudgetError::CircuitOpen),
            _ => Ok(()),
        }
    }

    fn backoff_millis(&self, attempt: u32) -> u64 {
        let factor = match attempt.min(5) {
            0 => 1,
            1 => 2,
            2 => 4,
            3 => 8,
            4 => 16,
            _ => 32,
        };
        let base = 1_000u64.saturating_mul(factor);
        let jitter = self
            .seed
            .wrapping_add(u64::from(attempt))
            .wrapping_mul(0x9E37_79B9_7F4A_7C15);
        base.saturating_add(jitter % (base / 4 + 1))
    }
}

fn refill_pool(available: u32, cap: u32, elapsed_millis: u64, remainder: &mut u64) -> u32 {
    if cap == 0 {
        *remainder = 0;
        return 0;
    }
    if available >= cap {
        *remainder = 0;
        return cap;
    }
    let credit = u64::from(cap)
        .saturating_mul(elapsed_millis)
        .saturating_add(*remainder);
    let add = u32::try_from(credit / 60_000).unwrap_or(u32::MAX);
    *remainder = credit % 60_000;
    let next = cap.min(available.saturating_add(add));
    if next >= cap {
        *remainder = 0;
    }
    next
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum BudgetError {
    #[error("info request cost specification is invalid")]
    InvalidCost,
    #[error("info request cost does not match spec §12.1")]
    CostMismatch,
    #[error("info request budget configuration is invalid")]
    InvalidConfig,
    #[error("info request budget has insufficient weight")]
    Insufficient,
    #[error("info request budget circuit is open after 429")]
    CircuitOpen,
    #[error("info request lease does not belong to this egress")]
    LeaseMismatch,
    #[error("info request lease is invalid")]
    InvalidLease,
}

impl BudgetError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidCost => "capture_info.invalid_cost",
            Self::CostMismatch => "capture_info.cost_mismatch",
            Self::InvalidConfig => "capture_info.invalid_budget",
            Self::Insufficient => "capture_info.insufficient_budget",
            Self::CircuitOpen => "capture_info.circuit_open",
            Self::LeaseMismatch => "capture_info.lease_mismatch",
            Self::InvalidLease => "capture_info.invalid_lease",
        }
    }
}
