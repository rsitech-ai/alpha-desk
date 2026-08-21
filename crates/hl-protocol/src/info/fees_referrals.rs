use domain_types::{Address, Decimal};
use serde_json::Value;

use super::decode::{
    InfoObservationKind, expect_capability, malformed, optional_str, parse_family, require_address,
    require_array, require_decimal, require_i64, require_object, require_str,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const USER_FEES_KNOWN_FIELDS: &[&str] = &[
    "/dailyUserVlm",
    "/dailyUserVlm/date",
    "/dailyUserVlm/userCross",
    "/dailyUserVlm/userAdd",
    "/dailyUserVlm/exchange",
    "/feeSchedule",
    "/feeSchedule/cross",
    "/feeSchedule/add",
    "/feeSchedule/spotCross",
    "/feeSchedule/spotAdd",
    "/feeSchedule/tiers",
    "/feeSchedule/tiers/vip",
    "/feeSchedule/tiers/vip/ntlCutoff",
    "/feeSchedule/tiers/vip/cross",
    "/feeSchedule/tiers/vip/add",
    "/feeSchedule/tiers/vip/spotCross",
    "/feeSchedule/tiers/vip/spotAdd",
    "/feeSchedule/tiers/mm",
    "/feeSchedule/tiers/mm/makerFractionCutoff",
    "/feeSchedule/tiers/mm/add",
    "/feeSchedule/referralDiscount",
    "/feeSchedule/stakingDiscountTiers",
    "/feeSchedule/stakingDiscountTiers/bpsOfMaxSupply",
    "/feeSchedule/stakingDiscountTiers/discount",
    "/userCrossRate",
    "/userAddRate",
    "/userSpotCrossRate",
    "/userSpotAddRate",
    "/activeReferralDiscount",
    "/trial",
    "/feeTrialReward",
    "/nextTrialAvailableTimestamp",
    "/stakingLink",
    "/stakingLink/type",
    "/stakingLink/stakingUser",
    "/activeStakingDiscount",
    "/activeStakingDiscount/bpsOfMaxSupply",
    "/activeStakingDiscount/discount",
];

pub const REFERRAL_KNOWN_FIELDS: &[&str] = &[
    "/referredBy",
    "/referredBy/referrer",
    "/referredBy/code",
    "/cumVlm",
    "/unclaimedRewards",
    "/claimedRewards",
    "/builderRewards",
    "/tokenToState",
    "/tokenToState/cumVlm",
    "/tokenToState/unclaimedRewards",
    "/tokenToState/claimedRewards",
    "/tokenToState/builderRewards",
    "/referrerState",
    "/referrerState/stage",
    "/referrerState/data",
    "/referrerState/data/code",
    "/referrerState/data/referralStates",
    "/referrerState/data/referralStates/cumVlm",
    "/referrerState/data/referralStates/cumRewardedFeesSinceReferred",
    "/referrerState/data/referralStates/cumFeesRewardedToReferrer",
    "/referrerState/data/referralStates/timeJoined",
    "/referrerState/data/referralStates/user",
    "/rewardHistory",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DailyUserVlm {
    date: String,
    user_cross: Decimal,
    user_add: Decimal,
    exchange: Decimal,
}

impl DailyUserVlm {
    #[must_use]
    pub fn date(&self) -> &str {
        &self.date
    }

    #[must_use]
    pub const fn user_cross(&self) -> Decimal {
        self.user_cross
    }

    #[must_use]
    pub const fn user_add(&self) -> Decimal {
        self.user_add
    }

    #[must_use]
    pub const fn exchange(&self) -> Decimal {
        self.exchange
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFees {
    daily_user_vlm: Vec<DailyUserVlm>,
    user_cross_rate: Decimal,
    user_add_rate: Decimal,
    user_spot_cross_rate: Decimal,
    user_spot_add_rate: Decimal,
    active_referral_discount: Decimal,
    fee_schedule: Value,
    staking_link_user: Option<Address>,
}

impl UserFees {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn daily_user_vlm(&self) -> &[DailyUserVlm] {
        &self.daily_user_vlm
    }

    #[must_use]
    pub const fn user_cross_rate(&self) -> Decimal {
        self.user_cross_rate
    }

    #[must_use]
    pub const fn user_add_rate(&self) -> Decimal {
        self.user_add_rate
    }

    #[must_use]
    pub const fn user_spot_cross_rate(&self) -> Decimal {
        self.user_spot_cross_rate
    }

    #[must_use]
    pub const fn user_spot_add_rate(&self) -> Decimal {
        self.user_spot_add_rate
    }

    #[must_use]
    pub const fn active_referral_discount(&self) -> Decimal {
        self.active_referral_discount
    }

    #[must_use]
    pub const fn fee_schedule(&self) -> &Value {
        &self.fee_schedule
    }

    #[must_use]
    pub const fn staking_link_user(&self) -> Option<Address> {
        self.staking_link_user
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserFees {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_fees"])?;
        let object = require_object(parsed.value(), "")?;
        let daily = require_array(
            object
                .get("dailyUserVlm")
                .ok_or_else(|| malformed("/dailyUserVlm", "missing field"))?,
            "/dailyUserVlm",
        )?;
        let daily_user_vlm = daily
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/dailyUserVlm/{index}");
                let row = require_object(value, &path)?;
                Ok(DailyUserVlm {
                    date: require_str(row, &path, "date")?.to_owned(),
                    user_cross: require_decimal(row, &path, "userCross")?,
                    user_add: require_decimal(row, &path, "userAdd")?,
                    exchange: require_decimal(row, &path, "exchange")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let staking_link_user = match object.get("stakingLink") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let link = require_object(value, "/stakingLink")?;
                Some(require_address(link, "/stakingLink", "stakingUser")?)
            }
        };
        Ok(Self {
            daily_user_vlm,
            user_cross_rate: require_decimal(object, "", "userCrossRate")?,
            user_add_rate: require_decimal(object, "", "userAddRate")?,
            user_spot_cross_rate: require_decimal(object, "", "userSpotCrossRate")?,
            user_spot_add_rate: require_decimal(object, "", "userSpotAddRate")?,
            active_referral_discount: require_decimal(object, "", "activeReferralDiscount")?,
            fee_schedule: object
                .get("feeSchedule")
                .cloned()
                .ok_or_else(|| malformed("/feeSchedule", "missing field"))?,
            staking_link_user,
        })
    }
}

pub fn parse_user_fees(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserFees), InfoError> {
    parse_family(
        "official.info.user_fees",
        raw,
        context,
        USER_FEES_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferredBy {
    referrer: Address,
    code: String,
}

impl ReferredBy {
    #[must_use]
    pub const fn referrer(&self) -> Address {
        self.referrer
    }

    #[must_use]
    pub fn code(&self) -> &str {
        &self.code
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferralState {
    cum_vlm: Decimal,
    cum_rewarded_fees_since_referred: Decimal,
    cum_fees_rewarded_to_referrer: Decimal,
    time_joined_millis: i64,
    user: Address,
}

impl ReferralState {
    #[must_use]
    pub const fn cum_vlm(&self) -> Decimal {
        self.cum_vlm
    }

    #[must_use]
    pub const fn user(&self) -> Address {
        self.user
    }

    #[must_use]
    pub const fn time_joined_millis(&self) -> i64 {
        self.time_joined_millis
    }

    #[must_use]
    pub const fn cum_rewarded_fees_since_referred(&self) -> Decimal {
        self.cum_rewarded_fees_since_referred
    }

    #[must_use]
    pub const fn cum_fees_rewarded_to_referrer(&self) -> Decimal {
        self.cum_fees_rewarded_to_referrer
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Referral {
    referred_by: Option<ReferredBy>,
    cum_vlm: Decimal,
    unclaimed_rewards: Decimal,
    claimed_rewards: Decimal,
    builder_rewards: Decimal,
    referrer_code: Option<String>,
    referral_states: Vec<ReferralState>,
}

impl Referral {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn referred_by(&self) -> Option<&ReferredBy> {
        self.referred_by.as_ref()
    }

    #[must_use]
    pub const fn cum_vlm(&self) -> Decimal {
        self.cum_vlm
    }

    #[must_use]
    pub const fn unclaimed_rewards(&self) -> Decimal {
        self.unclaimed_rewards
    }

    #[must_use]
    pub const fn claimed_rewards(&self) -> Decimal {
        self.claimed_rewards
    }

    #[must_use]
    pub const fn builder_rewards(&self) -> Decimal {
        self.builder_rewards
    }

    #[must_use]
    pub fn referrer_code(&self) -> Option<&str> {
        self.referrer_code.as_deref()
    }

    #[must_use]
    pub fn referral_states(&self) -> &[ReferralState] {
        &self.referral_states
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for Referral {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.referral"])?;
        let object = require_object(parsed.value(), "")?;
        let referred_by = match object.get("referredBy") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let row = require_object(value, "/referredBy")?;
                Some(ReferredBy {
                    referrer: require_address(row, "/referredBy", "referrer")?,
                    code: require_str(row, "/referredBy", "code")?.to_owned(),
                })
            }
        };
        let (referrer_code, referral_states) = match object.get("referrerState") {
            None | Some(Value::Null) => (None, Vec::new()),
            Some(value) => {
                let state = require_object(value, "/referrerState")?;
                match state.get("data") {
                    None | Some(Value::Null) => (
                        optional_str(state, "/referrerState", "code")?.map(str::to_owned),
                        Vec::new(),
                    ),
                    Some(data) => {
                        let data_object = require_object(data, "/referrerState/data")?;
                        let code = optional_str(data_object, "/referrerState/data", "code")?
                            .map(str::to_owned);
                        let states = match data_object.get("referralStates") {
                            None => Vec::new(),
                            Some(value) => {
                                require_array(value, "/referrerState/data/referralStates")?
                                    .iter()
                                    .enumerate()
                                    .map(|(index, row)| {
                                        let path =
                                            format!("/referrerState/data/referralStates/{index}");
                                        let row = require_object(row, &path)?;
                                        Ok(ReferralState {
                                            cum_vlm: require_decimal(row, &path, "cumVlm")?,
                                            cum_rewarded_fees_since_referred: require_decimal(
                                                row,
                                                &path,
                                                "cumRewardedFeesSinceReferred",
                                            )?,
                                            cum_fees_rewarded_to_referrer: require_decimal(
                                                row,
                                                &path,
                                                "cumFeesRewardedToReferrer",
                                            )?,
                                            time_joined_millis: require_i64(
                                                row,
                                                &path,
                                                "timeJoined",
                                            )?,
                                            user: require_address(row, &path, "user")?,
                                        })
                                    })
                                    .collect::<Result<Vec<_>, _>>()?
                            }
                        };
                        (code, states)
                    }
                }
            }
        };
        Ok(Self {
            referred_by,
            cum_vlm: require_decimal(object, "", "cumVlm")?,
            unclaimed_rewards: require_decimal(object, "", "unclaimedRewards")?,
            claimed_rewards: require_decimal(object, "", "claimedRewards")?,
            builder_rewards: require_decimal(object, "", "builderRewards")?,
            referrer_code,
            referral_states,
        })
    }
}

pub fn parse_referral(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, Referral), InfoError> {
    parse_family(
        "official.info.referral",
        raw,
        context,
        REFERRAL_KNOWN_FIELDS,
        &[],
    )
}
