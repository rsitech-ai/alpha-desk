use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};

use lz4_flex::frame::{FrameDecoder, FrameEncoder};
use serde_json::{Map as JsonMap, Value as JsonValue};

use super::{EvmError, MAX_ARCHIVE_BYTES};

const MAX_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum WireValue {
    Nil,
    Bool(bool),
    Int(i64),
    Uint(u64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<WireValue>),
    Map(Vec<(WireValue, WireValue)>),
}

impl WireValue {
    pub(crate) fn as_map(&self) -> Result<&[(WireValue, WireValue)], EvmError> {
        match self {
            Self::Map(entries) => Ok(entries),
            _ => Err(EvmError::MalformedPayload(
                "HyperEVM msgpack value is not a map".to_owned(),
            )),
        }
    }
}

pub(crate) fn map_get<'a>(
    entries: &'a [(WireValue, WireValue)],
    names: &[&str],
) -> Option<&'a WireValue> {
    entries.iter().find_map(|(key, value)| {
        let key = match key {
            WireValue::String(text) => text.as_str(),
            _ => return None,
        };
        names.contains(&key).then_some(value)
    })
}

pub(crate) fn tagged_enum(value: &WireValue) -> Result<(&str, &WireValue), EvmError> {
    let entries = value.as_map()?;
    if entries.len() != 1 {
        return Err(EvmError::SchemaDrift(
            "expected a single-key tagged HyperEVM object".to_owned(),
        ));
    }
    match &entries[0] {
        (WireValue::String(tag), inner) => Ok((tag, inner)),
        _ => Err(EvmError::SchemaDrift(
            "tagged HyperEVM object key is not a string".to_owned(),
        )),
    }
}

pub(crate) fn as_bytes(value: &WireValue) -> Result<Vec<u8>, EvmError> {
    match value {
        WireValue::Bytes(bytes) => Ok(bytes.clone()),
        WireValue::String(text) => decode_hex_bytes(text),
        WireValue::Array(items) => items
            .iter()
            .map(|item| match item {
                WireValue::Uint(byte) if *byte <= 255 => Ok(*byte as u8),
                WireValue::Int(byte) if (0..=255).contains(byte) => Ok(*byte as u8),
                _ => Err(EvmError::MalformedPayload(
                    "byte array entry is not a byte".to_owned(),
                )),
            })
            .collect(),
        WireValue::Map(entries) => {
            if map_get(entries, &["type"]).and_then(as_str) == Some("Buffer") {
                match map_get(entries, &["data"]) {
                    Some(data) => as_bytes(data),
                    None => Ok(Vec::new()),
                }
            } else {
                Err(EvmError::MalformedPayload(
                    "cannot coerce map to bytes".to_owned(),
                ))
            }
        }
        _ => Err(EvmError::MalformedPayload(
            "HyperEVM value is not bytes".to_owned(),
        )),
    }
}

pub(crate) fn as_str(value: &WireValue) -> Option<&str> {
    match value {
        WireValue::String(text) => Some(text),
        _ => None,
    }
}

pub(crate) fn leftover_json(
    entries: &[(WireValue, WireValue)],
    taken: &[&str],
) -> BTreeMap<String, JsonValue> {
    let mut extra = BTreeMap::new();
    for (key, value) in entries {
        let Some(name) = as_str(key) else {
            continue;
        };
        if taken.contains(&name) {
            continue;
        }
        extra.insert(name.to_owned(), to_json(value));
    }
    extra
}

pub(crate) fn json_to_wire(value: &JsonValue) -> WireValue {
    match value {
        JsonValue::Null => WireValue::Nil,
        JsonValue::Bool(flag) => WireValue::Bool(*flag),
        JsonValue::Number(number) => {
            if let Some(unsigned) = number.as_u64() {
                WireValue::Uint(unsigned)
            } else if let Some(signed) = number.as_i64() {
                WireValue::Int(signed)
            } else {
                WireValue::String(number.to_string())
            }
        }
        JsonValue::String(text) => match decode_hex_bytes(text) {
            Ok(bytes) if text.starts_with("0x") => WireValue::Bytes(bytes),
            _ => WireValue::String(text.clone()),
        },
        JsonValue::Array(items) => WireValue::Array(items.iter().map(json_to_wire).collect()),
        JsonValue::Object(object) => WireValue::Map(
            object
                .iter()
                .map(|(key, value)| (WireValue::String(key.clone()), json_to_wire(value)))
                .collect(),
        ),
    }
}

pub(crate) fn extra_to_wire(extra: &BTreeMap<String, JsonValue>) -> Vec<(WireValue, WireValue)> {
    extra
        .iter()
        .map(|(key, value)| (WireValue::String(key.clone()), json_to_wire(value)))
        .collect()
}

pub(crate) fn string_map(pairs: Vec<(&str, WireValue)>) -> WireValue {
    WireValue::Map(
        pairs
            .into_iter()
            .map(|(key, value)| (WireValue::String(key.to_owned()), value))
            .collect(),
    )
}

pub(crate) fn bin_bytes(bytes: &[u8]) -> WireValue {
    WireValue::Bytes(bytes.to_vec())
}

pub(crate) fn to_json(value: &WireValue) -> JsonValue {
    match value {
        WireValue::Nil => JsonValue::Null,
        WireValue::Bool(flag) => JsonValue::Bool(*flag),
        WireValue::Int(number) => JsonValue::Number((*number).into()),
        WireValue::Uint(number) => JsonValue::Number((*number).into()),
        WireValue::String(text) => JsonValue::String(text.clone()),
        WireValue::Bytes(bytes) => JsonValue::String(format!("0x{}", hex::encode(bytes))),
        WireValue::Array(items) => JsonValue::Array(items.iter().map(to_json).collect()),
        WireValue::Map(entries) => {
            let mut object = JsonMap::new();
            for (key, value) in entries {
                let name = match key {
                    WireValue::String(text) => text.clone(),
                    WireValue::Bytes(bytes) => format!("0x{}", hex::encode(bytes)),
                    other => format!("{other:?}"),
                };
                object.insert(name, to_json(value));
            }
            JsonValue::Object(object)
        }
    }
}

pub(crate) fn decompress_lz4_frame(bytes: &[u8]) -> Result<Vec<u8>, EvmError> {
    let mut decoder = FrameDecoder::new(Cursor::new(bytes)).take(MAX_ARCHIVE_BYTES as u64 + 1);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|error| EvmError::MalformedPayload(format!("lz4 frame: {error}")))?;
    if out.len() > MAX_ARCHIVE_BYTES {
        return Err(EvmError::MalformedPayload(
            "lz4 frame exceeds the HyperEVM archive size cap".to_owned(),
        ));
    }
    Ok(out)
}

pub(crate) fn compress_lz4_frame(bytes: &[u8]) -> Result<Vec<u8>, EvmError> {
    let mut encoder = FrameEncoder::new(Vec::new());
    encoder
        .write_all(bytes)
        .map_err(|error| EvmError::MalformedPayload(format!("lz4 frame: {error}")))?;
    encoder
        .finish()
        .map_err(|error| EvmError::MalformedPayload(format!("lz4 frame: {error}")))
}

pub(crate) fn parse_msgpack(bytes: &[u8]) -> Result<WireValue, EvmError> {
    let mut cursor = bytes;
    let value = read_value(&mut cursor, 0)?;
    if !cursor.is_empty() {
        return Err(EvmError::MalformedPayload(
            "msgpack trailing bytes".to_owned(),
        ));
    }
    Ok(value)
}

pub(crate) fn encode_msgpack(value: &WireValue) -> Result<Vec<u8>, EvmError> {
    let mut out = Vec::new();
    write_value(&mut out, value)?;
    Ok(out)
}

fn read_value(buf: &mut &[u8], depth: usize) -> Result<WireValue, EvmError> {
    if depth > MAX_DEPTH {
        return Err(EvmError::MalformedPayload(
            "msgpack nesting too deep".to_owned(),
        ));
    }
    let marker = take_u8(buf)?;
    match marker {
        0xc0 => Ok(WireValue::Nil),
        0xc2 => Ok(WireValue::Bool(false)),
        0xc3 => Ok(WireValue::Bool(true)),
        0xcc => Ok(WireValue::Uint(u64::from(take_u8(buf)?))),
        0xcd => Ok(WireValue::Uint(u64::from(take_u16(buf)?))),
        0xce => Ok(WireValue::Uint(u64::from(take_u32(buf)?))),
        0xcf => Ok(WireValue::Uint(take_u64(buf)?)),
        0xd0 => Ok(WireValue::Int(i64::from(take_u8(buf)? as i8))),
        0xd1 => Ok(WireValue::Int(i64::from(take_i16(buf)?))),
        0xd2 => Ok(WireValue::Int(i64::from(take_i32(buf)?))),
        0xd3 => Ok(WireValue::Int(take_i64(buf)?)),
        0xc4 => {
            let len = usize::from(take_u8(buf)?);
            read_bin(buf, len)
        }
        0xc5 => {
            let len = usize::from(take_u16(buf)?);
            read_bin(buf, len)
        }
        0xc6 => {
            let len = usize::try_from(take_u32(buf)?)
                .map_err(|_| EvmError::MalformedPayload("bin32 too large".to_owned()))?;
            read_bin(buf, len)
        }
        0xd9 => {
            let len = usize::from(take_u8(buf)?);
            read_str(buf, len)
        }
        0xda => {
            let len = usize::from(take_u16(buf)?);
            read_str(buf, len)
        }
        0xdb => {
            let len = usize::try_from(take_u32(buf)?)
                .map_err(|_| EvmError::MalformedPayload("str32 too large".to_owned()))?;
            read_str(buf, len)
        }
        0xdc => {
            let len = usize::from(take_u16(buf)?);
            read_array(buf, len, depth)
        }
        0xdd => {
            let len = usize::try_from(take_u32(buf)?)
                .map_err(|_| EvmError::MalformedPayload("array32 too large".to_owned()))?;
            read_array(buf, len, depth)
        }
        0xde => {
            let len = usize::from(take_u16(buf)?);
            read_map(buf, len, depth)
        }
        0xdf => {
            let len = usize::try_from(take_u32(buf)?)
                .map_err(|_| EvmError::MalformedPayload("map32 too large".to_owned()))?;
            read_map(buf, len, depth)
        }
        0xca => {
            let _ = take(buf, 4)?;
            Err(EvmError::MalformedPayload(
                "msgpack f32 is not used in HyperEVM archives".to_owned(),
            ))
        }
        0xcb => {
            let _ = take(buf, 8)?;
            Err(EvmError::MalformedPayload(
                "msgpack f64 is not used in HyperEVM archives".to_owned(),
            ))
        }
        marker if marker < 0x80 => Ok(WireValue::Uint(u64::from(marker))),
        marker if marker < 0x90 => read_map(buf, usize::from(marker & 0x0f), depth),
        marker if marker < 0xa0 => read_array(buf, usize::from(marker & 0x0f), depth),
        marker if marker < 0xc0 => read_str(buf, usize::from(marker & 0x1f)),
        marker if marker >= 0xe0 => Ok(WireValue::Int(i64::from(marker as i8))),
        _ => Err(EvmError::MalformedPayload(format!(
            "unsupported msgpack marker 0x{marker:02x}"
        ))),
    }
}

fn read_bin(buf: &mut &[u8], len: usize) -> Result<WireValue, EvmError> {
    Ok(WireValue::Bytes(take(buf, len)?.to_vec()))
}

fn read_str(buf: &mut &[u8], len: usize) -> Result<WireValue, EvmError> {
    let bytes = take(buf, len)?;
    let text = std::str::from_utf8(bytes)
        .map_err(|_| EvmError::MalformedPayload("msgpack string is not utf8".to_owned()))?;
    Ok(WireValue::String(text.to_owned()))
}

fn read_array(buf: &mut &[u8], len: usize, depth: usize) -> Result<WireValue, EvmError> {
    let mut items = Vec::with_capacity(len);
    for _ in 0..len {
        items.push(read_value(buf, depth + 1)?);
    }
    Ok(WireValue::Array(items))
}

fn read_map(buf: &mut &[u8], len: usize, depth: usize) -> Result<WireValue, EvmError> {
    let mut entries = Vec::with_capacity(len);
    for _ in 0..len {
        let key = read_value(buf, depth + 1)?;
        let value = read_value(buf, depth + 1)?;
        entries.push((key, value));
    }
    Ok(WireValue::Map(entries))
}

fn write_value(out: &mut Vec<u8>, value: &WireValue) -> Result<(), EvmError> {
    match value {
        WireValue::Nil => out.push(0xc0),
        WireValue::Bool(false) => out.push(0xc2),
        WireValue::Bool(true) => out.push(0xc3),
        WireValue::Int(number) => write_int(out, *number),
        WireValue::Uint(number) => write_uint(out, *number),
        WireValue::String(text) => write_str(out, text.as_bytes())?,
        WireValue::Bytes(bytes) => write_bin(out, bytes)?,
        WireValue::Array(items) => {
            write_len_mark(out, items.len(), 0x90, 0xdc, 0xdd)?;
            for item in items {
                write_value(out, item)?;
            }
        }
        WireValue::Map(entries) => {
            write_len_mark(out, entries.len(), 0x80, 0xde, 0xdf)?;
            for (key, value) in entries {
                write_value(out, key)?;
                write_value(out, value)?;
            }
        }
    }
    Ok(())
}

fn write_uint(out: &mut Vec<u8>, number: u64) {
    if number < 128 {
        out.push(number as u8);
    } else if number <= u64::from(u8::MAX) {
        out.push(0xcc);
        out.push(number as u8);
    } else if number <= u64::from(u16::MAX) {
        out.push(0xcd);
        out.extend_from_slice(&(number as u16).to_be_bytes());
    } else if number <= u64::from(u32::MAX) {
        out.push(0xce);
        out.extend_from_slice(&(number as u32).to_be_bytes());
    } else {
        out.push(0xcf);
        out.extend_from_slice(&number.to_be_bytes());
    }
}

fn write_int(out: &mut Vec<u8>, number: i64) {
    if (0..128).contains(&number) {
        write_uint(out, number as u64);
    } else if (-32..0).contains(&number) {
        out.push(number as u8);
    } else if (i64::from(i8::MIN)..=i64::from(i8::MAX)).contains(&number) {
        out.push(0xd0);
        out.push(number as u8);
    } else if (i64::from(i16::MIN)..=i64::from(i16::MAX)).contains(&number) {
        out.push(0xd1);
        out.extend_from_slice(&(number as i16).to_be_bytes());
    } else if (i64::from(i32::MIN)..=i64::from(i32::MAX)).contains(&number) {
        out.push(0xd2);
        out.extend_from_slice(&(number as i32).to_be_bytes());
    } else {
        out.push(0xd3);
        out.extend_from_slice(&number.to_be_bytes());
    }
}

fn write_str(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), EvmError> {
    if bytes.len() < 32 {
        out.push(0xa0 | bytes.len() as u8);
    } else if bytes.len() <= 255 {
        out.push(0xd9);
        out.push(bytes.len() as u8);
    } else if bytes.len() <= usize::from(u16::MAX) {
        out.push(0xda);
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    } else {
        let len = u32::try_from(bytes.len())
            .map_err(|_| EvmError::MalformedPayload("string too large".to_owned()))?;
        out.push(0xdb);
        out.extend_from_slice(&len.to_be_bytes());
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_bin(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), EvmError> {
    if bytes.len() <= 255 {
        out.push(0xc4);
        out.push(bytes.len() as u8);
    } else if bytes.len() <= usize::from(u16::MAX) {
        out.push(0xc5);
        out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    } else {
        let len = u32::try_from(bytes.len())
            .map_err(|_| EvmError::MalformedPayload("bin too large".to_owned()))?;
        out.push(0xc6);
        out.extend_from_slice(&len.to_be_bytes());
    }
    out.extend_from_slice(bytes);
    Ok(())
}

fn write_len_mark(
    out: &mut Vec<u8>,
    len: usize,
    fix_base: u8,
    mark16: u8,
    mark32: u8,
) -> Result<(), EvmError> {
    if fix_base != 0 && len < 16 {
        out.push(fix_base | len as u8);
        Ok(())
    } else if len <= usize::from(u16::MAX) {
        out.push(mark16);
        out.extend_from_slice(&(len as u16).to_be_bytes());
        Ok(())
    } else {
        let encoded = u32::try_from(len)
            .map_err(|_| EvmError::MalformedPayload("msgpack collection too large".to_owned()))?;
        out.push(mark32);
        out.extend_from_slice(&encoded.to_be_bytes());
        Ok(())
    }
}

fn take<'a>(buf: &mut &'a [u8], n: usize) -> Result<&'a [u8], EvmError> {
    if buf.len() < n {
        return Err(EvmError::MalformedPayload("truncated msgpack".to_owned()));
    }
    let (head, rest) = buf.split_at(n);
    *buf = rest;
    Ok(head)
}

fn take_u8(buf: &mut &[u8]) -> Result<u8, EvmError> {
    Ok(take(buf, 1)?[0])
}

fn take_u16(buf: &mut &[u8]) -> Result<u16, EvmError> {
    let bytes: [u8; 2] = take(buf, 2)?.try_into().expect("u16");
    Ok(u16::from_be_bytes(bytes))
}

fn take_u32(buf: &mut &[u8]) -> Result<u32, EvmError> {
    let bytes: [u8; 4] = take(buf, 4)?.try_into().expect("u32");
    Ok(u32::from_be_bytes(bytes))
}

fn take_u64(buf: &mut &[u8]) -> Result<u64, EvmError> {
    let bytes: [u8; 8] = take(buf, 8)?.try_into().expect("u64");
    Ok(u64::from_be_bytes(bytes))
}

fn take_i16(buf: &mut &[u8]) -> Result<i16, EvmError> {
    Ok(take_u16(buf)? as i16)
}

fn take_i32(buf: &mut &[u8]) -> Result<i32, EvmError> {
    Ok(take_u32(buf)? as i32)
}

fn take_i64(buf: &mut &[u8]) -> Result<i64, EvmError> {
    Ok(take_u64(buf)? as i64)
}

fn decode_hex_bytes(text: &str) -> Result<Vec<u8>, EvmError> {
    let hex_value = text
        .strip_prefix("0x")
        .ok_or_else(|| EvmError::MalformedPayload("expected 0x-prefixed hex bytes".to_owned()))?;
    if hex_value.len() % 2 != 0
        || hex_value
            .bytes()
            .any(|byte| !byte.is_ascii_hexdigit() || byte.is_ascii_uppercase())
    {
        return Err(EvmError::MalformedPayload(
            "hex bytes must be lowercase even length".to_owned(),
        ));
    }
    hex::decode(hex_value).map_err(|_| EvmError::MalformedPayload("invalid hex bytes".to_owned()))
}
