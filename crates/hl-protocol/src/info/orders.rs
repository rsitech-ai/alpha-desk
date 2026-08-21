use domain_types::{Decimal, MarketId};
use serde_json::Value;

use super::decode::{
    BookSide, InfoObservationKind, UserHistoryMeta, child, expect_capability, history_coverage,
    malformed, market_id_from_coin, optional_decimal, optional_str, optional_u64, parse_family,
    require_array, require_bool, require_decimal, require_i64, require_object, require_side,
    require_str, require_u64,
};
use super::{InfoEnumField, InfoError, InfoParseContext, ParsedInfoResponse};

pub const USER_FILLS_PAGE_LIMIT: usize = 2000;
pub const USER_FILLS_AVAILABLE_CAP: usize = 2000;
pub const USER_FILLS_BY_TIME_PAGE_LIMIT: usize = 2000;
pub const USER_FILLS_BY_TIME_AVAILABLE_CAP: usize = 10_000;
pub const HISTORICAL_ORDERS_PAGE_LIMIT: usize = 2000;

pub const ORDER_STATUS_NAMES: &[&str] = &[
    "open",
    "filled",
    "canceled",
    "triggered",
    "rejected",
    "marginCanceled",
    "vaultWithdrawalCanceled",
    "openInterestCapCanceled",
    "selfTradeCanceled",
    "reduceOnlyCanceled",
    "siblingFilledCanceled",
    "delistedCanceled",
    "liquidatedCanceled",
    "scheduledCancel",
    "tickRejected",
    "minTradeNtlRejected",
    "perpMarginRejected",
    "reduceOnlyRejected",
    "badAloPxRejected",
    "iocCancelRejected",
    "badTriggerPxRejected",
    "marketOrderNoLiquidityRejected",
    "positionIncreaseAtOpenInterestCapRejected",
    "positionFlipAtOpenInterestCapRejected",
    "tooAggressiveAtOpenInterestCapRejected",
    "openInterestIncreaseRejected",
    "insufficientSpotBalanceRejected",
    "oracleRejected",
    "perpMaxPositionRejected",
];

pub const ORDER_LOOKUP_NAMES: &[&str] = &["order", "unknownOid", "unknownCloid"];

pub const HISTORICAL_ORDER_ENUM_FIELDS: &[InfoEnumField] =
    &[InfoEnumField::new("/status", ORDER_STATUS_NAMES)];

pub const ORDER_STATUS_ENUM_FIELDS: &[InfoEnumField] = &[
    InfoEnumField::new("/status", ORDER_LOOKUP_NAMES),
    InfoEnumField::new("/order/status", ORDER_STATUS_NAMES),
];

pub const OPEN_ORDER_KNOWN_FIELDS: &[&str] = &[
    "/coin",
    "/limitPx",
    "/oid",
    "/side",
    "/sz",
    "/timestamp",
    "/origSz",
    "/cloid",
];

pub const FRONTEND_ORDER_KNOWN_FIELDS: &[&str] = &[
    "/coin",
    "/limitPx",
    "/oid",
    "/side",
    "/sz",
    "/timestamp",
    "/origSz",
    "/cloid",
    "/isPositionTpsl",
    "/isTrigger",
    "/orderType",
    "/reduceOnly",
    "/triggerCondition",
    "/triggerPx",
    "/children",
    "/tif",
    "/children/coin",
    "/children/limitPx",
    "/children/oid",
    "/children/side",
    "/children/sz",
    "/children/timestamp",
    "/children/origSz",
    "/children/cloid",
    "/children/isPositionTpsl",
    "/children/isTrigger",
    "/children/orderType",
    "/children/reduceOnly",
    "/children/triggerCondition",
    "/children/triggerPx",
    "/children/children",
    "/children/tif",
];

pub const FILL_KNOWN_FIELDS: &[&str] = &[
    "/closedPnl",
    "/coin",
    "/crossed",
    "/dir",
    "/hash",
    "/oid",
    "/px",
    "/side",
    "/startPosition",
    "/sz",
    "/time",
    "/fee",
    "/feeToken",
    "/builderFee",
    "/tid",
    "/twapId",
];

pub const HISTORICAL_ORDER_KNOWN_FIELDS: &[&str] = &[
    "/status",
    "/statusTimestamp",
    "/order",
    "/order/coin",
    "/order/limitPx",
    "/order/oid",
    "/order/side",
    "/order/sz",
    "/order/timestamp",
    "/order/origSz",
    "/order/cloid",
    "/order/isPositionTpsl",
    "/order/isTrigger",
    "/order/orderType",
    "/order/reduceOnly",
    "/order/triggerCondition",
    "/order/triggerPx",
    "/order/children",
    "/order/tif",
    "/order/children/coin",
    "/order/children/limitPx",
    "/order/children/oid",
    "/order/children/side",
    "/order/children/sz",
    "/order/children/timestamp",
    "/order/children/origSz",
    "/order/children/cloid",
    "/order/children/isPositionTpsl",
    "/order/children/isTrigger",
    "/order/children/orderType",
    "/order/children/reduceOnly",
    "/order/children/triggerCondition",
    "/order/children/triggerPx",
    "/order/children/children",
    "/order/children/tif",
];

pub const ORDER_STATUS_KNOWN_FIELDS: &[&str] = &[
    "/status",
    "/order",
    "/order/status",
    "/order/statusTimestamp",
    "/order/order",
    "/order/order/coin",
    "/order/order/limitPx",
    "/order/order/oid",
    "/order/order/side",
    "/order/order/sz",
    "/order/order/timestamp",
    "/order/order/origSz",
    "/order/order/cloid",
    "/order/order/isPositionTpsl",
    "/order/order/isTrigger",
    "/order/order/orderType",
    "/order/order/reduceOnly",
    "/order/order/triggerCondition",
    "/order/order/triggerPx",
    "/order/order/children",
    "/order/order/tif",
    "/order/order/children/coin",
    "/order/order/children/limitPx",
    "/order/order/children/oid",
    "/order/order/children/side",
    "/order/order/children/sz",
    "/order/order/children/timestamp",
    "/order/order/children/origSz",
    "/order/order/children/cloid",
    "/order/order/children/isPositionTpsl",
    "/order/order/children/isTrigger",
    "/order/order/children/orderType",
    "/order/order/children/reduceOnly",
    "/order/order/children/triggerCondition",
    "/order/order/children/triggerPx",
    "/order/order/children/children",
    "/order/order/children/tif",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatus {
    Open,
    Filled,
    Canceled,
    Triggered,
    Rejected,
    MarginCanceled,
    VaultWithdrawalCanceled,
    OpenInterestCapCanceled,
    SelfTradeCanceled,
    ReduceOnlyCanceled,
    SiblingFilledCanceled,
    DelistedCanceled,
    LiquidatedCanceled,
    ScheduledCancel,
    TickRejected,
    MinTradeNtlRejected,
    PerpMarginRejected,
    ReduceOnlyRejected,
    BadAloPxRejected,
    IocCancelRejected,
    BadTriggerPxRejected,
    MarketOrderNoLiquidityRejected,
    PositionIncreaseAtOpenInterestCapRejected,
    PositionFlipAtOpenInterestCapRejected,
    TooAggressiveAtOpenInterestCapRejected,
    OpenInterestIncreaseRejected,
    InsufficientSpotBalanceRejected,
    OracleRejected,
    PerpMaxPositionRejected,
}

impl OrderStatus {
    pub fn from_wire(path: &str, value: &str) -> Result<Self, InfoError> {
        let status = match value {
            "open" => Self::Open,
            "filled" => Self::Filled,
            "canceled" => Self::Canceled,
            "triggered" => Self::Triggered,
            "rejected" => Self::Rejected,
            "marginCanceled" => Self::MarginCanceled,
            "vaultWithdrawalCanceled" => Self::VaultWithdrawalCanceled,
            "openInterestCapCanceled" => Self::OpenInterestCapCanceled,
            "selfTradeCanceled" => Self::SelfTradeCanceled,
            "reduceOnlyCanceled" => Self::ReduceOnlyCanceled,
            "siblingFilledCanceled" => Self::SiblingFilledCanceled,
            "delistedCanceled" => Self::DelistedCanceled,
            "liquidatedCanceled" => Self::LiquidatedCanceled,
            "scheduledCancel" => Self::ScheduledCancel,
            "tickRejected" => Self::TickRejected,
            "minTradeNtlRejected" => Self::MinTradeNtlRejected,
            "perpMarginRejected" => Self::PerpMarginRejected,
            "reduceOnlyRejected" => Self::ReduceOnlyRejected,
            "badAloPxRejected" => Self::BadAloPxRejected,
            "iocCancelRejected" => Self::IocCancelRejected,
            "badTriggerPxRejected" => Self::BadTriggerPxRejected,
            "marketOrderNoLiquidityRejected" => Self::MarketOrderNoLiquidityRejected,
            "positionIncreaseAtOpenInterestCapRejected" => {
                Self::PositionIncreaseAtOpenInterestCapRejected
            }
            "positionFlipAtOpenInterestCapRejected" => Self::PositionFlipAtOpenInterestCapRejected,
            "tooAggressiveAtOpenInterestCapRejected" => {
                Self::TooAggressiveAtOpenInterestCapRejected
            }
            "openInterestIncreaseRejected" => Self::OpenInterestIncreaseRejected,
            "insufficientSpotBalanceRejected" => Self::InsufficientSpotBalanceRejected,
            "oracleRejected" => Self::OracleRejected,
            "perpMaxPositionRejected" => Self::PerpMaxPositionRejected,
            other => {
                return Err(InfoError::UnknownStateAffectingVariant {
                    path: path.to_owned(),
                    value: other.to_owned(),
                });
            }
        };
        Ok(status)
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Filled => "filled",
            Self::Canceled => "canceled",
            Self::Triggered => "triggered",
            Self::Rejected => "rejected",
            Self::MarginCanceled => "marginCanceled",
            Self::VaultWithdrawalCanceled => "vaultWithdrawalCanceled",
            Self::OpenInterestCapCanceled => "openInterestCapCanceled",
            Self::SelfTradeCanceled => "selfTradeCanceled",
            Self::ReduceOnlyCanceled => "reduceOnlyCanceled",
            Self::SiblingFilledCanceled => "siblingFilledCanceled",
            Self::DelistedCanceled => "delistedCanceled",
            Self::LiquidatedCanceled => "liquidatedCanceled",
            Self::ScheduledCancel => "scheduledCancel",
            Self::TickRejected => "tickRejected",
            Self::MinTradeNtlRejected => "minTradeNtlRejected",
            Self::PerpMarginRejected => "perpMarginRejected",
            Self::ReduceOnlyRejected => "reduceOnlyRejected",
            Self::BadAloPxRejected => "badAloPxRejected",
            Self::IocCancelRejected => "iocCancelRejected",
            Self::BadTriggerPxRejected => "badTriggerPxRejected",
            Self::MarketOrderNoLiquidityRejected => "marketOrderNoLiquidityRejected",
            Self::PositionIncreaseAtOpenInterestCapRejected => {
                "positionIncreaseAtOpenInterestCapRejected"
            }
            Self::PositionFlipAtOpenInterestCapRejected => "positionFlipAtOpenInterestCapRejected",
            Self::TooAggressiveAtOpenInterestCapRejected => {
                "tooAggressiveAtOpenInterestCapRejected"
            }
            Self::OpenInterestIncreaseRejected => "openInterestIncreaseRejected",
            Self::InsufficientSpotBalanceRejected => "insufficientSpotBalanceRejected",
            Self::OracleRejected => "oracleRejected",
            Self::PerpMaxPositionRejected => "perpMaxPositionRejected",
        }
    }

    #[must_use]
    pub const fn is_cancel(self) -> bool {
        matches!(
            self,
            Self::Canceled
                | Self::MarginCanceled
                | Self::VaultWithdrawalCanceled
                | Self::OpenInterestCapCanceled
                | Self::SelfTradeCanceled
                | Self::ReduceOnlyCanceled
                | Self::SiblingFilledCanceled
                | Self::DelistedCanceled
                | Self::LiquidatedCanceled
                | Self::ScheduledCancel
        )
    }

    #[must_use]
    pub const fn is_reject(self) -> bool {
        matches!(
            self,
            Self::Rejected
                | Self::TickRejected
                | Self::MinTradeNtlRejected
                | Self::PerpMarginRejected
                | Self::ReduceOnlyRejected
                | Self::BadAloPxRejected
                | Self::IocCancelRejected
                | Self::BadTriggerPxRejected
                | Self::MarketOrderNoLiquidityRejected
                | Self::PositionIncreaseAtOpenInterestCapRejected
                | Self::PositionFlipAtOpenInterestCapRejected
                | Self::TooAggressiveAtOpenInterestCapRejected
                | Self::OpenInterestIncreaseRejected
                | Self::InsufficientSpotBalanceRejected
                | Self::OracleRejected
                | Self::PerpMaxPositionRejected
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderLookupStatus {
    Order,
    UnknownOid,
    UnknownCloid,
}

impl OrderLookupStatus {
    fn from_wire(path: &str, value: &str) -> Result<Self, InfoError> {
        match value {
            "order" => Ok(Self::Order),
            "unknownOid" => Ok(Self::UnknownOid),
            "unknownCloid" => Ok(Self::UnknownCloid),
            other => Err(InfoError::UnknownStateAffectingVariant {
                path: path.to_owned(),
                value: other.to_owned(),
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenOrder {
    coin: String,
    market_id: MarketId,
    limit_px: Decimal,
    oid: u64,
    side: BookSide,
    sz: Decimal,
    timestamp_millis: i64,
    orig_sz: Option<Decimal>,
    cloid: Option<String>,
}

impl OpenOrder {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn limit_px(&self) -> Decimal {
        self.limit_px
    }

    #[must_use]
    pub const fn oid(&self) -> u64 {
        self.oid
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
    pub const fn timestamp_millis(&self) -> i64 {
        self.timestamp_millis
    }

    #[must_use]
    pub const fn orig_sz(&self) -> Option<Decimal> {
        self.orig_sz
    }

    #[must_use]
    pub fn cloid(&self) -> Option<&str> {
        self.cloid.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendOrder {
    open: OpenOrder,
    is_position_tpsl: bool,
    is_trigger: bool,
    order_type: String,
    reduce_only: bool,
    trigger_condition: String,
    trigger_px: Decimal,
    orig_sz: Decimal,
    children: Vec<Value>,
    tif: Option<String>,
}

impl FrontendOrder {
    #[must_use]
    pub const fn open(&self) -> &OpenOrder {
        &self.open
    }

    #[must_use]
    pub const fn is_position_tpsl(&self) -> bool {
        self.is_position_tpsl
    }

    #[must_use]
    pub const fn is_trigger(&self) -> bool {
        self.is_trigger
    }

    #[must_use]
    pub fn order_type(&self) -> &str {
        &self.order_type
    }

    #[must_use]
    pub const fn reduce_only(&self) -> bool {
        self.reduce_only
    }

    #[must_use]
    pub fn trigger_condition(&self) -> &str {
        &self.trigger_condition
    }

    #[must_use]
    pub const fn trigger_px(&self) -> Decimal {
        self.trigger_px
    }

    #[must_use]
    pub const fn orig_sz(&self) -> Decimal {
        self.orig_sz
    }

    #[must_use]
    pub fn children(&self) -> &[Value] {
        &self.children
    }

    #[must_use]
    pub fn tif(&self) -> Option<&str> {
        self.tif.as_deref()
    }
}

fn parse_open_order(
    object: &serde_json::Map<String, Value>,
    path: &str,
) -> Result<OpenOrder, InfoError> {
    let coin = require_str(object, path, "coin")?.to_owned();
    Ok(OpenOrder {
        market_id: market_id_from_coin(&coin)?,
        coin,
        limit_px: require_decimal(object, path, "limitPx")?,
        oid: require_u64(object, path, "oid")?,
        side: require_side(object, path)?,
        sz: require_decimal(object, path, "sz")?,
        timestamp_millis: require_i64(object, path, "timestamp")?,
        orig_sz: optional_decimal(object, path, "origSz")?,
        cloid: optional_str(object, path, "cloid")?.map(str::to_owned),
    })
}

fn parse_frontend_order(value: &Value, path: &str) -> Result<FrontendOrder, InfoError> {
    let object = require_object(value, path)?;
    let children = match object.get("children") {
        None => Vec::new(),
        Some(value) => require_array(value, &child(path, "children"))?.clone(),
    };
    Ok(FrontendOrder {
        open: parse_open_order(object, path)?,
        is_position_tpsl: require_bool(object, path, "isPositionTpsl")?,
        is_trigger: require_bool(object, path, "isTrigger")?,
        order_type: require_str(object, path, "orderType")?.to_owned(),
        reduce_only: require_bool(object, path, "reduceOnly")?,
        trigger_condition: require_str(object, path, "triggerCondition")?.to_owned(),
        trigger_px: require_decimal(object, path, "triggerPx")?,
        orig_sz: require_decimal(object, path, "origSz")?,
        children,
        tif: optional_str(object, path, "tif")?.map(str::to_owned),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenOrders {
    orders: Vec<OpenOrder>,
}

impl OpenOrders {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReconciledSnapshot
    }

    #[must_use]
    pub fn orders(&self) -> &[OpenOrder] {
        &self.orders
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for OpenOrders {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.open_orders"])?;
        let orders = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                parse_open_order(require_object(value, &path)?, &path)
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { orders })
    }
}

pub fn parse_open_orders(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, OpenOrders), InfoError> {
    parse_family(
        "official.info.open_orders",
        raw,
        context,
        OPEN_ORDER_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FrontendOpenOrders {
    orders: Vec<FrontendOrder>,
}

impl FrontendOpenOrders {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReconciledSnapshot
    }

    #[must_use]
    pub fn orders(&self) -> &[FrontendOrder] {
        &self.orders
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for FrontendOpenOrders {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.frontend_open_orders"])?;
        let orders = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| parse_frontend_order(value, &format!("/{index}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { orders })
    }
}

pub fn parse_frontend_open_orders(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, FrontendOpenOrders), InfoError> {
    parse_family(
        "official.info.frontend_open_orders",
        raw,
        context,
        FRONTEND_ORDER_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFill {
    coin: String,
    market_id: MarketId,
    closed_pnl: Decimal,
    crossed: bool,
    dir: String,
    hash: String,
    oid: u64,
    px: Decimal,
    side: BookSide,
    start_position: Decimal,
    sz: Decimal,
    time_millis: i64,
    fee: Decimal,
    fee_token: String,
    builder_fee: Option<Decimal>,
    tid: u64,
    twap_id: Option<u64>,
}

impl UserFill {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn closed_pnl(&self) -> Decimal {
        self.closed_pnl
    }

    #[must_use]
    pub const fn crossed(&self) -> bool {
        self.crossed
    }

    #[must_use]
    pub fn dir(&self) -> &str {
        &self.dir
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub const fn oid(&self) -> u64 {
        self.oid
    }

    #[must_use]
    pub const fn px(&self) -> Decimal {
        self.px
    }

    #[must_use]
    pub const fn side(&self) -> BookSide {
        self.side
    }

    #[must_use]
    pub const fn start_position(&self) -> Decimal {
        self.start_position
    }

    #[must_use]
    pub const fn sz(&self) -> Decimal {
        self.sz
    }

    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub const fn fee(&self) -> Decimal {
        self.fee
    }

    #[must_use]
    pub fn fee_token(&self) -> &str {
        &self.fee_token
    }

    #[must_use]
    pub const fn builder_fee(&self) -> Option<Decimal> {
        self.builder_fee
    }

    #[must_use]
    pub const fn tid(&self) -> u64 {
        self.tid
    }

    #[must_use]
    pub fn fill_id(&self) -> String {
        self.tid.to_string()
    }

    #[must_use]
    pub const fn twap_id(&self) -> Option<u64> {
        self.twap_id
    }
}

pub(crate) fn parse_user_fill(value: &Value, path: &str) -> Result<UserFill, InfoError> {
    let object = require_object(value, path)?;
    let coin = require_str(object, path, "coin")?.to_owned();
    Ok(UserFill {
        market_id: market_id_from_coin(&coin)?,
        coin,
        closed_pnl: require_decimal(object, path, "closedPnl")?,
        crossed: require_bool(object, path, "crossed")?,
        dir: require_str(object, path, "dir")?.to_owned(),
        hash: require_str(object, path, "hash")?.to_owned(),
        oid: require_u64(object, path, "oid")?,
        px: require_decimal(object, path, "px")?,
        side: require_side(object, path)?,
        start_position: require_decimal(object, path, "startPosition")?,
        sz: require_decimal(object, path, "sz")?,
        time_millis: require_i64(object, path, "time")?,
        fee: require_decimal(object, path, "fee")?,
        fee_token: require_str(object, path, "feeToken")?.to_owned(),
        builder_fee: optional_decimal(object, path, "builderFee")?,
        tid: require_u64(object, path, "tid")?,
        twap_id: optional_u64(object, path, "twapId")?,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFills {
    fills: Vec<UserFill>,
    history: UserHistoryMeta,
    by_time: bool,
}

impl UserFills {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn fills(&self) -> &[UserFill] {
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

fn fills_from_parsed(
    parsed: &ParsedInfoResponse<Value>,
    by_time: bool,
) -> Result<UserFills, InfoError> {
    let fills = require_array(parsed.value(), "")?
        .iter()
        .enumerate()
        .map(|(index, value)| parse_user_fill(value, &format!("/{index}")))
        .collect::<Result<Vec<_>, _>>()?;
    let earliest = fills.iter().map(|fill| fill.time_millis).min();
    let (page_limit, available_cap) = if by_time {
        (
            USER_FILLS_BY_TIME_PAGE_LIMIT,
            USER_FILLS_BY_TIME_AVAILABLE_CAP,
        )
    } else {
        (USER_FILLS_PAGE_LIMIT, USER_FILLS_AVAILABLE_CAP)
    };
    Ok(UserFills {
        history: history_coverage(fills.len(), page_limit, available_cap, earliest)?,
        fills,
        by_time,
    })
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserFills {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        match parsed.capability_id().as_str() {
            "official.info.user_fills" => fills_from_parsed(parsed, false),
            "official.info.user_fills_by_time" => fills_from_parsed(parsed, true),
            _ => Err(malformed("", "unexpected info capability")),
        }
    }
}

pub fn parse_user_fills(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserFills), InfoError> {
    parse_family(
        "official.info.user_fills",
        raw,
        context,
        FILL_KNOWN_FIELDS,
        &[],
    )
}

pub fn parse_user_fills_by_time(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserFills), InfoError> {
    parse_family(
        "official.info.user_fills_by_time",
        raw,
        context,
        FILL_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalOrder {
    order: FrontendOrder,
    status: OrderStatus,
    status_timestamp_millis: i64,
}

impl HistoricalOrder {
    #[must_use]
    pub const fn order(&self) -> &FrontendOrder {
        &self.order
    }

    #[must_use]
    pub const fn status(&self) -> OrderStatus {
        self.status
    }

    #[must_use]
    pub const fn status_timestamp_millis(&self) -> i64 {
        self.status_timestamp_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoricalOrders {
    orders: Vec<HistoricalOrder>,
    history: UserHistoryMeta,
}

impl HistoricalOrders {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn orders(&self) -> &[HistoricalOrder] {
        &self.orders
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for HistoricalOrders {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.historical_orders"])?;
        let orders = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                Ok(HistoricalOrder {
                    order: parse_frontend_order(
                        object
                            .get("order")
                            .ok_or_else(|| malformed(&child(&path, "order"), "missing field"))?,
                        &child(&path, "order"),
                    )?,
                    status: OrderStatus::from_wire(
                        &child(&path, "status"),
                        require_str(object, &path, "status")?,
                    )?,
                    status_timestamp_millis: require_i64(object, &path, "statusTimestamp")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let earliest = orders
            .iter()
            .map(|order| order.status_timestamp_millis)
            .min();
        Ok(Self {
            history: history_coverage(
                orders.len(),
                HISTORICAL_ORDERS_PAGE_LIMIT,
                HISTORICAL_ORDERS_PAGE_LIMIT,
                earliest,
            )?,
            orders,
        })
    }
}

pub fn parse_historical_orders(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, HistoricalOrders), InfoError> {
    parse_family(
        "official.info.historical_orders",
        raw,
        context,
        HISTORICAL_ORDER_KNOWN_FIELDS,
        HISTORICAL_ORDER_ENUM_FIELDS,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderStatusLookup {
    lookup: OrderLookupStatus,
    order: Option<HistoricalOrder>,
}

impl OrderStatusLookup {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::DirectLookup
    }

    #[must_use]
    pub const fn lookup(&self) -> OrderLookupStatus {
        self.lookup
    }

    #[must_use]
    pub const fn order(&self) -> Option<&HistoricalOrder> {
        self.order.as_ref()
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for OrderStatusLookup {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.order_status"])?;
        let object = require_object(parsed.value(), "")?;
        let lookup = OrderLookupStatus::from_wire("/status", require_str(object, "", "status")?)?;
        let order = match lookup {
            OrderLookupStatus::UnknownOid | OrderLookupStatus::UnknownCloid => None,
            OrderLookupStatus::Order => {
                let wrapper = require_object(
                    object
                        .get("order")
                        .ok_or_else(|| malformed("/order", "missing field"))?,
                    "/order",
                )?;
                Some(HistoricalOrder {
                    order: parse_frontend_order(
                        wrapper
                            .get("order")
                            .ok_or_else(|| malformed("/order/order", "missing field"))?,
                        "/order/order",
                    )?,
                    status: OrderStatus::from_wire(
                        "/order/status",
                        require_str(wrapper, "/order", "status")?,
                    )?,
                    status_timestamp_millis: require_i64(wrapper, "/order", "statusTimestamp")?,
                })
            }
        };
        Ok(Self { lookup, order })
    }
}

pub fn parse_order_status(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, OrderStatusLookup), InfoError> {
    parse_family(
        "official.info.order_status",
        raw,
        context,
        ORDER_STATUS_KNOWN_FIELDS,
        ORDER_STATUS_ENUM_FIELDS,
    )
}
