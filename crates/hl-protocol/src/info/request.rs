use std::collections::BTreeMap;

use bytes::Bytes;
use serde_json::{Map, Value};

use super::{CapabilityId, InfoError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedInfoRequest {
    capability_id: CapabilityId,
    identifier: String,
    body: Bytes,
    content_hash: blake3::Hash,
}

impl EncodedInfoRequest {
    #[must_use]
    pub fn capability_id(&self) -> &CapabilityId {
        &self.capability_id
    }

    #[must_use]
    pub fn identifier(&self) -> &str {
        &self.identifier
    }

    #[must_use]
    pub fn body(&self) -> &Bytes {
        &self.body
    }

    #[must_use]
    pub const fn content_hash(&self) -> blake3::Hash {
        self.content_hash
    }
}

pub fn encode_info_request(
    capability_id: &str,
    identifier: &str,
    params: &BTreeMap<String, Value>,
) -> Result<EncodedInfoRequest, InfoError> {
    if params.contains_key("type") {
        return Err(InfoError::TypeFieldConflict);
    }
    let capability_id = CapabilityId::new(capability_id)?;
    let mut object = Map::new();
    object.insert("type".to_owned(), Value::String(identifier.to_owned()));
    for (key, value) in params {
        reject_json_floats(value, &super::child_path("", key))?;
        object.insert(key.clone(), value.clone());
    }
    let body = Bytes::from(
        serde_json::to_vec(&canonicalize_value(&Value::Object(object)))
            .map_err(|_| InfoError::MalformedJson)?,
    );
    let content_hash = blake3::hash(&body);
    Ok(EncodedInfoRequest {
        capability_id,
        identifier: identifier.to_owned(),
        body,
        content_hash,
    })
}

pub(crate) fn canonicalize_value(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let mut ordered = Map::new();
            for key in keys {
                ordered.insert(key.clone(), canonicalize_value(&map[key]));
            }
            Value::Object(ordered)
        }
        Value::Array(items) => Value::Array(items.iter().map(canonicalize_value).collect()),
        other => other.clone(),
    }
}

pub(crate) fn reject_json_floats(value: &Value, path: &str) -> Result<(), InfoError> {
    match value {
        Value::Number(number) if number.is_i64() || number.is_u64() => Ok(()),
        Value::Number(_) => Err(InfoError::ForbiddenJsonNumber {
            path: path.to_owned(),
        }),
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                reject_json_floats(item, &format!("{path}/{index}"))?;
            }
            Ok(())
        }
        Value::Object(map) => {
            for (key, child) in map {
                reject_json_floats(child, &super::child_path(path, key))?;
            }
            Ok(())
        }
        Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
    }
}
