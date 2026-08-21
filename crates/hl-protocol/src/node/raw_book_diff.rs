use bytes::Bytes;
use serde_json::{Map, Value};

use super::order_status::{BookSide, parse_book_side};
use super::v1::{
    NodeRecordKind, NodeStreamKind, require_string, require_u64, require_value_object,
};
use crate::SourceError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RawBookDiffOp {
    New { sz: String },
    Update { sz: String },
    Remove,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBookDiffV1 {
    user: String,
    oid: u64,
    coin: String,
    side: BookSide,
    px: String,
    op: RawBookDiffOp,
}

impl RawBookDiffV1 {
    #[must_use]
    pub fn user(&self) -> &str {
        &self.user
    }

    #[must_use]
    pub const fn oid(&self) -> u64 {
        self.oid
    }

    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn side(&self) -> BookSide {
        self.side
    }

    #[must_use]
    pub fn px(&self) -> &str {
        &self.px
    }

    #[must_use]
    pub fn op(&self) -> &RawBookDiffOp {
        &self.op
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBookDiffBatchV1 {
    block_number: Option<u64>,
    block_time: Option<String>,
    diffs: Vec<RawBookDiffV1>,
}

impl RawBookDiffBatchV1 {
    #[must_use]
    pub const fn block_number(&self) -> Option<u64> {
        self.block_number
    }

    #[must_use]
    pub fn block_time(&self) -> Option<&str> {
        self.block_time.as_deref()
    }

    #[must_use]
    pub fn diffs(&self) -> &[RawBookDiffV1] {
        &self.diffs
    }
}

pub fn classify_raw_book_diff(event: &Map<String, Value>) -> Result<NodeRecordKind, SourceError> {
    parse_raw_book_diff_event(event)?;
    Ok(NodeRecordKind::RawBookDiff)
}

pub fn parse_raw_book_diff_batch(payload: Bytes) -> Result<RawBookDiffBatchV1, SourceError> {
    let record = super::v1::parse_node_record(NodeStreamKind::RawBookDiffs, payload)?;
    if record.kind() != NodeRecordKind::RawBookDiff && record.kind() != NodeRecordKind::EmptyBatch {
        return Err(SourceError::MalformedPayload(
            "payload is not a raw-book-diff record".to_owned(),
        ));
    }
    let root: Value = serde_json::from_slice(record.payload())
        .map_err(|_| SourceError::MalformedPayload("node record is not valid JSON".to_owned()))?;
    let object = root.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("node record root must be an object".to_owned())
    })?;
    parse_raw_book_diff_root(object)
}

fn parse_raw_book_diff_root(root: &Map<String, Value>) -> Result<RawBookDiffBatchV1, SourceError> {
    let Some(events) = root.get("events") else {
        let diff = parse_raw_book_diff_event(root)?;
        return Ok(RawBookDiffBatchV1 {
            block_number: None,
            block_time: None,
            diffs: vec![diff],
        });
    };
    let block_time = require_string(root, "block_time")?.to_owned();
    let block_number = require_u64(root, "block_number")?;
    let events = events.as_array().ok_or_else(|| {
        SourceError::MalformedPayload("batched node events must be an array".to_owned())
    })?;
    let mut diffs = Vec::with_capacity(events.len());
    for event in events {
        let event = event.as_object().ok_or_else(|| {
            SourceError::MalformedPayload("node event must be an object".to_owned())
        })?;
        diffs.push(parse_raw_book_diff_event(event)?);
    }
    Ok(RawBookDiffBatchV1 {
        block_number: Some(block_number),
        block_time: Some(block_time),
        diffs,
    })
}

fn parse_raw_book_diff_event(event: &Map<String, Value>) -> Result<RawBookDiffV1, SourceError> {
    let oid = require_u64(event, "oid")?;
    let coin = require_string(event, "coin")?.to_owned();
    let user = require_string(event, "user")?.to_owned();
    let side = parse_book_side(require_string(event, "side")?)?;
    let px = require_string(event, "px")?.to_owned();
    let op = parse_diff_op(event.get("raw_book_diff"))?;
    Ok(RawBookDiffV1 {
        user,
        oid,
        coin,
        side,
        px,
        op,
    })
}

fn parse_diff_op(value: Option<&Value>) -> Result<RawBookDiffOp, SourceError> {
    match value {
        Some(Value::String(operation)) if operation == "remove" => Ok(RawBookDiffOp::Remove),
        Some(Value::Object(operation)) if operation.len() == 1 => {
            let Some((variant, payload)) = operation.iter().next() else {
                return Err(SourceError::MalformedPayload(
                    "raw book diff is empty".to_owned(),
                ));
            };
            let body = require_value_object(payload, "raw book diff payload")?;
            let sz = require_string(body, "sz")?.to_owned();
            match variant.as_str() {
                "new" => Ok(RawBookDiffOp::New { sz }),
                "update" => Ok(RawBookDiffOp::Update { sz }),
                _ => Err(SourceError::SchemaDrift(
                    "unknown raw-book-diff variant".to_owned(),
                )),
            }
        }
        Some(_) => Err(SourceError::MalformedPayload(
            "raw book diff has an invalid shape".to_owned(),
        )),
        None => Err(SourceError::MalformedPayload(
            "raw book diff is missing".to_owned(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SourceError;

    fn documented_new() -> Value {
        serde_json::json!({
            "user": "0x768484f7e2ebb675c57838366c02ae99ba2a9b08",
            "oid": 35061046831_u64,
            "coin": "CHILLGUY",
            "side": "Bid",
            "px": "1.36",
            "raw_book_diff": { "new": { "sz": "186910.0" } }
        })
    }

    #[test]
    fn new_update_and_remove_parse() {
        let new =
            parse_raw_book_diff_batch(Bytes::from(serde_json::to_vec(&documented_new()).unwrap()))
                .unwrap();
        assert!(matches!(new.diffs()[0].op(), RawBookDiffOp::New { sz } if sz == "186910.0"));
        assert_eq!(new.diffs()[0].side(), BookSide::Bid);

        let mut update = documented_new();
        update["raw_book_diff"] = serde_json::json!({ "update": { "sz": "100.0" } });
        let parsed =
            parse_raw_book_diff_batch(Bytes::from(serde_json::to_vec(&update).unwrap())).unwrap();
        assert!(matches!(parsed.diffs()[0].op(), RawBookDiffOp::Update { sz } if sz == "100.0"));

        let mut remove = documented_new();
        remove["raw_book_diff"] = serde_json::json!("remove");
        let parsed =
            parse_raw_book_diff_batch(Bytes::from(serde_json::to_vec(&remove).unwrap())).unwrap();
        assert_eq!(parsed.diffs()[0].op(), &RawBookDiffOp::Remove);
    }

    #[test]
    fn unknown_diff_variant_is_schema_drift() {
        let mut body = documented_new();
        body["raw_book_diff"] = serde_json::json!({ "replace": { "sz": "1" } });
        let error = parse_raw_book_diff_batch(Bytes::from(serde_json::to_vec(&body).unwrap()))
            .expect_err("unknown variant");
        assert!(matches!(error, SourceError::SchemaDrift(_)));
        assert_eq!(error.reason_code(), "source.schema_drift");
    }

    #[test]
    fn json_float_size_is_malformed() {
        let mut body = documented_new();
        body["raw_book_diff"] = serde_json::json!({ "new": { "sz": 186910.0 } });
        let error = parse_raw_book_diff_batch(Bytes::from(serde_json::to_vec(&body).unwrap()))
            .expect_err("float");
        assert!(matches!(error, SourceError::MalformedPayload(_)));
        assert_eq!(error.reason_code(), "source.malformed_payload");
    }
}
