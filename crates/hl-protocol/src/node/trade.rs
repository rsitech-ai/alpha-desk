use bytes::Bytes;
use serde_json::{Map, Value};

use super::v1::{
    NodeRecordKind, NodeStreamKind, require_optional_string, require_optional_u64, require_string,
    require_u64,
};
use crate::SourceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeAggressor {
    Buyer,
    Seller,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeSideV1 {
    user: String,
    start_pos: String,
    oid: u64,
    twap_id: Option<u64>,
    cloid: Option<String>,
}

impl TradeSideV1 {
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    #[must_use]
    pub fn start_pos(&self) -> &str {
        &self.start_pos
    }

    #[must_use]
    pub const fn oid(&self) -> u64 {
        self.oid
    }

    #[must_use]
    pub const fn twap_id(&self) -> Option<u64> {
        self.twap_id
    }

    #[must_use]
    pub fn cloid(&self) -> Option<&str> {
        self.cloid.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeV1 {
    coin: String,
    side: String,
    time: String,
    px: String,
    sz: String,
    hash: String,
    trade_dir_override: String,
    buyer: TradeSideV1,
    seller: TradeSideV1,
    aggressor: TradeAggressor,
}

impl TradeV1 {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub fn side(&self) -> &str {
        &self.side
    }

    #[must_use]
    pub fn time(&self) -> &str {
        &self.time
    }

    #[must_use]
    pub fn px(&self) -> &str {
        &self.px
    }

    #[must_use]
    pub fn sz(&self) -> &str {
        &self.sz
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn trade_dir_override(&self) -> &str {
        &self.trade_dir_override
    }

    #[must_use]
    pub fn buyer(&self) -> &TradeSideV1 {
        &self.buyer
    }

    #[must_use]
    pub fn seller(&self) -> &TradeSideV1 {
        &self.seller
    }

    #[must_use]
    pub const fn aggressor(&self) -> TradeAggressor {
        self.aggressor
    }

    #[must_use]
    pub const fn taker_oid(&self) -> u64 {
        match self.aggressor {
            TradeAggressor::Buyer => self.buyer.oid,
            TradeAggressor::Seller => self.seller.oid,
        }
    }

    #[must_use]
    pub const fn maker_oid(&self) -> u64 {
        match self.aggressor {
            TradeAggressor::Buyer => self.seller.oid,
            TradeAggressor::Seller => self.buyer.oid,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TradeBatchV1 {
    block_number: Option<u64>,
    block_time: Option<String>,
    trades: Vec<TradeV1>,
}

impl TradeBatchV1 {
    #[must_use]
    pub const fn block_number(&self) -> Option<u64> {
        self.block_number
    }

    #[must_use]
    pub fn block_time(&self) -> Option<&str> {
        self.block_time.as_deref()
    }

    #[must_use]
    pub fn trades(&self) -> &[TradeV1] {
        &self.trades
    }
}

pub fn classify_trade(event: &Map<String, Value>) -> Result<NodeRecordKind, SourceError> {
    parse_trade_event(event)?;
    Ok(NodeRecordKind::Trade)
}

pub fn parse_trade_batch(payload: Bytes) -> Result<TradeBatchV1, SourceError> {
    let record = super::v1::parse_node_record(NodeStreamKind::Trades, payload)?;
    if record.kind() != NodeRecordKind::Trade && record.kind() != NodeRecordKind::EmptyBatch {
        return Err(SourceError::MalformedPayload(
            "payload is not a trade record".to_owned(),
        ));
    }
    let root: Value = serde_json::from_slice(record.payload())
        .map_err(|_| SourceError::MalformedPayload("node record is not valid JSON".to_owned()))?;
    let object = root.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("node record root must be an object".to_owned())
    })?;
    parse_trade_root(object)
}

fn parse_trade_root(root: &Map<String, Value>) -> Result<TradeBatchV1, SourceError> {
    let Some(events) = root.get("events") else {
        let trade = parse_trade_event(root)?;
        return Ok(TradeBatchV1 {
            block_number: None,
            block_time: None,
            trades: vec![trade],
        });
    };
    let block_time = require_string(root, "block_time")?.to_owned();
    let block_number = require_u64(root, "block_number")?;
    let events = events.as_array().ok_or_else(|| {
        SourceError::MalformedPayload("batched node events must be an array".to_owned())
    })?;
    let mut trades = Vec::with_capacity(events.len());
    for event in events {
        let event = event.as_object().ok_or_else(|| {
            SourceError::MalformedPayload("node event must be an object".to_owned())
        })?;
        trades.push(parse_trade_event(event)?);
    }
    Ok(TradeBatchV1 {
        block_number: Some(block_number),
        block_time: Some(block_time),
        trades,
    })
}

fn parse_trade_event(event: &Map<String, Value>) -> Result<TradeV1, SourceError> {
    let coin = require_string(event, "coin")?.to_owned();
    let side = require_string(event, "side")?.to_owned();
    let time = require_string(event, "time")?.to_owned();
    let px = require_string(event, "px")?.to_owned();
    let sz = require_string(event, "sz")?.to_owned();
    let hash = require_string(event, "hash")?.to_owned();
    let trade_dir_override = require_string(event, "trade_dir_override")?.to_owned();
    let side_info = event
        .get("side_info")
        .and_then(Value::as_array)
        .filter(|value| value.len() == 2)
        .ok_or_else(|| {
            SourceError::MalformedPayload(
                "node trade side_info must contain buyer and seller".to_owned(),
            )
        })?;
    let buyer = parse_trade_side(side_object(&side_info[0])?)?;
    let seller = parse_trade_side(side_object(&side_info[1])?)?;
    let aggressor = trade_aggressor(&side, &trade_dir_override)?;
    Ok(TradeV1 {
        coin,
        side,
        time,
        px,
        sz,
        hash,
        trade_dir_override,
        buyer,
        seller,
        aggressor,
    })
}

fn side_object(value: &Value) -> Result<&Map<String, Value>, SourceError> {
    value.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("node trade side_info entry must be an object".to_owned())
    })
}

fn parse_trade_side(side: &Map<String, Value>) -> Result<TradeSideV1, SourceError> {
    let user = require_string(side, "user")?.to_owned();
    let start_pos = require_string(side, "start_pos")?.to_owned();
    let oid = require_u64(side, "oid")?;
    require_optional_u64(side, "twap_id")?;
    require_optional_string(side, "cloid")?;
    let twap_id = match side.get("twap_id") {
        Some(Value::Null) | None => None,
        Some(Value::Number(value)) => Some(value.as_u64().ok_or_else(|| {
            SourceError::MalformedPayload(
                "node record field twap_id must be null or an unsigned integer".to_owned(),
            )
        })?),
        Some(_) => {
            return Err(SourceError::MalformedPayload(
                "node record field twap_id must be null or an unsigned integer".to_owned(),
            ));
        }
    };
    let cloid = match side.get("cloid") {
        Some(Value::Null) | None => None,
        Some(Value::String(value)) => Some(value.clone()),
        Some(_) => {
            return Err(SourceError::MalformedPayload(
                "node record field cloid must be null or a string".to_owned(),
            ));
        }
    };
    Ok(TradeSideV1 {
        user,
        start_pos,
        oid,
        twap_id,
        cloid,
    })
}

fn trade_aggressor(side: &str, trade_dir_override: &str) -> Result<TradeAggressor, SourceError> {
    if trade_dir_override != "Na" {
        return Err(SourceError::SchemaDrift(format!(
            "unknown node trade_dir_override {trade_dir_override}"
        )));
    }
    match side {
        "B" => Ok(TradeAggressor::Buyer),
        "A" => Ok(TradeAggressor::Seller),
        _ => Err(SourceError::SchemaDrift(format!(
            "unknown node trade side {side}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceError;

    fn documented_trade() -> Value {
        serde_json::json!({
            "local_time": "2026-07-29T12:00:00",
            "block_time": "2024-07-26T08:26:25.899",
            "block_number": 42_u64,
            "events": [{
                "coin": "COMP",
                "side": "B",
                "time": "2024-07-26T08:26:25.899",
                "px": "51.367",
                "sz": "0.31",
                "hash": "0xad8e0566e813bdf98176040e6d51bd011100efa789e89430cdf17964235f55d8",
                "trade_dir_override": "Na",
                "side_info": [
                    {
                        "user": "0xc64cc00b46101bd40aa1c3121195e85c0b0918d8",
                        "start_pos": "996.67",
                        "oid": 12212201265_u64,
                        "twap_id": null,
                        "cloid": null
                    },
                    {
                        "user": "0x768484f7e2ebb675c57838366c02ae99ba2a9b08",
                        "start_pos": "-996.7",
                        "oid": 12212198275_u64,
                        "twap_id": null,
                        "cloid": null
                    }
                ]
            }]
        })
    }

    #[test]
    fn documented_buy_aggressor_makes_buyer_the_taker() {
        let parsed = parse_trade_batch(Bytes::from(
            serde_json::to_vec(&documented_trade()).unwrap(),
        ))
        .expect("documented trade");
        assert_eq!(parsed.block_number(), Some(42));
        assert_eq!(parsed.trades().len(), 1);
        let trade = &parsed.trades()[0];
        assert_eq!(trade.aggressor(), TradeAggressor::Buyer);
        assert_eq!(trade.taker_oid(), 12212201265);
        assert_eq!(trade.maker_oid(), 12212198275);
        assert_eq!(trade.buyer().oid(), 12212201265);
        assert_eq!(trade.seller().oid(), 12212198275);
        let replay = parse_trade_batch(Bytes::from(
            serde_json::to_vec(&documented_trade()).unwrap(),
        ))
        .unwrap();
        assert_eq!(parsed, replay);
    }

    #[test]
    fn ask_aggressor_makes_seller_the_taker() {
        let mut body = documented_trade();
        body["events"][0]["side"] = serde_json::json!("A");
        let parsed = parse_trade_batch(Bytes::from(serde_json::to_vec(&body).unwrap())).unwrap();
        let trade = &parsed.trades()[0];
        assert_eq!(trade.aggressor(), TradeAggressor::Seller);
        assert_eq!(trade.taker_oid(), 12212198275);
        assert_eq!(trade.maker_oid(), 12212201265);
    }

    #[test]
    fn unknown_trade_dir_override_is_schema_drift() {
        let mut body = documented_trade();
        body["events"][0]["trade_dir_override"] = serde_json::json!("Buy");
        let error = parse_trade_batch(Bytes::from(serde_json::to_vec(&body).unwrap()))
            .expect_err("undocumented override");
        assert!(matches!(error, SourceError::SchemaDrift(_)));
        assert_eq!(error.reason_code(), "source.schema_drift");
    }

    #[test]
    fn json_float_price_is_malformed() {
        let mut body = documented_trade();
        body["events"][0]["px"] = serde_json::json!(51.367);
        let error =
            parse_trade_batch(Bytes::from(serde_json::to_vec(&body).unwrap())).expect_err("float");
        assert!(matches!(error, SourceError::MalformedPayload(_)));
        assert_eq!(error.reason_code(), "source.malformed_payload");
    }
}
