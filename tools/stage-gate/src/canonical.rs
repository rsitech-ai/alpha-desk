use std::{
    cmp::Ordering,
    collections::BTreeSet,
    fmt::{self, Write as _},
};

use serde::{
    Deserialize, Deserializer,
    de::{MapAccess, SeqAccess, Visitor},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalErrorCode {
    InvalidJson,
    DuplicateProperty,
    Serialization,
}

#[derive(Debug, thiserror::Error)]
pub enum CanonicalError {
    #[error("input is not valid I-JSON: {0}")]
    InvalidJson(String),
    #[error("JSON object repeats property {0}")]
    DuplicateProperty(String),
    #[error("canonical serialization failed")]
    Serialization,
}

impl CanonicalError {
    #[must_use]
    pub const fn code(&self) -> CanonicalErrorCode {
        match self {
            Self::InvalidJson(_) => CanonicalErrorCode::InvalidJson,
            Self::DuplicateProperty(_) => CanonicalErrorCode::DuplicateProperty,
            Self::Serialization => CanonicalErrorCode::Serialization,
        }
    }
}

#[derive(Debug)]
enum CanonicalValue {
    Null,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Self>),
    Object(Vec<(String, Self)>),
}

impl<'de> Deserialize<'de> for CanonicalValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CanonicalValueVisitor)
    }
}

struct CanonicalValueVisitor;

impl<'de> Visitor<'de> for CanonicalValueVisitor {
    type Value = CanonicalValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("an I-JSON value without duplicate object names")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Number(value as f64))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Number(value as f64))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if value.is_finite() {
            Ok(CanonicalValue::Number(value))
        } else {
            Err(E::custom("non-finite JSON number"))
        }
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        Ok(CanonicalValue::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(CanonicalValue::String(value))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Null)
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(CanonicalValue::Null)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element()? {
            values.push(value);
        }
        Ok(CanonicalValue::Array(values))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut names = BTreeSet::new();
        let mut values = Vec::new();
        while let Some((name, value)) = map.next_entry::<String, CanonicalValue>()? {
            if !names.insert(name.clone()) {
                return Err(serde::de::Error::custom(format!(
                    "duplicate JSON property name:{name}"
                )));
            }
            values.push((name, value));
        }
        Ok(CanonicalValue::Object(values))
    }
}

pub fn canonicalize_json_str(source: &str) -> Result<Vec<u8>, CanonicalError> {
    let mut deserializer = serde_json::Deserializer::from_str(source);
    let value = CanonicalValue::deserialize(&mut deserializer).map_err(map_json_error)?;
    deserializer.end().map_err(map_json_error)?;
    let mut output = String::new();
    write_value(&value, &mut output)?;
    Ok(output.into_bytes())
}

pub fn canonicalize<T>(value: &T) -> Result<Vec<u8>, CanonicalError>
where
    T: serde::Serialize,
{
    let serialized = serde_json::to_string(value).map_err(|_| CanonicalError::Serialization)?;
    canonicalize_json_str(&serialized)
}

fn map_json_error(error: serde_json::Error) -> CanonicalError {
    let message = error.to_string();
    if let Some(rest) = message.split("duplicate JSON property name:").nth(1) {
        let name = rest.split(" at line").next().unwrap_or(rest).to_owned();
        CanonicalError::DuplicateProperty(name)
    } else {
        CanonicalError::InvalidJson(message)
    }
}

fn write_value(value: &CanonicalValue, output: &mut String) -> Result<(), CanonicalError> {
    match value {
        CanonicalValue::Null => output.push_str("null"),
        CanonicalValue::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        CanonicalValue::Number(value) => {
            let mut buffer = ryu_js::Buffer::new();
            output
                .write_str(buffer.format(*value))
                .map_err(|_| CanonicalError::Serialization)?;
        }
        CanonicalValue::String(value) => write_string(value, output)?,
        CanonicalValue::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_value(value, output)?;
            }
            output.push(']');
        }
        CanonicalValue::Object(properties) => {
            let mut properties = properties.iter().collect::<Vec<_>>();
            properties.sort_by(|left, right| compare_utf16(&left.0, &right.0));
            output.push('{');
            for (index, (name, value)) in properties.into_iter().enumerate() {
                if index != 0 {
                    output.push(',');
                }
                write_string(name, output)?;
                output.push(':');
                write_value(value, output)?;
            }
            output.push('}');
        }
    }
    Ok(())
}

fn write_string(value: &str, output: &mut String) -> Result<(), CanonicalError> {
    let encoded = serde_json::to_string(value).map_err(|_| CanonicalError::Serialization)?;
    output.push_str(&encoded);
    Ok(())
}

fn compare_utf16(left: &str, right: &str) -> Ordering {
    left.encode_utf16().cmp(right.encode_utf16())
}
