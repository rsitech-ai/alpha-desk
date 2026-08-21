use std::collections::BTreeSet;
use std::str::FromStr;

use bytes::Bytes;
use domain_types::{Decimal, KnownTime, ValueError};
use serde_json::Value;

use crate::ParseWarning;

use super::{
    ArchiveRef, CapabilityId, InfoError, JsonPath, SchemaFingerprint, request::reject_json_floats,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InfoEnumField {
    path: &'static str,
    allowed: &'static [&'static str],
}

impl InfoEnumField {
    #[must_use]
    pub const fn new(path: &'static str, allowed: &'static [&'static str]) -> Self {
        Self { path, allowed }
    }

    #[must_use]
    pub const fn path(self) -> &'static str {
        self.path
    }

    #[must_use]
    pub const fn allowed(self) -> &'static [&'static str] {
        self.allowed
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InfoParseContext {
    request_hash: blake3::Hash,
    received_at: KnownTime,
    raw_archive_ref: ArchiveRef,
    known_fields: Option<&'static [&'static str]>,
    enum_fields: &'static [InfoEnumField],
}

impl InfoParseContext {
    #[must_use]
    pub fn new(
        request_hash: blake3::Hash,
        received_at: KnownTime,
        raw_archive_ref: ArchiveRef,
    ) -> Self {
        Self {
            request_hash,
            received_at,
            raw_archive_ref,
            known_fields: None,
            enum_fields: &[],
        }
    }

    #[must_use]
    pub fn with_known_fields(mut self, known_fields: &'static [&'static str]) -> Self {
        self.known_fields = Some(known_fields);
        self
    }

    #[must_use]
    pub fn with_enum_fields(mut self, enum_fields: &'static [InfoEnumField]) -> Self {
        self.enum_fields = enum_fields;
        self
    }

    #[must_use]
    pub const fn request_hash(&self) -> blake3::Hash {
        self.request_hash
    }

    #[must_use]
    pub const fn received_at(&self) -> KnownTime {
        self.received_at
    }

    #[must_use]
    pub fn raw_archive_ref(&self) -> &ArchiveRef {
        &self.raw_archive_ref
    }
}

#[derive(Debug, Clone)]
pub struct ParsedInfoResponse<T> {
    capability_id: CapabilityId,
    request_hash: blake3::Hash,
    response_hash: blake3::Hash,
    schema_fingerprint: SchemaFingerprint,
    received_at: KnownTime,
    raw_archive_ref: ArchiveRef,
    raw: Bytes,
    value: T,
    unknown_fields: Vec<JsonPath>,
    warnings: Vec<ParseWarning>,
}

impl<T> ParsedInfoResponse<T> {
    #[must_use]
    pub fn capability_id(&self) -> &CapabilityId {
        &self.capability_id
    }

    #[must_use]
    pub const fn request_hash(&self) -> blake3::Hash {
        self.request_hash
    }

    #[must_use]
    pub const fn response_hash(&self) -> blake3::Hash {
        self.response_hash
    }

    #[must_use]
    pub const fn schema_fingerprint(&self) -> SchemaFingerprint {
        self.schema_fingerprint
    }

    #[must_use]
    pub const fn received_at(&self) -> KnownTime {
        self.received_at
    }

    #[must_use]
    pub fn raw_archive_ref(&self) -> &ArchiveRef {
        &self.raw_archive_ref
    }

    #[must_use]
    pub fn raw(&self) -> &Bytes {
        &self.raw
    }

    #[must_use]
    pub const fn value(&self) -> &T {
        &self.value
    }

    #[must_use]
    pub fn unknown_fields(&self) -> &[JsonPath] {
        &self.unknown_fields
    }

    #[must_use]
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }
}

pub fn parse_info_response(
    capability_id: &str,
    raw: &[u8],
    context: &InfoParseContext,
    state_affecting: bool,
) -> Result<ParsedInfoResponse<Value>, InfoError> {
    if raw.is_empty() {
        return Err(InfoError::EmptyPayload);
    }
    let value: Value = serde_json::from_slice(raw).map_err(|_| InfoError::MalformedJson)?;
    reject_json_floats(&value, "")?;

    let mut paths = BTreeSet::new();
    let mut warnings = Vec::new();
    walk_value(
        &value,
        "",
        &mut paths,
        context,
        state_affecting,
        &mut warnings,
    )?;

    let unknown_fields = match context.known_fields {
        Some(known) => {
            let known: BTreeSet<&str> = known.iter().copied().collect();
            paths
                .iter()
                .filter(|path| !path_is_known(path, &known))
                .map(|path| JsonPath::new(path.clone()))
                .collect::<Result<Vec<_>, _>>()?
        }
        None => Vec::new(),
    };
    if context.known_fields.is_some() {
        for field in &unknown_fields {
            warnings.push(
                ParseWarning::new("info.unknown_field", field.as_str())
                    .map_err(|_| InfoError::InvalidJsonPath)?,
            );
        }
    }

    let fingerprint_input = paths
        .iter()
        .map(|path| shape_path(path))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join("\n");
    Ok(ParsedInfoResponse {
        capability_id: CapabilityId::new(capability_id)?,
        request_hash: context.request_hash,
        response_hash: blake3::hash(raw),
        schema_fingerprint: SchemaFingerprint::from_hash(blake3::hash(
            fingerprint_input.as_bytes(),
        )),
        received_at: context.received_at,
        raw_archive_ref: context.raw_archive_ref.clone(),
        raw: Bytes::copy_from_slice(raw),
        value,
        unknown_fields,
        warnings,
    })
}

pub fn require_known_variant(
    path: &str,
    value: &str,
    allowed: &[&str],
    state_affecting: bool,
) -> Result<(), InfoError> {
    if allowed.contains(&value) {
        return Ok(());
    }
    if state_affecting {
        return Err(InfoError::UnknownStateAffectingVariant {
            path: path.to_owned(),
            value: value.to_owned(),
        });
    }
    Ok(())
}

fn walk_value(
    value: &Value,
    path: &str,
    paths: &mut BTreeSet<String>,
    context: &InfoParseContext,
    state_affecting: bool,
    warnings: &mut Vec<ParseWarning>,
) -> Result<(), InfoError> {
    if !path.is_empty() {
        paths.insert(path.to_owned());
    }
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                walk_value(
                    child,
                    &super::child_path(path, key),
                    paths,
                    context,
                    state_affecting,
                    warnings,
                )?;
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                walk_value(
                    child,
                    &format!("{path}/{index}"),
                    paths,
                    context,
                    state_affecting,
                    warnings,
                )?;
            }
        }
        Value::String(text) => {
            if is_decimal_string(text) {
                parse_info_decimal(path, text)?;
            }
            if let Some(field) = context
                .enum_fields
                .iter()
                .find(|field| path_matches_declared(path, field.path))
            {
                require_known_variant(path, text, field.allowed, state_affecting)?;
                if !state_affecting && !field.allowed.contains(&text.as_str()) {
                    warnings.push(
                        ParseWarning::new("info.unknown_enum_variant", path)
                            .map_err(|_| InfoError::InvalidJsonPath)?,
                    );
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
    Ok(())
}

fn parse_info_decimal(path: &str, text: &str) -> Result<Decimal, InfoError> {
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

fn is_decimal_string(text: &str) -> bool {
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    if digits.is_empty() || (negative && digits == "0") {
        return false;
    }
    let mut parts = digits.split('.');
    let whole = parts.next().unwrap_or("");
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
    {
        return false;
    }
    match fraction {
        None => true,
        Some(frac) if !frac.is_empty() && frac.bytes().all(|byte| byte.is_ascii_digit()) => true,
        Some(_) => false,
    }
}

fn is_array_index(segment: &str) -> bool {
    !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_schema_field(segment: &str) -> bool {
    let mut chars = segment.chars();
    let Some('a'..='z') = chars.next() else {
        return false;
    };
    segment
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn shape_path(path: &str) -> String {
    let mut out = String::new();
    for segment in path.split('/').filter(|segment| !segment.is_empty()) {
        out.push('/');
        if is_array_index(segment) {
            out.push_str("[]");
        } else if is_schema_field(segment) {
            out.push_str(segment);
        } else {
            // ponytail: keys that are not camelCase fields (BTC, @1) collapse to *
            // so listing a market does not fork the fingerprint. A map whose keys
            // look like schema fields still forks per key; T06 can mark those maps.
            out.push('*');
        }
    }
    out
}

fn known_field_key(shape: &str) -> String {
    let mut out = String::new();
    for segment in shape.split('/').filter(|segment| !segment.is_empty()) {
        if segment == "[]" {
            continue;
        }
        out.push('/');
        out.push_str(segment);
    }
    out
}

fn path_is_known(path: &str, known: &BTreeSet<&str>) -> bool {
    if known.contains(path) {
        return true;
    }
    let shape = shape_path(path);
    if known.contains(shape.as_str()) {
        return true;
    }
    let key = known_field_key(&shape);
    key.is_empty() || known.contains(key.as_str())
}

fn path_matches_declared(concrete: &str, declared: &str) -> bool {
    if concrete == declared {
        return true;
    }
    let shape = shape_path(concrete);
    if shape == declared {
        return true;
    }
    known_field_key(&shape) == declared
}
