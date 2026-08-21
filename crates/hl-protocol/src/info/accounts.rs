use domain_types::{Address, Decimal};
use serde_json::Value;

use super::decode::{
    InfoObservationKind, UserHistoryMeta, address_from_str, child, expect_capability,
    history_coverage, malformed, pair_entries, parse_family, require_address, require_array,
    require_decimal, require_i64, require_object, require_str, require_u64,
};
use super::{InfoEnumField, InfoError, InfoParseContext, ParsedInfoResponse};

pub const USER_NON_FUNDING_PAGE_LIMIT: usize = 2000;

pub const LEDGER_DELTA_TYPES: &[&str] = &[
    "deposit",
    "withdraw",
    "internalTransfer",
    "subAccountTransfer",
    "liquidation",
    "vaultCreate",
    "vaultDeposit",
    "vaultDistribution",
    "vaultWithdraw",
    "vaultLeaderCommission",
    "spotTransfer",
    "accountClassTransfer",
    "spotGenesis",
    "rewardsClaim",
];

pub const USER_ROLE_NAMES: &[&str] = &["missing", "user", "agent", "vault", "subAccount"];

pub const USER_ABSTRACTION_NAMES: &[&str] = &[
    "unifiedAccount",
    "portfolioMargin",
    "disabled",
    "default",
    "dexAbstraction",
];

pub const PORTFOLIO_PERIODS: &[&str] = &[
    "day",
    "week",
    "month",
    "allTime",
    "perpDay",
    "perpWeek",
    "perpMonth",
    "perpAllTime",
];

pub const USER_ROLE_ENUM_FIELDS: &[InfoEnumField] = &[InfoEnumField::new("/role", USER_ROLE_NAMES)];

pub const LEDGER_ENUM_FIELDS: &[InfoEnumField] =
    &[InfoEnumField::new("/delta/type", LEDGER_DELTA_TYPES)];

pub const PORTFOLIO_KNOWN_FIELDS: &[&str] = &["/accountValueHistory", "/pnlHistory", "/vlm"];
pub const SUB_ACCOUNT_KNOWN_FIELDS: &[&str] = &[
    "/name",
    "/subAccountUser",
    "/master",
    "/clearinghouseState",
    "/spotState",
    "/clearinghouseState/marginSummary",
    "/clearinghouseState/marginSummary/accountValue",
    "/clearinghouseState/marginSummary/totalNtlPos",
    "/clearinghouseState/marginSummary/totalRawUsd",
    "/clearinghouseState/marginSummary/totalMarginUsed",
    "/clearinghouseState/crossMarginSummary",
    "/clearinghouseState/crossMarginSummary/accountValue",
    "/clearinghouseState/crossMarginSummary/totalNtlPos",
    "/clearinghouseState/crossMarginSummary/totalRawUsd",
    "/clearinghouseState/crossMarginSummary/totalMarginUsed",
    "/clearinghouseState/crossMaintenanceMarginUsed",
    "/clearinghouseState/withdrawable",
    "/clearinghouseState/assetPositions",
    "/clearinghouseState/time",
    "/spotState/balances",
    "/spotState/balances/coin",
    "/spotState/balances/token",
    "/spotState/balances/total",
    "/spotState/balances/hold",
    "/spotState/balances/entryNtl",
];
pub const USER_ROLE_KNOWN_FIELDS: &[&str] = &["/role", "/data", "/data/user", "/data/master"];
pub const USER_RATE_LIMIT_KNOWN_FIELDS: &[&str] = &[
    "/cumVlm",
    "/nRequestsUsed",
    "/nRequestsCap",
    "/nRequestsSurplus",
];
pub const MULTI_SIG_KNOWN_FIELDS: &[&str] = &["/authorizedUsers", "/threshold"];
pub const LEDGER_KNOWN_FIELDS: &[&str] = &["/time", "/hash", "/delta", "/delta/type"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserRole {
    Missing,
    User,
    Agent { user: Address },
    Vault,
    SubAccount { master: Address },
}

impl UserRole {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::User => "user",
            Self::Agent { .. } => "agent",
            Self::Vault => "vault",
            Self::SubAccount { .. } => "subAccount",
        }
    }

    #[must_use]
    pub const fn related_account(&self) -> Option<&Address> {
        match self {
            Self::Agent { user } => Some(user),
            Self::SubAccount { master } => Some(master),
            Self::Missing | Self::User | Self::Vault => None,
        }
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserRole {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_role"])?;
        let object = require_object(parsed.value(), "")?;
        let role = require_str(object, "", "role")?;
        match role {
            "missing" => Ok(Self::Missing),
            "user" => Ok(Self::User),
            "vault" => Ok(Self::Vault),
            "agent" => {
                let data = require_object(
                    object
                        .get("data")
                        .ok_or_else(|| malformed("/data", "missing field"))?,
                    "/data",
                )?;
                Ok(Self::Agent {
                    user: require_address(data, "/data", "user")?,
                })
            }
            "subAccount" => {
                let data = require_object(
                    object
                        .get("data")
                        .ok_or_else(|| malformed("/data", "missing field"))?,
                    "/data",
                )?;
                Ok(Self::SubAccount {
                    master: require_address(data, "/data", "master")?,
                })
            }
            other => Err(InfoError::UnknownStateAffectingVariant {
                path: "/role".to_owned(),
                value: other.to_owned(),
            }),
        }
    }
}

pub fn parse_user_role(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserRole), InfoError> {
    parse_family(
        "official.info.user_role",
        raw,
        context,
        USER_ROLE_KNOWN_FIELDS,
        USER_ROLE_ENUM_FIELDS,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserAbstraction {
    UnifiedAccount,
    PortfolioMargin,
    Disabled,
    Default,
    DexAbstraction,
}

impl UserAbstraction {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::UnifiedAccount => "unifiedAccount",
            Self::PortfolioMargin => "portfolioMargin",
            Self::Disabled => "disabled",
            Self::Default => "default",
            Self::DexAbstraction => "dexAbstraction",
        }
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserAbstraction {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_abstraction"])?;
        let value = parsed
            .value()
            .as_str()
            .ok_or_else(|| malformed("", "expected abstraction string"))?;
        match value {
            "unifiedAccount" => Ok(Self::UnifiedAccount),
            "portfolioMargin" => Ok(Self::PortfolioMargin),
            "disabled" => Ok(Self::Disabled),
            "default" => Ok(Self::Default),
            "dexAbstraction" => Ok(Self::DexAbstraction),
            other => Err(InfoError::UnknownStateAffectingVariant {
                path: "".to_owned(),
                value: other.to_owned(),
            }),
        }
    }
}

pub fn parse_user_abstraction(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserAbstraction), InfoError> {
    parse_family("official.info.user_abstraction", raw, context, &[], &[])
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UserDexAbstraction {
    enabled: bool,
}

impl UserDexAbstraction {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn enabled(self) -> bool {
        self.enabled
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserDexAbstraction {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_dex_abstraction"])?;
        parsed
            .value()
            .as_bool()
            .map(|enabled| Self { enabled })
            .ok_or_else(|| malformed("", "expected bool"))
    }
}

pub fn parse_user_dex_abstraction(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserDexAbstraction), InfoError> {
    parse_family("official.info.user_dex_abstraction", raw, context, &[], &[])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserRateLimit {
    cum_vlm: Decimal,
    n_requests_used: u64,
    n_requests_cap: u64,
    n_requests_surplus: u64,
}

impl UserRateLimit {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn cum_vlm(&self) -> Decimal {
        self.cum_vlm
    }

    #[must_use]
    pub const fn n_requests_used(&self) -> u64 {
        self.n_requests_used
    }

    #[must_use]
    pub const fn n_requests_cap(&self) -> u64 {
        self.n_requests_cap
    }

    #[must_use]
    pub const fn n_requests_surplus(&self) -> u64 {
        self.n_requests_surplus
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserRateLimit {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_rate_limit"])?;
        let object = require_object(parsed.value(), "")?;
        Ok(Self {
            cum_vlm: require_decimal(object, "", "cumVlm")?,
            n_requests_used: require_u64(object, "", "nRequestsUsed")?,
            n_requests_cap: require_u64(object, "", "nRequestsCap")?,
            n_requests_surplus: require_u64(object, "", "nRequestsSurplus")?,
        })
    }
}

pub fn parse_user_rate_limit(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserRateLimit), InfoError> {
    parse_family(
        "official.info.user_rate_limit",
        raw,
        context,
        USER_RATE_LIMIT_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultiSigSigners {
    authorized_users: Vec<Address>,
    threshold: u64,
}

impl MultiSigSigners {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn authorized_users(&self) -> &[Address] {
        &self.authorized_users
    }

    #[must_use]
    pub const fn threshold(&self) -> u64 {
        self.threshold
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for Option<MultiSigSigners> {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_to_multi_sig_signers"])?;
        if parsed.value().is_null() {
            return Ok(None);
        }
        let object = require_object(parsed.value(), "")?;
        let users = require_array(
            object
                .get("authorizedUsers")
                .ok_or_else(|| malformed("/authorizedUsers", "missing field"))?,
            "/authorizedUsers",
        )?;
        let authorized_users = users
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/authorizedUsers/{index}");
                let text = value
                    .as_str()
                    .ok_or_else(|| malformed(&path, "expected address"))?;
                address_from_str(text, &path)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(MultiSigSigners {
            authorized_users,
            threshold: require_u64(object, "", "threshold")?,
        }))
    }
}

pub fn parse_user_to_multi_sig_signers(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, Option<MultiSigSigners>), InfoError> {
    parse_family(
        "official.info.user_to_multi_sig_signers",
        raw,
        context,
        MULTI_SIG_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubAccount {
    name: String,
    sub_account_user: Address,
    master: Address,
    clearinghouse_state: Value,
    spot_state: Value,
}

impl SubAccount {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn sub_account_user(&self) -> Address {
        self.sub_account_user
    }

    #[must_use]
    pub const fn master(&self) -> Address {
        self.master
    }

    #[must_use]
    pub const fn clearinghouse_state(&self) -> &Value {
        &self.clearinghouse_state
    }

    #[must_use]
    pub const fn spot_state(&self) -> &Value {
        &self.spot_state
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubAccounts {
    accounts: Vec<SubAccount>,
}

impl SubAccounts {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn accounts(&self) -> &[SubAccount] {
        &self.accounts
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for SubAccounts {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.sub_accounts"])?;
        let accounts = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                let sub_account_user = require_address(object, &path, "subAccountUser")?;
                let master = require_address(object, &path, "master")?;
                if sub_account_user == master {
                    return Err(malformed(
                        &child(&path, "subAccountUser"),
                        "subaccount matches master",
                    ));
                }
                Ok(SubAccount {
                    name: require_str(object, &path, "name")?.to_owned(),
                    sub_account_user,
                    master,
                    // ponytail: nested perp/spot state is T07's clearinghouse/spot types
                    clearinghouse_state: object
                        .get("clearinghouseState")
                        .cloned()
                        .unwrap_or(Value::Null),
                    spot_state: object.get("spotState").cloned().unwrap_or(Value::Null),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { accounts })
    }
}

pub fn parse_sub_accounts(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, SubAccounts), InfoError> {
    parse_family(
        "official.info.sub_accounts",
        raw,
        context,
        SUB_ACCOUNT_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortfolioWindow {
    period: String,
    account_value_history: Vec<(i64, Decimal)>,
    pnl_history: Vec<(i64, Decimal)>,
    vlm: Decimal,
}

impl PortfolioWindow {
    #[must_use]
    pub fn period(&self) -> &str {
        &self.period
    }

    #[must_use]
    pub fn account_value_history(&self) -> &[(i64, Decimal)] {
        &self.account_value_history
    }

    #[must_use]
    pub fn pnl_history(&self) -> &[(i64, Decimal)] {
        &self.pnl_history
    }

    #[must_use]
    pub const fn vlm(&self) -> Decimal {
        self.vlm
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Portfolio {
    windows: Vec<PortfolioWindow>,
}

impl Portfolio {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReconciledSnapshot
    }

    #[must_use]
    pub fn windows(&self) -> &[PortfolioWindow] {
        &self.windows
    }
}

fn parse_history_points(value: &Value, path: &str) -> Result<Vec<(i64, Decimal)>, InfoError> {
    pair_entries(value, path)?
        .into_iter()
        .enumerate()
        .map(|(index, (time, amount))| {
            let item = format!("{path}/{index}");
            Ok((
                super::decode::i64_from_value(time, &format!("{item}/0"))?,
                super::decode::decimal_from_value(amount, &format!("{item}/1"))?,
            ))
        })
        .collect()
}

impl TryFrom<&ParsedInfoResponse<Value>> for Portfolio {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.portfolio"])?;
        let windows = pair_entries(parsed.value(), "")?
            .into_iter()
            .enumerate()
            .map(|(index, (period, body))| {
                let path = format!("/{index}");
                let period = period
                    .as_str()
                    .ok_or_else(|| malformed(&format!("{path}/0"), "expected period"))?;
                if !PORTFOLIO_PERIODS.contains(&period) {
                    return Err(InfoError::UnknownStateAffectingVariant {
                        path: format!("{path}/0"),
                        value: period.to_owned(),
                    });
                }
                let object = require_object(body, &format!("{path}/1"))?;
                Ok(PortfolioWindow {
                    period: period.to_owned(),
                    account_value_history: parse_history_points(
                        object.get("accountValueHistory").ok_or_else(|| {
                            malformed(&format!("{path}/1/accountValueHistory"), "missing field")
                        })?,
                        &format!("{path}/1/accountValueHistory"),
                    )?,
                    pnl_history: parse_history_points(
                        object.get("pnlHistory").ok_or_else(|| {
                            malformed(&format!("{path}/1/pnlHistory"), "missing field")
                        })?,
                        &format!("{path}/1/pnlHistory"),
                    )?,
                    vlm: require_decimal(object, &format!("{path}/1"), "vlm")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { windows })
    }
}

pub fn parse_portfolio(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, Portfolio), InfoError> {
    parse_family(
        "official.info.portfolio",
        raw,
        context,
        PORTFOLIO_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LedgerUpdate {
    time_millis: i64,
    hash: String,
    delta_type: String,
    delta: Value,
}

impl LedgerUpdate {
    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn delta_type(&self) -> &str {
        &self.delta_type
    }

    #[must_use]
    pub const fn delta(&self) -> &Value {
        &self.delta
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserNonFundingLedgerUpdates {
    updates: Vec<LedgerUpdate>,
    history: UserHistoryMeta,
}

impl UserNonFundingLedgerUpdates {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn updates(&self) -> &[LedgerUpdate] {
        &self.updates
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserNonFundingLedgerUpdates {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_non_funding_ledger_updates"])?;
        let updates = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                let delta = object
                    .get("delta")
                    .cloned()
                    .ok_or_else(|| malformed(&child(&path, "delta"), "missing field"))?;
                let delta_object = require_object(&delta, &child(&path, "delta"))?;
                let delta_type =
                    require_str(delta_object, &child(&path, "delta"), "type")?.to_owned();
                if !LEDGER_DELTA_TYPES.contains(&delta_type.as_str()) {
                    return Err(InfoError::UnknownStateAffectingVariant {
                        path: child(&path, "delta/type"),
                        value: delta_type,
                    });
                }
                Ok(LedgerUpdate {
                    time_millis: require_i64(object, &path, "time")?,
                    hash: require_str(object, &path, "hash")?.to_owned(),
                    delta_type,
                    delta,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let earliest = updates.iter().map(|update| update.time_millis).min();
        Ok(Self {
            history: history_coverage(
                updates.len(),
                USER_NON_FUNDING_PAGE_LIMIT,
                USER_NON_FUNDING_PAGE_LIMIT,
                earliest,
            )?,
            updates,
        })
    }
}

pub fn parse_user_non_funding_ledger_updates(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserNonFundingLedgerUpdates), InfoError> {
    parse_family(
        "official.info.user_non_funding_ledger_updates",
        raw,
        context,
        LEDGER_KNOWN_FIELDS,
        LEDGER_ENUM_FIELDS,
    )
}
