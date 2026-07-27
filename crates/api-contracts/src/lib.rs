#![forbid(unsafe_code)]

use prost::Message;

#[allow(clippy::all, dead_code)]
mod generated {
    pub(crate) mod hl {
        pub(crate) mod common {
            pub(crate) mod v1 {
                include!(concat!(env!("OUT_DIR"), "/hl.common.v1.rs"));
            }
        }

        pub(crate) mod canonical {
            pub(crate) mod v1 {
                include!(concat!(env!("OUT_DIR"), "/hl.canonical.v1.rs"));
            }
        }

        pub(crate) mod health {
            pub(crate) mod v1 {
                include!(concat!(env!("OUT_DIR"), "/hl.health.v1.rs"));
            }
        }

        pub(crate) mod stream {
            pub(crate) mod v1 {
                include!(concat!(env!("OUT_DIR"), "/hl.stream.v1.rs"));
            }
        }
    }
}

pub const FILE_DESCRIPTOR_SET: &[u8] =
    include_bytes!(concat!(env!("OUT_DIR"), "/alpha-desk-v1.pb"));

const GENERATED_RUST_ARTIFACTS: &[(&str, &[u8])] = &[
    (
        "hl.canonical.v1.rs",
        include_bytes!(concat!(env!("OUT_DIR"), "/hl.canonical.v1.rs")),
    ),
    (
        "hl.common.v1.rs",
        include_bytes!(concat!(env!("OUT_DIR"), "/hl.common.v1.rs")),
    ),
    (
        "hl.health.v1.rs",
        include_bytes!(concat!(env!("OUT_DIR"), "/hl.health.v1.rs")),
    ),
    (
        "hl.stream.v1.rs",
        include_bytes!(concat!(env!("OUT_DIR"), "/hl.stream.v1.rs")),
    ),
];

const SCHEMA_MATERIAL_HEADER: &[u8] = b"alpha-desk-schema-material-v1\n";
const SCHEMA_MATERIAL_LINE_BYTES: usize = 60;
const MAX_SCHEMA_FILES: usize = 4_096;
const MAX_SCHEMA_MATERIAL_BYTES: usize = 64 * 1024 * 1024;

pub fn export_contract_artifacts(
    descriptor_path: impl AsRef<std::path::Path>,
    rust_output_directory: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind, Write as _};

    let descriptor_path = descriptor_path.as_ref();
    let rust_output_directory = rust_output_directory.as_ref();
    if descriptor_path.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            "descriptor output already exists",
        ));
    }
    let descriptor_parent = descriptor_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    if !descriptor_parent.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "descriptor output parent must be a directory",
        ));
    }
    let descriptor_name = descriptor_path.file_name().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "descriptor output has no file name",
        )
    })?;
    let canonical_descriptor = descriptor_parent.canonicalize()?.join(descriptor_name);
    let rust_parent = rust_output_directory
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."));
    if !rust_parent.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "generated Rust output parent must be a directory",
        ));
    }
    let rust_output_existed = rust_output_directory.exists();
    if rust_output_directory.exists() {
        if !rust_output_directory.is_dir() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "generated Rust output is not a directory",
            ));
        }
        if std::fs::read_dir(rust_output_directory)?.next().is_some() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "generated Rust output directory is not empty",
            ));
        }
    }
    let rust_name = rust_output_directory.file_name().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "generated Rust output has no directory name",
        )
    })?;
    let canonical_rust_output = if rust_output_existed {
        rust_output_directory.canonicalize()?
    } else {
        rust_parent.canonicalize()?.join(rust_name)
    };
    if canonical_descriptor.starts_with(&canonical_rust_output)
        || canonical_rust_output.starts_with(&canonical_descriptor)
    {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "descriptor and generated Rust outputs must not overlap",
        ));
    }

    let mut staged_descriptor = tempfile::Builder::new()
        .prefix(".schema-descriptor-")
        .tempfile_in(descriptor_parent)?;
    staged_descriptor.write_all(FILE_DESCRIPTOR_SET)?;
    staged_descriptor.as_file().sync_all()?;

    let staged_rust = tempfile::Builder::new()
        .prefix(".schema-rust-")
        .tempdir_in(rust_parent)?;
    for (name, bytes) in GENERATED_RUST_ARTIFACTS {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(staged_rust.path().join(name))?;
        output.write_all(bytes)?;
        output.sync_all()?;
    }
    sync_directory(staged_rust.path())?;

    let backup = if rust_output_existed {
        let holder = tempfile::Builder::new()
            .prefix(".schema-rust-backup-")
            .tempdir_in(rust_parent)?;
        let backup_path = holder.path().join("original");
        std::fs::rename(rust_output_directory, &backup_path)?;
        Some((holder, backup_path))
    } else {
        None
    };
    let staged_rust_path = staged_rust.keep();
    if let Err(error) = std::fs::rename(&staged_rust_path, rust_output_directory) {
        if let Some((_, backup_path)) = &backup {
            let _ = std::fs::rename(backup_path, rust_output_directory);
        }
        let _ = std::fs::remove_dir_all(&staged_rust_path);
        return Err(error);
    }

    if let Err(error) = staged_descriptor.persist_noclobber(descriptor_path) {
        let _ = std::fs::remove_dir_all(rust_output_directory);
        if let Some((_, backup_path)) = &backup {
            let _ = std::fs::rename(backup_path, rust_output_directory);
        }
        return Err(error.error);
    }
    sync_directory(rust_parent)?;
    if descriptor_parent != rust_parent {
        sync_directory(descriptor_parent)?;
    }
    Ok(())
}

pub fn write_schema_material(
    schema_root: impl AsRef<std::path::Path>,
    output_path: impl AsRef<std::path::Path>,
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind, Read as _, Write as _};

    let schema_root = schema_root.as_ref();
    let output_path = output_path.as_ref();
    let root_metadata = std::fs::symlink_metadata(schema_root)?;
    if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "schema root must be a real directory",
        ));
    }
    let canonical_root = schema_root.canonicalize()?;
    let output_parent = output_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| std::path::Path::new("."))
        .canonicalize()?;
    let output_name = output_path
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "material output has no file name"))?;
    let canonical_output = output_parent.join(output_name);
    if canonical_output.starts_with(&canonical_root) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "material output must not be inside schema root",
        ));
    }
    if output_path.exists() {
        return Err(Error::new(
            ErrorKind::AlreadyExists,
            "schema material output already exists",
        ));
    }

    let mut files = Vec::new();
    collect_schema_files(&canonical_root, &canonical_root, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    if files.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "schema root contains no regular files",
        ));
    }

    let mut material = Vec::new();
    for (relative, path) in files {
        let framing_size = 16usize
            .checked_add(relative.len())
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "schema material size overflow"))?;
        let remaining = MAX_SCHEMA_MATERIAL_BYTES
            .checked_sub(material.len().saturating_add(framing_size))
            .ok_or_else(|| {
                Error::new(
                    ErrorKind::InvalidData,
                    "schema material exceeds the byte limit",
                )
            })?;
        let mut source = std::fs::File::open(path)?;
        let metadata = source.metadata()?;
        if !metadata.is_file() || metadata.len() > u64::try_from(remaining).unwrap_or(u64::MAX) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "schema file exceeds the byte limit",
            ));
        }
        let expected_length = metadata.len();
        let mut bytes = Vec::with_capacity(
            usize::try_from(expected_length)
                .unwrap_or(remaining)
                .min(remaining),
        );
        std::io::Read::take(
            &mut source,
            u64::try_from(remaining)
                .unwrap_or(u64::MAX)
                .saturating_add(1),
        )
        .read_to_end(&mut bytes)?;
        if bytes.len() > remaining
            || u64::try_from(bytes.len()).unwrap_or(u64::MAX) != expected_length
        {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "schema file changed or exceeded the byte limit during generation",
            ));
        }
        append_material_record(&mut material, relative.as_bytes(), &bytes)?;
    }
    let mut document = Vec::with_capacity(
        SCHEMA_MATERIAL_HEADER
            .len()
            .saturating_add(material.len().saturating_mul(2))
            .saturating_add(material.len() / SCHEMA_MATERIAL_LINE_BYTES)
            .saturating_add(1),
    );
    document.extend_from_slice(SCHEMA_MATERIAL_HEADER);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in material.iter().copied().enumerate() {
        if index > 0 && index.is_multiple_of(SCHEMA_MATERIAL_LINE_BYTES) {
            document.push(b'\n');
        }
        document.push(HEX[usize::from(byte >> 4)]);
        document.push(HEX[usize::from(byte & 0x0f)]);
    }
    document.push(b'\n');

    let mut output = tempfile::Builder::new()
        .prefix(".schema-material-")
        .tempfile_in(&output_parent)?;
    output.write_all(&document)?;
    output.as_file().sync_all()?;
    output
        .persist_noclobber(output_path)
        .map_err(|error| error.error)?;
    sync_directory(&output_parent)?;
    Ok(())
}

fn sync_directory(directory: &std::path::Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::fs::File::open(directory)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = directory;
        Ok(())
    }
}

fn collect_schema_files(
    root: &std::path::Path,
    directory: &std::path::Path,
    files: &mut Vec<(String, std::path::PathBuf)>,
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};

    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "schema tree contains a symlink or special file",
            ));
        }
        if file_type.is_dir() {
            collect_schema_files(root, &entry.path(), files)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "schema tree contains a symlink or special file",
            ));
        }
        if files.len() >= MAX_SCHEMA_FILES {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "schema tree exceeds the file-count limit",
            ));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| Error::new(ErrorKind::InvalidData, "schema path escaped root"))?
            .to_str()
            .ok_or_else(|| Error::new(ErrorKind::InvalidData, "schema path is not valid UTF-8"))?
            .replace(std::path::MAIN_SEPARATOR, "/");
        if relative.starts_with('/')
            || relative.contains('\\')
            || relative.chars().any(char::is_control)
            || relative
                .split('/')
                .any(|segment| matches!(segment, "" | "." | ".."))
        {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "schema path is not portable",
            ));
        }
        files.push((relative, entry.path()));
    }
    Ok(())
}

fn append_material_record(
    material: &mut Vec<u8>,
    path: &[u8],
    content: &[u8],
) -> std::io::Result<()> {
    use std::io::{Error, ErrorKind};

    let required = 16usize
        .checked_add(path.len())
        .and_then(|size| size.checked_add(content.len()))
        .ok_or_else(|| Error::new(ErrorKind::InvalidData, "schema material size overflow"))?;
    if material.len().saturating_add(required) > MAX_SCHEMA_MATERIAL_BYTES {
        return Err(Error::new(
            ErrorKind::InvalidData,
            "schema material exceeds the byte limit",
        ));
    }
    material.extend_from_slice(
        &u64::try_from(path.len())
            .map_err(|_| Error::new(ErrorKind::InvalidData, "schema path is too large"))?
            .to_be_bytes(),
    );
    material.extend_from_slice(path);
    material.extend_from_slice(
        &u64::try_from(content.len())
            .map_err(|_| Error::new(ErrorKind::InvalidData, "schema file is too large"))?
            .to_be_bytes(),
    );
    material.extend_from_slice(content);
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireSourceEvidence {
    pub source_id: String,
    pub source_version: String,
    pub source_offset: String,
    pub content_hash: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireCanonicalEventEnvelope {
    pub schema_version: String,
    pub chain_id: String,
    pub block_height: u64,
    pub block_time_micros: i64,
    pub transaction_id: String,
    pub transaction_index: u32,
    pub event_index: u32,
    pub event_id: String,
    pub event_kind: String,
    pub market_ids: Vec<String>,
    pub account_ids: Vec<String>,
    pub source_evidence: Vec<WireSourceEvidence>,
    pub confirmation_class: i32,
    pub observed_at_micros: i64,
    pub ingested_at_micros: i64,
    pub canonicalized_at_micros: i64,
    pub payload_hash: Vec<u8>,
    pub parser_version: String,
    pub payload: Vec<u8>,
}

impl WireCanonicalEventEnvelope {
    #[must_use]
    pub fn encode_to_vec(&self) -> Vec<u8> {
        generated::hl::canonical::v1::CanonicalEventEnvelope::from(self).encode_to_vec()
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, prost::DecodeError> {
        generated::hl::canonical::v1::CanonicalEventEnvelope::decode(bytes).map(Into::into)
    }
}

impl From<&WireCanonicalEventEnvelope> for generated::hl::canonical::v1::CanonicalEventEnvelope {
    fn from(value: &WireCanonicalEventEnvelope) -> Self {
        Self {
            schema_version: value.schema_version.clone(),
            chain_id: value.chain_id.clone(),
            block_height: value.block_height,
            block_time_micros: value.block_time_micros,
            transaction_id: value.transaction_id.clone(),
            transaction_index: value.transaction_index,
            event_index: value.event_index,
            event_id: value.event_id.clone(),
            event_kind: value.event_kind.clone(),
            market_ids: value.market_ids.clone(),
            account_ids: value.account_ids.clone(),
            source_evidence: value.source_evidence.iter().map(Into::into).collect(),
            confirmation_class: value.confirmation_class,
            observed_at_micros: value.observed_at_micros,
            ingested_at_micros: value.ingested_at_micros,
            canonicalized_at_micros: value.canonicalized_at_micros,
            payload_hash: value.payload_hash.clone(),
            parser_version: value.parser_version.clone(),
            payload: value.payload.clone(),
        }
    }
}

impl From<&WireSourceEvidence> for generated::hl::canonical::v1::SourceEvidence {
    fn from(value: &WireSourceEvidence) -> Self {
        Self {
            source_id: value.source_id.clone(),
            source_version: value.source_version.clone(),
            source_offset: value.source_offset.clone(),
            content_hash: value.content_hash.clone(),
        }
    }
}

impl From<generated::hl::canonical::v1::CanonicalEventEnvelope> for WireCanonicalEventEnvelope {
    fn from(value: generated::hl::canonical::v1::CanonicalEventEnvelope) -> Self {
        Self {
            schema_version: value.schema_version,
            chain_id: value.chain_id,
            block_height: value.block_height,
            block_time_micros: value.block_time_micros,
            transaction_id: value.transaction_id,
            transaction_index: value.transaction_index,
            event_index: value.event_index,
            event_id: value.event_id,
            event_kind: value.event_kind,
            market_ids: value.market_ids,
            account_ids: value.account_ids,
            source_evidence: value.source_evidence.into_iter().map(Into::into).collect(),
            confirmation_class: value.confirmation_class,
            observed_at_micros: value.observed_at_micros,
            ingested_at_micros: value.ingested_at_micros,
            canonicalized_at_micros: value.canonicalized_at_micros,
            payload_hash: value.payload_hash,
            parser_version: value.parser_version,
            payload: value.payload,
        }
    }
}

impl From<generated::hl::canonical::v1::SourceEvidence> for WireSourceEvidence {
    fn from(value: generated::hl::canonical::v1::SourceEvidence) -> Self {
        Self {
            source_id: value.source_id,
            source_version: value.source_version,
            source_offset: value.source_offset,
            content_hash: value.content_hash,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PayloadCodecError {
    #[error("unknown event payload kind {0}")]
    UnknownKind(String),
    #[error("payload kind mismatch: expected {expected}, received {actual}")]
    KindMismatch { expected: String, actual: String },
    #[error("failed to decode {kind} payload: {source}")]
    Decode {
        kind: String,
        #[source]
        source: prost::DecodeError,
    },
    #[error("invalid {kind} payload: {reason}")]
    Invalid { kind: String, reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTradeMatched {
    pub price: String,
    pub quantity: String,
    pub deterministic_seed: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOrderAccepted {
    pub order_id: String,
    pub account_id: String,
    pub market_id: String,
    pub side: String,
    pub limit_price: String,
    pub quantity: String,
}

#[must_use]
pub fn encode_order_accepted(value: &WireOrderAccepted) -> Vec<u8> {
    wrap_payload(
        "OrderAccepted",
        generated::hl::canonical::v1::OrderAccepted {
            order_id: value.order_id.clone(),
            account_id: value.account_id.clone(),
            market_id: value.market_id.clone(),
            side: value.side.clone(),
            limit_price: value.limit_price.clone(),
            quantity: value.quantity.clone(),
        }
        .encode_to_vec(),
    )
}

pub fn encode_default_event_payload(kind: &str) -> Result<Vec<u8>, PayloadCodecError> {
    let message = match kind {
        "OrderAccepted" => default_message::<generated::hl::canonical::v1::OrderAccepted>(),
        "OrderRested" => default_message::<generated::hl::canonical::v1::OrderRested>(),
        "OrderModified" => default_message::<generated::hl::canonical::v1::OrderModified>(),
        "OrderPartiallyFilled" => {
            default_message::<generated::hl::canonical::v1::OrderPartiallyFilled>()
        }
        "OrderFilled" => default_message::<generated::hl::canonical::v1::OrderFilled>(),
        "OrderCancelled" => default_message::<generated::hl::canonical::v1::OrderCancelled>(),
        "OrderRejected" => default_message::<generated::hl::canonical::v1::OrderRejected>(),
        "TriggerOrderActivated" => {
            default_message::<generated::hl::canonical::v1::TriggerOrderActivated>()
        }
        "TwapStarted" => default_message::<generated::hl::canonical::v1::TwapStarted>(),
        "TwapSliceFilled" => default_message::<generated::hl::canonical::v1::TwapSliceFilled>(),
        "TwapCompleted" => default_message::<generated::hl::canonical::v1::TwapCompleted>(),
        "TradeMatched" => {
            return encode_trade_matched(&WireTradeMatched {
                price: "0".to_owned(),
                quantity: "0".to_owned(),
                deterministic_seed: 0,
            });
        }
        "DepositCredited" => default_message::<generated::hl::canonical::v1::DepositCredited>(),
        "WithdrawalDebited" => default_message::<generated::hl::canonical::v1::WithdrawalDebited>(),
        "SpotTransfer" => default_message::<generated::hl::canonical::v1::SpotTransfer>(),
        "PerpTransfer" => default_message::<generated::hl::canonical::v1::PerpTransfer>(),
        "SubaccountTransfer" => {
            default_message::<generated::hl::canonical::v1::SubaccountTransfer>()
        }
        "VaultDeposit" => default_message::<generated::hl::canonical::v1::VaultDeposit>(),
        "VaultWithdrawal" => default_message::<generated::hl::canonical::v1::VaultWithdrawal>(),
        "FeeCharged" => default_message::<generated::hl::canonical::v1::FeeCharged>(),
        "BuilderFeeCharged" => default_message::<generated::hl::canonical::v1::BuilderFeeCharged>(),
        "FundingPaid" => default_message::<generated::hl::canonical::v1::FundingPaid>(),
        "FundingReceived" => default_message::<generated::hl::canonical::v1::FundingReceived>(),
        "ReferralReward" => default_message::<generated::hl::canonical::v1::ReferralReward>(),
        "AccountModeChanged" => {
            default_message::<generated::hl::canonical::v1::AccountModeChanged>()
        }
        "MarginModeChanged" => default_message::<generated::hl::canonical::v1::MarginModeChanged>(),
        "LeverageChanged" => default_message::<generated::hl::canonical::v1::LeverageChanged>(),
        "LiquidationStarted" => {
            default_message::<generated::hl::canonical::v1::LiquidationStarted>()
        }
        "LiquidationFill" => default_message::<generated::hl::canonical::v1::LiquidationFill>(),
        "BackstopLiquidation" => {
            default_message::<generated::hl::canonical::v1::BackstopLiquidation>()
        }
        "PositionSettled" => default_message::<generated::hl::canonical::v1::PositionSettled>(),
        "MarketHalted" => default_message::<generated::hl::canonical::v1::MarketHalted>(),
        "MarketResumed" => default_message::<generated::hl::canonical::v1::MarketResumed>(),
        "OpenInterestCapChanged" => {
            default_message::<generated::hl::canonical::v1::OpenInterestCapChanged>()
        }
        "MarginTableChanged" => {
            default_message::<generated::hl::canonical::v1::MarginTableChanged>()
        }
        "MarketCreated" => default_message::<generated::hl::canonical::v1::MarketCreated>(),
        "MarketMetadataChanged" => {
            default_message::<generated::hl::canonical::v1::MarketMetadataChanged>()
        }
        "OracleUpdated" => default_message::<generated::hl::canonical::v1::OracleUpdated>(),
        "FundingRateUpdated" => {
            default_message::<generated::hl::canonical::v1::FundingRateUpdated>()
        }
        "AssetContextUpdated" => {
            default_message::<generated::hl::canonical::v1::AssetContextUpdated>()
        }
        "DexCreated" => default_message::<generated::hl::canonical::v1::DexCreated>(),
        "OutcomeCreated" => default_message::<generated::hl::canonical::v1::OutcomeCreated>(),
        "OutcomeResolved" => default_message::<generated::hl::canonical::v1::OutcomeResolved>(),
        other => return Err(PayloadCodecError::UnknownKind(other.to_owned())),
    };
    Ok(wrap_payload(kind, message))
}

pub fn validate_event_payload(kind: &str, bytes: &[u8]) -> Result<(), PayloadCodecError> {
    let message = unwrap_payload(kind, bytes)?;
    macro_rules! decode {
        ($type:ty) => {
            <$type>::decode(message.as_slice())
                .map(|_| ())
                .map_err(|source| PayloadCodecError::Decode {
                    kind: kind.to_owned(),
                    source,
                })
        };
    }
    match kind {
        "OrderAccepted" => decode!(generated::hl::canonical::v1::OrderAccepted),
        "OrderRested" => decode!(generated::hl::canonical::v1::OrderRested),
        "OrderModified" => decode!(generated::hl::canonical::v1::OrderModified),
        "OrderPartiallyFilled" => decode!(generated::hl::canonical::v1::OrderPartiallyFilled),
        "OrderFilled" => decode!(generated::hl::canonical::v1::OrderFilled),
        "OrderCancelled" => decode!(generated::hl::canonical::v1::OrderCancelled),
        "OrderRejected" => decode!(generated::hl::canonical::v1::OrderRejected),
        "TriggerOrderActivated" => decode!(generated::hl::canonical::v1::TriggerOrderActivated),
        "TwapStarted" => decode!(generated::hl::canonical::v1::TwapStarted),
        "TwapSliceFilled" => decode!(generated::hl::canonical::v1::TwapSliceFilled),
        "TwapCompleted" => decode!(generated::hl::canonical::v1::TwapCompleted),
        "TradeMatched" => decode_trade_matched(bytes).map(|_| ()),
        "DepositCredited" => decode!(generated::hl::canonical::v1::DepositCredited),
        "WithdrawalDebited" => decode!(generated::hl::canonical::v1::WithdrawalDebited),
        "SpotTransfer" => decode!(generated::hl::canonical::v1::SpotTransfer),
        "PerpTransfer" => decode!(generated::hl::canonical::v1::PerpTransfer),
        "SubaccountTransfer" => decode!(generated::hl::canonical::v1::SubaccountTransfer),
        "VaultDeposit" => decode!(generated::hl::canonical::v1::VaultDeposit),
        "VaultWithdrawal" => decode!(generated::hl::canonical::v1::VaultWithdrawal),
        "FeeCharged" => decode!(generated::hl::canonical::v1::FeeCharged),
        "BuilderFeeCharged" => decode!(generated::hl::canonical::v1::BuilderFeeCharged),
        "FundingPaid" => decode!(generated::hl::canonical::v1::FundingPaid),
        "FundingReceived" => decode!(generated::hl::canonical::v1::FundingReceived),
        "ReferralReward" => decode!(generated::hl::canonical::v1::ReferralReward),
        "AccountModeChanged" => decode!(generated::hl::canonical::v1::AccountModeChanged),
        "MarginModeChanged" => decode!(generated::hl::canonical::v1::MarginModeChanged),
        "LeverageChanged" => decode!(generated::hl::canonical::v1::LeverageChanged),
        "LiquidationStarted" => decode!(generated::hl::canonical::v1::LiquidationStarted),
        "LiquidationFill" => decode!(generated::hl::canonical::v1::LiquidationFill),
        "BackstopLiquidation" => decode!(generated::hl::canonical::v1::BackstopLiquidation),
        "PositionSettled" => decode!(generated::hl::canonical::v1::PositionSettled),
        "MarketHalted" => decode!(generated::hl::canonical::v1::MarketHalted),
        "MarketResumed" => decode!(generated::hl::canonical::v1::MarketResumed),
        "OpenInterestCapChanged" => decode!(generated::hl::canonical::v1::OpenInterestCapChanged),
        "MarginTableChanged" => decode!(generated::hl::canonical::v1::MarginTableChanged),
        "MarketCreated" => decode!(generated::hl::canonical::v1::MarketCreated),
        "MarketMetadataChanged" => decode!(generated::hl::canonical::v1::MarketMetadataChanged),
        "OracleUpdated" => decode!(generated::hl::canonical::v1::OracleUpdated),
        "FundingRateUpdated" => decode!(generated::hl::canonical::v1::FundingRateUpdated),
        "AssetContextUpdated" => decode!(generated::hl::canonical::v1::AssetContextUpdated),
        "DexCreated" => decode!(generated::hl::canonical::v1::DexCreated),
        "OutcomeCreated" => decode!(generated::hl::canonical::v1::OutcomeCreated),
        "OutcomeResolved" => decode!(generated::hl::canonical::v1::OutcomeResolved),
        other => Err(PayloadCodecError::UnknownKind(other.to_owned())),
    }
}

pub fn encode_trade_matched(value: &WireTradeMatched) -> Result<Vec<u8>, PayloadCodecError> {
    if value.price.is_empty() || value.quantity.is_empty() {
        return Err(PayloadCodecError::Invalid {
            kind: "TradeMatched".to_owned(),
            reason: "price and quantity are required".to_owned(),
        });
    }
    let message = generated::hl::canonical::v1::TradeMatched {
        trade_id: String::new(),
        market_id: String::new(),
        maker_order_id: String::new(),
        taker_order_id: String::new(),
        price: Some(generated::hl::common::v1::DecimalValue {
            value: value.price.clone(),
        }),
        quantity: Some(generated::hl::common::v1::DecimalValue {
            value: value.quantity.clone(),
        }),
        deterministic_seed: value.deterministic_seed,
    };
    Ok(wrap_payload("TradeMatched", message.encode_to_vec()))
}

pub fn decode_trade_matched(bytes: &[u8]) -> Result<WireTradeMatched, PayloadCodecError> {
    let body = unwrap_payload("TradeMatched", bytes)?;
    let message =
        generated::hl::canonical::v1::TradeMatched::decode(body.as_slice()).map_err(|source| {
            PayloadCodecError::Decode {
                kind: "TradeMatched".to_owned(),
                source,
            }
        })?;
    if !message.trade_id.is_empty()
        || !message.market_id.is_empty()
        || !message.maker_order_id.is_empty()
        || !message.taker_order_id.is_empty()
    {
        return Err(PayloadCodecError::Invalid {
            kind: "TradeMatched".to_owned(),
            reason: "legacy trade/order identifiers are unsupported by the V1 domain payload"
                .to_owned(),
        });
    }
    let price = message.price.ok_or_else(|| PayloadCodecError::Invalid {
        kind: "TradeMatched".to_owned(),
        reason: "missing price".to_owned(),
    })?;
    let quantity = message.quantity.ok_or_else(|| PayloadCodecError::Invalid {
        kind: "TradeMatched".to_owned(),
        reason: "missing quantity".to_owned(),
    })?;
    Ok(WireTradeMatched {
        price: price.value,
        quantity: quantity.value,
        deterministic_seed: message.deterministic_seed,
    })
}

fn default_message<M: Message + Default>() -> Vec<u8> {
    M::default().encode_to_vec()
}

fn wrap_payload(kind: &str, message: Vec<u8>) -> Vec<u8> {
    generated::hl::canonical::v1::TypedPayloadEnvelope {
        event_kind: kind.to_owned(),
        message,
    }
    .encode_to_vec()
}

fn unwrap_payload(kind: &str, bytes: &[u8]) -> Result<Vec<u8>, PayloadCodecError> {
    let envelope =
        generated::hl::canonical::v1::TypedPayloadEnvelope::decode(bytes).map_err(|source| {
            PayloadCodecError::Decode {
                kind: "TypedPayloadEnvelope".to_owned(),
                source,
            }
        })?;
    if envelope.event_kind != kind {
        return Err(PayloadCodecError::KindMismatch {
            expected: kind.to_owned(),
            actual: envelope.event_kind,
        });
    }
    Ok(envelope.message)
}
