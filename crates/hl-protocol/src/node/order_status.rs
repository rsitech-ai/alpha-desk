use bytes::Bytes;
use serde_json::{Map, Value};

use super::v1::{NodeRecordKind, NodeStreamKind, require_object, require_string, require_u64};
use crate::SourceError;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderStatusClass {
    Open,
    Filled,
    Canceled,
    Triggered,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookSide {
    Bid,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestingOrderV1 {
    coin: String,
    side: BookSide,
    limit_px: String,
    sz: String,
    oid: u64,
    timestamp: u64,
    trigger_condition: String,
    is_trigger: bool,
    trigger_px: String,
    is_position_tpsl: bool,
    reduce_only: bool,
    order_type: String,
    orig_sz: String,
    tif: Option<String>,
    cloid: Option<String>,
}

impl RestingOrderV1 {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn side(&self) -> BookSide {
        self.side
    }

    #[must_use]
    pub fn limit_px(&self) -> &str {
        &self.limit_px
    }

    #[must_use]
    pub fn sz(&self) -> &str {
        &self.sz
    }

    #[must_use]
    pub const fn oid(&self) -> u64 {
        self.oid
    }

    #[must_use]
    pub const fn timestamp(&self) -> u64 {
        self.timestamp
    }

    #[must_use]
    pub fn trigger_condition(&self) -> &str {
        &self.trigger_condition
    }

    #[must_use]
    pub const fn is_trigger(&self) -> bool {
        self.is_trigger
    }

    #[must_use]
    pub fn trigger_px(&self) -> &str {
        &self.trigger_px
    }

    #[must_use]
    pub const fn is_position_tpsl(&self) -> bool {
        self.is_position_tpsl
    }

    #[must_use]
    pub const fn reduce_only(&self) -> bool {
        self.reduce_only
    }

    #[must_use]
    pub fn order_type(&self) -> &str {
        &self.order_type
    }

    #[must_use]
    pub fn orig_sz(&self) -> &str {
        &self.orig_sz
    }

    #[must_use]
    pub fn tif(&self) -> Option<&str> {
        self.tif.as_deref()
    }

    #[must_use]
    pub fn cloid(&self) -> Option<&str> {
        self.cloid.as_deref()
    }

    #[must_use]
    pub const fn time_priority_key(&self) -> (u64, u64) {
        (self.timestamp, self.oid)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderStatusV1 {
    time: String,
    user: String,
    status: &'static str,
    class: OrderStatusClass,
    order: RestingOrderV1,
}

impl OrderStatusV1 {
    #[must_use]
    pub fn time(&self) -> &str {
        &self.time
    }

    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    #[must_use]
    pub const fn status(&self) -> &'static str {
        self.status
    }

    #[must_use]
    pub const fn class(&self) -> OrderStatusClass {
        self.class
    }

    #[must_use]
    pub fn order(&self) -> &RestingOrderV1 {
        &self.order
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrderStatusBatchV1 {
    block_number: Option<u64>,
    block_time: Option<String>,
    statuses: Vec<OrderStatusV1>,
}

impl OrderStatusBatchV1 {
    #[must_use]
    pub const fn block_number(&self) -> Option<u64> {
        self.block_number
    }

    #[must_use]
    pub fn block_time(&self) -> Option<&str> {
        self.block_time.as_deref()
    }

    #[must_use]
    pub fn statuses(&self) -> &[OrderStatusV1] {
        &self.statuses
    }
}

pub fn classify_order_status(event: &Map<String, Value>) -> Result<NodeRecordKind, SourceError> {
    parse_order_status_event(event)?;
    Ok(NodeRecordKind::OrderStatus)
}

pub fn is_known_order_status(status: &str) -> bool {
    ORDER_STATUS_NAMES.contains(&status)
}

pub fn parse_order_status_batch(payload: Bytes) -> Result<OrderStatusBatchV1, SourceError> {
    let record = super::v1::parse_node_record(NodeStreamKind::OrderStatuses, payload)?;
    if record.kind() != NodeRecordKind::OrderStatus && record.kind() != NodeRecordKind::EmptyBatch {
        return Err(SourceError::MalformedPayload(
            "payload is not an order-status record".to_owned(),
        ));
    }
    let root: Value = serde_json::from_slice(record.payload())
        .map_err(|_| SourceError::MalformedPayload("node record is not valid JSON".to_owned()))?;
    let object = root.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("node record root must be an object".to_owned())
    })?;
    parse_order_status_root(object)
}

pub fn parse_resting_order(order: &Map<String, Value>) -> Result<RestingOrderV1, SourceError> {
    let coin = require_string(order, "coin")?.to_owned();
    let side = parse_book_side(require_string(order, "side")?)?;
    let limit_px = require_string(order, "limitPx")?.to_owned();
    let sz = require_string(order, "sz")?.to_owned();
    let oid = require_u64(order, "oid")?;
    let timestamp = require_u64(order, "timestamp")?;
    let trigger_condition = require_string(order, "triggerCondition")?.to_owned();
    let is_trigger = require_bool(order, "isTrigger")?;
    let trigger_px = require_string(order, "triggerPx")?.to_owned();
    let is_position_tpsl = require_bool(order, "isPositionTpsl")?;
    let reduce_only = require_bool(order, "reduceOnly")?;
    let order_type = require_string(order, "orderType")?.to_owned();
    let orig_sz = require_string(order, "origSz")?.to_owned();
    let tif = match order.get("tif") {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Null) => None,
        _ => {
            return Err(SourceError::MalformedPayload(
                "order tif must be a string or null".to_owned(),
            ));
        }
    };
    let cloid = match order.get("cloid") {
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Null) | None => None,
        Some(_) => {
            return Err(SourceError::MalformedPayload(
                "order cloid must be a string or null".to_owned(),
            ));
        }
    };
    Ok(RestingOrderV1 {
        coin,
        side,
        limit_px,
        sz,
        oid,
        timestamp,
        trigger_condition,
        is_trigger,
        trigger_px,
        is_position_tpsl,
        reduce_only,
        order_type,
        orig_sz,
        tif,
        cloid,
    })
}

pub fn parse_book_side(side: &str) -> Result<BookSide, SourceError> {
    match side {
        "B" | "Bid" | "buy" => Ok(BookSide::Bid),
        "A" | "Ask" | "sell" => Ok(BookSide::Ask),
        _ => Err(SourceError::SchemaDrift(format!(
            "unknown node order side {side}"
        ))),
    }
}

fn parse_order_status_root(root: &Map<String, Value>) -> Result<OrderStatusBatchV1, SourceError> {
    let Some(events) = root.get("events") else {
        let status = parse_order_status_event(root)?;
        return Ok(OrderStatusBatchV1 {
            block_number: None,
            block_time: None,
            statuses: vec![status],
        });
    };
    let block_time = require_string(root, "block_time")?.to_owned();
    let block_number = require_u64(root, "block_number")?;
    let events = events.as_array().ok_or_else(|| {
        SourceError::MalformedPayload("batched node events must be an array".to_owned())
    })?;
    let mut statuses = Vec::with_capacity(events.len());
    for event in events {
        let event = event.as_object().ok_or_else(|| {
            SourceError::MalformedPayload("node event must be an object".to_owned())
        })?;
        statuses.push(parse_order_status_event(event)?);
    }
    Ok(OrderStatusBatchV1 {
        block_number: Some(block_number),
        block_time: Some(block_time),
        statuses,
    })
}

fn parse_order_status_event(event: &Map<String, Value>) -> Result<OrderStatusV1, SourceError> {
    let time = require_string(event, "time")?.to_owned();
    let user = require_string(event, "user")?.to_owned();
    let status = require_string(event, "status")?;
    let status = ORDER_STATUS_NAMES
        .iter()
        .copied()
        .find(|known| *known == status)
        .ok_or_else(|| SourceError::SchemaDrift("unknown node order-status variant".to_owned()))?;
    let order = parse_resting_order(require_object(event, "order")?)?;
    Ok(OrderStatusV1 {
        time,
        user,
        status,
        class: status_class(status),
        order,
    })
}

fn status_class(status: &'static str) -> OrderStatusClass {
    match status {
        "open" => OrderStatusClass::Open,
        "filled" => OrderStatusClass::Filled,
        "triggered" => OrderStatusClass::Triggered,
        "rejected"
        | "tickRejected"
        | "minTradeNtlRejected"
        | "perpMarginRejected"
        | "reduceOnlyRejected"
        | "badAloPxRejected"
        | "iocCancelRejected"
        | "badTriggerPxRejected"
        | "marketOrderNoLiquidityRejected"
        | "positionIncreaseAtOpenInterestCapRejected"
        | "positionFlipAtOpenInterestCapRejected"
        | "tooAggressiveAtOpenInterestCapRejected"
        | "openInterestIncreaseRejected"
        | "insufficientSpotBalanceRejected"
        | "oracleRejected"
        | "perpMaxPositionRejected" => OrderStatusClass::Rejected,
        "canceled"
        | "marginCanceled"
        | "vaultWithdrawalCanceled"
        | "openInterestCapCanceled"
        | "selfTradeCanceled"
        | "reduceOnlyCanceled"
        | "siblingFilledCanceled"
        | "delistedCanceled"
        | "liquidatedCanceled"
        | "scheduledCancel" => OrderStatusClass::Canceled,
        _ => unreachable!("ORDER_STATUS_NAMES is the closed catalog"),
    }
}

fn require_bool(object: &Map<String, Value>, field: &str) -> Result<bool, SourceError> {
    object.get(field).and_then(Value::as_bool).ok_or_else(|| {
        SourceError::MalformedPayload(format!("node record has no boolean field {field}"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceError;

    fn documented_status() -> Value {
        serde_json::json!({
            "time": "2024-07-26T08:31:48.717",
            "user": "0xc64cc00b46101bd40aa1c3121195e85c0b0918d8",
            "status": "canceled",
            "order": {
                "coin": "INJ",
                "side": "A",
                "limitPx": "25.381",
                "sz": "257.0",
                "oid": 12212359592_u64,
                "timestamp": 1721982700270_u64,
                "triggerCondition": "N/A",
                "isTrigger": false,
                "triggerPx": "0.0",
                "children": [],
                "isPositionTpsl": false,
                "reduceOnly": false,
                "orderType": "Limit",
                "origSz": "257.0",
                "tif": "Alo",
                "cloid": null
            }
        })
    }

    #[test]
    fn every_documented_status_parses() {
        for name in ORDER_STATUS_NAMES {
            let mut body = documented_status();
            body["status"] = serde_json::json!(name);
            let parsed = parse_order_status_batch(Bytes::from(serde_json::to_vec(&body).unwrap()))
                .unwrap_or_else(|_| panic!("{name} must parse"));
            assert_eq!(parsed.statuses()[0].status(), *name);
        }
        assert_eq!(ORDER_STATUS_NAMES.len(), 29);
    }

    #[test]
    fn unknown_status_is_schema_drift() {
        let mut body = documented_status();
        body["status"] = serde_json::json!("mysteryCanceled");
        let error = parse_order_status_batch(Bytes::from(serde_json::to_vec(&body).unwrap()))
            .expect_err("unknown status");
        assert!(matches!(error, SourceError::SchemaDrift(_)));
        assert_eq!(error.reason_code(), "source.schema_drift");
    }

    #[test]
    fn same_price_orders_sort_by_timestamp_then_oid() {
        let mut first = documented_status();
        first["order"]["limitPx"] = serde_json::json!("25.381");
        first["order"]["timestamp"] = serde_json::json!(100_u64);
        first["order"]["oid"] = serde_json::json!(2_u64);
        let mut second = documented_status();
        second["order"]["limitPx"] = serde_json::json!("25.381");
        second["order"]["timestamp"] = serde_json::json!(100_u64);
        second["order"]["oid"] = serde_json::json!(1_u64);
        let mut earlier = documented_status();
        earlier["order"]["limitPx"] = serde_json::json!("25.381");
        earlier["order"]["timestamp"] = serde_json::json!(50_u64);
        earlier["order"]["oid"] = serde_json::json!(9_u64);
        let mut orders = [
            parse_order_status_batch(Bytes::from(serde_json::to_vec(&first).unwrap())).unwrap(),
            parse_order_status_batch(Bytes::from(serde_json::to_vec(&second).unwrap())).unwrap(),
            parse_order_status_batch(Bytes::from(serde_json::to_vec(&earlier).unwrap())).unwrap(),
        ];
        orders.sort_by_key(|batch| batch.statuses()[0].order().time_priority_key());
        assert_eq!(orders[0].statuses()[0].order().oid(), 9);
        assert_eq!(orders[1].statuses()[0].order().oid(), 1);
        assert_eq!(orders[2].statuses()[0].order().oid(), 2);
    }

    #[test]
    fn trigger_metadata_is_retained() {
        let mut body = documented_status();
        body["status"] = serde_json::json!("triggered");
        body["order"]["isTrigger"] = serde_json::json!(true);
        body["order"]["triggerPx"] = serde_json::json!("24.5");
        body["order"]["triggerCondition"] = serde_json::json!("Price below 24.5");
        body["order"]["isPositionTpsl"] = serde_json::json!(true);
        let parsed =
            parse_order_status_batch(Bytes::from(serde_json::to_vec(&body).unwrap())).unwrap();
        let order = parsed.statuses()[0].order();
        assert!(order.is_trigger());
        assert_eq!(order.trigger_px(), "24.5");
        assert_eq!(order.trigger_condition(), "Price below 24.5");
        assert!(order.is_position_tpsl());
        assert_eq!(parsed.statuses()[0].class(), OrderStatusClass::Triggered);
    }
}
