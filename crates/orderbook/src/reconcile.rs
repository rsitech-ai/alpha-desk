use domain_types::{Price, Quantity};

use crate::book::L2Level;

pub const L2_RECONCILE_POLICY_V1: &str = "hyperliquid-alpha-desk/l2-reconcile-policy/v1";
pub const L2_RECONCILE_MAX_TIME_SKEW_MILLIS_V1: u64 = 1_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2ReconcilePolicyV1 {
    version: &'static str,
    max_time_skew_millis: u64,
    tick_size: Option<Price>,
    lot_size: Option<Quantity>,
}

impl L2ReconcilePolicyV1 {
    #[must_use]
    pub const fn exact() -> Self {
        Self {
            version: L2_RECONCILE_POLICY_V1,
            max_time_skew_millis: 0,
            tick_size: None,
            lot_size: None,
        }
    }

    #[must_use]
    pub const fn for_market(tick_size: Price, lot_size: Quantity) -> Self {
        Self {
            version: L2_RECONCILE_POLICY_V1,
            max_time_skew_millis: L2_RECONCILE_MAX_TIME_SKEW_MILLIS_V1,
            tick_size: Some(tick_size),
            lot_size: Some(lot_size),
        }
    }

    #[must_use]
    pub const fn version(&self) -> &'static str {
        self.version
    }

    #[must_use]
    pub const fn max_time_skew_millis(&self) -> u64 {
        self.max_time_skew_millis
    }

    #[must_use]
    pub const fn tick_size(&self) -> Option<Price> {
        self.tick_size
    }

    #[must_use]
    pub const fn lot_size(&self) -> Option<Quantity> {
        self.lot_size
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum L2ReconcileDecision {
    Match,
    Quarantine { reason: String },
}

pub fn reconcile_derived_l2(
    derived_bids: &[L2Level],
    derived_asks: &[L2Level],
    official_bids: &[L2Level],
    official_asks: &[L2Level],
    derived_time_millis: Option<u64>,
    official_time_millis: Option<u64>,
    policy: &L2ReconcilePolicyV1,
) -> L2ReconcileDecision {
    if let Err(reason) = timing_ok(derived_time_millis, official_time_millis, policy) {
        return L2ReconcileDecision::Quarantine { reason };
    }
    if let Err(reason) = levels_ok(derived_bids, official_bids, "bid", policy) {
        return L2ReconcileDecision::Quarantine { reason };
    }
    if let Err(reason) = levels_ok(derived_asks, official_asks, "ask", policy) {
        return L2ReconcileDecision::Quarantine { reason };
    }
    L2ReconcileDecision::Match
}

fn timing_ok(
    derived: Option<u64>,
    official: Option<u64>,
    policy: &L2ReconcilePolicyV1,
) -> Result<(), String> {
    match (derived, official) {
        (None, None) => Ok(()),
        (Some(_), None) | (None, Some(_)) => Err("official l2 clock is not comparable".to_owned()),
        (Some(left), Some(right)) => {
            let skew = left.abs_diff(right);
            if skew > policy.max_time_skew_millis {
                Err("official l2 timing skew exceeds policy".to_owned())
            } else {
                Ok(())
            }
        }
    }
}

fn levels_ok(
    derived: &[L2Level],
    official: &[L2Level],
    side: &str,
    policy: &L2ReconcilePolicyV1,
) -> Result<(), String> {
    if derived.len() != official.len() {
        return Err(format!("{side} level count diverges"));
    }
    for (want, got) in derived.iter().zip(official) {
        if want.price != got.price {
            return Err(format!("{side} price diverges"));
        }
        if want.quantity != got.quantity {
            return Err(format!("{side} size diverges"));
        }
        if want.order_count != got.order_count {
            return Err(format!("{side} order count diverges"));
        }
        if let Some(tick) = policy.tick_size
            && !aligned(got.price.raw(), tick.raw(), got.price.scale(), tick.scale())
        {
            return Err(format!("{side} price is not tick aligned"));
        }
        if let Some(lot) = policy.lot_size
            && !aligned(
                got.quantity.raw(),
                lot.raw(),
                got.quantity.scale(),
                lot.scale(),
            )
        {
            return Err(format!("{side} size is not lot aligned"));
        }
    }
    Ok(())
}

fn aligned(value: i128, unit: i128, value_scale: u8, unit_scale: u8) -> bool {
    if unit <= 0 {
        return false;
    }
    let (left, right, scale_ok) = match value_scale.cmp(&unit_scale) {
        std::cmp::Ordering::Equal => (value, unit, true),
        std::cmp::Ordering::Greater => {
            let factor = pow10(value_scale - unit_scale);
            (value, unit.saturating_mul(factor), factor != 0)
        }
        std::cmp::Ordering::Less => {
            let factor = pow10(unit_scale - value_scale);
            (value.saturating_mul(factor), unit, factor != 0)
        }
    };
    scale_ok && right != 0 && left % right == 0
}

fn pow10(exp: u8) -> i128 {
    10_i128.checked_pow(u32::from(exp)).unwrap_or(0)
}
