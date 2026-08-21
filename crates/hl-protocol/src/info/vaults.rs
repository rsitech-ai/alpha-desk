use domain_types::{Address, Decimal};
use serde_json::Value;

use super::accounts::PORTFOLIO_PERIODS;
use super::decode::{
    InfoObservationKind, child, expect_capability, malformed, optional_bool, pair_entries,
    parse_family, require_address, require_array, require_array_field, require_bool,
    require_decimal, require_i64, require_object, require_object_field, require_str, require_u64,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const VAULT_EQUITY_KNOWN_FIELDS: &[&str] = &["/vaultAddress", "/equity"];
pub const VAULT_DETAILS_KNOWN_FIELDS: &[&str] = &[
    "/name",
    "/vaultAddress",
    "/leader",
    "/description",
    "/portfolio",
    "/portfolio/accountValueHistory",
    "/portfolio/pnlHistory",
    "/portfolio/vlm",
    "/apr",
    "/followerState",
    "/followerState/user",
    "/followerState/vaultEquity",
    "/followerState/pnl",
    "/followerState/allTimePnl",
    "/followerState/daysFollowing",
    "/followerState/vaultEntryTime",
    "/followerState/lockupUntil",
    "/leaderFraction",
    "/leaderCommission",
    "/followers",
    "/followers/user",
    "/followers/vaultEquity",
    "/followers/pnl",
    "/followers/allTimePnl",
    "/followers/daysFollowing",
    "/followers/vaultEntryTime",
    "/followers/lockupUntil",
    "/maxDistributable",
    "/maxWithdrawable",
    "/isClosed",
    "/relationship",
    "/relationship/type",
    "/relationship/data",
    "/relationship/data/childAddresses",
    "/relationship/data/parent",
    "/allowDeposits",
    "/alwaysCloseOnWithdraw",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultEquity {
    vault_address: Address,
    equity: Decimal,
}

impl VaultEquity {
    #[must_use]
    pub const fn vault_address(&self) -> Address {
        self.vault_address
    }

    #[must_use]
    pub const fn equity(&self) -> Decimal {
        self.equity
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserVaultEquities {
    equities: Vec<VaultEquity>,
}

impl UserVaultEquities {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn equities(&self) -> &[VaultEquity] {
        &self.equities
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserVaultEquities {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_vault_equities"])?;
        let equities = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                Ok(VaultEquity {
                    vault_address: require_address(object, &path, "vaultAddress")?,
                    equity: require_decimal(object, &path, "equity")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { equities })
    }
}

pub fn parse_user_vault_equities(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserVaultEquities), InfoError> {
    parse_family(
        "official.info.user_vault_equities",
        raw,
        context,
        VAULT_EQUITY_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultPortfolioWindow {
    period: String,
    account_value_history: Vec<(i64, Decimal)>,
    pnl_history: Vec<(i64, Decimal)>,
    vlm: Decimal,
}

impl VaultPortfolioWindow {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultFollower {
    user: Address,
    vault_equity: Decimal,
    pnl: Decimal,
    all_time_pnl: Decimal,
    days_following: u64,
    vault_entry_time_millis: i64,
    lockup_until_millis: i64,
}

impl VaultFollower {
    #[must_use]
    pub const fn user(&self) -> Address {
        self.user
    }

    #[must_use]
    pub const fn vault_equity(&self) -> Decimal {
        self.vault_equity
    }

    #[must_use]
    pub const fn pnl(&self) -> Decimal {
        self.pnl
    }

    #[must_use]
    pub const fn all_time_pnl(&self) -> Decimal {
        self.all_time_pnl
    }

    #[must_use]
    pub const fn days_following(&self) -> u64 {
        self.days_following
    }

    #[must_use]
    pub const fn vault_entry_time_millis(&self) -> i64 {
        self.vault_entry_time_millis
    }

    #[must_use]
    pub const fn lockup_until_millis(&self) -> i64 {
        self.lockup_until_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultRelationship {
    Parent { child_addresses: Vec<Address> },
    Child { parent: Address },
}

impl VaultRelationship {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Parent { .. } => "parent",
            Self::Child { .. } => "child",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultDetails {
    name: String,
    vault_address: Address,
    leader: Address,
    description: String,
    portfolio: Vec<VaultPortfolioWindow>,
    apr: Decimal,
    follower_state: Option<VaultFollower>,
    leader_fraction: Decimal,
    leader_commission: Decimal,
    followers: Vec<VaultFollower>,
    max_distributable: Decimal,
    max_withdrawable: Decimal,
    is_closed: bool,
    relationship: Option<VaultRelationship>,
    allow_deposits: Option<bool>,
    always_close_on_withdraw: Option<bool>,
}

impl VaultDetails {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::DirectLookup
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn vault_address(&self) -> Address {
        self.vault_address
    }

    #[must_use]
    pub const fn leader(&self) -> Address {
        self.leader
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn portfolio(&self) -> &[VaultPortfolioWindow] {
        &self.portfolio
    }

    #[must_use]
    pub const fn apr(&self) -> Decimal {
        self.apr
    }

    #[must_use]
    pub const fn follower_state(&self) -> Option<&VaultFollower> {
        self.follower_state.as_ref()
    }

    #[must_use]
    pub const fn leader_fraction(&self) -> Decimal {
        self.leader_fraction
    }

    #[must_use]
    pub const fn leader_commission(&self) -> Decimal {
        self.leader_commission
    }

    #[must_use]
    pub fn followers(&self) -> &[VaultFollower] {
        &self.followers
    }

    #[must_use]
    pub const fn max_distributable(&self) -> Decimal {
        self.max_distributable
    }

    #[must_use]
    pub const fn max_withdrawable(&self) -> Decimal {
        self.max_withdrawable
    }

    #[must_use]
    pub const fn is_closed(&self) -> bool {
        self.is_closed
    }

    #[must_use]
    pub const fn relationship(&self) -> Option<&VaultRelationship> {
        self.relationship.as_ref()
    }

    #[must_use]
    pub const fn allow_deposits(&self) -> Option<bool> {
        self.allow_deposits
    }

    #[must_use]
    pub const fn always_close_on_withdraw(&self) -> Option<bool> {
        self.always_close_on_withdraw
    }
}

fn parse_vault_follower(value: &Value, path: &str) -> Result<VaultFollower, InfoError> {
    let object = require_object(value, path)?;
    Ok(VaultFollower {
        user: require_address(object, path, "user")?,
        vault_equity: require_decimal(object, path, "vaultEquity")?,
        pnl: require_decimal(object, path, "pnl")?,
        all_time_pnl: require_decimal(object, path, "allTimePnl")?,
        days_following: require_u64(object, path, "daysFollowing")?,
        vault_entry_time_millis: require_i64(object, path, "vaultEntryTime")?,
        lockup_until_millis: require_i64(object, path, "lockupUntil")?,
    })
}

fn parse_relationship(value: &Value, path: &str) -> Result<Option<VaultRelationship>, InfoError> {
    if value.is_null() {
        return Ok(None);
    }
    let object = require_object(value, path)?;
    let kind = require_str(object, path, "type")?;
    let data_path = child(path, "data");
    let data = require_object_field(object, path, "data")?;
    match kind {
        "parent" => {
            let children = require_array_field(data, &data_path, "childAddresses")?
                .iter()
                .enumerate()
                .map(|(index, address)| {
                    let item = format!("{data_path}/childAddresses/{index}");
                    let text = address
                        .as_str()
                        .ok_or_else(|| malformed(&item, "expected address"))?;
                    super::decode::address_from_str(text, &item)
                })
                .collect::<Result<Vec<_>, _>>()?;
            Ok(Some(VaultRelationship::Parent {
                child_addresses: children,
            }))
        }
        "child" => Ok(Some(VaultRelationship::Child {
            parent: require_address(data, &data_path, "parent")?,
        })),
        other => Err(InfoError::UnknownStateAffectingVariant {
            path: child(path, "type"),
            value: other.to_owned(),
        }),
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for VaultDetails {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.vault_details"])?;
        let object = require_object(parsed.value(), "")?;
        let portfolio = pair_entries(
            object
                .get("portfolio")
                .ok_or_else(|| malformed("/portfolio", "missing field"))?,
            "/portfolio",
        )?
        .into_iter()
        .enumerate()
        .map(|(index, (period, body))| {
            let path = format!("/portfolio/{index}");
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
            Ok(VaultPortfolioWindow {
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
        let followers = match object.get("followers") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => require_array(value, "/followers")?
                .iter()
                .enumerate()
                .map(|(index, follower)| {
                    parse_vault_follower(follower, &format!("/followers/{index}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        let follower_state = match object.get("followerState") {
            None | Some(Value::Null) => None,
            Some(value) => Some(parse_vault_follower(value, "/followerState")?),
        };
        let relationship = match object.get("relationship") {
            None | Some(Value::Null) => None,
            Some(value) => parse_relationship(value, "/relationship")?,
        };
        Ok(Self {
            name: require_str(object, "", "name")?.to_owned(),
            vault_address: require_address(object, "", "vaultAddress")?,
            leader: require_address(object, "", "leader")?,
            description: require_str(object, "", "description")?.to_owned(),
            portfolio,
            apr: require_decimal(object, "", "apr")?,
            follower_state,
            leader_fraction: require_decimal(object, "", "leaderFraction")?,
            leader_commission: require_decimal(object, "", "leaderCommission")?,
            followers,
            max_distributable: require_decimal(object, "", "maxDistributable")?,
            max_withdrawable: require_decimal(object, "", "maxWithdrawable")?,
            is_closed: require_bool(object, "", "isClosed")?,
            relationship,
            allow_deposits: optional_bool(object, "", "allowDeposits")?,
            always_close_on_withdraw: optional_bool(object, "", "alwaysCloseOnWithdraw")?,
        })
    }
}

pub fn parse_vault_details(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, VaultDetails), InfoError> {
    parse_family(
        "official.info.vault_details",
        raw,
        context,
        VAULT_DETAILS_KNOWN_FIELDS,
        &[],
    )
}
