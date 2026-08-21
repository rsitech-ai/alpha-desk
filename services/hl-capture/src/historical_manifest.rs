//! Official historical S3 dataset layout, format selection, and limitations.

use domain_types::{BlockHeight, KnownTime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum HistoricalError {
    #[error("historical S3 requester-pays is required")]
    RequesterPaysRequired,
    #[error("historical S3 dataset is unknown")]
    UnknownDataset,
    #[error("historical S3 format is unknown")]
    UnknownFormat,
    #[error("historical S3 dataset format is incompatible")]
    FormatMismatch,
    #[error("historical S3 bucket is not an official historical bucket")]
    UnknownBucket,
    #[error("historical S3 HyperEVM datasets are owned by T25")]
    HyperEvmDeferred,
    #[error("historical S3 key range is invalid")]
    InvalidRange,
    #[error("historical S3 object etag or content hash mismatch")]
    HashMismatch,
    #[error("historical S3 object conflicts with a prior import")]
    Conflict,
    #[error("historical S3 archive failed")]
    Archive,
    #[error("historical S3 progress failed")]
    Progress,
    #[error("historical S3 checkpoint failed")]
    Checkpoint,
    #[error("historical S3 object store failed")]
    Store,
    #[error("injected historical S3 fault")]
    InjectedFault(HistoricalFaultPoint),
}

impl HistoricalError {
    #[must_use]
    pub const fn reason_code(self) -> &'static str {
        match self {
            Self::RequesterPaysRequired => "historical_s3.requester_pays_required",
            Self::UnknownDataset => "historical_s3.unknown_dataset",
            Self::UnknownFormat => "historical_s3.unknown_format",
            Self::FormatMismatch => "historical_s3.format_mismatch",
            Self::UnknownBucket => "historical_s3.unknown_bucket",
            Self::HyperEvmDeferred => "historical_s3.hyperevm_deferred",
            Self::InvalidRange => "historical_s3.invalid_range",
            Self::HashMismatch => "historical_s3.hash_mismatch",
            Self::Conflict => "historical_s3.conflict",
            Self::Archive => "historical_s3.archive",
            Self::Progress => "historical_s3.progress",
            Self::Checkpoint => "historical_s3.checkpoint",
            Self::Store => "historical_s3.store",
            Self::InjectedFault(_) => "historical_s3.injected_fault",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoricalFaultPoint {
    AfterArchive,
}

pub const PARSER_BUILD: &str = "hl-capture-historical-s3-v1";
pub const DATASET_VERSION: &str = "official-s3-v1";
pub const MARKET_BUCKET: &str = "hyperliquid-archive";
pub const NODE_MAINNET_BUCKET: &str = "hl-mainnet-node-data";
pub const NODE_TESTNET_BUCKET: &str = "hl-testnet-node-data";

const MARKET_LIMITATIONS: &str = "Official hyperliquid-archive uploads approximately once a month. There is no guarantee of timely updates and data may be missing. Only L2 book snapshots and asset contexts are published. Candles and spot asset data are not on S3.";
const NODE_LIMITATIONS: &str = "Official hl-mainnet-node-data prefixes are requester-pays. Older fills and trades live under node_fills and node_trades. Current fills are node_fills_by_block.";
const HYPEREVM_LIMITATIONS: &str =
    "HyperEVM buckets reuse this object-manifest infrastructure in T25, not T16.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetFormat {
    Current,
    Legacy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatasetKind {
    L2Snapshots,
    AssetContexts,
    NodeFillsByBlock,
    NodeFillsLegacy,
    NodeTradesLegacy,
    ExplorerBlocks,
    ReplicaCmds,
}

impl DatasetFormat {
    pub fn parse(value: &str) -> Result<Self, HistoricalError> {
        match value {
            "current" => Ok(Self::Current),
            "legacy" => Ok(Self::Legacy),
            _ => Err(HistoricalError::UnknownFormat),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Legacy => "legacy",
        }
    }
}

impl DatasetKind {
    pub fn parse(identifier: &str) -> Result<Self, HistoricalError> {
        match identifier {
            "l2-snapshots" => Ok(Self::L2Snapshots),
            "asset-contexts" => Ok(Self::AssetContexts),
            "node_fills_by_block" => Ok(Self::NodeFillsByBlock),
            "node-fills-legacy" => Ok(Self::NodeFillsLegacy),
            "node-trades-legacy" => Ok(Self::NodeTradesLegacy),
            "explorer-blocks" => Ok(Self::ExplorerBlocks),
            "replica_cmds" => Ok(Self::ReplicaCmds),
            "hyperevm-blocks" | "hyperevm-receipts" => Err(HistoricalError::HyperEvmDeferred),
            _ => Err(HistoricalError::UnknownDataset),
        }
    }

    #[must_use]
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::L2Snapshots => "l2-snapshots",
            Self::AssetContexts => "asset-contexts",
            Self::NodeFillsByBlock => "node_fills_by_block",
            Self::NodeFillsLegacy => "node-fills-legacy",
            Self::NodeTradesLegacy => "node-trades-legacy",
            Self::ExplorerBlocks => "explorer-blocks",
            Self::ReplicaCmds => "replica_cmds",
        }
    }

    #[must_use]
    pub const fn required_format(self) -> DatasetFormat {
        match self {
            Self::NodeFillsLegacy | Self::NodeTradesLegacy => DatasetFormat::Legacy,
            Self::L2Snapshots
            | Self::AssetContexts
            | Self::NodeFillsByBlock
            | Self::ExplorerBlocks
            | Self::ReplicaCmds => DatasetFormat::Current,
        }
    }

    pub fn validate_format(self, format: DatasetFormat) -> Result<(), HistoricalError> {
        if self.required_format() == format {
            Ok(())
        } else {
            Err(HistoricalError::FormatMismatch)
        }
    }

    #[must_use]
    pub const fn bucket_mainnet(self) -> &'static str {
        match self {
            Self::L2Snapshots | Self::AssetContexts => MARKET_BUCKET,
            Self::NodeFillsByBlock
            | Self::NodeFillsLegacy
            | Self::NodeTradesLegacy
            | Self::ExplorerBlocks
            | Self::ReplicaCmds => NODE_MAINNET_BUCKET,
        }
    }

    pub fn accept_bucket(self, bucket: &str) -> Result<(), HistoricalError> {
        match self {
            Self::L2Snapshots | Self::AssetContexts if bucket == MARKET_BUCKET => Ok(()),
            Self::NodeFillsByBlock
            | Self::NodeFillsLegacy
            | Self::NodeTradesLegacy
            | Self::ExplorerBlocks
            | Self::ReplicaCmds
                if bucket == NODE_MAINNET_BUCKET || bucket == NODE_TESTNET_BUCKET =>
            {
                Ok(())
            }
            _ => Err(HistoricalError::UnknownBucket),
        }
    }

    #[must_use]
    pub const fn key_prefix(self) -> &'static str {
        match self {
            Self::L2Snapshots => "market_data/",
            Self::AssetContexts => "asset_ctxs/",
            Self::NodeFillsByBlock => "node_fills_by_block/",
            Self::NodeFillsLegacy => "node_fills/",
            Self::NodeTradesLegacy => "node_trades/",
            Self::ExplorerBlocks => "explorer_blocks/",
            Self::ReplicaCmds => "replica_cmds/",
        }
    }

    #[must_use]
    pub const fn official_limitations(self) -> &'static str {
        match self {
            Self::L2Snapshots | Self::AssetContexts => MARKET_LIMITATIONS,
            Self::NodeFillsByBlock
            | Self::NodeFillsLegacy
            | Self::NodeTradesLegacy
            | Self::ExplorerBlocks
            | Self::ReplicaCmds => NODE_LIMITATIONS,
        }
    }
}

#[must_use]
pub const fn hyperevm_limitations() -> &'static str {
    HYPEREVM_LIMITATIONS
}

#[must_use]
pub fn select_fills_dataset(format: DatasetFormat) -> DatasetKind {
    match format {
        DatasetFormat::Current => DatasetKind::NodeFillsByBlock,
        DatasetFormat::Legacy => DatasetKind::NodeFillsLegacy,
    }
}

pub fn select_trades_dataset(format: DatasetFormat) -> Result<DatasetKind, HistoricalError> {
    match format {
        DatasetFormat::Legacy => Ok(DatasetKind::NodeTradesLegacy),
        DatasetFormat::Current => Err(HistoricalError::FormatMismatch),
    }
}

#[must_use]
pub fn l2_object_key(date: &str, hour: u8, coin: &str) -> String {
    format!("market_data/{date}/{hour}/l2Book/{coin}.lz4")
}

#[must_use]
pub fn asset_ctx_key(date: &str) -> String {
    format!("asset_ctxs/{date}.csv.lz4")
}

#[must_use]
pub fn node_object_key(kind: DatasetKind, date: &str, name: &str) -> String {
    format!("{}{date}/{name}", kind.key_prefix())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnumerationSpec {
    L2Book {
        date: String,
        hour: u8,
        coins: Vec<String>,
    },
    AssetCtxs {
        dates: Vec<String>,
    },
    NodeObjects {
        kind: DatasetKind,
        date: String,
        names: Vec<String>,
    },
}

pub fn enumerate_keys(spec: &EnumerationSpec) -> Result<Vec<String>, HistoricalError> {
    match spec {
        EnumerationSpec::L2Book { date, hour, coins } => {
            validate_date(date)?;
            if coins.is_empty() {
                return Err(HistoricalError::InvalidRange);
            }
            Ok(coins
                .iter()
                .map(|coin| l2_object_key(date, *hour, coin))
                .collect())
        }
        EnumerationSpec::AssetCtxs { dates } => {
            if dates.is_empty() {
                return Err(HistoricalError::InvalidRange);
            }
            dates.iter().try_fold(Vec::new(), |mut keys, date| {
                validate_date(date)?;
                keys.push(asset_ctx_key(date));
                Ok(keys)
            })
        }
        EnumerationSpec::NodeObjects { kind, date, names } => match kind {
            DatasetKind::L2Snapshots | DatasetKind::AssetContexts => {
                Err(HistoricalError::UnknownDataset)
            }
            DatasetKind::NodeFillsByBlock
            | DatasetKind::NodeFillsLegacy
            | DatasetKind::NodeTradesLegacy
            | DatasetKind::ExplorerBlocks
            | DatasetKind::ReplicaCmds => {
                validate_date(date)?;
                if names.is_empty() {
                    return Err(HistoricalError::InvalidRange);
                }
                Ok(names
                    .iter()
                    .map(|name| node_object_key(*kind, date, name))
                    .collect())
            }
        },
    }
}

pub fn keys_in_range<'a>(
    keys: &'a [String],
    start_key: &str,
    end_key: &str,
) -> Result<Vec<&'a str>, HistoricalError> {
    if start_key > end_key {
        return Err(HistoricalError::InvalidRange);
    }
    Ok(keys
        .iter()
        .map(String::as_str)
        .filter(|key| *key >= start_key && *key <= end_key)
        .collect())
}

pub fn coverage_event_time(date: &str, hour: Option<u8>) -> Result<KnownTime, HistoricalError> {
    validate_date(date)?;
    let year: i32 = date[0..4]
        .parse()
        .map_err(|_| HistoricalError::InvalidRange)?;
    let month: u32 = date[4..6]
        .parse()
        .map_err(|_| HistoricalError::InvalidRange)?;
    let day: u32 = date[6..8]
        .parse()
        .map_err(|_| HistoricalError::InvalidRange)?;
    let hour = u32::from(hour.unwrap_or(0));
    if hour > 23 {
        return Err(HistoricalError::InvalidRange);
    }
    let micros = civil_unix_micros(year, month, day, hour).ok_or(HistoricalError::InvalidRange)?;
    KnownTime::from_unix_micros(micros).map_err(|_| HistoricalError::InvalidRange)
}

pub fn coverage_block(name: &str) -> Option<BlockHeight> {
    name.parse::<u64>().ok().map(BlockHeight::new)
}

fn validate_date(date: &str) -> Result<(), HistoricalError> {
    if date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit()) {
        Ok(())
    } else {
        Err(HistoricalError::InvalidRange)
    }
}

fn civil_unix_micros(year: i32, month: u32, day: u32, hour: u32) -> Option<i64> {
    if !(1..=12).contains(&month) || day == 0 || day > 31 {
        return None;
    }
    let year = if month <= 2 { year - 1 } else { year };
    let era = i64::from(year.div_euclid(400));
    let yoe = u32::try_from(year.rem_euclid(400)).ok()?;
    let month_prime = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = yoe * 365 + yoe / 4 - yoe / 100 + day_of_year;
    let days = era
        .checked_mul(146_097)?
        .checked_add(i64::from(day_of_era))?
        .checked_sub(719_468)?;
    days.checked_mul(86_400)?
        .checked_add(i64::from(hour) * 3_600)?
        .checked_mul(1_000_000)
}
