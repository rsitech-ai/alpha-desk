use bytes::Bytes;
use serde_json::{Map, Value};

use super::v1::{NodeRecordKind, NodeRecordV1, NodeStreamKind};
use crate::SourceError;

/// Official ABCI/L4 snapshot cadence: one file every 10,000 committed heights.
pub const PERIODIC_SNAPSHOT_STRIDE: u64 = 10_000;

const L4_ORDER_STRING_FIELDS: &[&str] = &[
    "coin",
    "side",
    "limitPx",
    "sz",
    "triggerCondition",
    "triggerPx",
    "orderType",
    "origSz",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct L4SnapshotV1 {
    coins: usize,
    orders: usize,
    content_hash: blake3::Hash,
}

impl L4SnapshotV1 {
    #[must_use]
    pub const fn coins(&self) -> usize {
        self.coins
    }

    #[must_use]
    pub const fn orders(&self) -> usize {
        self.orders
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        self.content_hash
    }
}

pub fn parse_abci_state_snapshot(payload: Bytes) -> Result<NodeRecordV1, SourceError> {
    // ponytail: official L1 docs give `periodic_abci_states/{date}/{height}.rmp`
    // and a 10_000-block cadence, not the MessagePack object layout. Exact-byte
    // BLAKE3 is the snapshot identity until a qualified corpus documents fields.
    Ok(NodeRecordV1::from_parts(
        NodeStreamKind::AbciStateSnapshots,
        NodeRecordKind::AbciStateSnapshot,
        None,
        payload,
    ))
}

pub fn parse_l4_snapshot(payload: Bytes) -> Result<NodeRecordV1, SourceError> {
    parse_l4_snapshot_record(&payload)?;
    Ok(NodeRecordV1::from_parts(
        NodeStreamKind::L4Snapshots,
        NodeRecordKind::L4Snapshot,
        None,
        payload,
    ))
}

pub fn parse_l4_snapshot_record(payload: &[u8]) -> Result<L4SnapshotV1, SourceError> {
    let root: Value = serde_json::from_slice(payload)
        .map_err(|_| SourceError::MalformedPayload("l4 snapshot is not valid JSON".to_owned()))?;
    let markets = root.as_array().ok_or_else(|| {
        SourceError::MalformedPayload("l4 snapshot root must be an array".to_owned())
    })?;
    let mut orders = 0_usize;
    for market in markets {
        let pair = market
            .as_array()
            .filter(|value| value.len() == 2)
            .ok_or_else(|| {
                SourceError::MalformedPayload(
                    "l4 snapshot market must be a coin/book pair".to_owned(),
                )
            })?;
        if pair[0].as_str().filter(|coin| !coin.is_empty()).is_none() {
            return Err(SourceError::MalformedPayload(
                "l4 snapshot market has no coin".to_owned(),
            ));
        }
        let sides = pair[1]
            .as_array()
            .filter(|value| value.len() == 2)
            .ok_or_else(|| {
                SourceError::MalformedPayload("l4 snapshot book must be bids then asks".to_owned())
            })?;
        orders += count_side(&sides[0])?;
        orders += count_side(&sides[1])?;
    }
    Ok(L4SnapshotV1 {
        coins: markets.len(),
        orders,
        content_hash: blake3::hash(payload),
    })
}

fn count_side(side: &Value) -> Result<usize, SourceError> {
    let orders = side.as_array().ok_or_else(|| {
        SourceError::MalformedPayload("l4 snapshot side must be an array".to_owned())
    })?;
    for order in orders {
        let order = order.as_object().ok_or_else(|| {
            SourceError::MalformedPayload("l4 snapshot order must be an object".to_owned())
        })?;
        validate_l4_order(order)?;
    }
    Ok(orders.len())
}

fn validate_l4_order(order: &Map<String, Value>) -> Result<(), SourceError> {
    for field in L4_ORDER_STRING_FIELDS {
        match order.get(*field).and_then(Value::as_str) {
            Some(value) if !value.is_empty() => {}
            _ => {
                return Err(SourceError::MalformedPayload(format!(
                    "l4 snapshot order has no non-empty string field {field}"
                )));
            }
        }
    }
    if order.get("oid").and_then(Value::as_u64).is_none()
        || order.get("timestamp").and_then(Value::as_u64).is_none()
    {
        return Err(SourceError::MalformedPayload(
            "l4 snapshot order has no oid or timestamp".to_owned(),
        ));
    }
    for field in ["isTrigger", "isPositionTpsl", "reduceOnly"] {
        if order.get(field).and_then(Value::as_bool).is_none() {
            return Err(SourceError::MalformedPayload(format!(
                "l4 snapshot order has no boolean field {field}"
            )));
        }
    }
    if !matches!(order.get("tif"), Some(Value::String(_) | Value::Null)) {
        return Err(SourceError::MalformedPayload(
            "l4 snapshot order tif must be a string or null".to_owned(),
        ));
    }
    if !matches!(
        order.get("cloid"),
        Some(Value::String(_) | Value::Null) | None
    ) {
        return Err(SourceError::MalformedPayload(
            "l4 snapshot order cloid must be a string or null".to_owned(),
        ));
    }
    if !matches!(order.get("children"), Some(Value::Array(_)) | None) {
        return Err(SourceError::MalformedPayload(
            "l4 snapshot order children must be an array".to_owned(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::v1::parse_node_record;

    fn l4_fixture() -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!([[
            "BTC",
            [
                [{
                    "coin": "BTC",
                    "side": "B",
                    "limitPx": "103988.0",
                    "sz": "0.2782",
                    "oid": 30112287571_u64,
                    "timestamp": 1747157301016_u64,
                    "triggerCondition": "N/A",
                    "isTrigger": false,
                    "triggerPx": "0.0",
                    "children": [],
                    "isPositionTpsl": false,
                    "reduceOnly": false,
                    "orderType": "Limit",
                    "origSz": "0.2782",
                    "tif": "Alo",
                    "cloid": null
                }],
                [{
                    "coin": "BTC",
                    "side": "A",
                    "limitPx": "93708.0",
                    "sz": "0.00047",
                    "oid": 30073539988_u64,
                    "timestamp": 1747128626867_u64,
                    "triggerCondition": "Price below 101856",
                    "isTrigger": true,
                    "triggerPx": "101856.0",
                    "children": [],
                    "isPositionTpsl": false,
                    "reduceOnly": true,
                    "orderType": "Stop Market",
                    "origSz": "0.00047",
                    "tif": null,
                    "cloid": null
                }]
            ]
        ]]))
        .expect("l4 json")
    }

    #[test]
    fn abci_snapshot_hash_is_reproducible_over_exact_bytes() {
        let payload =
            Bytes::from_static(&[0x81, 0xa5, b'r', b'o', b'u', b'n', b'd', 0xcd, 0x27, 0x10]);
        let first = parse_abci_state_snapshot(payload.clone()).expect("first");
        let second = parse_abci_state_snapshot(payload.clone()).expect("second");
        assert_eq!(first.kind(), NodeRecordKind::AbciStateSnapshot);
        assert_eq!(first.content_hash(), second.content_hash());
        assert_eq!(first.content_hash(), blake3::hash(&payload));
        assert_eq!(first.payload().as_ref(), payload.as_ref());
    }

    #[test]
    fn l4_snapshot_hash_is_reproducible_and_counts_orders() {
        let payload = l4_fixture();
        let parsed = parse_l4_snapshot_record(&payload).expect("l4");
        assert_eq!(parsed.coins(), 1);
        assert_eq!(parsed.orders(), 2);
        let record = parse_node_record(NodeStreamKind::L4Snapshots, Bytes::from(payload.clone()))
            .expect("record");
        assert_eq!(record.kind(), NodeRecordKind::L4Snapshot);
        assert_eq!(record.content_hash(), parsed.content_hash());
        assert_eq!(record.content_hash(), blake3::hash(&payload));
        let again = parse_l4_snapshot_record(&payload).expect("replay");
        assert_eq!(parsed, again);
    }

    #[test]
    fn unknown_l4_order_shape_is_malformed() {
        let payload = serde_json::to_vec(&serde_json::json!([["BTC", [[], [{"coin": "BTC"}]]]]))
            .expect("json");
        let error = parse_l4_snapshot(Bytes::from(payload)).expect_err("missing fields");
        assert!(matches!(error, SourceError::MalformedPayload(_)));
    }
}
