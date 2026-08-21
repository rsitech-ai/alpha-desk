use domain_types::{Address, Decimal};
use serde_json::Value;

use super::decode::{
    InfoObservationKind, UserHistoryMeta, child, expect_capability, history_coverage, malformed,
    optional_bool, optional_i64, optional_str, parse_family, require_address, require_array,
    require_bool, require_decimal, require_i64, require_object, require_object_field, require_str,
    require_u64,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const DELEGATOR_HISTORY_PAGE_LIMIT: usize = 2000;
pub const DELEGATOR_REWARDS_PAGE_LIMIT: usize = 2000;

pub const DELEGATOR_DELTA_KEYS: &[&str] = &["delegate", "cDeposit", "withdrawal"];
pub const DELEGATOR_REWARD_SOURCES: &[&str] = &["delegation", "commission"];
pub const VALIDATOR_STAT_PERIODS: &[&str] = &["day", "week", "month"];
pub const WITHDRAWAL_PHASE_NAMES: &[&str] = &["initiated", "finalized"];

pub const DELEGATOR_SUMMARY_KNOWN_FIELDS: &[&str] = &[
    "/delegated",
    "/undelegated",
    "/totalPendingWithdrawal",
    "/nPendingWithdrawals",
];
pub const DELEGATION_KNOWN_FIELDS: &[&str] = &["/validator", "/amount", "/lockedUntilTimestamp"];
pub const DELEGATOR_HISTORY_KNOWN_FIELDS: &[&str] = &[
    "/time",
    "/hash",
    "/delta",
    "/delta/delegate",
    "/delta/delegate/validator",
    "/delta/delegate/amount",
    "/delta/delegate/isUndelegate",
    "/delta/cDeposit",
    "/delta/cDeposit/amount",
    "/delta/withdrawal",
    "/delta/withdrawal/amount",
    "/delta/withdrawal/phase",
];
pub const DELEGATOR_REWARD_KNOWN_FIELDS: &[&str] = &["/time", "/source", "/totalAmount"];
pub const VALIDATOR_STATS_KNOWN_FIELDS: &[&str] = &[
    "/validator",
    "/signer",
    "/name",
    "/description",
    "/nRecentBlocks",
    "/stake",
    "/isJailed",
    "/unjailableAfter",
    "/isActive",
    "/commission",
    "/stats",
    "/stats/uptimeFraction",
    "/stats/predictedApr",
    "/stats/nSamples",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatorSummary {
    delegated: Decimal,
    undelegated: Decimal,
    total_pending_withdrawal: Decimal,
    n_pending_withdrawals: u64,
}

impl DelegatorSummary {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn delegated(&self) -> Decimal {
        self.delegated
    }

    #[must_use]
    pub const fn undelegated(&self) -> Decimal {
        self.undelegated
    }

    #[must_use]
    pub const fn total_pending_withdrawal(&self) -> Decimal {
        self.total_pending_withdrawal
    }

    #[must_use]
    pub const fn n_pending_withdrawals(&self) -> u64 {
        self.n_pending_withdrawals
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for DelegatorSummary {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.delegator_summary"])?;
        let object = require_object(parsed.value(), "")?;
        Ok(Self {
            delegated: require_decimal(object, "", "delegated")?,
            undelegated: require_decimal(object, "", "undelegated")?,
            total_pending_withdrawal: require_decimal(object, "", "totalPendingWithdrawal")?,
            n_pending_withdrawals: require_u64(object, "", "nPendingWithdrawals")?,
        })
    }
}

pub fn parse_delegator_summary(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, DelegatorSummary), InfoError> {
    parse_family(
        "official.info.delegator_summary",
        raw,
        context,
        DELEGATOR_SUMMARY_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegation {
    validator: Address,
    amount: Decimal,
    locked_until_millis: i64,
}

impl Delegation {
    #[must_use]
    pub const fn validator(&self) -> Address {
        self.validator
    }

    #[must_use]
    pub const fn amount(&self) -> Decimal {
        self.amount
    }

    #[must_use]
    pub const fn locked_until_millis(&self) -> i64 {
        self.locked_until_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Delegations {
    delegations: Vec<Delegation>,
}

impl Delegations {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn delegations(&self) -> &[Delegation] {
        &self.delegations
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for Delegations {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.delegations"])?;
        let delegations = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                Ok(Delegation {
                    validator: require_address(object, &path, "validator")?,
                    amount: require_decimal(object, &path, "amount")?,
                    locked_until_millis: require_i64(object, &path, "lockedUntilTimestamp")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { delegations })
    }
}

pub fn parse_delegations(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, Delegations), InfoError> {
    parse_family(
        "official.info.delegations",
        raw,
        context,
        DELEGATION_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatorHistoryEntry {
    time_millis: i64,
    hash: String,
    delta_key: String,
    validator: Option<Address>,
    amount: Option<Decimal>,
    is_undelegate: Option<bool>,
    phase: Option<String>,
}

impl DelegatorHistoryEntry {
    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn delta_key(&self) -> &str {
        &self.delta_key
    }

    #[must_use]
    pub const fn validator(&self) -> Option<Address> {
        self.validator
    }

    #[must_use]
    pub const fn amount(&self) -> Option<Decimal> {
        self.amount
    }

    #[must_use]
    pub const fn is_undelegate(&self) -> Option<bool> {
        self.is_undelegate
    }

    #[must_use]
    pub fn phase(&self) -> Option<&str> {
        self.phase.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatorHistory {
    updates: Vec<DelegatorHistoryEntry>,
    history: UserHistoryMeta,
}

impl DelegatorHistory {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn updates(&self) -> &[DelegatorHistoryEntry] {
        &self.updates
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for DelegatorHistory {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.delegator_history"])?;
        let updates = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                let delta_path = child(&path, "delta");
                let delta = require_object_field(object, &path, "delta")?;
                let mut found = None;
                for key in delta.keys() {
                    if !DELEGATOR_DELTA_KEYS.contains(&key.as_str()) {
                        return Err(InfoError::UnknownStateAffectingVariant {
                            path: child(&delta_path, key),
                            value: key.clone(),
                        });
                    }
                    if found.is_some() {
                        return Err(malformed(&delta_path, "mixed delegator delta"));
                    }
                    found = Some(key.clone());
                }
                let delta_key = found.ok_or_else(|| malformed(&delta_path, "empty delta"))?;
                let body_path = child(&delta_path, &delta_key);
                let body = require_object_field(delta, &delta_path, &delta_key)?;
                let phase = if delta_key == "withdrawal" {
                    match optional_str(body, &body_path, "phase")? {
                        None => None,
                        Some(phase) => {
                            if !WITHDRAWAL_PHASE_NAMES.contains(&phase) {
                                return Err(InfoError::UnknownStateAffectingVariant {
                                    path: child(&body_path, "phase"),
                                    value: phase.to_owned(),
                                });
                            }
                            Some(phase.to_owned())
                        }
                    }
                } else {
                    None
                };
                Ok(DelegatorHistoryEntry {
                    time_millis: require_i64(object, &path, "time")?,
                    hash: require_str(object, &path, "hash")?.to_owned(),
                    validator: match body.get("validator") {
                        None | Some(Value::Null) => None,
                        Some(_) => Some(require_address(body, &body_path, "validator")?),
                    },
                    amount: match body.get("amount") {
                        None | Some(Value::Null) => None,
                        Some(_) => Some(require_decimal(body, &body_path, "amount")?),
                    },
                    is_undelegate: optional_bool(body, &body_path, "isUndelegate")?,
                    phase,
                    delta_key,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let earliest = updates.iter().map(|update| update.time_millis).min();
        Ok(Self {
            history: history_coverage(
                updates.len(),
                DELEGATOR_HISTORY_PAGE_LIMIT,
                DELEGATOR_HISTORY_PAGE_LIMIT,
                earliest,
            )?,
            updates,
        })
    }
}

pub fn parse_delegator_history(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, DelegatorHistory), InfoError> {
    parse_family(
        "official.info.delegator_history",
        raw,
        context,
        DELEGATOR_HISTORY_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatorReward {
    time_millis: i64,
    source: String,
    total_amount: Decimal,
}

impl DelegatorReward {
    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub const fn total_amount(&self) -> Decimal {
        self.total_amount
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DelegatorRewards {
    rewards: Vec<DelegatorReward>,
    history: UserHistoryMeta,
}

impl DelegatorRewards {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn rewards(&self) -> &[DelegatorReward] {
        &self.rewards
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for DelegatorRewards {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.delegator_rewards"])?;
        let rewards = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                let source = require_str(object, &path, "source")?.to_owned();
                if !DELEGATOR_REWARD_SOURCES.contains(&source.as_str()) {
                    return Err(InfoError::UnknownStateAffectingVariant {
                        path: child(&path, "source"),
                        value: source,
                    });
                }
                Ok(DelegatorReward {
                    time_millis: require_i64(object, &path, "time")?,
                    total_amount: require_decimal(object, &path, "totalAmount")?,
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let earliest = rewards.iter().map(|reward| reward.time_millis).min();
        Ok(Self {
            history: history_coverage(
                rewards.len(),
                DELEGATOR_REWARDS_PAGE_LIMIT,
                DELEGATOR_REWARDS_PAGE_LIMIT,
                earliest,
            )?,
            rewards,
        })
    }
}

pub fn parse_delegator_rewards(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, DelegatorRewards), InfoError> {
    parse_family(
        "official.info.delegator_rewards",
        raw,
        context,
        DELEGATOR_REWARD_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorPeriodStats {
    period: String,
    uptime_fraction: Decimal,
    predicted_apr: Decimal,
    n_samples: u64,
}

impl ValidatorPeriodStats {
    #[must_use]
    pub fn period(&self) -> &str {
        &self.period
    }

    #[must_use]
    pub const fn uptime_fraction(&self) -> Decimal {
        self.uptime_fraction
    }

    #[must_use]
    pub const fn predicted_apr(&self) -> Decimal {
        self.predicted_apr
    }

    #[must_use]
    pub const fn n_samples(&self) -> u64 {
        self.n_samples
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorStat {
    validator: Address,
    signer: Address,
    name: String,
    description: String,
    n_recent_blocks: u64,
    stake: u64,
    is_jailed: bool,
    unjailable_after_millis: Option<i64>,
    is_active: bool,
    commission: Decimal,
    stats: Vec<ValidatorPeriodStats>,
}

impl ValidatorStat {
    #[must_use]
    pub const fn validator(&self) -> Address {
        self.validator
    }

    #[must_use]
    pub const fn signer(&self) -> Address {
        self.signer
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub const fn n_recent_blocks(&self) -> u64 {
        self.n_recent_blocks
    }

    #[must_use]
    pub const fn stake(&self) -> u64 {
        self.stake
    }

    #[must_use]
    pub const fn is_jailed(&self) -> bool {
        self.is_jailed
    }

    #[must_use]
    pub const fn unjailable_after_millis(&self) -> Option<i64> {
        self.unjailable_after_millis
    }

    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.is_active
    }

    #[must_use]
    pub const fn commission(&self) -> Decimal {
        self.commission
    }

    #[must_use]
    pub fn stats(&self) -> &[ValidatorPeriodStats] {
        &self.stats
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorStats {
    validators: Vec<ValidatorStat>,
}

impl ValidatorStats {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn validators(&self) -> &[ValidatorStat] {
        &self.validators
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for ValidatorStats {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.validator_stats"])?;
        let validators = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                let stats = super::decode::pair_entries(
                    object
                        .get("stats")
                        .ok_or_else(|| malformed(&child(&path, "stats"), "missing field"))?,
                    &child(&path, "stats"),
                )?
                .into_iter()
                .enumerate()
                .map(|(stat_index, (period, body))| {
                    let item = format!("{path}/stats/{stat_index}");
                    let period = period
                        .as_str()
                        .ok_or_else(|| malformed(&format!("{item}/0"), "expected period"))?;
                    if !VALIDATOR_STAT_PERIODS.contains(&period) {
                        return Err(InfoError::UnknownStateAffectingVariant {
                            path: format!("{item}/0"),
                            value: period.to_owned(),
                        });
                    }
                    let body_object = require_object(body, &format!("{item}/1"))?;
                    Ok(ValidatorPeriodStats {
                        period: period.to_owned(),
                        uptime_fraction: require_decimal(
                            body_object,
                            &format!("{item}/1"),
                            "uptimeFraction",
                        )?,
                        predicted_apr: require_decimal(
                            body_object,
                            &format!("{item}/1"),
                            "predictedApr",
                        )?,
                        n_samples: require_u64(body_object, &format!("{item}/1"), "nSamples")?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
                Ok(ValidatorStat {
                    validator: require_address(object, &path, "validator")?,
                    signer: require_address(object, &path, "signer")?,
                    name: require_str(object, &path, "name")?.to_owned(),
                    description: require_str(object, &path, "description")?.to_owned(),
                    n_recent_blocks: require_u64(object, &path, "nRecentBlocks")?,
                    stake: require_u64(object, &path, "stake")?,
                    is_jailed: require_bool(object, &path, "isJailed")?,
                    unjailable_after_millis: optional_i64(object, &path, "unjailableAfter")?,
                    is_active: require_bool(object, &path, "isActive")?,
                    commission: require_decimal(object, &path, "commission")?,
                    stats,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { validators })
    }
}

pub fn parse_validator_stats(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, ValidatorStats), InfoError> {
    parse_family(
        "official.info.validator_stats",
        raw,
        context,
        VALIDATOR_STATS_KNOWN_FIELDS,
        &[],
    )
}
