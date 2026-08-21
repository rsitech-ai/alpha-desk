use bytes::Bytes;
use serde_json::{Map, Value};

use super::registry::{
    PayloadShape, SnapshotPolicy, VariantClassifier, WsFamily, family_by_channel,
};
use super::subscription::{WsAckMethod, WsSubscription, parse_subscription};
use super::user_events::{
    UserEventKind, classify_ledger_updates, classify_user_event, object_has_state_affecting_key,
};
use crate::{ObservationClass, SourceError};

const MAX_WS_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum WsObservation {
    Ack(WsAck),
    Snapshot(WsSnapshot),
    Incremental(WsIncremental),
    Heartbeat(WsHeartbeat),
    Unknown(WsUnknown),
}

impl WsObservation {
    #[must_use]
    pub const fn payload(&self) -> &Bytes {
        match self {
            Self::Ack(value) => &value.payload,
            Self::Snapshot(value) => &value.payload,
            Self::Incremental(value) => &value.payload,
            Self::Heartbeat(value) => &value.payload,
            Self::Unknown(value) => &value.payload,
        }
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        match self {
            Self::Ack(value) => value.content_hash,
            Self::Snapshot(value) => value.content_hash,
            Self::Incremental(value) => value.content_hash,
            Self::Heartbeat(value) => value.content_hash,
            Self::Unknown(value) => value.content_hash,
        }
    }

    #[must_use]
    pub const fn observation_class(&self) -> ObservationClass {
        match self {
            Self::Ack(_) | Self::Heartbeat(_) | Self::Unknown(_) => {
                ObservationClass::ProvisionalFeed
            }
            Self::Snapshot(value) => value.observation_class,
            Self::Incremental(value) => value.observation_class,
        }
    }

    #[must_use]
    pub fn identifier(&self) -> Option<&str> {
        match self {
            Self::Ack(value) => Some(value.subscription.identifier()),
            Self::Snapshot(value) => Some(value.identifier),
            Self::Incremental(value) => Some(value.identifier),
            Self::Heartbeat(_) | Self::Unknown(_) => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WsAck {
    method: WsAckMethod,
    subscription: WsSubscription,
    payload: Bytes,
    content_hash: blake3::Hash,
}

impl WsAck {
    #[must_use]
    pub const fn method(&self) -> WsAckMethod {
        self.method
    }

    #[must_use]
    pub const fn subscription(&self) -> &WsSubscription {
        &self.subscription
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WsSnapshot {
    identifier: &'static str,
    channel: &'static str,
    flagged_is_snapshot: bool,
    user_event: Option<UserEventKind>,
    observation_class: ObservationClass,
    payload: Bytes,
    content_hash: blake3::Hash,
}

impl WsSnapshot {
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        self.identifier
    }

    #[must_use]
    pub const fn channel(&self) -> &'static str {
        self.channel
    }

    #[must_use]
    pub const fn flagged_is_snapshot(&self) -> bool {
        self.flagged_is_snapshot
    }

    #[must_use]
    pub const fn user_event(&self) -> Option<UserEventKind> {
        self.user_event
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WsIncremental {
    identifier: &'static str,
    channel: &'static str,
    flagged_is_snapshot: bool,
    user_event: Option<UserEventKind>,
    observation_class: ObservationClass,
    payload: Bytes,
    content_hash: blake3::Hash,
}

impl WsIncremental {
    #[must_use]
    pub const fn identifier(&self) -> &'static str {
        self.identifier
    }

    #[must_use]
    pub const fn channel(&self) -> &'static str {
        self.channel
    }

    #[must_use]
    pub const fn flagged_is_snapshot(&self) -> bool {
        self.flagged_is_snapshot
    }

    #[must_use]
    pub const fn user_event(&self) -> Option<UserEventKind> {
        self.user_event
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct WsHeartbeat {
    payload: Bytes,
    content_hash: blake3::Hash,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WsUnknown {
    channel: String,
    payload: Bytes,
    content_hash: blake3::Hash,
}

impl WsUnknown {
    #[must_use]
    pub fn channel(&self) -> &str {
        &self.channel
    }
}

pub fn parse_ws_message(payload: Bytes) -> Result<WsObservation, SourceError> {
    if payload.is_empty() || payload.len() > MAX_WS_MESSAGE_BYTES {
        return Err(SourceError::MalformedPayload(
            "websocket message size is outside the supported range".to_owned(),
        ));
    }
    let root: Value = serde_json::from_slice(&payload).map_err(|_| {
        SourceError::MalformedPayload("websocket message is not valid JSON".to_owned())
    })?;
    let object = root.as_object().ok_or_else(|| {
        SourceError::MalformedPayload("websocket message root must be an object".to_owned())
    })?;
    let content_hash = blake3::hash(&payload);
    if let Some(channel) = object.get("channel").and_then(Value::as_str) {
        return parse_channeled(channel, object, payload, content_hash);
    }
    if let Some(method) = object.get("method").and_then(Value::as_str)
        && matches!(method, "ping" | "pong")
    {
        return Ok(WsObservation::Heartbeat(WsHeartbeat {
            payload,
            content_hash,
        }));
    }
    Err(SourceError::MalformedPayload(
        "websocket message has no channel".to_owned(),
    ))
}

fn parse_channeled(
    channel: &str,
    object: &Map<String, Value>,
    payload: Bytes,
    content_hash: blake3::Hash,
) -> Result<WsObservation, SourceError> {
    match channel {
        "pong" => Ok(WsObservation::Heartbeat(WsHeartbeat {
            payload,
            content_hash,
        })),
        "subscriptionResponse" => parse_ack(object, payload, content_hash),
        other => match family_by_channel(other) {
            Some(family) => {
                let data = object.get("data").ok_or_else(|| {
                    SourceError::MalformedPayload(
                        "websocket data message has no data field".to_owned(),
                    )
                })?;
                parse_family(family, data, payload, content_hash)
            }
            None => parse_unknown(other, object.get("data"), payload, content_hash),
        },
    }
}

fn parse_ack(
    object: &Map<String, Value>,
    payload: Bytes,
    content_hash: blake3::Hash,
) -> Result<WsObservation, SourceError> {
    let data = object
        .get("data")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            SourceError::MalformedPayload("subscription ack has no data object".to_owned())
        })?;
    let method = match data.get("method").and_then(Value::as_str) {
        Some("subscribe") => WsAckMethod::Subscribe,
        Some("unsubscribe") => WsAckMethod::Unsubscribe,
        Some(other) => {
            return Err(SourceError::MalformedPayload(format!(
                "subscription ack has unknown method {other}"
            )));
        }
        None => {
            return Err(SourceError::MalformedPayload(
                "subscription ack has no method".to_owned(),
            ));
        }
    };
    let subscription = data.get("subscription").ok_or_else(|| {
        SourceError::MalformedPayload("subscription ack has no subscription".to_owned())
    })?;
    Ok(WsObservation::Ack(WsAck {
        method,
        subscription: parse_subscription(subscription)?,
        payload,
        content_hash,
    }))
}

fn parse_family(
    family: &WsFamily,
    data: &Value,
    payload: Bytes,
    content_hash: blake3::Hash,
) -> Result<WsObservation, SourceError> {
    match family.payload_shape {
        PayloadShape::Object => {
            data.as_object().ok_or_else(|| {
                SourceError::MalformedPayload(format!(
                    "{} data must be an object",
                    family.identifier
                ))
            })?;
        }
        PayloadShape::Array => {
            data.as_array().ok_or_else(|| {
                SourceError::MalformedPayload(format!(
                    "{} data must be an array",
                    family.identifier
                ))
            })?;
        }
        PayloadShape::Either => {
            if !data.is_object() && !data.is_array() {
                return Err(SourceError::MalformedPayload(format!(
                    "{} data must be an object or array",
                    family.identifier
                )));
            }
        }
    }
    if let Some(field) = family.data_array_field {
        let object = data.as_object().ok_or_else(|| {
            SourceError::MalformedPayload(format!(
                "{} data must be an object to hold {field}",
                family.identifier
            ))
        })?;
        let array = object.get(field).ok_or_else(|| {
            SourceError::MalformedPayload(format!(
                "{} data has no array field {field}",
                family.identifier
            ))
        })?;
        array.as_array().ok_or_else(|| {
            SourceError::MalformedPayload(format!(
                "{} field {field} must be an array",
                family.identifier
            ))
        })?;
        if family.variant_classifier == VariantClassifier::LedgerDelta {
            classify_ledger_updates(array)?;
        }
    }
    let user_event = match family.variant_classifier {
        VariantClassifier::None | VariantClassifier::LedgerDelta => None,
        VariantClassifier::UserEvent => Some(classify_user_event(data)?),
    };
    let flagged = data
        .as_object()
        .and_then(|object| object.get("isSnapshot"))
        .and_then(Value::as_bool);
    let as_snapshot = match family.snapshot_policy {
        SnapshotPolicy::FullReplace => true,
        SnapshotPolicy::Tagged => flagged == Some(true),
        SnapshotPolicy::Incremental => false,
    };
    let flagged_is_snapshot = flagged.unwrap_or(as_snapshot);
    if as_snapshot {
        Ok(WsObservation::Snapshot(WsSnapshot {
            identifier: family.identifier,
            channel: family.channel,
            flagged_is_snapshot,
            user_event,
            observation_class: family.snapshot_class,
            payload,
            content_hash,
        }))
    } else {
        Ok(WsObservation::Incremental(WsIncremental {
            identifier: family.identifier,
            channel: family.channel,
            flagged_is_snapshot,
            user_event,
            observation_class: family.incremental_class,
            payload,
            content_hash,
        }))
    }
}

fn parse_unknown(
    channel: &str,
    data: Option<&Value>,
    payload: Bytes,
    content_hash: blake3::Hash,
) -> Result<WsObservation, SourceError> {
    if looks_state_affecting(data) {
        return Err(SourceError::SchemaDrift(format!(
            "unknown state-affecting websocket channel {channel}"
        )));
    }
    Ok(WsObservation::Unknown(WsUnknown {
        channel: channel.to_owned(),
        payload,
        content_hash,
    }))
}

fn looks_state_affecting(data: Option<&Value>) -> bool {
    match data {
        Some(Value::Object(object)) => object_has_state_affecting_key(object),
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| item.as_object().is_some_and(object_has_state_affecting_key)),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ErrorDisposition;

    #[test]
    fn ws_malformed_json_is_quarantined() {
        let error = parse_ws_message(Bytes::from_static(br#"{"channel":"pong""#))
            .expect_err("truncated json");
        assert!(matches!(error, SourceError::MalformedPayload(_)));
        assert_eq!(error.disposition(), ErrorDisposition::Quarantine);
    }

    #[test]
    fn ws_pong_is_heartbeat() {
        let observation =
            parse_ws_message(Bytes::from_static(br#"{"channel":"pong"}"#)).expect("pong");
        assert!(matches!(observation, WsObservation::Heartbeat(_)));
        assert_ne!(
            observation.observation_class(),
            ObservationClass::CommittedBlock
        );
    }
}
