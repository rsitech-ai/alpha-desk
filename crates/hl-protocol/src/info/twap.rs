use domain_types::{Address, Decimal};
use serde_json::Value;

use super::decode::{
    BookSide, InfoObservationKind, UserHistoryMeta, child, expect_capability, history_coverage,
    malformed, optional_str, parse_family, require_array, require_bool, require_decimal,
    require_i64, require_object, require_side, require_str, require_u64,
};
use super::orders::{UserFill, parse_user_fill};
use super::{InfoEnumField, InfoError, InfoParseContext, ParsedInfoResponse};

pub const TWAP_SLICE_PAGE_LIMIT: usize = 2000;

pub const TWAP_STATUS_NAMES: &[&str] = &["activated", "terminated", "finished", "error"];

pub const TWAP_HISTORY_ENUM_FIELDS: &[InfoEnumField] =
    &[InfoEnumField::new("/status/status", TWAP_STATUS_NAMES)];

pub const TWAP_SLICE_KNOWN_FIELDS: &[&str] = &[
    "/fill",
    "/twapId",
    "/fill/closedPnl",
    "/fill/coin",
    "/fill/crossed",
    "/fill/dir",
    "/fill/hash",
    "/fill/oid",
    "/fill/px",
    "/fill/side",
    "/fill/startPosition",
    "/fill/sz",
    "/fill/time",
    "/fill/fee",
    "/fill/feeToken",
    "/fill/builderFee",
    "/fill/tid",
    "/fill/twapId",
];

pub const TWAP_HISTORY_KNOWN_FIELDS: &[&str] = &[
    "/time",
    "/twapId",
    "/state",
    "/state/coin",
    "/state/user",
    "/state/side",
    "/state/sz",
    "/state/executedSz",
    "/state/executedNtl",
    "/state/minutes",
    "/state/reduceOnly",
    "/state/randomize",
    "/state/timestamp",
    "/status",
    "/status/status",
    "/status/description",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwapSliceFill {
    fill: UserFill,
    twap_id: u64,
}

impl TwapSliceFill {
    #[must_use]
    pub const fn fill(&self) -> &UserFill {
        &self.fill
    }

    #[must_use]
    pub const fn twap_id(&self) -> u64 {
        self.twap_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserTwapSliceFills {
    fills: Vec<TwapSliceFill>,
    history: UserHistoryMeta,
    by_time: bool,
}

impl UserTwapSliceFills {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn fills(&self) -> &[TwapSliceFill] {
        &self.fills
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }

    #[must_use]
    pub const fn by_time(&self) -> bool {
        self.by_time
    }
}

fn twap_slices_from_parsed(
    parsed: &ParsedInfoResponse<Value>,
    by_time: bool,
) -> Result<UserTwapSliceFills, InfoError> {
    let fills = require_array(parsed.value(), "")?
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let path = format!("/{index}");
            let object = require_object(value, &path)?;
            Ok(TwapSliceFill {
                fill: parse_user_fill(
                    object
                        .get("fill")
                        .ok_or_else(|| malformed(&child(&path, "fill"), "missing field"))?,
                    &child(&path, "fill"),
                )?,
                twap_id: require_u64(object, &path, "twapId")?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let earliest = fills.iter().map(|item| item.fill.time_millis()).min();
    Ok(UserTwapSliceFills {
        history: history_coverage(
            fills.len(),
            TWAP_SLICE_PAGE_LIMIT,
            TWAP_SLICE_PAGE_LIMIT,
            earliest,
        )?,
        fills,
        by_time,
    })
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserTwapSliceFills {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        match parsed.capability_id().as_str() {
            "official.info.user_twap_slice_fills" => twap_slices_from_parsed(parsed, false),
            "official.info.user_twap_slice_fills_by_time" => twap_slices_from_parsed(parsed, true),
            _ => Err(malformed("", "unexpected info capability")),
        }
    }
}

pub fn parse_user_twap_slice_fills(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserTwapSliceFills), InfoError> {
    parse_family(
        "official.info.user_twap_slice_fills",
        raw,
        context,
        TWAP_SLICE_KNOWN_FIELDS,
        &[],
    )
}

pub fn parse_user_twap_slice_fills_by_time(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserTwapSliceFills), InfoError> {
    parse_family(
        "official.info.user_twap_slice_fills_by_time",
        raw,
        context,
        TWAP_SLICE_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TwapStatusKind {
    Activated,
    Terminated,
    Finished,
    Error,
}

impl TwapStatusKind {
    fn from_wire(path: &str, value: &str) -> Result<Self, InfoError> {
        match value {
            "activated" => Ok(Self::Activated),
            "terminated" => Ok(Self::Terminated),
            "finished" => Ok(Self::Finished),
            "error" => Ok(Self::Error),
            other => Err(InfoError::UnknownStateAffectingVariant {
                path: path.to_owned(),
                value: other.to_owned(),
            }),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Activated => "activated",
            Self::Terminated => "terminated",
            Self::Finished => "finished",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwapState {
    coin: String,
    user: Address,
    side: BookSide,
    sz: Decimal,
    executed_sz: Decimal,
    executed_ntl: Decimal,
    minutes: u64,
    reduce_only: bool,
    randomize: bool,
    timestamp_millis: i64,
}

impl TwapState {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn user(&self) -> Address {
        self.user
    }

    #[must_use]
    pub const fn side(&self) -> BookSide {
        self.side
    }

    #[must_use]
    pub const fn sz(&self) -> Decimal {
        self.sz
    }

    #[must_use]
    pub const fn executed_sz(&self) -> Decimal {
        self.executed_sz
    }

    #[must_use]
    pub const fn executed_ntl(&self) -> Decimal {
        self.executed_ntl
    }

    #[must_use]
    pub const fn minutes(&self) -> u64 {
        self.minutes
    }

    #[must_use]
    pub const fn reduce_only(&self) -> bool {
        self.reduce_only
    }

    #[must_use]
    pub const fn randomize(&self) -> bool {
        self.randomize
    }

    #[must_use]
    pub const fn timestamp_millis(&self) -> i64 {
        self.timestamp_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwapHistoryRecord {
    time_millis: i64,
    twap_id: Option<u64>,
    state: TwapState,
    status: TwapStatusKind,
    status_description: Option<String>,
}

impl TwapHistoryRecord {
    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub const fn twap_id(&self) -> Option<u64> {
        self.twap_id
    }

    #[must_use]
    pub const fn state(&self) -> &TwapState {
        &self.state
    }

    #[must_use]
    pub const fn status(&self) -> TwapStatusKind {
        self.status
    }

    #[must_use]
    pub fn status_description(&self) -> Option<&str> {
        self.status_description.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwapHistory {
    records: Vec<TwapHistoryRecord>,
}

impl TwapHistory {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn records(&self) -> &[TwapHistoryRecord] {
        &self.records
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for TwapHistory {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.twap_history"])?;
        let records = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                let state_path = child(&path, "state");
                let state_object = require_object(
                    object
                        .get("state")
                        .ok_or_else(|| malformed(&state_path, "missing field"))?,
                    &state_path,
                )?;
                let status_path = child(&path, "status");
                let status_object = require_object(
                    object
                        .get("status")
                        .ok_or_else(|| malformed(&status_path, "missing field"))?,
                    &status_path,
                )?;
                Ok(TwapHistoryRecord {
                    time_millis: require_i64(object, &path, "time")?,
                    twap_id: object
                        .get("twapId")
                        .filter(|value| !value.is_null())
                        .map(|value| super::decode::u64_from_value(value, &child(&path, "twapId")))
                        .transpose()?,
                    state: TwapState {
                        coin: require_str(state_object, &state_path, "coin")?.to_owned(),
                        user: super::decode::require_address(state_object, &state_path, "user")?,
                        side: require_side(state_object, &state_path)?,
                        sz: require_decimal(state_object, &state_path, "sz")?,
                        executed_sz: require_decimal(state_object, &state_path, "executedSz")?,
                        executed_ntl: require_decimal(state_object, &state_path, "executedNtl")?,
                        minutes: require_u64(state_object, &state_path, "minutes")?,
                        reduce_only: require_bool(state_object, &state_path, "reduceOnly")?,
                        randomize: require_bool(state_object, &state_path, "randomize")?,
                        timestamp_millis: require_i64(state_object, &state_path, "timestamp")?,
                    },
                    status: TwapStatusKind::from_wire(
                        &child(&status_path, "status"),
                        require_str(status_object, &status_path, "status")?,
                    )?,
                    status_description: optional_str(status_object, &status_path, "description")?
                        .map(str::to_owned),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { records })
    }
}

pub fn parse_twap_history(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, TwapHistory), InfoError> {
    parse_family(
        "official.info.twap_history",
        raw,
        context,
        TWAP_HISTORY_KNOWN_FIELDS,
        TWAP_HISTORY_ENUM_FIELDS,
    )
}
