use std::collections::BTreeMap;
use std::str::FromStr;

use domain_types::{Address, Decimal, MarketId, ValueError};
use serde_json::{Map, Value};

use super::{
    InfoEnumField, InfoError, InfoParseContext, InfoRegistry, ParsedInfoResponse, TimePageCoverage,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InfoObservationKind {
    ReferenceSnapshot,
    ReconciledSnapshot,
    BoundedHistory,
    DirectLookup,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookSide {
    Bid,
    Ask,
}

impl BookSide {
    pub fn from_wire(path: &str, value: &str) -> Result<Self, InfoError> {
        match value {
            "B" => Ok(Self::Bid),
            "A" => Ok(Self::Ask),
            _ => Err(InfoError::UnknownStateAffectingVariant {
                path: path.to_owned(),
                value: value.to_owned(),
            }),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bid => "B",
            Self::Ask => "A",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserHistoryMeta {
    page_limit: usize,
    available_cap: usize,
    received: usize,
    coverage: TimePageCoverage,
}

impl UserHistoryMeta {
    #[must_use]
    pub const fn page_limit(&self) -> usize {
        self.page_limit
    }

    #[must_use]
    pub const fn available_cap(&self) -> usize {
        self.available_cap
    }

    #[must_use]
    pub const fn received(&self) -> usize {
        self.received
    }

    #[must_use]
    pub const fn coverage(&self) -> &TimePageCoverage {
        &self.coverage
    }
}

pub fn history_coverage(
    received: usize,
    page_limit: usize,
    available_cap: usize,
    earliest_reliable_millis: Option<i64>,
) -> Result<UserHistoryMeta, InfoError> {
    let truncated = received >= page_limit;
    let earliest = if truncated {
        earliest_reliable_millis
    } else {
        None
    };
    Ok(UserHistoryMeta {
        page_limit,
        available_cap,
        received,
        coverage: TimePageCoverage::new(truncated, earliest, Vec::new())?,
    })
}

pub fn market_id_from_coin(coin: &str) -> Result<MarketId, InfoError> {
    if coin.is_empty() || coin.trim() != coin {
        return Err(malformed("/coin", "empty coin"));
    }
    // ponytail: protocol coin only. UI remaps (BTC/USDC vs UBTC/USDC) stay
    // presentation. T07 can join spotMeta token names if a registry is needed.
    let id = if coin.starts_with('@') || coin.contains('/') {
        format!("spot:{coin}")
    } else {
        format!("perp:{coin}")
    };
    MarketId::new(id).map_err(|_| malformed("/coin", "invalid market id"))
}

pub(crate) fn parse_family<T>(
    capability_id: &str,
    raw: &[u8],
    context: InfoParseContext,
    known_fields: &'static [&'static str],
    enum_fields: &'static [InfoEnumField],
) -> Result<(ParsedInfoResponse<Value>, T), InfoError>
where
    T: for<'a> TryFrom<&'a ParsedInfoResponse<Value>, Error = InfoError>,
{
    let parsed = InfoRegistry::official().get(capability_id)?.parse(
        raw,
        &context
            .with_known_fields(known_fields)
            .with_enum_fields(enum_fields),
    )?;
    let typed = T::try_from(&parsed)?;
    Ok((parsed, typed))
}

pub(crate) fn expect_capability(
    parsed: &ParsedInfoResponse<Value>,
    allowed: &[&str],
) -> Result<(), InfoError> {
    if allowed.contains(&parsed.capability_id().as_str()) {
        Ok(())
    } else {
        Err(malformed("", "unexpected info capability"))
    }
}

pub(crate) fn malformed(path: &str, reason: &'static str) -> InfoError {
    InfoError::MalformedPayload {
        path: path.to_owned(),
        reason,
    }
}

pub(crate) fn child(parent: &str, key: &str) -> String {
    if parent.is_empty() {
        format!("/{key}")
    } else {
        format!("{parent}/{key}")
    }
}

pub(crate) fn require_object<'a>(
    value: &'a Value,
    path: &str,
) -> Result<&'a Map<String, Value>, InfoError> {
    value
        .as_object()
        .ok_or_else(|| malformed(path, "expected object"))
}

pub(crate) fn require_array<'a>(value: &'a Value, path: &str) -> Result<&'a Vec<Value>, InfoError> {
    value
        .as_array()
        .ok_or_else(|| malformed(path, "expected array"))
}

pub(crate) fn field<'a>(
    object: &'a Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<&'a Value, InfoError> {
    object
        .get(key)
        .ok_or_else(|| malformed(&child(parent, key), "missing field"))
}

pub(crate) fn optional_field<'a>(object: &'a Map<String, Value>, key: &str) -> Option<&'a Value> {
    match object.get(key) {
        None | Some(Value::Null) => None,
        Some(value) => Some(value),
    }
}

pub(crate) fn require_str<'a>(
    object: &'a Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<&'a str, InfoError> {
    field(object, parent, key)?
        .as_str()
        .ok_or_else(|| malformed(&child(parent, key), "expected string"))
}

pub(crate) fn optional_str<'a>(
    object: &'a Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Option<&'a str>, InfoError> {
    match optional_field(object, key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(Some)
            .ok_or_else(|| malformed(&child(parent, key), "expected string")),
    }
}

pub(crate) fn require_bool(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<bool, InfoError> {
    field(object, parent, key)?
        .as_bool()
        .ok_or_else(|| malformed(&child(parent, key), "expected bool"))
}

pub(crate) fn optional_bool(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Option<bool>, InfoError> {
    match optional_field(object, key) {
        None => Ok(None),
        Some(value) => value
            .as_bool()
            .map(Some)
            .ok_or_else(|| malformed(&child(parent, key), "expected bool")),
    }
}

pub(crate) fn require_object_field<'a>(
    object: &'a Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<&'a Map<String, Value>, InfoError> {
    require_object(field(object, parent, key)?, &child(parent, key))
}

pub(crate) fn require_array_field<'a>(
    object: &'a Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<&'a Vec<Value>, InfoError> {
    require_array(field(object, parent, key)?, &child(parent, key))
}

pub(crate) fn optional_i64(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Option<i64>, InfoError> {
    match optional_field(object, key) {
        None => Ok(None),
        Some(value) => i64_from_value(value, &child(parent, key)).map(Some),
    }
}

pub(crate) fn optional_address(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Option<Address>, InfoError> {
    match optional_str(object, parent, key)? {
        None => Ok(None),
        Some(text) => address_from_str(text, &child(parent, key)).map(Some),
    }
}

pub(crate) fn require_u64(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<u64, InfoError> {
    u64_from_value(field(object, parent, key)?, &child(parent, key))
}

pub(crate) fn optional_u64(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Option<u64>, InfoError> {
    match optional_field(object, key) {
        None => Ok(None),
        Some(value) => u64_from_value(value, &child(parent, key)).map(Some),
    }
}

pub(crate) fn require_i64(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<i64, InfoError> {
    i64_from_value(field(object, parent, key)?, &child(parent, key))
}

pub(crate) fn u64_from_value(value: &Value, path: &str) -> Result<u64, InfoError> {
    value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .ok_or_else(|| malformed(path, "expected u64"))
}

pub(crate) fn i64_from_value(value: &Value, path: &str) -> Result<i64, InfoError> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|n| i64::try_from(n).ok()))
        .ok_or_else(|| malformed(path, "expected i64"))
}

pub(crate) fn require_decimal(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Decimal, InfoError> {
    decimal_from_value(field(object, parent, key)?, &child(parent, key))
}

pub(crate) fn optional_decimal(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Option<Decimal>, InfoError> {
    match optional_field(object, key) {
        None => Ok(None),
        Some(value) => decimal_from_value(value, &child(parent, key)).map(Some),
    }
}

pub(crate) fn decimal_from_value(value: &Value, path: &str) -> Result<Decimal, InfoError> {
    let text = value
        .as_str()
        .ok_or_else(|| malformed(path, "expected decimal string"))?;
    match Decimal::from_str(text) {
        Ok(value) => Ok(value),
        Err(ValueError::Overflow | ValueError::OutOfRange) => Err(InfoError::DecimalOverflow {
            path: path.to_owned(),
        }),
        Err(ValueError::ScaleOutOfRange { .. } | ValueError::ExcessPrecision { .. }) => {
            Err(InfoError::DecimalInvalidScale {
                path: path.to_owned(),
            })
        }
        Err(_) => Err(InfoError::DecimalInvalid {
            path: path.to_owned(),
        }),
    }
}

pub(crate) fn require_address(
    object: &Map<String, Value>,
    parent: &str,
    key: &str,
) -> Result<Address, InfoError> {
    address_from_str(require_str(object, parent, key)?, &child(parent, key))
}

pub(crate) fn address_from_str(text: &str, path: &str) -> Result<Address, InfoError> {
    Address::parse_api(text).map_err(|_| malformed(path, "invalid address"))
}

pub(crate) fn require_side(
    object: &Map<String, Value>,
    parent: &str,
) -> Result<BookSide, InfoError> {
    BookSide::from_wire(&child(parent, "side"), require_str(object, parent, "side")?)
}

pub(crate) fn pair_entries<'a>(
    value: &'a Value,
    path: &str,
) -> Result<Vec<(&'a Value, &'a Value)>, InfoError> {
    let array = require_array(value, path)?;
    array
        .iter()
        .enumerate()
        .map(|(index, entry)| {
            let item_path = format!("{path}/{index}");
            let pair = require_array(entry, &item_path)?;
            if pair.len() != 2 {
                return Err(malformed(&item_path, "expected [key, value] pair"));
            }
            Ok((&pair[0], &pair[1]))
        })
        .collect()
}

pub(crate) fn object_map_mids(
    value: &Value,
) -> Result<BTreeMap<String, (MarketId, Decimal)>, InfoError> {
    let object = require_object(value, "")?;
    let mut mids = BTreeMap::new();
    for (coin, px) in object {
        let path = child("", coin);
        let market_id = market_id_from_coin(coin)?;
        let px = decimal_from_value(px, &path)?;
        mids.insert(coin.clone(), (market_id, px));
    }
    Ok(mids)
}

pub(crate) fn string_list(value: &Value, path: &str) -> Result<Vec<String>, InfoError> {
    require_array(value, path)?
        .iter()
        .enumerate()
        .map(|(index, item)| {
            item.as_str()
                .map(str::to_owned)
                .ok_or_else(|| malformed(&format!("{path}/{index}"), "expected string"))
        })
        .collect()
}

pub(crate) fn decimal_list(value: &Value, path: &str) -> Result<Vec<Decimal>, InfoError> {
    require_array(value, path)?
        .iter()
        .enumerate()
        .map(|(index, item)| decimal_from_value(item, &format!("{path}/{index}")))
        .collect()
}

pub const DEPLOY_AUCTION_KNOWN_FIELDS: &[&str] = &[
    "/startTimeSeconds",
    "/durationSeconds",
    "/startGas",
    "/currentGas",
    "/endGas",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployAuction {
    start_time_seconds: i64,
    duration_seconds: u64,
    start_gas: Decimal,
    current_gas: Option<Decimal>,
    end_gas: Option<Decimal>,
}

impl DeployAuction {
    #[must_use]
    pub const fn start_time_seconds(&self) -> i64 {
        self.start_time_seconds
    }

    #[must_use]
    pub const fn duration_seconds(&self) -> u64 {
        self.duration_seconds
    }

    #[must_use]
    pub const fn start_gas(&self) -> Decimal {
        self.start_gas
    }

    #[must_use]
    pub const fn current_gas(&self) -> Option<Decimal> {
        self.current_gas
    }

    #[must_use]
    pub const fn end_gas(&self) -> Option<Decimal> {
        self.end_gas
    }

    pub(crate) fn from_value(value: &Value, path: &str) -> Result<Self, InfoError> {
        let object = require_object(value, path)?;
        Ok(Self {
            start_time_seconds: require_i64(object, path, "startTimeSeconds")?,
            duration_seconds: require_u64(object, path, "durationSeconds")?,
            start_gas: require_decimal(object, path, "startGas")?,
            current_gas: optional_decimal(object, path, "currentGas")?,
            end_gas: optional_decimal(object, path, "endGas")?,
        })
    }
}
