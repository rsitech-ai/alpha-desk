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
pub const MAX_CANONICAL_ACCOUNT_PAYLOAD_BYTES: usize = 16 * 1024;
pub const MAX_CANONICAL_TRADE_PAYLOAD_BYTES: usize = 16 * 1024;

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
const CANONICAL_ACCOUNT_PAYLOAD_SIZE_REASON: &str =
    "canonical account payload exceeds the 16384-byte limit";
const CANONICAL_TRADE_PAYLOAD_SIZE_REASON: &str =
    "canonical trade payload exceeds the 16384-byte limit";
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
    pub source_event_index: Option<u32>,
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
            source_event_index: value.source_event_index,
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
            source_event_index: value.source_event_index,
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
    pub trade_id: Option<String>,
    pub market_id: Option<String>,
    pub maker_order_id: Option<String>,
    pub taker_order_id: Option<String>,
    pub price: String,
    pub quantity: String,
    pub deterministic_seed: u64,
    pub participants: Option<[WireTradeParticipantV1; 2]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireTradeParticipantV1 {
    pub role: String,
    pub account_id: String,
    pub start_position: String,
    pub order_id: String,
    pub twap_id: Option<u64>,
    pub client_order_id: Option<String>,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOrderRested {
    pub order_id: String,
    pub market_id: String,
    pub remaining_quantity: String,
    pub limit_price: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOrderModified {
    pub order_id: String,
    pub previous_price: String,
    pub new_price: String,
    pub previous_quantity: String,
    pub new_quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOrderPartiallyFilled {
    pub order_id: String,
    pub trade_id: String,
    pub fill_price: String,
    pub fill_quantity: String,
    pub remaining_quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOrderFilled {
    pub order_id: String,
    pub trade_id: String,
    pub fill_price: String,
    pub fill_quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOrderCancelled {
    pub order_id: String,
    pub reason: String,
    pub remaining_quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOrderRejected {
    pub client_order_id: String,
    pub account_id: String,
    pub reason_code: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireDepositCredited {
    pub account_id: String,
    pub asset_id: String,
    pub amount: String,
    pub deposit_reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireWithdrawalDebited {
    pub account_id: String,
    pub asset_id: String,
    pub amount: String,
    pub withdrawal_reference: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireSpotTransfer {
    pub from_account_id: String,
    pub to_account_id: String,
    pub asset_id: String,
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirePerpTransfer {
    pub from_account_id: String,
    pub to_account_id: String,
    pub quote_amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireSubaccountTransfer {
    pub master_account_id: String,
    pub from_account_id: String,
    pub to_account_id: String,
    pub asset_id: String,
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireVaultDeposit {
    pub vault_id: String,
    pub account_id: String,
    pub amount: String,
    pub shares_issued: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireVaultWithdrawal {
    pub vault_id: String,
    pub account_id: String,
    pub amount: String,
    pub shares_redeemed: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFeeCharged {
    pub account_id: String,
    pub asset_id: String,
    pub amount: String,
    pub fee_rate: String,
    pub fee_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireBuilderFeeCharged {
    pub account_id: String,
    pub builder_account_id: String,
    pub asset_id: String,
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFundingPaid {
    pub account_id: String,
    pub market_id: String,
    pub amount: String,
    pub funding_rate: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFundingReceived {
    pub account_id: String,
    pub market_id: String,
    pub amount: String,
    pub funding_rate: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireReferralReward {
    pub account_id: String,
    pub referrer_account_id: String,
    pub asset_id: String,
    pub amount: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireAccountModeChanged {
    pub account_id: String,
    pub previous_mode: String,
    pub new_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMarginModeChanged {
    pub account_id: String,
    pub market_id: String,
    pub previous_mode: String,
    pub new_mode: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireLeverageChanged {
    pub account_id: String,
    pub market_id: String,
    pub previous_leverage: String,
    pub new_leverage: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireLiquidationStarted {
    pub account_id: String,
    pub liquidation_id: String,
    pub margin_value: String,
    pub maintenance_requirement: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireLiquidationFill {
    pub liquidation_id: String,
    pub account_id: String,
    pub market_id: String,
    pub price: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireBackstopLiquidation {
    pub liquidation_id: String,
    pub account_id: String,
    pub backstop_account_id: String,
    pub market_id: String,
    pub quantity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WirePositionSettled {
    pub account_id: String,
    pub market_id: String,
    pub settlement_price: String,
    pub settled_quantity: String,
    pub realized_pnl: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireDexCreated {
    pub dex_id: String,
    pub name: String,
    pub operator_account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireAssetContextUpdated {
    pub asset_id: String,
    pub context_version: String,
    pub context_hash: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMarketCreated {
    pub market_id: String,
    pub dex_id: String,
    pub base_asset_id: String,
    pub quote_asset_id: String,
    pub tick_size: String,
    pub lot_size: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMarketMetadataChanged {
    pub market_id: String,
    pub metadata_version: String,
    pub metadata_hash: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMarketHalted {
    pub market_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMarketResumed {
    pub market_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOpenInterestCapChanged {
    pub market_id: String,
    pub previous_cap: String,
    pub new_cap: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireMarginTableChanged {
    pub market_id: String,
    pub previous_table_hash: String,
    pub new_table_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOracleUpdated {
    pub market_id: String,
    pub oracle_price: String,
    pub source: String,
    pub effective_at_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFundingRateUpdated {
    pub market_id: String,
    pub funding_rate: String,
    pub effective_at_micros: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOutcomeCreated {
    pub market_id: String,
    pub outcome_id: String,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOutcomeResolved {
    pub market_id: String,
    pub outcome_id: String,
    pub settlement_value: String,
    pub resolved_at_micros: i64,
}

pub fn encode_order_accepted(value: &WireOrderAccepted) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_order_accepted(value.clone())?;
    Ok(wrap_payload(
        "OrderAccepted",
        generated::hl::canonical::v1::OrderAccepted {
            order_id: value.order_id,
            account_id: value.account_id,
            market_id: value.market_id,
            side: value.side,
            limit_price: value.limit_price,
            quantity: value.quantity,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_order_accepted(bytes: &[u8]) -> Result<WireOrderAccepted, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OrderAccepted::decode(
        unwrap_payload("OrderAccepted", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OrderAccepted".to_owned(),
        source,
    })?;
    validate_order_accepted(WireOrderAccepted {
        order_id: message.order_id,
        account_id: message.account_id,
        market_id: message.market_id,
        side: message.side,
        limit_price: message.limit_price,
        quantity: message.quantity,
    })
}

pub fn encode_order_rested(value: &WireOrderRested) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_order_rested(value.clone())?;
    Ok(wrap_payload(
        "OrderRested",
        generated::hl::canonical::v1::OrderRested {
            order_id: value.order_id,
            market_id: value.market_id,
            remaining_quantity: value.remaining_quantity,
            limit_price: value.limit_price,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_order_rested(bytes: &[u8]) -> Result<WireOrderRested, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OrderRested::decode(
        unwrap_payload("OrderRested", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OrderRested".to_owned(),
        source,
    })?;
    validate_order_rested(WireOrderRested {
        order_id: message.order_id,
        market_id: message.market_id,
        remaining_quantity: message.remaining_quantity,
        limit_price: message.limit_price,
    })
}

pub fn encode_order_modified(value: &WireOrderModified) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_order_modified(value.clone())?;
    Ok(wrap_payload(
        "OrderModified",
        generated::hl::canonical::v1::OrderModified {
            order_id: value.order_id,
            previous_price: value.previous_price,
            new_price: value.new_price,
            previous_quantity: value.previous_quantity,
            new_quantity: value.new_quantity,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_order_modified(bytes: &[u8]) -> Result<WireOrderModified, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OrderModified::decode(
        unwrap_payload("OrderModified", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OrderModified".to_owned(),
        source,
    })?;
    validate_order_modified(WireOrderModified {
        order_id: message.order_id,
        previous_price: message.previous_price,
        new_price: message.new_price,
        previous_quantity: message.previous_quantity,
        new_quantity: message.new_quantity,
    })
}

pub fn encode_order_partially_filled(
    value: &WireOrderPartiallyFilled,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_order_partially_filled(value.clone())?;
    Ok(wrap_payload(
        "OrderPartiallyFilled",
        generated::hl::canonical::v1::OrderPartiallyFilled {
            order_id: value.order_id,
            trade_id: value.trade_id,
            fill_price: value.fill_price,
            fill_quantity: value.fill_quantity,
            remaining_quantity: value.remaining_quantity,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_order_partially_filled(
    bytes: &[u8],
) -> Result<WireOrderPartiallyFilled, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OrderPartiallyFilled::decode(
        unwrap_payload("OrderPartiallyFilled", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OrderPartiallyFilled".to_owned(),
        source,
    })?;
    validate_order_partially_filled(WireOrderPartiallyFilled {
        order_id: message.order_id,
        trade_id: message.trade_id,
        fill_price: message.fill_price,
        fill_quantity: message.fill_quantity,
        remaining_quantity: message.remaining_quantity,
    })
}

pub fn encode_order_filled(value: &WireOrderFilled) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_order_filled(value.clone())?;
    Ok(wrap_payload(
        "OrderFilled",
        generated::hl::canonical::v1::OrderFilled {
            order_id: value.order_id,
            trade_id: value.trade_id,
            fill_price: value.fill_price,
            fill_quantity: value.fill_quantity,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_order_filled(bytes: &[u8]) -> Result<WireOrderFilled, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OrderFilled::decode(
        unwrap_payload("OrderFilled", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OrderFilled".to_owned(),
        source,
    })?;
    validate_order_filled(WireOrderFilled {
        order_id: message.order_id,
        trade_id: message.trade_id,
        fill_price: message.fill_price,
        fill_quantity: message.fill_quantity,
    })
}

pub fn encode_order_cancelled(value: &WireOrderCancelled) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_order_cancelled(value.clone())?;
    Ok(wrap_payload(
        "OrderCancelled",
        generated::hl::canonical::v1::OrderCancelled {
            order_id: value.order_id,
            reason: value.reason,
            remaining_quantity: value.remaining_quantity,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_order_cancelled(bytes: &[u8]) -> Result<WireOrderCancelled, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OrderCancelled::decode(
        unwrap_payload("OrderCancelled", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OrderCancelled".to_owned(),
        source,
    })?;
    validate_order_cancelled(WireOrderCancelled {
        order_id: message.order_id,
        reason: message.reason,
        remaining_quantity: message.remaining_quantity,
    })
}

pub fn encode_order_rejected(value: &WireOrderRejected) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_order_rejected(value.clone())?;
    Ok(wrap_payload(
        "OrderRejected",
        generated::hl::canonical::v1::OrderRejected {
            client_order_id: value.client_order_id,
            account_id: value.account_id,
            reason_code: value.reason_code,
            reason: value.reason,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_order_rejected(bytes: &[u8]) -> Result<WireOrderRejected, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OrderRejected::decode(
        unwrap_payload("OrderRejected", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OrderRejected".to_owned(),
        source,
    })?;
    validate_order_rejected(WireOrderRejected {
        client_order_id: message.client_order_id,
        account_id: message.account_id,
        reason_code: message.reason_code,
        reason: message.reason,
    })
}

pub fn encode_deposit_credited(value: &WireDepositCredited) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_deposit_credited(value.clone())?;
    bounded_account_payload(
        "DepositCredited",
        generated::hl::canonical::v1::DepositCredited {
            account_id: value.account_id,
            asset_id: value.asset_id,
            amount: value.amount,
            deposit_reference: value.deposit_reference,
        }
        .encode_to_vec(),
    )
}

pub fn decode_deposit_credited(bytes: &[u8]) -> Result<WireDepositCredited, PayloadCodecError> {
    validate_account_payload_size("DepositCredited", bytes)?;
    let message = generated::hl::canonical::v1::DepositCredited::decode(
        unwrap_payload("DepositCredited", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "DepositCredited".to_owned(),
        source,
    })?;
    validate_deposit_credited(WireDepositCredited {
        account_id: message.account_id,
        asset_id: message.asset_id,
        amount: message.amount,
        deposit_reference: message.deposit_reference,
    })
}

pub fn encode_withdrawal_debited(
    value: &WireWithdrawalDebited,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_withdrawal_debited(value.clone())?;
    bounded_account_payload(
        "WithdrawalDebited",
        generated::hl::canonical::v1::WithdrawalDebited {
            account_id: value.account_id,
            asset_id: value.asset_id,
            amount: value.amount,
            withdrawal_reference: value.withdrawal_reference,
        }
        .encode_to_vec(),
    )
}

pub fn decode_withdrawal_debited(bytes: &[u8]) -> Result<WireWithdrawalDebited, PayloadCodecError> {
    validate_account_payload_size("WithdrawalDebited", bytes)?;
    let message = generated::hl::canonical::v1::WithdrawalDebited::decode(
        unwrap_payload("WithdrawalDebited", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "WithdrawalDebited".to_owned(),
        source,
    })?;
    validate_withdrawal_debited(WireWithdrawalDebited {
        account_id: message.account_id,
        asset_id: message.asset_id,
        amount: message.amount,
        withdrawal_reference: message.withdrawal_reference,
    })
}

pub fn encode_spot_transfer(value: &WireSpotTransfer) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_spot_transfer(value.clone())?;
    bounded_account_payload(
        "SpotTransfer",
        generated::hl::canonical::v1::SpotTransfer {
            from_account_id: value.from_account_id,
            to_account_id: value.to_account_id,
            asset_id: value.asset_id,
            amount: value.amount,
        }
        .encode_to_vec(),
    )
}

pub fn decode_spot_transfer(bytes: &[u8]) -> Result<WireSpotTransfer, PayloadCodecError> {
    validate_account_payload_size("SpotTransfer", bytes)?;
    let message = generated::hl::canonical::v1::SpotTransfer::decode(
        unwrap_payload("SpotTransfer", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "SpotTransfer".to_owned(),
        source,
    })?;
    validate_spot_transfer(WireSpotTransfer {
        from_account_id: message.from_account_id,
        to_account_id: message.to_account_id,
        asset_id: message.asset_id,
        amount: message.amount,
    })
}

pub fn encode_perp_transfer(value: &WirePerpTransfer) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_perp_transfer(value.clone())?;
    bounded_account_payload(
        "PerpTransfer",
        generated::hl::canonical::v1::PerpTransfer {
            from_account_id: value.from_account_id,
            to_account_id: value.to_account_id,
            quote_amount: value.quote_amount,
        }
        .encode_to_vec(),
    )
}

pub fn decode_perp_transfer(bytes: &[u8]) -> Result<WirePerpTransfer, PayloadCodecError> {
    validate_account_payload_size("PerpTransfer", bytes)?;
    let message = generated::hl::canonical::v1::PerpTransfer::decode(
        unwrap_payload("PerpTransfer", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "PerpTransfer".to_owned(),
        source,
    })?;
    validate_perp_transfer(WirePerpTransfer {
        from_account_id: message.from_account_id,
        to_account_id: message.to_account_id,
        quote_amount: message.quote_amount,
    })
}

pub fn encode_subaccount_transfer(
    value: &WireSubaccountTransfer,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_subaccount_transfer(value.clone())?;
    bounded_account_payload(
        "SubaccountTransfer",
        generated::hl::canonical::v1::SubaccountTransfer {
            master_account_id: value.master_account_id,
            from_account_id: value.from_account_id,
            to_account_id: value.to_account_id,
            asset_id: value.asset_id,
            amount: value.amount,
        }
        .encode_to_vec(),
    )
}

pub fn decode_subaccount_transfer(
    bytes: &[u8],
) -> Result<WireSubaccountTransfer, PayloadCodecError> {
    validate_account_payload_size("SubaccountTransfer", bytes)?;
    let message = generated::hl::canonical::v1::SubaccountTransfer::decode(
        unwrap_payload("SubaccountTransfer", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "SubaccountTransfer".to_owned(),
        source,
    })?;
    validate_subaccount_transfer(WireSubaccountTransfer {
        master_account_id: message.master_account_id,
        from_account_id: message.from_account_id,
        to_account_id: message.to_account_id,
        asset_id: message.asset_id,
        amount: message.amount,
    })
}

pub fn encode_vault_deposit(value: &WireVaultDeposit) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_vault_deposit(value.clone())?;
    bounded_account_payload(
        "VaultDeposit",
        generated::hl::canonical::v1::VaultDeposit {
            vault_id: value.vault_id,
            account_id: value.account_id,
            amount: value.amount,
            shares_issued: value.shares_issued,
        }
        .encode_to_vec(),
    )
}

pub fn decode_vault_deposit(bytes: &[u8]) -> Result<WireVaultDeposit, PayloadCodecError> {
    validate_account_payload_size("VaultDeposit", bytes)?;
    let message = generated::hl::canonical::v1::VaultDeposit::decode(
        unwrap_payload("VaultDeposit", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "VaultDeposit".to_owned(),
        source,
    })?;
    validate_vault_deposit(WireVaultDeposit {
        vault_id: message.vault_id,
        account_id: message.account_id,
        amount: message.amount,
        shares_issued: message.shares_issued,
    })
}

pub fn encode_vault_withdrawal(value: &WireVaultWithdrawal) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_vault_withdrawal(value.clone())?;
    bounded_account_payload(
        "VaultWithdrawal",
        generated::hl::canonical::v1::VaultWithdrawal {
            vault_id: value.vault_id,
            account_id: value.account_id,
            amount: value.amount,
            shares_redeemed: value.shares_redeemed,
        }
        .encode_to_vec(),
    )
}

pub fn decode_vault_withdrawal(bytes: &[u8]) -> Result<WireVaultWithdrawal, PayloadCodecError> {
    validate_account_payload_size("VaultWithdrawal", bytes)?;
    let message = generated::hl::canonical::v1::VaultWithdrawal::decode(
        unwrap_payload("VaultWithdrawal", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "VaultWithdrawal".to_owned(),
        source,
    })?;
    validate_vault_withdrawal(WireVaultWithdrawal {
        vault_id: message.vault_id,
        account_id: message.account_id,
        amount: message.amount,
        shares_redeemed: message.shares_redeemed,
    })
}

pub fn encode_fee_charged(value: &WireFeeCharged) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_fee_charged(value.clone())?;
    bounded_account_payload(
        "FeeCharged",
        generated::hl::canonical::v1::FeeCharged {
            account_id: value.account_id,
            asset_id: value.asset_id,
            amount: value.amount,
            fee_rate: value.fee_rate,
            fee_type: value.fee_type,
        }
        .encode_to_vec(),
    )
}

pub fn decode_fee_charged(bytes: &[u8]) -> Result<WireFeeCharged, PayloadCodecError> {
    validate_account_payload_size("FeeCharged", bytes)?;
    let message = generated::hl::canonical::v1::FeeCharged::decode(
        unwrap_payload("FeeCharged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "FeeCharged".to_owned(),
        source,
    })?;
    validate_fee_charged(WireFeeCharged {
        account_id: message.account_id,
        asset_id: message.asset_id,
        amount: message.amount,
        fee_rate: message.fee_rate,
        fee_type: message.fee_type,
    })
}

pub fn encode_builder_fee_charged(
    value: &WireBuilderFeeCharged,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_builder_fee_charged(value.clone())?;
    bounded_account_payload(
        "BuilderFeeCharged",
        generated::hl::canonical::v1::BuilderFeeCharged {
            account_id: value.account_id,
            builder_account_id: value.builder_account_id,
            asset_id: value.asset_id,
            amount: value.amount,
        }
        .encode_to_vec(),
    )
}

pub fn decode_builder_fee_charged(
    bytes: &[u8],
) -> Result<WireBuilderFeeCharged, PayloadCodecError> {
    validate_account_payload_size("BuilderFeeCharged", bytes)?;
    let message = generated::hl::canonical::v1::BuilderFeeCharged::decode(
        unwrap_payload("BuilderFeeCharged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "BuilderFeeCharged".to_owned(),
        source,
    })?;
    validate_builder_fee_charged(WireBuilderFeeCharged {
        account_id: message.account_id,
        builder_account_id: message.builder_account_id,
        asset_id: message.asset_id,
        amount: message.amount,
    })
}

pub fn encode_funding_paid(value: &WireFundingPaid) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_funding_paid(value.clone())?;
    bounded_account_payload(
        "FundingPaid",
        generated::hl::canonical::v1::FundingPaid {
            account_id: value.account_id,
            market_id: value.market_id,
            amount: value.amount,
            funding_rate: value.funding_rate,
        }
        .encode_to_vec(),
    )
}

pub fn decode_funding_paid(bytes: &[u8]) -> Result<WireFundingPaid, PayloadCodecError> {
    validate_account_payload_size("FundingPaid", bytes)?;
    let message = generated::hl::canonical::v1::FundingPaid::decode(
        unwrap_payload("FundingPaid", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "FundingPaid".to_owned(),
        source,
    })?;
    validate_funding_paid(WireFundingPaid {
        account_id: message.account_id,
        market_id: message.market_id,
        amount: message.amount,
        funding_rate: message.funding_rate,
    })
}

pub fn encode_funding_received(value: &WireFundingReceived) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_funding_received(value.clone())?;
    bounded_account_payload(
        "FundingReceived",
        generated::hl::canonical::v1::FundingReceived {
            account_id: value.account_id,
            market_id: value.market_id,
            amount: value.amount,
            funding_rate: value.funding_rate,
        }
        .encode_to_vec(),
    )
}

pub fn decode_funding_received(bytes: &[u8]) -> Result<WireFundingReceived, PayloadCodecError> {
    validate_account_payload_size("FundingReceived", bytes)?;
    let message = generated::hl::canonical::v1::FundingReceived::decode(
        unwrap_payload("FundingReceived", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "FundingReceived".to_owned(),
        source,
    })?;
    validate_funding_received(WireFundingReceived {
        account_id: message.account_id,
        market_id: message.market_id,
        amount: message.amount,
        funding_rate: message.funding_rate,
    })
}

pub fn encode_referral_reward(value: &WireReferralReward) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_referral_reward(value.clone())?;
    bounded_account_payload(
        "ReferralReward",
        generated::hl::canonical::v1::ReferralReward {
            account_id: value.account_id,
            referrer_account_id: value.referrer_account_id,
            asset_id: value.asset_id,
            amount: value.amount,
        }
        .encode_to_vec(),
    )
}

pub fn decode_referral_reward(bytes: &[u8]) -> Result<WireReferralReward, PayloadCodecError> {
    validate_account_payload_size("ReferralReward", bytes)?;
    let message = generated::hl::canonical::v1::ReferralReward::decode(
        unwrap_payload("ReferralReward", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "ReferralReward".to_owned(),
        source,
    })?;
    validate_referral_reward(WireReferralReward {
        account_id: message.account_id,
        referrer_account_id: message.referrer_account_id,
        asset_id: message.asset_id,
        amount: message.amount,
    })
}

pub fn encode_account_mode_changed(
    value: &WireAccountModeChanged,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_account_mode_changed(value.clone())?;
    bounded_account_payload(
        "AccountModeChanged",
        generated::hl::canonical::v1::AccountModeChanged {
            account_id: value.account_id,
            previous_mode: value.previous_mode,
            new_mode: value.new_mode,
        }
        .encode_to_vec(),
    )
}

pub fn decode_account_mode_changed(
    bytes: &[u8],
) -> Result<WireAccountModeChanged, PayloadCodecError> {
    validate_account_payload_size("AccountModeChanged", bytes)?;
    let message = generated::hl::canonical::v1::AccountModeChanged::decode(
        unwrap_payload("AccountModeChanged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "AccountModeChanged".to_owned(),
        source,
    })?;
    validate_account_mode_changed(WireAccountModeChanged {
        account_id: message.account_id,
        previous_mode: message.previous_mode,
        new_mode: message.new_mode,
    })
}

pub fn encode_margin_mode_changed(
    value: &WireMarginModeChanged,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_margin_mode_changed(value.clone())?;
    bounded_account_payload(
        "MarginModeChanged",
        generated::hl::canonical::v1::MarginModeChanged {
            account_id: value.account_id,
            market_id: value.market_id,
            previous_mode: value.previous_mode,
            new_mode: value.new_mode,
        }
        .encode_to_vec(),
    )
}

pub fn decode_margin_mode_changed(
    bytes: &[u8],
) -> Result<WireMarginModeChanged, PayloadCodecError> {
    validate_account_payload_size("MarginModeChanged", bytes)?;
    let message = generated::hl::canonical::v1::MarginModeChanged::decode(
        unwrap_payload("MarginModeChanged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "MarginModeChanged".to_owned(),
        source,
    })?;
    validate_margin_mode_changed(WireMarginModeChanged {
        account_id: message.account_id,
        market_id: message.market_id,
        previous_mode: message.previous_mode,
        new_mode: message.new_mode,
    })
}

pub fn encode_leverage_changed(value: &WireLeverageChanged) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_leverage_changed(value.clone())?;
    bounded_account_payload(
        "LeverageChanged",
        generated::hl::canonical::v1::LeverageChanged {
            account_id: value.account_id,
            market_id: value.market_id,
            previous_leverage: value.previous_leverage,
            new_leverage: value.new_leverage,
        }
        .encode_to_vec(),
    )
}

pub fn decode_leverage_changed(bytes: &[u8]) -> Result<WireLeverageChanged, PayloadCodecError> {
    validate_account_payload_size("LeverageChanged", bytes)?;
    let message = generated::hl::canonical::v1::LeverageChanged::decode(
        unwrap_payload("LeverageChanged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "LeverageChanged".to_owned(),
        source,
    })?;
    validate_leverage_changed(WireLeverageChanged {
        account_id: message.account_id,
        market_id: message.market_id,
        previous_leverage: message.previous_leverage,
        new_leverage: message.new_leverage,
    })
}

pub fn encode_liquidation_started(
    value: &WireLiquidationStarted,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_liquidation_started(value.clone())?;
    bounded_account_payload(
        "LiquidationStarted",
        generated::hl::canonical::v1::LiquidationStarted {
            account_id: value.account_id,
            liquidation_id: value.liquidation_id,
            margin_value: value.margin_value,
            maintenance_requirement: value.maintenance_requirement,
        }
        .encode_to_vec(),
    )
}

pub fn decode_liquidation_started(
    bytes: &[u8],
) -> Result<WireLiquidationStarted, PayloadCodecError> {
    validate_account_payload_size("LiquidationStarted", bytes)?;
    let message = generated::hl::canonical::v1::LiquidationStarted::decode(
        unwrap_payload("LiquidationStarted", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "LiquidationStarted".to_owned(),
        source,
    })?;
    validate_liquidation_started(WireLiquidationStarted {
        account_id: message.account_id,
        liquidation_id: message.liquidation_id,
        margin_value: message.margin_value,
        maintenance_requirement: message.maintenance_requirement,
    })
}

pub fn encode_liquidation_fill(value: &WireLiquidationFill) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_liquidation_fill(value.clone())?;
    bounded_account_payload(
        "LiquidationFill",
        generated::hl::canonical::v1::LiquidationFill {
            liquidation_id: value.liquidation_id,
            account_id: value.account_id,
            market_id: value.market_id,
            price: value.price,
            quantity: value.quantity,
        }
        .encode_to_vec(),
    )
}

pub fn decode_liquidation_fill(bytes: &[u8]) -> Result<WireLiquidationFill, PayloadCodecError> {
    validate_account_payload_size("LiquidationFill", bytes)?;
    let message = generated::hl::canonical::v1::LiquidationFill::decode(
        unwrap_payload("LiquidationFill", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "LiquidationFill".to_owned(),
        source,
    })?;
    validate_liquidation_fill(WireLiquidationFill {
        liquidation_id: message.liquidation_id,
        account_id: message.account_id,
        market_id: message.market_id,
        price: message.price,
        quantity: message.quantity,
    })
}

pub fn encode_backstop_liquidation(
    value: &WireBackstopLiquidation,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_backstop_liquidation(value.clone())?;
    bounded_account_payload(
        "BackstopLiquidation",
        generated::hl::canonical::v1::BackstopLiquidation {
            liquidation_id: value.liquidation_id,
            account_id: value.account_id,
            backstop_account_id: value.backstop_account_id,
            market_id: value.market_id,
            quantity: value.quantity,
        }
        .encode_to_vec(),
    )
}

pub fn decode_backstop_liquidation(
    bytes: &[u8],
) -> Result<WireBackstopLiquidation, PayloadCodecError> {
    validate_account_payload_size("BackstopLiquidation", bytes)?;
    let message = generated::hl::canonical::v1::BackstopLiquidation::decode(
        unwrap_payload("BackstopLiquidation", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "BackstopLiquidation".to_owned(),
        source,
    })?;
    validate_backstop_liquidation(WireBackstopLiquidation {
        liquidation_id: message.liquidation_id,
        account_id: message.account_id,
        backstop_account_id: message.backstop_account_id,
        market_id: message.market_id,
        quantity: message.quantity,
    })
}

pub fn encode_position_settled(value: &WirePositionSettled) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_position_settled(value.clone())?;
    bounded_account_payload(
        "PositionSettled",
        generated::hl::canonical::v1::PositionSettled {
            account_id: value.account_id,
            market_id: value.market_id,
            settlement_price: value.settlement_price,
            settled_quantity: value.settled_quantity,
            realized_pnl: value.realized_pnl,
        }
        .encode_to_vec(),
    )
}

pub fn decode_position_settled(bytes: &[u8]) -> Result<WirePositionSettled, PayloadCodecError> {
    validate_account_payload_size("PositionSettled", bytes)?;
    let message = generated::hl::canonical::v1::PositionSettled::decode(
        unwrap_payload("PositionSettled", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "PositionSettled".to_owned(),
        source,
    })?;
    validate_position_settled(WirePositionSettled {
        account_id: message.account_id,
        market_id: message.market_id,
        settlement_price: message.settlement_price,
        settled_quantity: message.settled_quantity,
        realized_pnl: message.realized_pnl,
    })
}

pub fn encode_dex_created(value: &WireDexCreated) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_dex_created(value.clone())?;
    Ok(wrap_payload(
        "DexCreated",
        generated::hl::canonical::v1::DexCreated {
            dex_id: value.dex_id,
            name: value.name,
            operator_account_id: value.operator_account_id,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_dex_created(bytes: &[u8]) -> Result<WireDexCreated, PayloadCodecError> {
    let message = generated::hl::canonical::v1::DexCreated::decode(
        unwrap_payload("DexCreated", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "DexCreated".to_owned(),
        source,
    })?;
    validate_dex_created(WireDexCreated {
        dex_id: message.dex_id,
        name: message.name,
        operator_account_id: message.operator_account_id,
    })
}

pub fn encode_asset_context_updated(
    value: &WireAssetContextUpdated,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_asset_context_updated(value.clone())?;
    Ok(wrap_payload(
        "AssetContextUpdated",
        generated::hl::canonical::v1::AssetContextUpdated {
            asset_id: value.asset_id,
            context_version: value.context_version,
            context_hash: value.context_hash,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_asset_context_updated(
    bytes: &[u8],
) -> Result<WireAssetContextUpdated, PayloadCodecError> {
    let message = generated::hl::canonical::v1::AssetContextUpdated::decode(
        unwrap_payload("AssetContextUpdated", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "AssetContextUpdated".to_owned(),
        source,
    })?;
    validate_asset_context_updated(WireAssetContextUpdated {
        asset_id: message.asset_id,
        context_version: message.context_version,
        context_hash: message.context_hash,
    })
}

pub fn encode_market_created(value: &WireMarketCreated) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_market_created(value.clone())?;
    Ok(wrap_payload(
        "MarketCreated",
        generated::hl::canonical::v1::MarketCreated {
            market_id: value.market_id,
            dex_id: value.dex_id,
            base_asset_id: value.base_asset_id,
            quote_asset_id: value.quote_asset_id,
            tick_size: value.tick_size,
            lot_size: value.lot_size,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_market_created(bytes: &[u8]) -> Result<WireMarketCreated, PayloadCodecError> {
    let message = generated::hl::canonical::v1::MarketCreated::decode(
        unwrap_payload("MarketCreated", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "MarketCreated".to_owned(),
        source,
    })?;
    validate_market_created(WireMarketCreated {
        market_id: message.market_id,
        dex_id: message.dex_id,
        base_asset_id: message.base_asset_id,
        quote_asset_id: message.quote_asset_id,
        tick_size: message.tick_size,
        lot_size: message.lot_size,
    })
}

pub fn encode_market_metadata_changed(
    value: &WireMarketMetadataChanged,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_market_metadata_changed(value.clone())?;
    Ok(wrap_payload(
        "MarketMetadataChanged",
        generated::hl::canonical::v1::MarketMetadataChanged {
            market_id: value.market_id,
            metadata_version: value.metadata_version,
            metadata_hash: value.metadata_hash,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_market_metadata_changed(
    bytes: &[u8],
) -> Result<WireMarketMetadataChanged, PayloadCodecError> {
    let message = generated::hl::canonical::v1::MarketMetadataChanged::decode(
        unwrap_payload("MarketMetadataChanged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "MarketMetadataChanged".to_owned(),
        source,
    })?;
    validate_market_metadata_changed(WireMarketMetadataChanged {
        market_id: message.market_id,
        metadata_version: message.metadata_version,
        metadata_hash: message.metadata_hash,
    })
}

pub fn encode_market_halted(value: &WireMarketHalted) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_market_halted(value.clone())?;
    Ok(wrap_payload(
        "MarketHalted",
        generated::hl::canonical::v1::MarketHalted {
            market_id: value.market_id,
            reason: value.reason,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_market_halted(bytes: &[u8]) -> Result<WireMarketHalted, PayloadCodecError> {
    let message = generated::hl::canonical::v1::MarketHalted::decode(
        unwrap_payload("MarketHalted", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "MarketHalted".to_owned(),
        source,
    })?;
    validate_market_halted(WireMarketHalted {
        market_id: message.market_id,
        reason: message.reason,
    })
}

pub fn encode_market_resumed(value: &WireMarketResumed) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_market_resumed(value.clone())?;
    Ok(wrap_payload(
        "MarketResumed",
        generated::hl::canonical::v1::MarketResumed {
            market_id: value.market_id,
            reason: value.reason,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_market_resumed(bytes: &[u8]) -> Result<WireMarketResumed, PayloadCodecError> {
    let message = generated::hl::canonical::v1::MarketResumed::decode(
        unwrap_payload("MarketResumed", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "MarketResumed".to_owned(),
        source,
    })?;
    validate_market_resumed(WireMarketResumed {
        market_id: message.market_id,
        reason: message.reason,
    })
}

pub fn encode_open_interest_cap_changed(
    value: &WireOpenInterestCapChanged,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_open_interest_cap_changed(value.clone())?;
    Ok(wrap_payload(
        "OpenInterestCapChanged",
        generated::hl::canonical::v1::OpenInterestCapChanged {
            market_id: value.market_id,
            previous_cap: value.previous_cap,
            new_cap: value.new_cap,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_open_interest_cap_changed(
    bytes: &[u8],
) -> Result<WireOpenInterestCapChanged, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OpenInterestCapChanged::decode(
        unwrap_payload("OpenInterestCapChanged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OpenInterestCapChanged".to_owned(),
        source,
    })?;
    validate_open_interest_cap_changed(WireOpenInterestCapChanged {
        market_id: message.market_id,
        previous_cap: message.previous_cap,
        new_cap: message.new_cap,
    })
}

pub fn encode_margin_table_changed(
    value: &WireMarginTableChanged,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_margin_table_changed(value.clone())?;
    Ok(wrap_payload(
        "MarginTableChanged",
        generated::hl::canonical::v1::MarginTableChanged {
            market_id: value.market_id,
            previous_table_hash: value.previous_table_hash,
            new_table_hash: value.new_table_hash,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_margin_table_changed(
    bytes: &[u8],
) -> Result<WireMarginTableChanged, PayloadCodecError> {
    let message = generated::hl::canonical::v1::MarginTableChanged::decode(
        unwrap_payload("MarginTableChanged", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "MarginTableChanged".to_owned(),
        source,
    })?;
    validate_margin_table_changed(WireMarginTableChanged {
        market_id: message.market_id,
        previous_table_hash: message.previous_table_hash,
        new_table_hash: message.new_table_hash,
    })
}

pub fn encode_oracle_updated(value: &WireOracleUpdated) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_oracle_updated(value.clone())?;
    Ok(wrap_payload(
        "OracleUpdated",
        generated::hl::canonical::v1::OracleUpdated {
            market_id: value.market_id,
            oracle_price: value.oracle_price,
            source: value.source,
            effective_at_micros: value.effective_at_micros,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_oracle_updated(bytes: &[u8]) -> Result<WireOracleUpdated, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OracleUpdated::decode(
        unwrap_payload("OracleUpdated", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OracleUpdated".to_owned(),
        source,
    })?;
    validate_oracle_updated(WireOracleUpdated {
        market_id: message.market_id,
        oracle_price: message.oracle_price,
        source: message.source,
        effective_at_micros: message.effective_at_micros,
    })
}

pub fn encode_funding_rate_updated(
    value: &WireFundingRateUpdated,
) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_funding_rate_updated(value.clone())?;
    Ok(wrap_payload(
        "FundingRateUpdated",
        generated::hl::canonical::v1::FundingRateUpdated {
            market_id: value.market_id,
            funding_rate: value.funding_rate,
            effective_at_micros: value.effective_at_micros,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_funding_rate_updated(
    bytes: &[u8],
) -> Result<WireFundingRateUpdated, PayloadCodecError> {
    let message = generated::hl::canonical::v1::FundingRateUpdated::decode(
        unwrap_payload("FundingRateUpdated", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "FundingRateUpdated".to_owned(),
        source,
    })?;
    validate_funding_rate_updated(WireFundingRateUpdated {
        market_id: message.market_id,
        funding_rate: message.funding_rate,
        effective_at_micros: message.effective_at_micros,
    })
}

pub fn encode_outcome_created(value: &WireOutcomeCreated) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_outcome_created(value.clone())?;
    Ok(wrap_payload(
        "OutcomeCreated",
        generated::hl::canonical::v1::OutcomeCreated {
            market_id: value.market_id,
            outcome_id: value.outcome_id,
            description: value.description,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_outcome_created(bytes: &[u8]) -> Result<WireOutcomeCreated, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OutcomeCreated::decode(
        unwrap_payload("OutcomeCreated", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OutcomeCreated".to_owned(),
        source,
    })?;
    validate_outcome_created(WireOutcomeCreated {
        market_id: message.market_id,
        outcome_id: message.outcome_id,
        description: message.description,
    })
}

pub fn encode_outcome_resolved(value: &WireOutcomeResolved) -> Result<Vec<u8>, PayloadCodecError> {
    let value = validate_outcome_resolved(value.clone())?;
    Ok(wrap_payload(
        "OutcomeResolved",
        generated::hl::canonical::v1::OutcomeResolved {
            market_id: value.market_id,
            outcome_id: value.outcome_id,
            settlement_value: value.settlement_value,
            resolved_at_micros: value.resolved_at_micros,
        }
        .encode_to_vec(),
    ))
}

pub fn decode_outcome_resolved(bytes: &[u8]) -> Result<WireOutcomeResolved, PayloadCodecError> {
    let message = generated::hl::canonical::v1::OutcomeResolved::decode(
        unwrap_payload("OutcomeResolved", bytes)?.as_slice(),
    )
    .map_err(|source| PayloadCodecError::Decode {
        kind: "OutcomeResolved".to_owned(),
        source,
    })?;
    validate_outcome_resolved(WireOutcomeResolved {
        market_id: message.market_id,
        outcome_id: message.outcome_id,
        settlement_value: message.settlement_value,
        resolved_at_micros: message.resolved_at_micros,
    })
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
                trade_id: None,
                market_id: None,
                maker_order_id: None,
                taker_order_id: None,
                price: "1".to_owned(),
                quantity: "1".to_owned(),
                deterministic_seed: 0,
                participants: None,
            });
        }
        "DepositCredited" => {
            return encode_deposit_credited(&WireDepositCredited {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                deposit_reference: "synthetic-default-deposit".to_owned(),
            });
        }
        "WithdrawalDebited" => {
            return encode_withdrawal_debited(&WireWithdrawalDebited {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                withdrawal_reference: "synthetic-default-withdrawal".to_owned(),
            });
        }
        "SpotTransfer" => {
            return encode_spot_transfer(&WireSpotTransfer {
                from_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                to_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            });
        }
        "PerpTransfer" => {
            return encode_perp_transfer(&WirePerpTransfer {
                from_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                to_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
                quote_amount: "1".to_owned(),
            });
        }
        "SubaccountTransfer" => {
            return encode_subaccount_transfer(&WireSubaccountTransfer {
                master_account_id: "0x3333333333333333333333333333333333333333".to_owned(),
                from_account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                to_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            });
        }
        "VaultDeposit" => {
            return encode_vault_deposit(&WireVaultDeposit {
                vault_id: "synthetic-default-vault".to_owned(),
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                amount: "1".to_owned(),
                shares_issued: "1".to_owned(),
            });
        }
        "VaultWithdrawal" => {
            return encode_vault_withdrawal(&WireVaultWithdrawal {
                vault_id: "synthetic-default-vault".to_owned(),
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                amount: "1".to_owned(),
                shares_redeemed: "1".to_owned(),
            });
        }
        "FeeCharged" => {
            return encode_fee_charged(&WireFeeCharged {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
                fee_rate: "0.001".to_owned(),
                fee_type: "protocol".to_owned(),
            });
        }
        "BuilderFeeCharged" => {
            return encode_builder_fee_charged(&WireBuilderFeeCharged {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                builder_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            });
        }
        "FundingPaid" => {
            return encode_funding_paid(&WireFundingPaid {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                market_id: "perp:BTC".to_owned(),
                amount: "1".to_owned(),
                funding_rate: "-0.0001".to_owned(),
            });
        }
        "FundingReceived" => {
            return encode_funding_received(&WireFundingReceived {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                market_id: "perp:BTC".to_owned(),
                amount: "1".to_owned(),
                funding_rate: "0.0001".to_owned(),
            });
        }
        "ReferralReward" => {
            return encode_referral_reward(&WireReferralReward {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                referrer_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
                asset_id: "USDC".to_owned(),
                amount: "1".to_owned(),
            });
        }
        "AccountModeChanged" => {
            return encode_account_mode_changed(&WireAccountModeChanged {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                previous_mode: "standard".to_owned(),
                new_mode: "unified".to_owned(),
            });
        }
        "MarginModeChanged" => {
            return encode_margin_mode_changed(&WireMarginModeChanged {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                market_id: "perp:BTC".to_owned(),
                previous_mode: "cross".to_owned(),
                new_mode: "isolated".to_owned(),
            });
        }
        "LeverageChanged" => {
            return encode_leverage_changed(&WireLeverageChanged {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                market_id: "perp:BTC".to_owned(),
                previous_leverage: "1".to_owned(),
                new_leverage: "2".to_owned(),
            });
        }
        "LiquidationStarted" => {
            return encode_liquidation_started(&WireLiquidationStarted {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                liquidation_id: "synthetic-default-liquidation".to_owned(),
                margin_value: "0".to_owned(),
                maintenance_requirement: "1".to_owned(),
            });
        }
        "LiquidationFill" => {
            return encode_liquidation_fill(&WireLiquidationFill {
                liquidation_id: "synthetic-default-liquidation".to_owned(),
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                market_id: "perp:BTC".to_owned(),
                price: "1".to_owned(),
                quantity: "1".to_owned(),
            });
        }
        "BackstopLiquidation" => {
            return encode_backstop_liquidation(&WireBackstopLiquidation {
                liquidation_id: "synthetic-default-liquidation".to_owned(),
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                backstop_account_id: "0x2222222222222222222222222222222222222222".to_owned(),
                market_id: "perp:BTC".to_owned(),
                quantity: "1".to_owned(),
            });
        }
        "PositionSettled" => {
            return encode_position_settled(&WirePositionSettled {
                account_id: "0x1111111111111111111111111111111111111111".to_owned(),
                market_id: "perp:BTC".to_owned(),
                settlement_price: "0".to_owned(),
                settled_quantity: "1".to_owned(),
                realized_pnl: "0".to_owned(),
            });
        }
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
    if matches!(
        kind,
        "DepositCredited"
            | "WithdrawalDebited"
            | "SpotTransfer"
            | "PerpTransfer"
            | "SubaccountTransfer"
            | "VaultDeposit"
            | "VaultWithdrawal"
            | "FeeCharged"
            | "BuilderFeeCharged"
            | "FundingPaid"
            | "FundingReceived"
            | "ReferralReward"
            | "AccountModeChanged"
            | "MarginModeChanged"
            | "LeverageChanged"
            | "LiquidationStarted"
            | "LiquidationFill"
            | "BackstopLiquidation"
            | "PositionSettled"
    ) {
        validate_account_payload_size(kind, bytes)?;
    }
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
        "OrderAccepted" => decode_order_accepted(bytes).map(|_| ()),
        "OrderRested" => decode_order_rested(bytes).map(|_| ()),
        "OrderModified" => decode_order_modified(bytes).map(|_| ()),
        "OrderPartiallyFilled" => decode_order_partially_filled(bytes).map(|_| ()),
        "OrderFilled" => decode_order_filled(bytes).map(|_| ()),
        "OrderCancelled" => decode_order_cancelled(bytes).map(|_| ()),
        "OrderRejected" => decode_order_rejected(bytes).map(|_| ()),
        "TriggerOrderActivated" => decode!(generated::hl::canonical::v1::TriggerOrderActivated),
        "TwapStarted" => decode!(generated::hl::canonical::v1::TwapStarted),
        "TwapSliceFilled" => decode!(generated::hl::canonical::v1::TwapSliceFilled),
        "TwapCompleted" => decode!(generated::hl::canonical::v1::TwapCompleted),
        "TradeMatched" => decode_trade_matched(bytes).map(|_| ()),
        "DepositCredited" => decode_deposit_credited(bytes).map(|_| ()),
        "WithdrawalDebited" => decode_withdrawal_debited(bytes).map(|_| ()),
        "SpotTransfer" => decode_spot_transfer(bytes).map(|_| ()),
        "PerpTransfer" => decode_perp_transfer(bytes).map(|_| ()),
        "SubaccountTransfer" => decode_subaccount_transfer(bytes).map(|_| ()),
        "VaultDeposit" => decode_vault_deposit(bytes).map(|_| ()),
        "VaultWithdrawal" => decode_vault_withdrawal(bytes).map(|_| ()),
        "FeeCharged" => decode_fee_charged(bytes).map(|_| ()),
        "BuilderFeeCharged" => decode_builder_fee_charged(bytes).map(|_| ()),
        "FundingPaid" => decode_funding_paid(bytes).map(|_| ()),
        "FundingReceived" => decode_funding_received(bytes).map(|_| ()),
        "ReferralReward" => decode_referral_reward(bytes).map(|_| ()),
        "AccountModeChanged" => decode_account_mode_changed(bytes).map(|_| ()),
        "MarginModeChanged" => decode_margin_mode_changed(bytes).map(|_| ()),
        "LeverageChanged" => decode_leverage_changed(bytes).map(|_| ()),
        "LiquidationStarted" => decode_liquidation_started(bytes).map(|_| ()),
        "LiquidationFill" => decode_liquidation_fill(bytes).map(|_| ()),
        "BackstopLiquidation" => decode_backstop_liquidation(bytes).map(|_| ()),
        "PositionSettled" => decode_position_settled(bytes).map(|_| ()),
        "MarketHalted" => decode_market_halted(bytes).map(|_| ()),
        "MarketResumed" => decode_market_resumed(bytes).map(|_| ()),
        "OpenInterestCapChanged" => decode_open_interest_cap_changed(bytes).map(|_| ()),
        "MarginTableChanged" => decode_margin_table_changed(bytes).map(|_| ()),
        "MarketCreated" => decode_market_created(bytes).map(|_| ()),
        "MarketMetadataChanged" => decode_market_metadata_changed(bytes).map(|_| ()),
        "OracleUpdated" => decode_oracle_updated(bytes).map(|_| ()),
        "FundingRateUpdated" => decode_funding_rate_updated(bytes).map(|_| ()),
        "AssetContextUpdated" => decode_asset_context_updated(bytes).map(|_| ()),
        "DexCreated" => decode_dex_created(bytes).map(|_| ()),
        "OutcomeCreated" => decode_outcome_created(bytes).map(|_| ()),
        "OutcomeResolved" => decode_outcome_resolved(bytes).map(|_| ()),
        other => Err(PayloadCodecError::UnknownKind(other.to_owned())),
    }
}

pub fn encode_trade_matched(value: &WireTradeMatched) -> Result<Vec<u8>, PayloadCodecError> {
    require_positive_wire_decimal("TradeMatched", "price", &value.price)?;
    require_positive_wire_decimal("TradeMatched", "quantity", &value.quantity)?;
    let participants = value
        .participants
        .clone()
        .map(validate_trade_participants)
        .transpose()?
        .map(|participants| {
            participants
                .into_iter()
                .map(encode_trade_participant)
                .collect()
        })
        .unwrap_or_default();
    let message = generated::hl::canonical::v1::TradeMatched {
        trade_id: encode_optional_identity("trade_id", &value.trade_id)?,
        market_id: encode_optional_identity("market_id", &value.market_id)?,
        maker_order_id: encode_optional_identity("maker_order_id", &value.maker_order_id)?,
        taker_order_id: encode_optional_identity("taker_order_id", &value.taker_order_id)?,
        price: Some(generated::hl::common::v1::DecimalValue {
            value: value.price.clone(),
        }),
        quantity: Some(generated::hl::common::v1::DecimalValue {
            value: value.quantity.clone(),
        }),
        deterministic_seed: value.deterministic_seed,
        participants,
    };
    bounded_trade_payload("TradeMatched", message.encode_to_vec())
}

pub fn decode_trade_matched(bytes: &[u8]) -> Result<WireTradeMatched, PayloadCodecError> {
    validate_trade_payload_size("TradeMatched", bytes)?;
    let body = unwrap_payload("TradeMatched", bytes)?;
    let message =
        generated::hl::canonical::v1::TradeMatched::decode(body.as_slice()).map_err(|source| {
            PayloadCodecError::Decode {
                kind: "TradeMatched".to_owned(),
                source,
            }
        })?;
    let price = message.price.ok_or_else(|| PayloadCodecError::Invalid {
        kind: "TradeMatched".to_owned(),
        reason: "missing price".to_owned(),
    })?;
    let quantity = message.quantity.ok_or_else(|| PayloadCodecError::Invalid {
        kind: "TradeMatched".to_owned(),
        reason: "missing quantity".to_owned(),
    })?;
    require_positive_wire_decimal("TradeMatched", "price", &price.value)?;
    require_positive_wire_decimal("TradeMatched", "quantity", &quantity.value)?;
    let participants: Option<[WireTradeParticipantV1; 2]> = match message.participants.len() {
        0 => None,
        2 => {
            let participants = message
                .participants
                .into_iter()
                .map(decode_trade_participant)
                .collect::<Result<Vec<_>, _>>()?
                .try_into()
                .expect("length checked before conversion");
            Some(validate_trade_participants(participants)?)
        }
        actual => {
            return Err(PayloadCodecError::Invalid {
                kind: "TradeMatched".to_owned(),
                reason: format!(
                    "participants must contain zero or exactly two entries, got {actual}"
                ),
            });
        }
    };
    Ok(WireTradeMatched {
        trade_id: decode_optional_identity("trade_id", message.trade_id)?,
        market_id: decode_optional_identity("market_id", message.market_id)?,
        maker_order_id: decode_optional_identity("maker_order_id", message.maker_order_id)?,
        taker_order_id: decode_optional_identity("taker_order_id", message.taker_order_id)?,
        price: price.value,
        quantity: quantity.value,
        deterministic_seed: message.deterministic_seed,
        participants,
    })
}

fn validate_trade_participants(
    mut participants: [WireTradeParticipantV1; 2],
) -> Result<[WireTradeParticipantV1; 2], PayloadCodecError> {
    for participant in &mut participants {
        participant.account_id = required_api_address(
            "TradeMatched",
            "participants.account_id",
            std::mem::take(&mut participant.account_id),
        )?;
        participant.start_position = required_payload_field(
            "TradeMatched",
            "participants.start_position",
            std::mem::take(&mut participant.start_position),
        )?;
        parse_wire_decimal(
            "TradeMatched",
            "participants.start_position",
            &participant.start_position,
        )?;
        participant.order_id = required_payload_field(
            "TradeMatched",
            "participants.order_id",
            std::mem::take(&mut participant.order_id),
        )?;
        participant.client_order_id = participant
            .client_order_id
            .take()
            .map(|value| {
                required_bounded_text("TradeMatched", "participants.client_order_id", value, 256)
            })
            .transpose()?;
    }
    if participants[0].role != "buyer" || participants[1].role != "seller" {
        return Err(PayloadCodecError::Invalid {
            kind: "TradeMatched".to_owned(),
            reason: "participants must be ordered buyer then seller".to_owned(),
        });
    }
    require_distinct_accounts(
        "TradeMatched",
        "participants[0].account_id",
        &participants[0].account_id,
        "participants[1].account_id",
        &participants[1].account_id,
    )?;
    Ok(participants)
}

fn encode_trade_participant(
    value: WireTradeParticipantV1,
) -> generated::hl::canonical::v1::TradeParticipantV1 {
    let role = match value.role.as_str() {
        "buyer" => generated::hl::canonical::v1::TradeParticipantRoleV1::Buyer as i32,
        "seller" => generated::hl::canonical::v1::TradeParticipantRoleV1::Seller as i32,
        _ => unreachable!("roles are validated before encoding"),
    };
    generated::hl::canonical::v1::TradeParticipantV1 {
        role,
        account_id: value.account_id,
        start_position: value.start_position,
        order_id: value.order_id,
        twap_id: value.twap_id,
        client_order_id: value.client_order_id.unwrap_or_default(),
    }
}

fn decode_trade_participant(
    value: generated::hl::canonical::v1::TradeParticipantV1,
) -> Result<WireTradeParticipantV1, PayloadCodecError> {
    let role = match generated::hl::canonical::v1::TradeParticipantRoleV1::try_from(value.role) {
        Ok(generated::hl::canonical::v1::TradeParticipantRoleV1::Buyer) => "buyer",
        Ok(generated::hl::canonical::v1::TradeParticipantRoleV1::Seller) => "seller",
        _ => {
            return Err(PayloadCodecError::Invalid {
                kind: "TradeMatched".to_owned(),
                reason: "participant role must be buyer or seller".to_owned(),
            });
        }
    };
    Ok(WireTradeParticipantV1 {
        role: role.to_owned(),
        account_id: value.account_id,
        start_position: value.start_position,
        order_id: value.order_id,
        twap_id: value.twap_id,
        client_order_id: decode_optional_identity(
            "participants.client_order_id",
            value.client_order_id,
        )?,
    })
}

fn encode_optional_identity(
    field: &str,
    value: &Option<String>,
) -> Result<String, PayloadCodecError> {
    match value {
        None => Ok(String::new()),
        Some(value) if value.is_empty() || value.trim() != value => {
            Err(PayloadCodecError::Invalid {
                kind: "TradeMatched".to_owned(),
                reason: format!("{field} must be non-empty without surrounding whitespace"),
            })
        }
        Some(value) => Ok(value.clone()),
    }
}

fn validate_order_accepted(
    mut value: WireOrderAccepted,
) -> Result<WireOrderAccepted, PayloadCodecError> {
    value.order_id = required_payload_field("OrderAccepted", "order_id", value.order_id)?;
    value.account_id = required_payload_field("OrderAccepted", "account_id", value.account_id)?;
    value.market_id = required_payload_field("OrderAccepted", "market_id", value.market_id)?;
    value.side = required_payload_field("OrderAccepted", "side", value.side)?;
    value.limit_price = required_payload_field("OrderAccepted", "limit_price", value.limit_price)?;
    value.quantity = required_payload_field("OrderAccepted", "quantity", value.quantity)?;
    Ok(value)
}

fn validate_order_rested(mut value: WireOrderRested) -> Result<WireOrderRested, PayloadCodecError> {
    value.order_id = required_payload_field("OrderRested", "order_id", value.order_id)?;
    value.market_id = required_payload_field("OrderRested", "market_id", value.market_id)?;
    value.remaining_quantity = required_payload_field(
        "OrderRested",
        "remaining_quantity",
        value.remaining_quantity,
    )?;
    value.limit_price = required_payload_field("OrderRested", "limit_price", value.limit_price)?;
    Ok(value)
}

fn validate_order_modified(
    mut value: WireOrderModified,
) -> Result<WireOrderModified, PayloadCodecError> {
    value.order_id = required_payload_field("OrderModified", "order_id", value.order_id)?;
    value.previous_price =
        required_payload_field("OrderModified", "previous_price", value.previous_price)?;
    value.new_price = required_payload_field("OrderModified", "new_price", value.new_price)?;
    value.previous_quantity = required_payload_field(
        "OrderModified",
        "previous_quantity",
        value.previous_quantity,
    )?;
    value.new_quantity =
        required_payload_field("OrderModified", "new_quantity", value.new_quantity)?;
    Ok(value)
}

fn validate_order_partially_filled(
    mut value: WireOrderPartiallyFilled,
) -> Result<WireOrderPartiallyFilled, PayloadCodecError> {
    value.order_id = required_payload_field("OrderPartiallyFilled", "order_id", value.order_id)?;
    value.trade_id = required_payload_field("OrderPartiallyFilled", "trade_id", value.trade_id)?;
    value.fill_price =
        required_payload_field("OrderPartiallyFilled", "fill_price", value.fill_price)?;
    value.fill_quantity =
        required_payload_field("OrderPartiallyFilled", "fill_quantity", value.fill_quantity)?;
    value.remaining_quantity = required_payload_field(
        "OrderPartiallyFilled",
        "remaining_quantity",
        value.remaining_quantity,
    )?;
    Ok(value)
}

fn validate_order_filled(mut value: WireOrderFilled) -> Result<WireOrderFilled, PayloadCodecError> {
    value.order_id = required_payload_field("OrderFilled", "order_id", value.order_id)?;
    value.trade_id = required_payload_field("OrderFilled", "trade_id", value.trade_id)?;
    value.fill_price = required_payload_field("OrderFilled", "fill_price", value.fill_price)?;
    value.fill_quantity =
        required_payload_field("OrderFilled", "fill_quantity", value.fill_quantity)?;
    Ok(value)
}

fn validate_order_cancelled(
    mut value: WireOrderCancelled,
) -> Result<WireOrderCancelled, PayloadCodecError> {
    value.order_id = required_payload_field("OrderCancelled", "order_id", value.order_id)?;
    value.reason = required_bounded_text("OrderCancelled", "reason", value.reason, 1_024)?;
    value.remaining_quantity = required_payload_field(
        "OrderCancelled",
        "remaining_quantity",
        value.remaining_quantity,
    )?;
    Ok(value)
}

fn validate_order_rejected(
    mut value: WireOrderRejected,
) -> Result<WireOrderRejected, PayloadCodecError> {
    value.client_order_id =
        required_payload_field("OrderRejected", "client_order_id", value.client_order_id)?;
    value.account_id = required_payload_field("OrderRejected", "account_id", value.account_id)?;
    value.reason_code =
        required_bounded_text("OrderRejected", "reason_code", value.reason_code, 128)?;
    value.reason = required_bounded_text("OrderRejected", "reason", value.reason, 1_024)?;
    Ok(value)
}

fn validate_deposit_credited(
    mut value: WireDepositCredited,
) -> Result<WireDepositCredited, PayloadCodecError> {
    value.account_id = required_api_address("DepositCredited", "account_id", value.account_id)?;
    value.asset_id = required_payload_field("DepositCredited", "asset_id", value.asset_id)?;
    value.amount = required_payload_field("DepositCredited", "amount", value.amount)?;
    value.deposit_reference = required_reference(
        "DepositCredited",
        "deposit_reference",
        value.deposit_reference,
    )?;
    Ok(value)
}

fn validate_withdrawal_debited(
    mut value: WireWithdrawalDebited,
) -> Result<WireWithdrawalDebited, PayloadCodecError> {
    value.account_id = required_api_address("WithdrawalDebited", "account_id", value.account_id)?;
    value.asset_id = required_payload_field("WithdrawalDebited", "asset_id", value.asset_id)?;
    value.amount = required_payload_field("WithdrawalDebited", "amount", value.amount)?;
    value.withdrawal_reference = required_reference(
        "WithdrawalDebited",
        "withdrawal_reference",
        value.withdrawal_reference,
    )?;
    Ok(value)
}

fn validate_spot_transfer(
    mut value: WireSpotTransfer,
) -> Result<WireSpotTransfer, PayloadCodecError> {
    value.from_account_id =
        required_api_address("SpotTransfer", "from_account_id", value.from_account_id)?;
    value.to_account_id =
        required_api_address("SpotTransfer", "to_account_id", value.to_account_id)?;
    require_distinct_endpoints("SpotTransfer", &value.from_account_id, &value.to_account_id)?;
    value.asset_id = required_payload_field("SpotTransfer", "asset_id", value.asset_id)?;
    value.amount = required_payload_field("SpotTransfer", "amount", value.amount)?;
    Ok(value)
}

fn validate_perp_transfer(
    mut value: WirePerpTransfer,
) -> Result<WirePerpTransfer, PayloadCodecError> {
    value.from_account_id =
        required_api_address("PerpTransfer", "from_account_id", value.from_account_id)?;
    value.to_account_id =
        required_api_address("PerpTransfer", "to_account_id", value.to_account_id)?;
    require_distinct_endpoints("PerpTransfer", &value.from_account_id, &value.to_account_id)?;
    value.quote_amount =
        required_payload_field("PerpTransfer", "quote_amount", value.quote_amount)?;
    Ok(value)
}

fn validate_subaccount_transfer(
    mut value: WireSubaccountTransfer,
) -> Result<WireSubaccountTransfer, PayloadCodecError> {
    value.master_account_id = required_api_address(
        "SubaccountTransfer",
        "master_account_id",
        value.master_account_id,
    )?;
    value.from_account_id = required_api_address(
        "SubaccountTransfer",
        "from_account_id",
        value.from_account_id,
    )?;
    value.to_account_id =
        required_api_address("SubaccountTransfer", "to_account_id", value.to_account_id)?;
    require_distinct_endpoints(
        "SubaccountTransfer",
        &value.from_account_id,
        &value.to_account_id,
    )?;
    value.asset_id = required_payload_field("SubaccountTransfer", "asset_id", value.asset_id)?;
    value.amount = required_payload_field("SubaccountTransfer", "amount", value.amount)?;
    Ok(value)
}

fn validate_vault_deposit(
    mut value: WireVaultDeposit,
) -> Result<WireVaultDeposit, PayloadCodecError> {
    value.vault_id = required_payload_field("VaultDeposit", "vault_id", value.vault_id)?;
    value.account_id = required_api_address("VaultDeposit", "account_id", value.account_id)?;
    value.amount = required_payload_field("VaultDeposit", "amount", value.amount)?;
    value.shares_issued =
        required_payload_field("VaultDeposit", "shares_issued", value.shares_issued)?;
    Ok(value)
}

fn validate_vault_withdrawal(
    mut value: WireVaultWithdrawal,
) -> Result<WireVaultWithdrawal, PayloadCodecError> {
    value.vault_id = required_payload_field("VaultWithdrawal", "vault_id", value.vault_id)?;
    value.account_id = required_api_address("VaultWithdrawal", "account_id", value.account_id)?;
    value.amount = required_payload_field("VaultWithdrawal", "amount", value.amount)?;
    value.shares_redeemed =
        required_payload_field("VaultWithdrawal", "shares_redeemed", value.shares_redeemed)?;
    Ok(value)
}

fn validate_fee_charged(mut value: WireFeeCharged) -> Result<WireFeeCharged, PayloadCodecError> {
    const CHARGED_FEE_TYPES: [&str; 4] = ["maker", "taker", "referral_discount", "protocol"];
    value.account_id = required_api_address("FeeCharged", "account_id", value.account_id)?;
    value.asset_id = required_payload_field("FeeCharged", "asset_id", value.asset_id)?;
    value.amount = required_payload_field("FeeCharged", "amount", value.amount)?;
    value.fee_rate = required_payload_field("FeeCharged", "fee_rate", value.fee_rate)?;
    value.fee_type = required_wire_value(
        "FeeCharged",
        "fee_type",
        value.fee_type,
        &[
            "maker",
            "taker",
            "maker_rebate",
            "referral_discount",
            "protocol",
        ],
    )?;
    let sign = decimal_wire_sign("FeeCharged", "fee_rate", &value.fee_rate)?;
    let valid_sign = if value.fee_type == "maker_rebate" {
        sign < 0
    } else {
        CHARGED_FEE_TYPES.contains(&value.fee_type.as_str()) && sign > 0
    };
    if !valid_sign {
        return Err(PayloadCodecError::Invalid {
            kind: "FeeCharged".to_owned(),
            reason: "maker_rebate requires a negative fee_rate; charged fees require a positive fee_rate"
                .to_owned(),
        });
    }
    Ok(value)
}

fn validate_builder_fee_charged(
    mut value: WireBuilderFeeCharged,
) -> Result<WireBuilderFeeCharged, PayloadCodecError> {
    value.account_id = required_api_address("BuilderFeeCharged", "account_id", value.account_id)?;
    value.builder_account_id = required_api_address(
        "BuilderFeeCharged",
        "builder_account_id",
        value.builder_account_id,
    )?;
    require_distinct_accounts(
        "BuilderFeeCharged",
        "account_id",
        &value.account_id,
        "builder_account_id",
        &value.builder_account_id,
    )?;
    value.asset_id = required_payload_field("BuilderFeeCharged", "asset_id", value.asset_id)?;
    value.amount = required_payload_field("BuilderFeeCharged", "amount", value.amount)?;
    Ok(value)
}

fn validate_funding_paid(mut value: WireFundingPaid) -> Result<WireFundingPaid, PayloadCodecError> {
    value.account_id = required_api_address("FundingPaid", "account_id", value.account_id)?;
    value.market_id = required_payload_field("FundingPaid", "market_id", value.market_id)?;
    value.amount = required_payload_field("FundingPaid", "amount", value.amount)?;
    value.funding_rate = required_payload_field("FundingPaid", "funding_rate", value.funding_rate)?;
    Ok(value)
}

fn validate_funding_received(
    mut value: WireFundingReceived,
) -> Result<WireFundingReceived, PayloadCodecError> {
    value.account_id = required_api_address("FundingReceived", "account_id", value.account_id)?;
    value.market_id = required_payload_field("FundingReceived", "market_id", value.market_id)?;
    value.amount = required_payload_field("FundingReceived", "amount", value.amount)?;
    value.funding_rate =
        required_payload_field("FundingReceived", "funding_rate", value.funding_rate)?;
    Ok(value)
}

fn validate_referral_reward(
    mut value: WireReferralReward,
) -> Result<WireReferralReward, PayloadCodecError> {
    value.account_id = required_api_address("ReferralReward", "account_id", value.account_id)?;
    value.referrer_account_id = required_api_address(
        "ReferralReward",
        "referrer_account_id",
        value.referrer_account_id,
    )?;
    require_distinct_accounts(
        "ReferralReward",
        "account_id",
        &value.account_id,
        "referrer_account_id",
        &value.referrer_account_id,
    )?;
    value.asset_id = required_payload_field("ReferralReward", "asset_id", value.asset_id)?;
    value.amount = required_payload_field("ReferralReward", "amount", value.amount)?;
    Ok(value)
}

fn validate_account_mode_changed(
    mut value: WireAccountModeChanged,
) -> Result<WireAccountModeChanged, PayloadCodecError> {
    value.account_id = required_api_address("AccountModeChanged", "account_id", value.account_id)?;
    value.previous_mode = required_wire_value(
        "AccountModeChanged",
        "previous_mode",
        value.previous_mode,
        &["standard", "unified", "portfolio", "dex_abstraction"],
    )?;
    value.new_mode = required_wire_value(
        "AccountModeChanged",
        "new_mode",
        value.new_mode,
        &["standard", "unified", "portfolio", "dex_abstraction"],
    )?;
    require_changed(
        "AccountModeChanged",
        "previous_mode",
        &value.previous_mode,
        "new_mode",
        &value.new_mode,
    )?;
    Ok(value)
}

fn validate_margin_mode_changed(
    mut value: WireMarginModeChanged,
) -> Result<WireMarginModeChanged, PayloadCodecError> {
    value.account_id = required_api_address("MarginModeChanged", "account_id", value.account_id)?;
    value.market_id = required_payload_field("MarginModeChanged", "market_id", value.market_id)?;
    value.previous_mode = required_wire_value(
        "MarginModeChanged",
        "previous_mode",
        value.previous_mode,
        &["cross", "isolated", "strict_isolated"],
    )?;
    value.new_mode = required_wire_value(
        "MarginModeChanged",
        "new_mode",
        value.new_mode,
        &["cross", "isolated", "strict_isolated"],
    )?;
    require_changed(
        "MarginModeChanged",
        "previous_mode",
        &value.previous_mode,
        "new_mode",
        &value.new_mode,
    )?;
    Ok(value)
}

fn validate_leverage_changed(
    mut value: WireLeverageChanged,
) -> Result<WireLeverageChanged, PayloadCodecError> {
    value.account_id = required_api_address("LeverageChanged", "account_id", value.account_id)?;
    value.market_id = required_payload_field("LeverageChanged", "market_id", value.market_id)?;
    value.previous_leverage = required_payload_field(
        "LeverageChanged",
        "previous_leverage",
        value.previous_leverage,
    )?;
    value.new_leverage =
        required_payload_field("LeverageChanged", "new_leverage", value.new_leverage)?;
    require_changed(
        "LeverageChanged",
        "previous_leverage",
        &value.previous_leverage,
        "new_leverage",
        &value.new_leverage,
    )?;
    Ok(value)
}

fn validate_liquidation_started(
    mut value: WireLiquidationStarted,
) -> Result<WireLiquidationStarted, PayloadCodecError> {
    const KIND: &str = "LiquidationStarted";
    value.account_id = required_api_address(KIND, "account_id", value.account_id)?;
    value.liquidation_id = required_payload_field(KIND, "liquidation_id", value.liquidation_id)?;
    value.margin_value = required_payload_field(KIND, "margin_value", value.margin_value)?;
    value.maintenance_requirement = required_payload_field(
        KIND,
        "maintenance_requirement",
        value.maintenance_requirement,
    )?;
    let (margin_raw, margin_scale) = parse_wire_decimal(KIND, "margin_value", &value.margin_value)?;
    let (maintenance_raw, maintenance_scale) = parse_wire_decimal(
        KIND,
        "maintenance_requirement",
        &value.maintenance_requirement,
    )?;
    if margin_raw < 0 || maintenance_raw < 0 {
        return Err(PayloadCodecError::Invalid {
            kind: KIND.to_owned(),
            reason: "margin_value and maintenance_requirement must be nonnegative".to_owned(),
        });
    }
    if margin_scale != maintenance_scale {
        return Err(PayloadCodecError::Invalid {
            kind: KIND.to_owned(),
            reason: "margin_value and maintenance_requirement must use the same scale".to_owned(),
        });
    }
    if margin_raw >= maintenance_raw {
        return Err(PayloadCodecError::Invalid {
            kind: KIND.to_owned(),
            reason: "margin_value must be less than maintenance_requirement".to_owned(),
        });
    }
    Ok(value)
}

fn validate_liquidation_fill(
    mut value: WireLiquidationFill,
) -> Result<WireLiquidationFill, PayloadCodecError> {
    const KIND: &str = "LiquidationFill";
    value.liquidation_id = required_payload_field(KIND, "liquidation_id", value.liquidation_id)?;
    value.account_id = required_api_address(KIND, "account_id", value.account_id)?;
    value.market_id = required_payload_field(KIND, "market_id", value.market_id)?;
    value.price = required_payload_field(KIND, "price", value.price)?;
    value.quantity = required_payload_field(KIND, "quantity", value.quantity)?;
    require_positive_wire_decimal(KIND, "price", &value.price)?;
    require_positive_wire_decimal(KIND, "quantity", &value.quantity)?;
    Ok(value)
}

fn validate_backstop_liquidation(
    mut value: WireBackstopLiquidation,
) -> Result<WireBackstopLiquidation, PayloadCodecError> {
    const KIND: &str = "BackstopLiquidation";
    value.liquidation_id = required_payload_field(KIND, "liquidation_id", value.liquidation_id)?;
    value.account_id = required_api_address(KIND, "account_id", value.account_id)?;
    value.backstop_account_id =
        required_api_address(KIND, "backstop_account_id", value.backstop_account_id)?;
    require_distinct_accounts(
        KIND,
        "account_id",
        &value.account_id,
        "backstop_account_id",
        &value.backstop_account_id,
    )?;
    value.market_id = required_payload_field(KIND, "market_id", value.market_id)?;
    value.quantity = required_payload_field(KIND, "quantity", value.quantity)?;
    require_positive_wire_decimal(KIND, "quantity", &value.quantity)?;
    Ok(value)
}

fn validate_position_settled(
    mut value: WirePositionSettled,
) -> Result<WirePositionSettled, PayloadCodecError> {
    const KIND: &str = "PositionSettled";
    value.account_id = required_api_address(KIND, "account_id", value.account_id)?;
    value.market_id = required_payload_field(KIND, "market_id", value.market_id)?;
    value.settlement_price =
        required_payload_field(KIND, "settlement_price", value.settlement_price)?;
    value.settled_quantity =
        required_payload_field(KIND, "settled_quantity", value.settled_quantity)?;
    value.realized_pnl = required_payload_field(KIND, "realized_pnl", value.realized_pnl)?;
    require_nonnegative_wire_decimal(KIND, "settlement_price", &value.settlement_price)?;
    require_positive_wire_decimal(KIND, "settled_quantity", &value.settled_quantity)?;
    parse_wire_decimal(KIND, "realized_pnl", &value.realized_pnl)?;
    Ok(value)
}

fn validate_dex_created(mut value: WireDexCreated) -> Result<WireDexCreated, PayloadCodecError> {
    value.dex_id = required_payload_field("DexCreated", "dex_id", value.dex_id)?;
    value.name = required_bounded_text("DexCreated", "name", value.name, 256)?;
    value.operator_account_id = required_payload_field(
        "DexCreated",
        "operator_account_id",
        value.operator_account_id,
    )?;
    Ok(value)
}

fn validate_asset_context_updated(
    mut value: WireAssetContextUpdated,
) -> Result<WireAssetContextUpdated, PayloadCodecError> {
    value.asset_id = required_payload_field("AssetContextUpdated", "asset_id", value.asset_id)?;
    value.context_version = required_bounded_text(
        "AssetContextUpdated",
        "context_version",
        value.context_version,
        128,
    )?;
    validate_hash_bytes("AssetContextUpdated", "context_hash", &value.context_hash)?;
    Ok(value)
}

fn validate_market_created(
    mut value: WireMarketCreated,
) -> Result<WireMarketCreated, PayloadCodecError> {
    value.market_id = required_payload_field("MarketCreated", "market_id", value.market_id)?;
    value.dex_id = required_payload_field("MarketCreated", "dex_id", value.dex_id)?;
    value.base_asset_id =
        required_payload_field("MarketCreated", "base_asset_id", value.base_asset_id)?;
    value.quote_asset_id =
        required_payload_field("MarketCreated", "quote_asset_id", value.quote_asset_id)?;
    value.tick_size = required_payload_field("MarketCreated", "tick_size", value.tick_size)?;
    value.lot_size = required_payload_field("MarketCreated", "lot_size", value.lot_size)?;
    Ok(value)
}

fn validate_market_metadata_changed(
    mut value: WireMarketMetadataChanged,
) -> Result<WireMarketMetadataChanged, PayloadCodecError> {
    value.market_id =
        required_payload_field("MarketMetadataChanged", "market_id", value.market_id)?;
    value.metadata_version = required_bounded_text(
        "MarketMetadataChanged",
        "metadata_version",
        value.metadata_version,
        128,
    )?;
    validate_hash_bytes(
        "MarketMetadataChanged",
        "metadata_hash",
        &value.metadata_hash,
    )?;
    Ok(value)
}

fn validate_market_halted(
    mut value: WireMarketHalted,
) -> Result<WireMarketHalted, PayloadCodecError> {
    value.market_id = required_payload_field("MarketHalted", "market_id", value.market_id)?;
    value.reason = required_bounded_text("MarketHalted", "reason", value.reason, 1_024)?;
    Ok(value)
}

fn validate_market_resumed(
    mut value: WireMarketResumed,
) -> Result<WireMarketResumed, PayloadCodecError> {
    value.market_id = required_payload_field("MarketResumed", "market_id", value.market_id)?;
    value.reason = required_bounded_text("MarketResumed", "reason", value.reason, 1_024)?;
    Ok(value)
}

fn validate_open_interest_cap_changed(
    mut value: WireOpenInterestCapChanged,
) -> Result<WireOpenInterestCapChanged, PayloadCodecError> {
    value.market_id =
        required_payload_field("OpenInterestCapChanged", "market_id", value.market_id)?;
    value.previous_cap =
        required_payload_field("OpenInterestCapChanged", "previous_cap", value.previous_cap)?;
    value.new_cap = required_payload_field("OpenInterestCapChanged", "new_cap", value.new_cap)?;
    if value.previous_cap == value.new_cap {
        return Err(PayloadCodecError::Invalid {
            kind: "OpenInterestCapChanged".to_owned(),
            reason: "previous_cap and new_cap must differ".to_owned(),
        });
    }
    Ok(value)
}

fn validate_margin_table_changed(
    mut value: WireMarginTableChanged,
) -> Result<WireMarginTableChanged, PayloadCodecError> {
    value.market_id = required_payload_field("MarginTableChanged", "market_id", value.market_id)?;
    value.previous_table_hash = required_bounded_text(
        "MarginTableChanged",
        "previous_table_hash",
        value.previous_table_hash,
        256,
    )?;
    value.new_table_hash = required_bounded_text(
        "MarginTableChanged",
        "new_table_hash",
        value.new_table_hash,
        256,
    )?;
    if value.previous_table_hash == value.new_table_hash {
        return Err(PayloadCodecError::Invalid {
            kind: "MarginTableChanged".to_owned(),
            reason: "previous_table_hash and new_table_hash must differ".to_owned(),
        });
    }
    Ok(value)
}

fn validate_oracle_updated(
    mut value: WireOracleUpdated,
) -> Result<WireOracleUpdated, PayloadCodecError> {
    value.market_id = required_payload_field("OracleUpdated", "market_id", value.market_id)?;
    value.oracle_price =
        required_payload_field("OracleUpdated", "oracle_price", value.oracle_price)?;
    value.source = required_bounded_text("OracleUpdated", "source", value.source, 256)?;
    validate_nonnegative_time(
        "OracleUpdated",
        "effective_at_micros",
        value.effective_at_micros,
    )?;
    Ok(value)
}

fn validate_funding_rate_updated(
    mut value: WireFundingRateUpdated,
) -> Result<WireFundingRateUpdated, PayloadCodecError> {
    value.market_id = required_payload_field("FundingRateUpdated", "market_id", value.market_id)?;
    value.funding_rate =
        required_payload_field("FundingRateUpdated", "funding_rate", value.funding_rate)?;
    validate_nonnegative_time(
        "FundingRateUpdated",
        "effective_at_micros",
        value.effective_at_micros,
    )?;
    Ok(value)
}

fn validate_outcome_created(
    mut value: WireOutcomeCreated,
) -> Result<WireOutcomeCreated, PayloadCodecError> {
    value.market_id = required_payload_field("OutcomeCreated", "market_id", value.market_id)?;
    value.outcome_id = required_payload_field("OutcomeCreated", "outcome_id", value.outcome_id)?;
    value.description =
        required_bounded_text("OutcomeCreated", "description", value.description, 2_048)?;
    Ok(value)
}

fn validate_outcome_resolved(
    mut value: WireOutcomeResolved,
) -> Result<WireOutcomeResolved, PayloadCodecError> {
    value.market_id = required_payload_field("OutcomeResolved", "market_id", value.market_id)?;
    value.outcome_id = required_payload_field("OutcomeResolved", "outcome_id", value.outcome_id)?;
    value.settlement_value = required_payload_field(
        "OutcomeResolved",
        "settlement_value",
        value.settlement_value,
    )?;
    validate_nonnegative_time(
        "OutcomeResolved",
        "resolved_at_micros",
        value.resolved_at_micros,
    )?;
    Ok(value)
}

fn validate_nonnegative_time(kind: &str, field: &str, value: i64) -> Result<(), PayloadCodecError> {
    if value < 0 {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be nonnegative"),
        });
    }
    Ok(())
}

fn validate_hash_bytes(kind: &str, field: &str, value: &[u8]) -> Result<(), PayloadCodecError> {
    if value.len() != 32 {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must contain exactly 32 bytes"),
        });
    }
    Ok(())
}

fn required_payload_field(
    kind: &str,
    field: &str,
    value: String,
) -> Result<String, PayloadCodecError> {
    if value.is_empty() || value.trim() != value {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be non-empty without surrounding whitespace"),
        });
    }
    Ok(value)
}

fn required_bounded_text(
    kind: &str,
    field: &str,
    value: String,
    max_bytes: usize,
) -> Result<String, PayloadCodecError> {
    let value = required_payload_field(kind, field, value)?;
    if value.len() > max_bytes || value.chars().any(char::is_control) {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be control-free and at most {max_bytes} bytes"),
        });
    }
    Ok(value)
}

fn required_reference(kind: &str, field: &str, value: String) -> Result<String, PayloadCodecError> {
    let value = required_payload_field(kind, field, value)?;
    if value.len() > 256 || value.bytes().any(|byte| byte.is_ascii_control()) {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must have 1..=256 bytes and no ASCII controls"),
        });
    }
    Ok(value)
}

fn required_api_address(
    kind: &str,
    field: &str,
    value: String,
) -> Result<String, PayloadCodecError> {
    let value = required_payload_field(kind, field, value)?;
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be a lowercase API address"),
        });
    };
    if hex.len() != 40
        || !hex
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be a lowercase API address"),
        });
    }
    Ok(value)
}

fn require_distinct_endpoints(
    kind: &str,
    from_account_id: &str,
    to_account_id: &str,
) -> Result<(), PayloadCodecError> {
    if from_account_id == to_account_id {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: "from_account_id and to_account_id must differ".to_owned(),
        });
    }
    Ok(())
}

fn require_distinct_accounts(
    kind: &str,
    left_field: &str,
    left: &str,
    right_field: &str,
    right: &str,
) -> Result<(), PayloadCodecError> {
    if left == right {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{left_field} and {right_field} must differ"),
        });
    }
    Ok(())
}

fn require_changed(
    kind: &str,
    previous_field: &str,
    previous: &str,
    new_field: &str,
    new: &str,
) -> Result<(), PayloadCodecError> {
    if previous == new {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{previous_field} and {new_field} must differ"),
        });
    }
    Ok(())
}

fn required_wire_value(
    kind: &str,
    field: &str,
    value: String,
    allowed: &[&str],
) -> Result<String, PayloadCodecError> {
    let value = required_payload_field(kind, field, value)?;
    if !allowed.contains(&value.as_str()) {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} has an unknown wire value"),
        });
    }
    Ok(value)
}

fn require_positive_wire_decimal(
    kind: &str,
    field: &str,
    value: &str,
) -> Result<(), PayloadCodecError> {
    let (raw, _) = parse_wire_decimal(kind, field, value)?;
    if raw <= 0 {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be positive"),
        });
    }
    Ok(())
}

fn require_nonnegative_wire_decimal(
    kind: &str,
    field: &str,
    value: &str,
) -> Result<(), PayloadCodecError> {
    let (raw, _) = parse_wire_decimal(kind, field, value)?;
    if raw < 0 {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be nonnegative"),
        });
    }
    Ok(())
}

fn parse_wire_decimal(
    kind: &str,
    field: &str,
    value: &str,
) -> Result<(i128, u8), PayloadCodecError> {
    const MAX_DECIMAL_SCALE: usize = 38;
    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, value),
    };
    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next().unwrap_or_default();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || (!fraction.is_empty() && !fraction.bytes().all(|byte| byte.is_ascii_digit()))
        || value.ends_with('.')
    {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be a canonical decimal string"),
        });
    }
    if fraction.len() > MAX_DECIMAL_SCALE {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} exceeds the frozen 38-digit decimal scale"),
        });
    }

    let magnitude = whole
        .parse::<u128>()
        .ok()
        .and_then(|whole| {
            let factor = 10_u128.checked_pow(u32::try_from(fraction.len()).ok()?)?;
            let fractional = if fraction.is_empty() {
                Some(0)
            } else {
                fraction.parse::<u128>().ok()
            }?;
            whole
                .checked_mul(factor)
                .and_then(|scaled| scaled.checked_add(fractional))
        })
        .ok_or_else(|| PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} is outside the fixed-point range"),
        })?;
    let positive_limit = i128::MAX as u128;
    let negative_limit = positive_limit + 1;
    let raw = if negative {
        if magnitude > negative_limit {
            return Err(PayloadCodecError::Invalid {
                kind: kind.to_owned(),
                reason: format!("{field} is outside the fixed-point range"),
            });
        }
        if magnitude == negative_limit {
            i128::MIN
        } else {
            -(magnitude as i128)
        }
    } else {
        if magnitude > positive_limit {
            return Err(PayloadCodecError::Invalid {
                kind: kind.to_owned(),
                reason: format!("{field} is outside the fixed-point range"),
            });
        }
        magnitude as i128
    };
    Ok((raw, u8::try_from(fraction.len()).expect("scale is bounded")))
}

fn decimal_wire_sign(kind: &str, field: &str, value: &str) -> Result<i8, PayloadCodecError> {
    let (negative, unsigned) = match value.strip_prefix('-') {
        Some(unsigned) => (true, unsigned),
        None => (false, value),
    };
    let mut parts = unsigned.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    if parts.next().is_some()
        || whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || fraction.is_some_and(|digits| {
            digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: format!("{field} must be a canonical decimal string"),
        });
    }
    let nonzero = whole
        .bytes()
        .chain(fraction.unwrap_or_default().bytes())
        .any(|byte| byte != b'0');
    Ok(if !nonzero {
        0
    } else if negative {
        -1
    } else {
        1
    })
}

fn decode_optional_identity(
    field: &str,
    value: String,
) -> Result<Option<String>, PayloadCodecError> {
    if value.is_empty() {
        Ok(None)
    } else if value.trim() != value {
        Err(PayloadCodecError::Invalid {
            kind: "TradeMatched".to_owned(),
            reason: format!("{field} must not contain surrounding whitespace"),
        })
    } else {
        Ok(Some(value))
    }
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

fn bounded_account_payload(kind: &str, message: Vec<u8>) -> Result<Vec<u8>, PayloadCodecError> {
    let payload = wrap_payload(kind, message);
    validate_account_payload_size(kind, &payload)?;
    Ok(payload)
}

fn bounded_trade_payload(kind: &str, message: Vec<u8>) -> Result<Vec<u8>, PayloadCodecError> {
    let payload = wrap_payload(kind, message);
    validate_trade_payload_size(kind, &payload)?;
    Ok(payload)
}

fn validate_account_payload_size(kind: &str, bytes: &[u8]) -> Result<(), PayloadCodecError> {
    if bytes.len() > MAX_CANONICAL_ACCOUNT_PAYLOAD_BYTES {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: CANONICAL_ACCOUNT_PAYLOAD_SIZE_REASON.to_owned(),
        });
    }
    Ok(())
}

fn validate_trade_payload_size(kind: &str, bytes: &[u8]) -> Result<(), PayloadCodecError> {
    if bytes.len() > MAX_CANONICAL_TRADE_PAYLOAD_BYTES {
        return Err(PayloadCodecError::Invalid {
            kind: kind.to_owned(),
            reason: CANONICAL_TRADE_PAYLOAD_SIZE_REASON.to_owned(),
        });
    }
    Ok(())
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
