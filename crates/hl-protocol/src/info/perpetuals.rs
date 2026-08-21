use domain_types::{Address, Decimal, DexId, MarketId};
use serde_json::{Map, Value};

use super::decode::{
    DEPLOY_AUCTION_KNOWN_FIELDS, DeployAuction, InfoObservationKind, UserHistoryMeta, child,
    decimal_from_value, decimal_list, expect_capability, history_coverage, malformed,
    market_id_from_coin, optional_address, optional_bool, optional_decimal, optional_str,
    pair_entries, parse_family, require_array, require_array_field, require_decimal, require_i64,
    require_object, require_object_field, require_str, require_u64, string_list, u64_from_value,
};
use super::{InfoEnumField, InfoError, InfoParseContext, ParsedInfoResponse};

pub const USER_FUNDING_PAGE_LIMIT: usize = 2000;
pub const FUNDING_HISTORY_PAGE_LIMIT: usize = 2000;

pub const LEVERAGE_KIND_NAMES: &[&str] = &["cross", "isolated"];
pub const POSITION_MODE_NAMES: &[&str] = &["oneWay", "hedged"];
pub const FUNDING_DELTA_TYPES: &[&str] = &["funding"];

pub const CLEARINGHOUSE_ENUM_FIELDS: &[InfoEnumField] = &[
    InfoEnumField::new("/assetPositions/type", POSITION_MODE_NAMES),
    InfoEnumField::new(
        "/assetPositions/position/leverage/type",
        LEVERAGE_KIND_NAMES,
    ),
];
pub const FUNDING_ENUM_FIELDS: &[InfoEnumField] =
    &[InfoEnumField::new("/delta/type", FUNDING_DELTA_TYPES)];
pub const ACTIVE_ASSET_ENUM_FIELDS: &[InfoEnumField] =
    &[InfoEnumField::new("/leverage/type", LEVERAGE_KIND_NAMES)];

pub const PERP_DEX_KNOWN_FIELDS: &[&str] = &[
    "/name",
    "/fullName",
    "/deployer",
    "/oracleUpdater",
    "/feeRecipient",
    "/assetToStreamingOiCap",
    "/assetToFundingMultiplier",
];
pub const PERP_META_KNOWN_FIELDS: &[&str] = &[
    "/universe",
    "/universe/name",
    "/universe/szDecimals",
    "/universe/maxLeverage",
    "/universe/onlyIsolated",
    "/universe/isDelisted",
    "/universe/marginTableId",
    "/universe/marginMode",
    "/universe/growthMode",
    "/universe/lastGrowthModeChangeTime",
    "/marginTables",
    "/marginTables/description",
    "/marginTables/marginTiers",
    "/marginTables/marginTiers/lowerBound",
    "/marginTables/marginTiers/maxLeverage",
    "/collateralToken",
];
pub const CLEARINGHOUSE_KNOWN_FIELDS: &[&str] = &[
    "/marginSummary",
    "/marginSummary/accountValue",
    "/marginSummary/totalNtlPos",
    "/marginSummary/totalRawUsd",
    "/marginSummary/totalMarginUsed",
    "/crossMarginSummary",
    "/crossMarginSummary/accountValue",
    "/crossMarginSummary/totalNtlPos",
    "/crossMarginSummary/totalRawUsd",
    "/crossMarginSummary/totalMarginUsed",
    "/crossMaintenanceMarginUsed",
    "/withdrawable",
    "/assetPositions",
    "/assetPositions/type",
    "/assetPositions/position",
    "/assetPositions/position/coin",
    "/assetPositions/position/szi",
    "/assetPositions/position/leverage",
    "/assetPositions/position/leverage/type",
    "/assetPositions/position/leverage/value",
    "/assetPositions/position/leverage/rawUsd",
    "/assetPositions/position/entryPx",
    "/assetPositions/position/positionValue",
    "/assetPositions/position/unrealizedPnl",
    "/assetPositions/position/returnOnEquity",
    "/assetPositions/position/liquidationPx",
    "/assetPositions/position/marginUsed",
    "/assetPositions/position/maxLeverage",
    "/assetPositions/position/cumFunding",
    "/assetPositions/position/cumFunding/allTime",
    "/assetPositions/position/cumFunding/sinceOpen",
    "/assetPositions/position/cumFunding/sinceChange",
    "/time",
];
pub const FUNDING_UPDATE_KNOWN_FIELDS: &[&str] = &[
    "/time",
    "/hash",
    "/delta",
    "/delta/type",
    "/delta/coin",
    "/delta/usdc",
    "/delta/szi",
    "/delta/fundingRate",
    "/delta/nSamples",
    "/user",
];
pub const FUNDING_HISTORY_KNOWN_FIELDS: &[&str] = &["/coin", "/fundingRate", "/premium", "/time"];
pub const PREDICTED_FUNDING_KNOWN_FIELDS: &[&str] = &["/fundingRate", "/nextFundingTime"];
pub const ACTIVE_ASSET_KNOWN_FIELDS: &[&str] = &[
    "/user",
    "/coin",
    "/leverage",
    "/leverage/type",
    "/leverage/value",
    "/leverage/rawUsd",
    "/maxTradeSzs",
    "/availableToTrade",
    "/markPx",
];
pub const PERP_DEX_LIMITS_KNOWN_FIELDS: &[&str] = &[
    "/totalOiCap",
    "/oiSzCapPerPerp",
    "/maxTransferNtl",
    "/coinToOiCap",
];
pub const PERP_DEX_STATUS_KNOWN_FIELDS: &[&str] = &["/totalNetDeposit"];
pub const PERP_ANNOTATION_KNOWN_FIELDS: &[&str] = &["/category", "/description"];
pub const PERP_CONCISE_KNOWN_FIELDS: &[&str] = &["/category", "/keywords"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpDex {
    name: DexId,
    full_name: String,
    deployer: Address,
    oracle_updater: Option<Address>,
    fee_recipient: Option<Address>,
}

impl PerpDex {
    #[must_use]
    pub const fn name(&self) -> &DexId {
        &self.name
    }

    #[must_use]
    pub fn full_name(&self) -> &str {
        &self.full_name
    }

    #[must_use]
    pub const fn deployer(&self) -> Address {
        self.deployer
    }

    #[must_use]
    pub const fn oracle_updater(&self) -> Option<Address> {
        self.oracle_updater
    }

    #[must_use]
    pub const fn fee_recipient(&self) -> Option<Address> {
        self.fee_recipient
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpDexs {
    dexs: Vec<Option<PerpDex>>,
}

impl PerpDexs {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn dexs(&self) -> &[Option<PerpDex>] {
        &self.dexs
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpDexs {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perp_dexs"])?;
        let dexs = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                if value.is_null() {
                    return Ok(None);
                }
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                Ok(Some(PerpDex {
                    name: DexId::new(require_str(object, &path, "name")?)
                        .map_err(|_| malformed(&child(&path, "name"), "invalid dex id"))?,
                    full_name: require_str(object, &path, "fullName")?.to_owned(),
                    deployer: super::decode::require_address(object, &path, "deployer")?,
                    oracle_updater: optional_address(object, &path, "oracleUpdater")?,
                    fee_recipient: optional_address(object, &path, "feeRecipient")?,
                }))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { dexs })
    }
}

pub fn parse_perp_dexs(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpDexs), InfoError> {
    parse_family(
        "official.info.perp_dexs",
        raw,
        context,
        PERP_DEX_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpUniverseAsset {
    name: String,
    market_id: MarketId,
    sz_decimals: u64,
    max_leverage: u64,
    only_isolated: Option<bool>,
    is_delisted: Option<bool>,
    margin_table_id: Option<u64>,
    margin_mode: Option<String>,
}

impl PerpUniverseAsset {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn sz_decimals(&self) -> u64 {
        self.sz_decimals
    }

    #[must_use]
    pub const fn max_leverage(&self) -> u64 {
        self.max_leverage
    }

    #[must_use]
    pub const fn only_isolated(&self) -> Option<bool> {
        self.only_isolated
    }

    #[must_use]
    pub const fn is_delisted(&self) -> Option<bool> {
        self.is_delisted
    }

    #[must_use]
    pub const fn margin_table_id(&self) -> Option<u64> {
        self.margin_table_id
    }

    #[must_use]
    pub fn margin_mode(&self) -> Option<&str> {
        self.margin_mode.as_deref()
    }
}

fn parse_universe_asset(value: &Value, path: &str) -> Result<PerpUniverseAsset, InfoError> {
    let object = require_object(value, path)?;
    let name = require_str(object, path, "name")?.to_owned();
    Ok(PerpUniverseAsset {
        market_id: market_id_from_coin(&name)?,
        sz_decimals: require_u64(object, path, "szDecimals")?,
        max_leverage: require_u64(object, path, "maxLeverage")?,
        only_isolated: optional_bool(object, path, "onlyIsolated")?,
        is_delisted: optional_bool(object, path, "isDelisted")?,
        margin_table_id: super::decode::optional_u64(object, path, "marginTableId")?,
        margin_mode: optional_str(object, path, "marginMode")?.map(str::to_owned),
        name,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarginTier {
    lower_bound: Decimal,
    max_leverage: u64,
}

impl MarginTier {
    #[must_use]
    pub const fn lower_bound(&self) -> Decimal {
        self.lower_bound
    }

    #[must_use]
    pub const fn max_leverage(&self) -> u64 {
        self.max_leverage
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarginTable {
    id: u64,
    description: String,
    tiers: Vec<MarginTier>,
}

impl MarginTable {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }

    #[must_use]
    pub fn tiers(&self) -> &[MarginTier] {
        &self.tiers
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpMeta {
    universe: Vec<PerpUniverseAsset>,
    margin_tables: Vec<MarginTable>,
    collateral_token: Option<u64>,
}

impl PerpMeta {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn universe(&self) -> &[PerpUniverseAsset] {
        &self.universe
    }

    #[must_use]
    pub fn margin_tables(&self) -> &[MarginTable] {
        &self.margin_tables
    }

    #[must_use]
    pub const fn collateral_token(&self) -> Option<u64> {
        self.collateral_token
    }

    pub(crate) fn from_value(value: &Value, path: &str) -> Result<Self, InfoError> {
        let object = require_object(value, path)?;
        let universe_path = child(path, "universe");
        let universe = require_array_field(object, path, "universe")?
            .iter()
            .enumerate()
            .map(|(index, asset)| parse_universe_asset(asset, &format!("{universe_path}/{index}")))
            .collect::<Result<Vec<_>, _>>()?;
        let tables_path = child(path, "marginTables");
        let margin_tables = match object.get("marginTables") {
            None | Some(Value::Null) => Vec::new(),
            Some(value) => pair_entries(value, &tables_path)?
                .into_iter()
                .enumerate()
                .map(|(index, (id, body))| {
                    let item = format!("{tables_path}/{index}");
                    let body_object = require_object(body, &format!("{item}/1"))?;
                    let tiers_path = format!("{item}/1/marginTiers");
                    Ok(MarginTable {
                        id: u64_from_value(id, &format!("{item}/0"))?,
                        description: optional_str(
                            body_object,
                            &format!("{item}/1"),
                            "description",
                        )?
                        .unwrap_or("")
                        .to_owned(),
                        tiers: require_array_field(
                            body_object,
                            &format!("{item}/1"),
                            "marginTiers",
                        )?
                        .iter()
                        .enumerate()
                        .map(|(tier_index, tier)| {
                            let tier_path = format!("{tiers_path}/{tier_index}");
                            let tier_object = require_object(tier, &tier_path)?;
                            Ok(MarginTier {
                                lower_bound: require_decimal(
                                    tier_object,
                                    &tier_path,
                                    "lowerBound",
                                )?,
                                max_leverage: require_u64(tier_object, &tier_path, "maxLeverage")?,
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(Self {
            universe,
            margin_tables,
            collateral_token: super::decode::optional_u64(object, path, "collateralToken")?,
        })
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpMeta {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.meta"])?;
        Self::from_value(parsed.value(), "")
    }
}

pub fn parse_meta(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpMeta), InfoError> {
    parse_family(
        "official.info.meta",
        raw,
        context,
        PERP_META_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpAssetCtx {
    funding: Decimal,
    open_interest: Decimal,
    prev_day_px: Decimal,
    day_ntl_vlm: Decimal,
    premium: Option<Decimal>,
    oracle_px: Decimal,
    mark_px: Decimal,
    mid_px: Option<Decimal>,
    impact_pxs: Option<(Decimal, Decimal)>,
    day_base_vlm: Option<Decimal>,
}

impl PerpAssetCtx {
    #[must_use]
    pub const fn funding(&self) -> Decimal {
        self.funding
    }

    #[must_use]
    pub const fn open_interest(&self) -> Decimal {
        self.open_interest
    }

    #[must_use]
    pub const fn prev_day_px(&self) -> Decimal {
        self.prev_day_px
    }

    #[must_use]
    pub const fn day_ntl_vlm(&self) -> Decimal {
        self.day_ntl_vlm
    }

    #[must_use]
    pub const fn premium(&self) -> Option<Decimal> {
        self.premium
    }

    #[must_use]
    pub const fn oracle_px(&self) -> Decimal {
        self.oracle_px
    }

    #[must_use]
    pub const fn mark_px(&self) -> Decimal {
        self.mark_px
    }

    #[must_use]
    pub const fn mid_px(&self) -> Option<Decimal> {
        self.mid_px
    }

    #[must_use]
    pub const fn impact_pxs(&self) -> Option<(Decimal, Decimal)> {
        self.impact_pxs
    }

    #[must_use]
    pub const fn day_base_vlm(&self) -> Option<Decimal> {
        self.day_base_vlm
    }
}

fn parse_asset_ctx(value: &Value, path: &str) -> Result<PerpAssetCtx, InfoError> {
    let object = require_object(value, path)?;
    let impact = match optional_field_impact(object, path)? {
        None => None,
        Some(values) if values.len() == 2 => Some((values[0], values[1])),
        Some(_) => return Err(malformed(&child(path, "impactPxs"), "expected [bid, ask]")),
    };
    Ok(PerpAssetCtx {
        funding: require_decimal(object, path, "funding")?,
        open_interest: require_decimal(object, path, "openInterest")?,
        prev_day_px: require_decimal(object, path, "prevDayPx")?,
        day_ntl_vlm: require_decimal(object, path, "dayNtlVlm")?,
        premium: optional_decimal(object, path, "premium")?,
        oracle_px: require_decimal(object, path, "oraclePx")?,
        mark_px: require_decimal(object, path, "markPx")?,
        mid_px: optional_decimal(object, path, "midPx")?,
        impact_pxs: impact,
        day_base_vlm: optional_decimal(object, path, "dayBaseVlm")?,
    })
}

fn optional_field_impact(
    object: &Map<String, Value>,
    path: &str,
) -> Result<Option<Vec<Decimal>>, InfoError> {
    match object.get("impactPxs") {
        None | Some(Value::Null) => Ok(None),
        Some(value) => decimal_list(value, &child(path, "impactPxs")).map(Some),
    }
}

fn parse_ctxs(value: &Value, path: &str) -> Result<Vec<PerpAssetCtx>, InfoError> {
    require_array(value, path)?
        .iter()
        .enumerate()
        .map(|(index, ctx)| parse_asset_ctx(ctx, &format!("{path}/{index}")))
        .collect()
}

fn parse_meta_ctxs_pair(
    value: &Value,
    path: &str,
) -> Result<(PerpMeta, Vec<PerpAssetCtx>), InfoError> {
    let pair = require_array(value, path)?;
    if pair.len() != 2 {
        return Err(malformed(path, "expected [meta, ctxs]"));
    }
    Ok((
        PerpMeta::from_value(&pair[0], &format!("{path}/0"))?,
        parse_ctxs(&pair[1], &format!("{path}/1"))?,
    ))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetaAndAssetCtxs {
    meta: PerpMeta,
    ctxs: Vec<PerpAssetCtx>,
}

impl MetaAndAssetCtxs {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn meta(&self) -> &PerpMeta {
        &self.meta
    }

    #[must_use]
    pub fn ctxs(&self) -> &[PerpAssetCtx] {
        &self.ctxs
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for MetaAndAssetCtxs {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.meta_and_asset_ctxs"])?;
        let (meta, ctxs) = parse_meta_ctxs_pair(parsed.value(), "")?;
        Ok(Self { meta, ctxs })
    }
}

pub fn parse_meta_and_asset_ctxs(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, MetaAndAssetCtxs), InfoError> {
    parse_family(
        "official.info.meta_and_asset_ctxs",
        raw,
        context,
        PERP_META_AND_CTX_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllPerpMetas {
    dexs: Vec<MetaAndAssetCtxs>,
}

impl AllPerpMetas {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn dexs(&self) -> &[MetaAndAssetCtxs] {
        &self.dexs
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for AllPerpMetas {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.all_perp_metas"])?;
        let dexs = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let (meta, ctxs) = parse_meta_ctxs_pair(value, &format!("/{index}"))?;
                Ok(MetaAndAssetCtxs { meta, ctxs })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { dexs })
    }
}

pub fn parse_all_perp_metas(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, AllPerpMetas), InfoError> {
    parse_family(
        "official.info.all_perp_metas",
        raw,
        context,
        PERP_META_AND_CTX_KNOWN_FIELDS,
        &[],
    )
}

const PERP_META_AND_CTX_KNOWN_FIELDS: &[&str] = &[
    "/universe",
    "/universe/name",
    "/universe/szDecimals",
    "/universe/maxLeverage",
    "/universe/onlyIsolated",
    "/universe/isDelisted",
    "/universe/marginTableId",
    "/universe/marginMode",
    "/universe/growthMode",
    "/universe/lastGrowthModeChangeTime",
    "/marginTables",
    "/marginTables/description",
    "/marginTables/marginTiers",
    "/marginTables/marginTiers/lowerBound",
    "/marginTables/marginTiers/maxLeverage",
    "/collateralToken",
    "/funding",
    "/openInterest",
    "/prevDayPx",
    "/dayNtlVlm",
    "/premium",
    "/oraclePx",
    "/markPx",
    "/midPx",
    "/impactPxs",
    "/dayBaseVlm",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeverageKind {
    Cross,
    Isolated,
}

impl LeverageKind {
    fn from_wire(path: &str, value: &str) -> Result<Self, InfoError> {
        match value {
            "cross" => Ok(Self::Cross),
            "isolated" => Ok(Self::Isolated),
            other => Err(InfoError::UnknownStateAffectingVariant {
                path: path.to_owned(),
                value: other.to_owned(),
            }),
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Cross => "cross",
            Self::Isolated => "isolated",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Leverage {
    kind: LeverageKind,
    value: u64,
    raw_usd: Option<Decimal>,
}

impl Leverage {
    #[must_use]
    pub const fn kind(&self) -> LeverageKind {
        self.kind
    }

    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }

    #[must_use]
    pub const fn raw_usd(&self) -> Option<Decimal> {
        self.raw_usd
    }

    fn from_object(object: &Map<String, Value>, path: &str) -> Result<Self, InfoError> {
        Ok(Self {
            kind: LeverageKind::from_wire(
                &child(path, "type"),
                require_str(object, path, "type")?,
            )?,
            value: require_u64(object, path, "value")?,
            raw_usd: optional_decimal(object, path, "rawUsd")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CumFunding {
    all_time: Decimal,
    since_open: Decimal,
    since_change: Decimal,
}

impl CumFunding {
    #[must_use]
    pub const fn all_time(&self) -> Decimal {
        self.all_time
    }

    #[must_use]
    pub const fn since_open(&self) -> Decimal {
        self.since_open
    }

    #[must_use]
    pub const fn since_change(&self) -> Decimal {
        self.since_change
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssetPosition {
    mode: String,
    coin: String,
    market_id: MarketId,
    szi: Decimal,
    leverage: Leverage,
    entry_px: Decimal,
    position_value: Decimal,
    unrealized_pnl: Decimal,
    return_on_equity: Decimal,
    liquidation_px: Option<Decimal>,
    margin_used: Decimal,
    max_leverage: u64,
    cum_funding: CumFunding,
}

impl AssetPosition {
    #[must_use]
    pub fn mode(&self) -> &str {
        &self.mode
    }

    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn szi(&self) -> Decimal {
        self.szi
    }

    #[must_use]
    pub const fn leverage(&self) -> &Leverage {
        &self.leverage
    }

    #[must_use]
    pub const fn entry_px(&self) -> Decimal {
        self.entry_px
    }

    #[must_use]
    pub const fn position_value(&self) -> Decimal {
        self.position_value
    }

    #[must_use]
    pub const fn unrealized_pnl(&self) -> Decimal {
        self.unrealized_pnl
    }

    #[must_use]
    pub const fn return_on_equity(&self) -> Decimal {
        self.return_on_equity
    }

    #[must_use]
    pub const fn liquidation_px(&self) -> Option<Decimal> {
        self.liquidation_px
    }

    #[must_use]
    pub const fn margin_used(&self) -> Decimal {
        self.margin_used
    }

    #[must_use]
    pub const fn max_leverage(&self) -> u64 {
        self.max_leverage
    }

    #[must_use]
    pub const fn cum_funding(&self) -> &CumFunding {
        &self.cum_funding
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarginSummary {
    account_value: Decimal,
    total_ntl_pos: Decimal,
    total_raw_usd: Decimal,
    total_margin_used: Decimal,
}

impl MarginSummary {
    #[must_use]
    pub const fn account_value(&self) -> Decimal {
        self.account_value
    }

    #[must_use]
    pub const fn total_ntl_pos(&self) -> Decimal {
        self.total_ntl_pos
    }

    #[must_use]
    pub const fn total_raw_usd(&self) -> Decimal {
        self.total_raw_usd
    }

    #[must_use]
    pub const fn total_margin_used(&self) -> Decimal {
        self.total_margin_used
    }

    fn from_object(object: &Map<String, Value>, path: &str) -> Result<Self, InfoError> {
        Ok(Self {
            account_value: require_decimal(object, path, "accountValue")?,
            total_ntl_pos: require_decimal(object, path, "totalNtlPos")?,
            total_raw_usd: require_decimal(object, path, "totalRawUsd")?,
            total_margin_used: require_decimal(object, path, "totalMarginUsed")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClearinghouseState {
    margin_summary: MarginSummary,
    cross_margin_summary: Option<MarginSummary>,
    cross_maintenance_margin_used: Option<Decimal>,
    withdrawable: Option<Decimal>,
    asset_positions: Vec<AssetPosition>,
    time_millis: i64,
}

impl ClearinghouseState {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReconciledSnapshot
    }

    #[must_use]
    pub const fn margin_summary(&self) -> &MarginSummary {
        &self.margin_summary
    }

    #[must_use]
    pub const fn cross_margin_summary(&self) -> Option<&MarginSummary> {
        self.cross_margin_summary.as_ref()
    }

    #[must_use]
    pub const fn cross_maintenance_margin_used(&self) -> Option<Decimal> {
        self.cross_maintenance_margin_used
    }

    #[must_use]
    pub const fn withdrawable(&self) -> Option<Decimal> {
        self.withdrawable
    }

    #[must_use]
    pub fn asset_positions(&self) -> &[AssetPosition] {
        &self.asset_positions
    }

    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    pub(crate) fn from_value(value: &Value, path: &str) -> Result<Self, InfoError> {
        let object = require_object(value, path)?;
        let positions_path = child(path, "assetPositions");
        let asset_positions = require_array_field(object, path, "assetPositions")?
            .iter()
            .enumerate()
            .map(|(index, value)| parse_asset_position(value, &format!("{positions_path}/{index}")))
            .collect::<Result<Vec<_>, _>>()?;
        let cross = match object.get("crossMarginSummary") {
            None | Some(Value::Null) => None,
            Some(value) => {
                let cross_path = child(path, "crossMarginSummary");
                Some(MarginSummary::from_object(
                    require_object(value, &cross_path)?,
                    &cross_path,
                )?)
            }
        };
        Ok(Self {
            margin_summary: MarginSummary::from_object(
                require_object_field(object, path, "marginSummary")?,
                &child(path, "marginSummary"),
            )?,
            cross_margin_summary: cross,
            cross_maintenance_margin_used: optional_decimal(
                object,
                path,
                "crossMaintenanceMarginUsed",
            )?,
            withdrawable: optional_decimal(object, path, "withdrawable")?,
            asset_positions,
            time_millis: require_i64(object, path, "time")?,
        })
    }
}

fn parse_asset_position(value: &Value, path: &str) -> Result<AssetPosition, InfoError> {
    let object = require_object(value, path)?;
    let mode = require_str(object, path, "type")?.to_owned();
    if !POSITION_MODE_NAMES.contains(&mode.as_str()) {
        return Err(InfoError::UnknownStateAffectingVariant {
            path: child(path, "type"),
            value: mode,
        });
    }
    let position_path = child(path, "position");
    let position = require_object_field(object, path, "position")?;
    let coin = require_str(position, &position_path, "coin")?.to_owned();
    let funding_path = child(&position_path, "cumFunding");
    let funding = require_object_field(position, &position_path, "cumFunding")?;
    Ok(AssetPosition {
        mode,
        market_id: market_id_from_coin(&coin)?,
        szi: require_decimal(position, &position_path, "szi")?,
        leverage: Leverage::from_object(
            require_object_field(position, &position_path, "leverage")?,
            &child(&position_path, "leverage"),
        )?,
        entry_px: require_decimal(position, &position_path, "entryPx")?,
        position_value: require_decimal(position, &position_path, "positionValue")?,
        unrealized_pnl: require_decimal(position, &position_path, "unrealizedPnl")?,
        return_on_equity: require_decimal(position, &position_path, "returnOnEquity")?,
        liquidation_px: optional_decimal(position, &position_path, "liquidationPx")?,
        margin_used: require_decimal(position, &position_path, "marginUsed")?,
        max_leverage: require_u64(position, &position_path, "maxLeverage")?,
        cum_funding: CumFunding {
            all_time: require_decimal(funding, &funding_path, "allTime")?,
            since_open: require_decimal(funding, &funding_path, "sinceOpen")?,
            since_change: require_decimal(funding, &funding_path, "sinceChange")?,
        },
        coin,
    })
}

impl TryFrom<&ParsedInfoResponse<Value>> for ClearinghouseState {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.clearinghouse_state"])?;
        Self::from_value(parsed.value(), "")
    }
}

pub fn parse_clearinghouse_state(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, ClearinghouseState), InfoError> {
    parse_family(
        "official.info.clearinghouse_state",
        raw,
        context,
        CLEARINGHOUSE_KNOWN_FIELDS,
        CLEARINGHOUSE_ENUM_FIELDS,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundingUpdate {
    time_millis: i64,
    hash: String,
    coin: String,
    market_id: MarketId,
    usdc: Decimal,
    szi: Decimal,
    funding_rate: Decimal,
    n_samples: Option<u64>,
    user: Option<Address>,
}

impl FundingUpdate {
    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }

    #[must_use]
    pub fn hash(&self) -> &str {
        &self.hash
    }

    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn usdc(&self) -> Decimal {
        self.usdc
    }

    #[must_use]
    pub const fn szi(&self) -> Decimal {
        self.szi
    }

    #[must_use]
    pub const fn funding_rate(&self) -> Decimal {
        self.funding_rate
    }

    #[must_use]
    pub const fn n_samples(&self) -> Option<u64> {
        self.n_samples
    }

    #[must_use]
    pub const fn user(&self) -> Option<Address> {
        self.user
    }
}

fn parse_funding_update(value: &Value, path: &str) -> Result<FundingUpdate, InfoError> {
    let object = require_object(value, path)?;
    let delta_path = child(path, "delta");
    let delta = require_object_field(object, path, "delta")?;
    let delta_type = require_str(delta, &delta_path, "type")?;
    if !FUNDING_DELTA_TYPES.contains(&delta_type) {
        return Err(InfoError::UnknownStateAffectingVariant {
            path: child(&delta_path, "type"),
            value: delta_type.to_owned(),
        });
    }
    let coin = require_str(delta, &delta_path, "coin")?.to_owned();
    Ok(FundingUpdate {
        time_millis: require_i64(object, path, "time")?,
        hash: require_str(object, path, "hash")?.to_owned(),
        market_id: market_id_from_coin(&coin)?,
        usdc: require_decimal(delta, &delta_path, "usdc")?,
        szi: require_decimal(delta, &delta_path, "szi")?,
        funding_rate: require_decimal(delta, &delta_path, "fundingRate")?,
        n_samples: super::decode::optional_u64(delta, &delta_path, "nSamples")?,
        user: optional_address(object, path, "user")?,
        coin,
    })
}

fn parse_funding_updates(value: &Value) -> Result<Vec<FundingUpdate>, InfoError> {
    require_array(value, "")?
        .iter()
        .enumerate()
        .map(|(index, item)| parse_funding_update(item, &format!("/{index}")))
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UserFunding {
    updates: Vec<FundingUpdate>,
    history: UserHistoryMeta,
}

impl UserFunding {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn updates(&self) -> &[FundingUpdate] {
        &self.updates
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for UserFunding {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.user_funding"])?;
        let updates = parse_funding_updates(parsed.value())?;
        let earliest = updates.iter().map(|update| update.time_millis).min();
        Ok(Self {
            history: history_coverage(
                updates.len(),
                USER_FUNDING_PAGE_LIMIT,
                USER_FUNDING_PAGE_LIMIT,
                earliest,
            )?,
            updates,
        })
    }
}

pub fn parse_user_funding(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, UserFunding), InfoError> {
    parse_family(
        "official.info.user_funding",
        raw,
        context,
        FUNDING_UPDATE_KNOWN_FIELDS,
        FUNDING_ENUM_FIELDS,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NonUserFundingUpdates {
    updates: Vec<FundingUpdate>,
    history: UserHistoryMeta,
}

impl NonUserFundingUpdates {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn updates(&self) -> &[FundingUpdate] {
        &self.updates
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for NonUserFundingUpdates {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.non_user_funding_updates"])?;
        let updates = parse_funding_updates(parsed.value())?;
        let earliest = updates.iter().map(|update| update.time_millis).min();
        Ok(Self {
            history: history_coverage(
                updates.len(),
                USER_FUNDING_PAGE_LIMIT,
                USER_FUNDING_PAGE_LIMIT,
                earliest,
            )?,
            updates,
        })
    }
}

pub fn parse_non_user_funding_updates(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, NonUserFundingUpdates), InfoError> {
    parse_family(
        "official.info.non_user_funding_updates",
        raw,
        context,
        FUNDING_UPDATE_KNOWN_FIELDS,
        FUNDING_ENUM_FIELDS,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundingRateSample {
    coin: String,
    market_id: MarketId,
    funding_rate: Decimal,
    premium: Decimal,
    time_millis: i64,
}

impl FundingRateSample {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn funding_rate(&self) -> Decimal {
        self.funding_rate
    }

    #[must_use]
    pub const fn premium(&self) -> Decimal {
        self.premium
    }

    #[must_use]
    pub const fn time_millis(&self) -> i64 {
        self.time_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FundingHistory {
    samples: Vec<FundingRateSample>,
    history: UserHistoryMeta,
}

impl FundingHistory {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::BoundedHistory
    }

    #[must_use]
    pub fn samples(&self) -> &[FundingRateSample] {
        &self.samples
    }

    #[must_use]
    pub const fn history(&self) -> &UserHistoryMeta {
        &self.history
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for FundingHistory {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.funding_history"])?;
        let samples = require_array(parsed.value(), "")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/{index}");
                let object = require_object(value, &path)?;
                let coin = require_str(object, &path, "coin")?.to_owned();
                Ok(FundingRateSample {
                    market_id: market_id_from_coin(&coin)?,
                    funding_rate: require_decimal(object, &path, "fundingRate")?,
                    premium: require_decimal(object, &path, "premium")?,
                    time_millis: require_i64(object, &path, "time")?,
                    coin,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let earliest = samples.iter().map(|sample| sample.time_millis).min();
        Ok(Self {
            history: history_coverage(
                samples.len(),
                FUNDING_HISTORY_PAGE_LIMIT,
                FUNDING_HISTORY_PAGE_LIMIT,
                earliest,
            )?,
            samples,
        })
    }
}

pub fn parse_funding_history(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, FundingHistory), InfoError> {
    parse_family(
        "official.info.funding_history",
        raw,
        context,
        FUNDING_HISTORY_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredictedVenueFunding {
    venue: String,
    funding_rate: Decimal,
    next_funding_time_millis: i64,
}

impl PredictedVenueFunding {
    #[must_use]
    pub fn venue(&self) -> &str {
        &self.venue
    }

    #[must_use]
    pub const fn funding_rate(&self) -> Decimal {
        self.funding_rate
    }

    #[must_use]
    pub const fn next_funding_time_millis(&self) -> i64 {
        self.next_funding_time_millis
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredictedCoinFunding {
    coin: String,
    market_id: MarketId,
    venues: Vec<PredictedVenueFunding>,
}

impl PredictedCoinFunding {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub fn venues(&self) -> &[PredictedVenueFunding] {
        &self.venues
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PredictedFundings {
    coins: Vec<PredictedCoinFunding>,
}

impl PredictedFundings {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn coins(&self) -> &[PredictedCoinFunding] {
        &self.coins
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PredictedFundings {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.predicted_fundings"])?;
        let coins = pair_entries(parsed.value(), "")?
            .into_iter()
            .enumerate()
            .map(|(index, (coin, venues))| {
                let path = format!("/{index}");
                let coin = coin
                    .as_str()
                    .ok_or_else(|| malformed(&format!("{path}/0"), "expected coin"))?
                    .to_owned();
                let venues_path = format!("{path}/1");
                let venues = pair_entries(venues, &venues_path)?
                    .into_iter()
                    .enumerate()
                    .map(|(venue_index, (venue, body))| {
                        let item = format!("{venues_path}/{venue_index}");
                        let object = require_object(body, &format!("{item}/1"))?;
                        Ok(PredictedVenueFunding {
                            venue: venue
                                .as_str()
                                .ok_or_else(|| malformed(&format!("{item}/0"), "expected venue"))?
                                .to_owned(),
                            funding_rate: require_decimal(
                                object,
                                &format!("{item}/1"),
                                "fundingRate",
                            )?,
                            next_funding_time_millis: require_i64(
                                object,
                                &format!("{item}/1"),
                                "nextFundingTime",
                            )?,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(PredictedCoinFunding {
                    market_id: market_id_from_coin(&coin)?,
                    venues,
                    coin,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { coins })
    }
}

pub fn parse_predicted_fundings(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PredictedFundings), InfoError> {
    parse_family(
        "official.info.predicted_fundings",
        raw,
        context,
        PREDICTED_FUNDING_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpsAtOpenInterestCap {
    coins: Vec<(String, MarketId)>,
}

impl PerpsAtOpenInterestCap {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn coins(&self) -> &[(String, MarketId)] {
        &self.coins
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpsAtOpenInterestCap {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perps_at_open_interest_cap"])?;
        let coins = string_list(parsed.value(), "")?
            .into_iter()
            .map(|coin| {
                let market_id = market_id_from_coin(&coin)?;
                Ok((coin, market_id))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { coins })
    }
}

pub fn parse_perps_at_open_interest_cap(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpsAtOpenInterestCap), InfoError> {
    parse_family(
        "official.info.perps_at_open_interest_cap",
        raw,
        context,
        &[],
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpDeployAuctionStatus {
    auction: DeployAuction,
}

impl PerpDeployAuctionStatus {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn auction(&self) -> &DeployAuction {
        &self.auction
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpDeployAuctionStatus {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perp_deploy_auction_status"])?;
        Ok(Self {
            auction: DeployAuction::from_value(parsed.value(), "")?,
        })
    }
}

pub fn parse_perp_deploy_auction_status(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpDeployAuctionStatus), InfoError> {
    parse_family(
        "official.info.perp_deploy_auction_status",
        raw,
        context,
        DEPLOY_AUCTION_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveAssetData {
    user: Address,
    coin: String,
    market_id: MarketId,
    leverage: Leverage,
    max_trade_szs: Vec<Decimal>,
    available_to_trade: Vec<Decimal>,
    mark_px: Decimal,
}

impl ActiveAssetData {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReconciledSnapshot
    }

    #[must_use]
    pub const fn user(&self) -> Address {
        self.user
    }

    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn leverage(&self) -> &Leverage {
        &self.leverage
    }

    #[must_use]
    pub fn max_trade_szs(&self) -> &[Decimal] {
        &self.max_trade_szs
    }

    #[must_use]
    pub fn available_to_trade(&self) -> &[Decimal] {
        &self.available_to_trade
    }

    #[must_use]
    pub const fn mark_px(&self) -> Decimal {
        self.mark_px
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for ActiveAssetData {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.active_asset_data"])?;
        let object = require_object(parsed.value(), "")?;
        let coin = require_str(object, "", "coin")?.to_owned();
        Ok(Self {
            user: super::decode::require_address(object, "", "user")?,
            market_id: market_id_from_coin(&coin)?,
            leverage: Leverage::from_object(
                require_object_field(object, "", "leverage")?,
                "/leverage",
            )?,
            max_trade_szs: decimal_list(
                object
                    .get("maxTradeSzs")
                    .ok_or_else(|| malformed("/maxTradeSzs", "missing field"))?,
                "/maxTradeSzs",
            )?,
            available_to_trade: decimal_list(
                object
                    .get("availableToTrade")
                    .ok_or_else(|| malformed("/availableToTrade", "missing field"))?,
                "/availableToTrade",
            )?,
            mark_px: require_decimal(object, "", "markPx")?,
            coin,
        })
    }
}

pub fn parse_active_asset_data(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, ActiveAssetData), InfoError> {
    parse_family(
        "official.info.active_asset_data",
        raw,
        context,
        ACTIVE_ASSET_KNOWN_FIELDS,
        ACTIVE_ASSET_ENUM_FIELDS,
    )
}

fn coin_decimal_pairs(value: &Value, path: &str) -> Result<Vec<(MarketId, Decimal)>, InfoError> {
    pair_entries(value, path)?
        .into_iter()
        .enumerate()
        .map(|(index, (coin, amount))| {
            let item = format!("{path}/{index}");
            let coin = coin
                .as_str()
                .ok_or_else(|| malformed(&format!("{item}/0"), "expected coin"))?;
            Ok((
                market_id_from_coin(coin)?,
                decimal_from_value(amount, &format!("{item}/1"))?,
            ))
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpDexLimits {
    total_oi_cap: Decimal,
    oi_sz_cap_per_perp: Decimal,
    max_transfer_ntl: Decimal,
    coin_to_oi_cap: Vec<(MarketId, Decimal)>,
}

impl PerpDexLimits {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn total_oi_cap(&self) -> Decimal {
        self.total_oi_cap
    }

    #[must_use]
    pub const fn oi_sz_cap_per_perp(&self) -> Decimal {
        self.oi_sz_cap_per_perp
    }

    #[must_use]
    pub const fn max_transfer_ntl(&self) -> Decimal {
        self.max_transfer_ntl
    }

    #[must_use]
    pub fn coin_to_oi_cap(&self) -> &[(MarketId, Decimal)] {
        &self.coin_to_oi_cap
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpDexLimits {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perp_dex_limits"])?;
        let object = require_object(parsed.value(), "")?;
        Ok(Self {
            total_oi_cap: require_decimal(object, "", "totalOiCap")?,
            oi_sz_cap_per_perp: require_decimal(object, "", "oiSzCapPerPerp")?,
            max_transfer_ntl: require_decimal(object, "", "maxTransferNtl")?,
            coin_to_oi_cap: match object.get("coinToOiCap") {
                None | Some(Value::Null) => Vec::new(),
                Some(value) => coin_decimal_pairs(value, "/coinToOiCap")?,
            },
        })
    }
}

pub fn parse_perp_dex_limits(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpDexLimits), InfoError> {
    parse_family(
        "official.info.perp_dex_limits",
        raw,
        context,
        PERP_DEX_LIMITS_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpDexStatus {
    total_net_deposit: Decimal,
}

impl PerpDexStatus {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn total_net_deposit(&self) -> Decimal {
        self.total_net_deposit
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpDexStatus {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perp_dex_status"])?;
        let object = require_object(parsed.value(), "")?;
        Ok(Self {
            total_net_deposit: require_decimal(object, "", "totalNetDeposit")?,
        })
    }
}

pub fn parse_perp_dex_status(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpDexStatus), InfoError> {
    parse_family(
        "official.info.perp_dex_status",
        raw,
        context,
        PERP_DEX_STATUS_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpAnnotation {
    category: String,
    description: String,
}

impl PerpAnnotation {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }

    #[must_use]
    pub fn description(&self) -> &str {
        &self.description
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpAnnotation {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perp_annotation"])?;
        let object = require_object(parsed.value(), "")?;
        Ok(Self {
            category: require_str(object, "", "category")?.to_owned(),
            description: require_str(object, "", "description")?.to_owned(),
        })
    }
}

pub fn parse_perp_annotation(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpAnnotation), InfoError> {
    parse_family(
        "official.info.perp_annotation",
        raw,
        context,
        PERP_ANNOTATION_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpCategories {
    coins: Vec<(MarketId, String, String)>,
}

impl PerpCategories {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn coins(&self) -> &[(MarketId, String, String)] {
        &self.coins
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpCategories {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perp_categories"])?;
        let coins = pair_entries(parsed.value(), "")?
            .into_iter()
            .enumerate()
            .map(|(index, (coin, category))| {
                let path = format!("/{index}");
                let coin = coin
                    .as_str()
                    .ok_or_else(|| malformed(&format!("{path}/0"), "expected coin"))?;
                let category = category
                    .as_str()
                    .ok_or_else(|| malformed(&format!("{path}/1"), "expected category"))?;
                Ok((
                    market_id_from_coin(coin)?,
                    coin.to_owned(),
                    category.to_owned(),
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { coins })
    }
}

pub fn parse_perp_categories(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpCategories), InfoError> {
    parse_family("official.info.perp_categories", raw, context, &[], &[])
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpConciseAnnotation {
    coin: String,
    market_id: MarketId,
    category: String,
    keywords: Vec<String>,
}

impl PerpConciseAnnotation {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub fn category(&self) -> &str {
        &self.category
    }

    #[must_use]
    pub fn keywords(&self) -> &[String] {
        &self.keywords
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PerpConciseAnnotations {
    annotations: Vec<PerpConciseAnnotation>,
}

impl PerpConciseAnnotations {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn annotations(&self) -> &[PerpConciseAnnotation] {
        &self.annotations
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for PerpConciseAnnotations {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.perp_concise_annotations"])?;
        let annotations = pair_entries(parsed.value(), "")?
            .into_iter()
            .enumerate()
            .map(|(index, (coin, body))| {
                let path = format!("/{index}");
                let coin = coin
                    .as_str()
                    .ok_or_else(|| malformed(&format!("{path}/0"), "expected coin"))?
                    .to_owned();
                let object = require_object(body, &format!("{path}/1"))?;
                let keywords = match object.get("keywords") {
                    None | Some(Value::Null) => Vec::new(),
                    Some(value) => string_list(value, &format!("{path}/1/keywords"))?,
                };
                Ok(PerpConciseAnnotation {
                    market_id: market_id_from_coin(&coin)?,
                    category: require_str(object, &format!("{path}/1"), "category")?.to_owned(),
                    keywords,
                    coin,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { annotations })
    }
}

pub fn parse_perp_concise_annotations(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, PerpConciseAnnotations), InfoError> {
    parse_family(
        "official.info.perp_concise_annotations",
        raw,
        context,
        PERP_CONCISE_KNOWN_FIELDS,
        &[],
    )
}
