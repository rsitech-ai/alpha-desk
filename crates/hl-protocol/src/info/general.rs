use std::collections::BTreeMap;

use domain_types::{Decimal, MarketId};
use serde_json::Value;

use super::decode::{
    BookSide, InfoObservationKind, UserHistoryMeta, child, history_coverage, i64_from_value,
    malformed, market_id_from_coin, object_map_mids, parse_family, require_array, require_decimal,
    require_i64, require_object, require_str, require_u64, u64_from_value,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const CANDLE_AVAILABLE_CAP: usize = 5000;
pub const CANDLE_KNOWN_FIELDS: &[&str] =
    &["/T", "/c", "/h", "/i", "/l", "/n", "/o", "/s", "/t", "/v"];
pub const ALL_MIDS_KNOWN_FIELDS: &[&str] = &["/*"];
pub const L2_BOOK_KNOWN_FIELDS: &[&str] = &["/coin", "/time", "/levels", "/px", "/sz", "/n"];
pub const EXCHANGE_STATUS_KNOWN_FIELDS: &[&str] =
    &["/time", "/specialStatuses", "/specialStatuses/*"];
pub const RECENT_TRADE_KNOWN_FIELDS: &[&str] = &[
    "/coin", "/px", "/sz", "/side", "/time", "/tid", "/hash", "/users",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllMids {
    mids: BTreeMap<String, MidPrice>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidPrice {
    coin: String,
    market_id: MarketId,
    px: Decimal,
}

impl MidPrice {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn px(&self) -> Decimal {
        self.px
    }
}

impl AllMids {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn mids(&self) -> &BTreeMap<String, MidPrice> {
        &self.mids
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for AllMids {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        super::decode::expect_capability(parsed, &["official.info.all_mids"])?;
        let mut mids = BTreeMap::new();
        for (coin, (market_id, px)) in object_map_mids(parsed.value())? {
            mids.insert(
                coin.clone(),
                MidPrice {
                    coin,
                    market_id,
                    px,
                },
            );
        }
        Ok(Self { mids })
    }
}

pub fn parse_all_mids(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, AllMids), InfoError> {
    parse_family(
        "official.info.all_mids",
        raw,
        context,
        ALL_MIDS_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2Book {
    coin: String,
    market_id: MarketId,
    time_millis: i64,
    bids: Vec<L2Level>,
    asks: Vec<L2Level>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L2Level {
    px: Decimal,
    sz: Decimal,
    n: u64,
}

impl L2Level {
    #[must_use]
    pub const fn px(&self) -> Decimal {
        self.px
    }

    #[must_use]
    pub const fn sz(&self) -> Decimal {
        self.sz
    }

    #[must_use]
    pub const fn n(&self) -> u64 {
        self.n
    }
}

impl L2Book {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub fn bids(&self) -> &[L2Level] {
        &self.bids
    }

    #[must_use]
    pub fn asks(&self) -> &[L2Level] {
        &self.asks
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for L2Book {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        super::decode::expect_capability(parsed, &["official.info.l2_book"])?;
        let object = require_object(parsed.value(), "")?;
        let coin = require_str(object, "", "coin")?.to_owned();
        let market_id = market_id_from_coin(&coin)?;
        let levels = require_array(
            object
                .get("levels")
                .ok_or_else(|| malformed("/levels", "missing field"))?,
            "/levels",
        )?;
        if levels.len() != 2 {
            return Err(malformed("/levels", "expected [bids, asks]"));
        }
        Ok(Self {
            coin,
            market_id,
            time_millis: require_i64(object, "", "time")?,
            bids: parse_levels(&levels[0], "/levels/0")?,
            asks: parse_levels(&levels[1], "/levels/1")?,
        })
    }
}

fn parse_levels(value: &Value, path: &str) -> Result<Vec<L2Level>, InfoError> {
    require_array(value, path)?
        .iter()
        .enumerate()
        .map(|(index, level)| {
            let item = format!("{path}/{index}");
            let object = require_object(level, &item)?;
            Ok(L2Level {
                px: require_decimal(object, &item, "px")?,
                sz: require_decimal(object, &item, "sz")?,
                n: require_u64(object, &item, "n")?,
            })
        })
        .collect()
}

pub fn parse_l2_book(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, L2Book), InfoError> {
    parse_family(
        "official.info.l2_book",
        raw,
        context,
        L2_BOOK_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandleSnapshot {
    candles: Vec<Candle>,
    history: UserHistoryMeta,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candle {
    open_time_millis: i64,
    close_time_millis: i64,
    coin: String,
    market_id: MarketId,
    interval: String,
    open: Decimal,
    close: Decimal,
    high: Decimal,
    low: Decimal,
    volume: Decimal,
    trade_count: u64,
}

impl Candle {
    #[must_use]
    pub const fn open_time_millis(&self) -> i64 {
        self.open_time_millis
    }

    #[must_use]
    pub const fn close_time_millis(&self) -> i64 {
        self.close_time_millis
    }

    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub fn interval(&self) -> &str {
        &self.interval
    }

    #[must_use]
    pub const fn open(&self) -> Decimal {
        self.open
    }

    #[must_use]
    pub const fn close(&self) -> Decimal {
        self.close
    }

    #[must_use]
    pub const fn high(&self) -> Decimal {
        self.high
    }

    #[must_use]
    pub const fn low(&self) -> Decimal {
        self.low
    }

    #[must_use]
    pub const fn volume(&self) -> Decimal {
        self.volume
    }

    #[must_use]
    pub const fn trade_count(&self) -> u64 {
        self.trade_count
    }
}

impl CandleSnapshot {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn candles(&self) -> &[Candle] {
        &self.candles
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }

    #[must_use]
    pub fn known_fields() -> &'static [&'static str] {
        CANDLE_KNOWN_FIELDS
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for CandleSnapshot {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        super::decode::expect_capability(parsed, &["official.info.candle_snapshot"])?;
        let candles = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| parse_candle(value, &format!("/{index}")))
            .collect::<Result<Vec<_>, _>>()?;
        let earliest = candles.iter().map(|candle| candle.open_time_millis).min();
        Ok(Self {
            history: history_coverage(
                candles.len(),
                CANDLE_AVAILABLE_CAP,
                CANDLE_AVAILABLE_CAP,
                earliest,
            )?,
            candles,
        })
    }
}

fn parse_candle(value: &Value, path: &str) -> Result<Candle, InfoError> {
    let object = require_object(value, path)?;
    let coin = require_str(object, path, "s")?.to_owned();
    Ok(Candle {
        open_time_millis: require_i64(object, path, "t")?,
        close_time_millis: require_i64(object, path, "T")?,
        market_id: market_id_from_coin(&coin)?,
        coin,
        interval: require_str(object, path, "i")?.to_owned(),
        open: require_decimal(object, path, "o")?,
        close: require_decimal(object, path, "c")?,
        high: require_decimal(object, path, "h")?,
        low: require_decimal(object, path, "l")?,
        volume: require_decimal(object, path, "v")?,
        trade_count: require_u64(object, path, "n")?,
    })
}

pub fn parse_candle_snapshot(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, CandleSnapshot), InfoError> {
    parse_family(
        "official.info.candle_snapshot",
        raw,
        context,
        CANDLE_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeStatus {
    time_millis: i64,
    special_statuses: Value,
}

impl ExchangeStatus {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub const fn special_statuses(&self) -> &Value {
        &self.special_statuses
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for ExchangeStatus {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        super::decode::expect_capability(parsed, &["official.info.exchange_status"])?;
        let object = require_object(parsed.value(), "")?;
        Ok(Self {
            time_millis: require_i64(object, "", "time")?,
            special_statuses: object
                .get("specialStatuses")
                .cloned()
                .unwrap_or(Value::Null),
        })
    }
}

pub fn parse_exchange_status(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, ExchangeStatus), InfoError> {
    parse_family(
        "official.info.exchange_status",
        raw,
        context,
        EXCHANGE_STATUS_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentTrades {
    trades: Vec<PublicTrade>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicTrade {
    coin: Option<String>,
    market_id: Option<MarketId>,
    px: Decimal,
    sz: Decimal,
    side: BookSide,
    time_millis: i64,
    tid: u64,
    hash: Option<String>,
}

impl PublicTrade {
    #[must_use]
    pub fn coin(&self) -> Option<&str> {
        self.coin.as_deref()
    }

    #[must_use]
    pub const fn market_id(&self) -> Option<&MarketId> {
        self.market_id.as_ref()
    }

    #[must_use]
    pub const fn px(&self) -> Decimal {
        self.px
    }

    #[must_use]
    pub const fn sz(&self) -> Decimal {
        self.sz
    }

    #[must_use]
    pub const fn side(&self) -> BookSide {
        self.side
    }

    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub const fn tid(&self) -> u64 {
        self.tid
    }

    #[must_use]
    pub fn hash(&self) -> Option<&str> {
        self.hash.as_deref()
    }
}

impl RecentTrades {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn trades(&self) -> &[PublicTrade] {
        &self.trades
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for RecentTrades {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        super::decode::expect_capability(parsed, &["official.info.recent_trades"])?;
        let trades = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| parse_public_trade(value, &format!("/{index}")))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { trades })
    }
}

fn parse_public_trade(value: &Value, path: &str) -> Result<PublicTrade, InfoError> {
    let object = require_object(value, path)?;
    let coin = object
        .get("coin")
        .filter(|value| !value.is_null())
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| malformed(&child(path, "coin"), "expected string"))
                .map(str::to_owned)
        })
        .transpose()?;
    let market_id = coin.as_deref().map(market_id_from_coin).transpose()?;
    Ok(PublicTrade {
        coin,
        market_id,
        px: require_decimal(object, path, "px")?,
        sz: require_decimal(object, path, "sz")?,
        side: BookSide::from_wire(&child(path, "side"), require_str(object, path, "side")?)?,
        time_millis: match object.get("time") {
            Some(value) => i64_from_value(value, &child(path, "time"))?,
            None => return Err(malformed(&child(path, "time"), "missing field")),
        },
        tid: match object.get("tid") {
            Some(value) => u64_from_value(value, &child(path, "tid"))?,
            None => return Err(malformed(&child(path, "tid"), "missing field")),
        },
        hash: object
            .get("hash")
            .filter(|value| !value.is_null())
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| malformed(&child(path, "hash"), "expected string"))
                    .map(str::to_owned)
            })
            .transpose()?,
    })
}

pub fn parse_recent_trades(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, RecentTrades), InfoError> {
    parse_family(
        "official.info.recent_trades",
        raw,
        context,
        RECENT_TRADE_KNOWN_FIELDS,
        &[],
    )
}
