use std::collections::BTreeMap;

use bytes::Bytes;
use serde_json::{Map, Value};

use super::registry::family_by_identifier;
use crate::SourceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WsAckMethod {
    Subscribe,
    Unsubscribe,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WsSubscription {
    identifier: String,
    fields: BTreeMap<String, Value>,
    identity: blake3::Hash,
    canonical: String,
}

impl WsSubscription {
    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    #[must_use]
    pub const fn identity(&self) -> blake3::Hash {
        self.identity
    }

    #[must_use]
    pub fn canonical_json(&self) -> &str {
        &self.canonical
    }

    #[must_use]
    pub fn field(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    #[must_use]
    pub fn to_value(&self) -> Value {
        Value::Object(
            self.fields
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
        )
    }
}

pub fn parse_subscription(value: &Value) -> Result<WsSubscription, SourceError> {
    let object = value.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("websocket subscription must be an object".to_owned())
    })?;
    let identifier = object
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SourceError::MalformedPayload("websocket subscription has no non-empty type".to_owned())
        })?;
    let family = family_by_identifier(identifier).ok_or_else(|| {
        SourceError::SchemaDrift(format!("unknown websocket subscription type {identifier}"))
    })?;
    if family.user_scoped {
        require_non_empty_string(object, "user")?;
    }
    if family.coin_scoped {
        require_non_empty_string(object, "coin")?;
    }
    if family.requires_interval {
        require_non_empty_string(object, "interval")?;
    }
    let fields: BTreeMap<String, Value> = object
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let canonical_object: Map<String, Value> = fields
        .iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    let canonical = serde_json::to_string(&Value::Object(canonical_object)).map_err(|_| {
        SourceError::MalformedPayload("websocket subscription is not serializable".to_owned())
    })?;
    let identity = blake3::hash(canonical.as_bytes());
    Ok(WsSubscription {
        identifier: identifier.to_owned(),
        fields,
        identity,
        canonical,
    })
}

pub fn encode_subscribe(subscription: &WsSubscription) -> Result<Bytes, SourceError> {
    encode_method("subscribe", subscription)
}

pub fn encode_unsubscribe(subscription: &WsSubscription) -> Result<Bytes, SourceError> {
    encode_method("unsubscribe", subscription)
}

fn encode_method(method: &str, subscription: &WsSubscription) -> Result<Bytes, SourceError> {
    let mut root = Map::new();
    root.insert("method".to_owned(), Value::String(method.to_owned()));
    root.insert("subscription".to_owned(), subscription.to_value());
    let encoded = serde_json::to_vec(&Value::Object(root)).map_err(|_| {
        SourceError::MalformedPayload("websocket subscribe encoding failed".to_owned())
    })?;
    Ok(Bytes::from(encoded))
}

fn require_non_empty_string<'a>(
    object: &'a Map<String, Value>,
    field: &str,
) -> Result<&'a str, SourceError> {
    object
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            SourceError::MalformedPayload(format!(
                "websocket subscription has no non-empty string field {field}"
            ))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn ws_subscription_identity_is_stable_across_key_order() {
        let left = parse_subscription(&json!({"type":"trades","coin":"BTC"})).expect("left");
        let right = parse_subscription(&json!({"coin":"BTC","type":"trades"})).expect("right");
        assert_eq!(left.identity(), right.identity());
        assert_eq!(left.canonical_json(), right.canonical_json());
        assert_eq!(left.identifier(), "trades");
    }

    #[test]
    fn ws_user_subscription_requires_user() {
        let error =
            parse_subscription(&json!({"type":"userFills"})).expect_err("user-scoped subscription");
        assert!(matches!(error, SourceError::MalformedPayload(_)));
    }

    #[test]
    fn ws_unknown_subscription_type_is_schema_drift() {
        let error =
            parse_subscription(&json!({"type":"fastAssetCtxs"})).expect_err("not in T02 registry");
        assert!(matches!(error, SourceError::SchemaDrift(_)));
        assert_eq!(error.reason_code(), "source.schema_drift");
    }

    #[test]
    fn ws_subscribe_encoding_is_deterministic() {
        let subscription =
            parse_subscription(&json!({"type":"allMids","dex":"xyz"})).expect("subscription");
        let first = encode_subscribe(&subscription).expect("encode");
        let second = encode_subscribe(&subscription).expect("encode again");
        assert_eq!(first, second);
    }
}
