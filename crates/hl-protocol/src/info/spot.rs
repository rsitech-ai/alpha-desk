use domain_types::{Address, AssetId, Decimal, MarketId};
use serde_json::Value;

use super::decode::{
    DEPLOY_AUCTION_KNOWN_FIELDS, DeployAuction, InfoObservationKind, child, decimal_from_value,
    expect_capability, malformed, market_id_from_coin, optional_decimal, optional_str,
    parse_family, require_array, require_array_field, require_bool, require_decimal,
    require_object, require_object_field, require_str, require_u64,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const SPOT_TOKEN_KNOWN_FIELDS: &[&str] = &[
    "/tokens",
    "/tokens/name",
    "/tokens/szDecimals",
    "/tokens/weiDecimals",
    "/tokens/index",
    "/tokens/tokenId",
    "/tokens/isCanonical",
    "/tokens/evmContract",
    "/tokens/fullName",
    "/universe",
    "/universe/name",
    "/universe/tokens",
    "/universe/index",
    "/universe/isCanonical",
];
pub const SPOT_CLEARINGHOUSE_KNOWN_FIELDS: &[&str] = &[
    "/balances",
    "/balances/coin",
    "/balances/token",
    "/balances/total",
    "/balances/hold",
    "/balances/entryNtl",
];
pub const SPOT_DEPLOY_KNOWN_FIELDS: &[&str] = &[
    "/states",
    "/states/token",
    "/states/spec",
    "/states/spec/name",
    "/states/spec/szDecimals",
    "/states/spec/weiDecimals",
    "/states/fullName",
    "/states/spots",
    "/states/maxSupply",
    "/states/hyperliquidityGenesisBalance",
    "/states/totalGenesisBalanceWei",
    "/states/userGenesisBalances",
    "/states/existingTokenGenesisBalances",
    "/gasAuction",
    "/gasAuction/startTimeSeconds",
    "/gasAuction/durationSeconds",
    "/gasAuction/startGas",
    "/gasAuction/currentGas",
    "/gasAuction/endGas",
];
pub const TOKEN_DETAILS_KNOWN_FIELDS: &[&str] = &[
    "/name",
    "/maxSupply",
    "/totalSupply",
    "/circulatingSupply",
    "/szDecimals",
    "/weiDecimals",
    "/midPx",
    "/markPx",
    "/prevDayPx",
    "/genesis",
    "/genesis/userBalances",
    "/genesis/existingTokenBalances",
    "/deployer",
    "/deployGas",
    "/deployTime",
    "/seededUsdc",
    "/nonCirculatingUserBalances",
    "/futureEmissions",
];
const SPOT_META_AND_CTX_KNOWN_FIELDS: &[&str] = &[
    "/tokens",
    "/tokens/name",
    "/tokens/szDecimals",
    "/tokens/weiDecimals",
    "/tokens/index",
    "/tokens/tokenId",
    "/tokens/isCanonical",
    "/tokens/evmContract",
    "/tokens/fullName",
    "/universe",
    "/universe/name",
    "/universe/tokens",
    "/universe/index",
    "/universe/isCanonical",
    "/dayNtlVlm",
    "/markPx",
    "/midPx",
    "/prevDayPx",
    "/circulatingSupply",
    "/coin",
    "/dayBaseVlm",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotToken {
    name: String,
    sz_decimals: u64,
    wei_decimals: u64,
    index: u64,
    token_id: AssetId,
    is_canonical: bool,
    full_name: Option<String>,
}

impl SpotToken {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn sz_decimals(&self) -> u64 {
        self.sz_decimals
    }

    #[must_use]
    pub const fn wei_decimals(&self) -> u64 {
        self.wei_decimals
    }

    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    #[must_use]
    pub const fn token_id(&self) -> &AssetId {
        &self.token_id
    }

    #[must_use]
    pub const fn is_canonical(&self) -> bool {
        self.is_canonical
    }

    #[must_use]
    pub fn full_name(&self) -> Option<&str> {
        self.full_name.as_deref()
    }
}

fn parse_spot_token(value: &Value, path: &str) -> Result<SpotToken, InfoError> {
    let object = require_object(value, path)?;
    Ok(SpotToken {
        name: require_str(object, path, "name")?.to_owned(),
        sz_decimals: require_u64(object, path, "szDecimals")?,
        wei_decimals: require_u64(object, path, "weiDecimals")?,
        index: require_u64(object, path, "index")?,
        token_id: AssetId::new(require_str(object, path, "tokenId")?)
            .map_err(|_| malformed(&child(path, "tokenId"), "invalid token id"))?,
        is_canonical: require_bool(object, path, "isCanonical")?,
        full_name: optional_str(object, path, "fullName")?.map(str::to_owned),
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotPair {
    name: String,
    market_id: MarketId,
    index: u64,
    tokens: Vec<u64>,
    is_canonical: bool,
}

impl SpotPair {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn market_id(&self) -> &MarketId {
        &self.market_id
    }

    #[must_use]
    pub const fn index(&self) -> u64 {
        self.index
    }

    #[must_use]
    pub fn tokens(&self) -> &[u64] {
        &self.tokens
    }

    #[must_use]
    pub const fn is_canonical(&self) -> bool {
        self.is_canonical
    }
}

fn parse_spot_pair(value: &Value, path: &str) -> Result<SpotPair, InfoError> {
    let object = require_object(value, path)?;
    let name = require_str(object, path, "name")?.to_owned();
    let tokens_path = child(path, "tokens");
    let tokens = require_array_field(object, path, "tokens")?
        .iter()
        .enumerate()
        .map(|(index, token)| {
            super::decode::u64_from_value(token, &format!("{tokens_path}/{index}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(SpotPair {
        market_id: market_id_from_coin(&name)?,
        index: require_u64(object, path, "index")?,
        tokens,
        is_canonical: require_bool(object, path, "isCanonical")?,
        name,
    })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotMeta {
    tokens: Vec<SpotToken>,
    universe: Vec<SpotPair>,
}

impl SpotMeta {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn tokens(&self) -> &[SpotToken] {
        &self.tokens
    }

    #[must_use]
    pub fn universe(&self) -> &[SpotPair] {
        &self.universe
    }

    pub(crate) fn from_value(value: &Value, path: &str) -> Result<Self, InfoError> {
        let object = require_object(value, path)?;
        let tokens_path = child(path, "tokens");
        let universe_path = child(path, "universe");
        Ok(Self {
            tokens: require_array_field(object, path, "tokens")?
                .iter()
                .enumerate()
                .map(|(index, token)| parse_spot_token(token, &format!("{tokens_path}/{index}")))
                .collect::<Result<Vec<_>, _>>()?,
            universe: require_array_field(object, path, "universe")?
                .iter()
                .enumerate()
                .map(|(index, pair)| parse_spot_pair(pair, &format!("{universe_path}/{index}")))
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for SpotMeta {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.spot_meta"])?;
        Self::from_value(parsed.value(), "")
    }
}

pub fn parse_spot_meta(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, SpotMeta), InfoError> {
    parse_family(
        "official.info.spot_meta",
        raw,
        context,
        SPOT_TOKEN_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotAssetCtx {
    day_ntl_vlm: Decimal,
    mark_px: Decimal,
    mid_px: Option<Decimal>,
    prev_day_px: Decimal,
}

impl SpotAssetCtx {
    #[must_use]
    pub const fn day_ntl_vlm(&self) -> Decimal {
        self.day_ntl_vlm
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
    pub const fn prev_day_px(&self) -> Decimal {
        self.prev_day_px
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotMetaAndAssetCtxs {
    meta: SpotMeta,
    ctxs: Vec<SpotAssetCtx>,
}

impl SpotMetaAndAssetCtxs {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn meta(&self) -> &SpotMeta {
        &self.meta
    }

    #[must_use]
    pub fn ctxs(&self) -> &[SpotAssetCtx] {
        &self.ctxs
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for SpotMetaAndAssetCtxs {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.spot_meta_and_asset_ctxs"])?;
        let pair = require_array(parsed.value(), "")?;
        if pair.len() != 2 {
            return Err(malformed("", "expected [spotMeta, ctxs]"));
        }
        let ctxs = require_array(&pair[1], "/1")?
            .iter()
            .enumerate()
            .map(|(index, ctx)| {
                let path = format!("/1/{index}");
                let object = require_object(ctx, &path)?;
                Ok(SpotAssetCtx {
                    day_ntl_vlm: require_decimal(object, &path, "dayNtlVlm")?,
                    mark_px: require_decimal(object, &path, "markPx")?,
                    mid_px: optional_decimal(object, &path, "midPx")?,
                    prev_day_px: require_decimal(object, &path, "prevDayPx")?,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            meta: SpotMeta::from_value(&pair[0], "/0")?,
            ctxs,
        })
    }
}

pub fn parse_spot_meta_and_asset_ctxs(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, SpotMetaAndAssetCtxs), InfoError> {
    parse_family(
        "official.info.spot_meta_and_asset_ctxs",
        raw,
        context,
        SPOT_META_AND_CTX_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotBalance {
    coin: String,
    token: u64,
    total: Decimal,
    hold: Decimal,
    entry_ntl: Decimal,
}

impl SpotBalance {
    #[must_use]
    pub fn coin(&self) -> &str {
        &self.coin
    }

    #[must_use]
    pub const fn token(&self) -> u64 {
        self.token
    }

    #[must_use]
    pub const fn total(&self) -> Decimal {
        self.total
    }

    #[must_use]
    pub const fn hold(&self) -> Decimal {
        self.hold
    }

    #[must_use]
    pub const fn entry_ntl(&self) -> Decimal {
        self.entry_ntl
    }

    fn from_value(value: &Value, path: &str) -> Result<Self, InfoError> {
        let object = require_object(value, path)?;
        Ok(Self {
            coin: require_str(object, path, "coin")?.to_owned(),
            token: require_u64(object, path, "token")?,
            total: require_decimal(object, path, "total")?,
            hold: require_decimal(object, path, "hold")?,
            entry_ntl: require_decimal(object, path, "entryNtl")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotClearinghouseState {
    balances: Vec<SpotBalance>,
}

impl SpotClearinghouseState {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReconciledSnapshot
    }

    #[must_use]
    pub fn balances(&self) -> &[SpotBalance] {
        &self.balances
    }

    pub(crate) fn from_value(value: &Value, path: &str) -> Result<Self, InfoError> {
        let object = require_object(value, path)?;
        let balances_path = child(path, "balances");
        Ok(Self {
            balances: require_array_field(object, path, "balances")?
                .iter()
                .enumerate()
                .map(|(index, balance)| {
                    SpotBalance::from_value(balance, &format!("{balances_path}/{index}"))
                })
                .collect::<Result<Vec<_>, _>>()?,
        })
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for SpotClearinghouseState {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.spot_clearinghouse_state"])?;
        Self::from_value(parsed.value(), "")
    }
}

pub fn parse_spot_clearinghouse_state(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, SpotClearinghouseState), InfoError> {
    parse_family(
        "official.info.spot_clearinghouse_state",
        raw,
        context,
        SPOT_CLEARINGHOUSE_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotDeploySpec {
    name: String,
    sz_decimals: u64,
    wei_decimals: u64,
}

impl SpotDeploySpec {
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn sz_decimals(&self) -> u64 {
        self.sz_decimals
    }

    #[must_use]
    pub const fn wei_decimals(&self) -> u64 {
        self.wei_decimals
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotDeployTokenState {
    token: u64,
    spec: SpotDeploySpec,
    full_name: Option<String>,
    spots: Vec<u64>,
    max_supply: Option<Decimal>,
}

impl SpotDeployTokenState {
    #[must_use]
    pub const fn token(&self) -> u64 {
        self.token
    }

    #[must_use]
    pub const fn spec(&self) -> &SpotDeploySpec {
        &self.spec
    }

    #[must_use]
    pub fn full_name(&self) -> Option<&str> {
        self.full_name.as_deref()
    }

    #[must_use]
    pub fn spots(&self) -> &[u64] {
        &self.spots
    }

    #[must_use]
    pub const fn max_supply(&self) -> Option<Decimal> {
        self.max_supply
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotDeployState {
    states: Vec<SpotDeployTokenState>,
    gas_auction: DeployAuction,
}

impl SpotDeployState {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn states(&self) -> &[SpotDeployTokenState] {
        &self.states
    }

    #[must_use]
    pub const fn gas_auction(&self) -> &DeployAuction {
        &self.gas_auction
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for SpotDeployState {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.spot_deploy_state"])?;
        let object = require_object(parsed.value(), "")?;
        let states = require_array_field(object, "", "states")?
            .iter()
            .enumerate()
            .map(|(index, value)| {
                let path = format!("/states/{index}");
                let object = require_object(value, &path)?;
                let spec_path = child(&path, "spec");
                let spec = require_object_field(object, &path, "spec")?;
                let spots_path = child(&path, "spots");
                Ok(SpotDeployTokenState {
                    token: require_u64(object, &path, "token")?,
                    spec: SpotDeploySpec {
                        name: require_str(spec, &spec_path, "name")?.to_owned(),
                        sz_decimals: require_u64(spec, &spec_path, "szDecimals")?,
                        wei_decimals: require_u64(spec, &spec_path, "weiDecimals")?,
                    },
                    full_name: optional_str(object, &path, "fullName")?.map(str::to_owned),
                    spots: match object.get("spots") {
                        None | Some(Value::Null) => Vec::new(),
                        Some(value) => require_array(value, &spots_path)?
                            .iter()
                            .enumerate()
                            .map(|(spot_index, spot)| {
                                super::decode::u64_from_value(
                                    spot,
                                    &format!("{spots_path}/{spot_index}"),
                                )
                            })
                            .collect::<Result<Vec<_>, _>>()?,
                    },
                    max_supply: match object.get("maxSupply") {
                        None | Some(Value::Null) => None,
                        Some(Value::Number(number)) => number
                            .as_u64()
                            .map(|value| Decimal::from_raw(i128::from(value), 0))
                            .transpose()
                            .map_err(|_| malformed(&child(&path, "maxSupply"), "invalid supply"))?,
                        Some(value) => Some(decimal_from_value(value, &child(&path, "maxSupply"))?),
                    },
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            states,
            gas_auction: DeployAuction::from_value(
                object
                    .get("gasAuction")
                    .ok_or_else(|| malformed("/gasAuction", "missing field"))?,
                "/gasAuction",
            )?,
        })
    }
}

pub fn parse_spot_deploy_state(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, SpotDeployState), InfoError> {
    parse_family(
        "official.info.spot_deploy_state",
        raw,
        context,
        SPOT_DEPLOY_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpotPairDeployAuctionStatus {
    auction: DeployAuction,
}

impl SpotPairDeployAuctionStatus {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn auction(&self) -> &DeployAuction {
        &self.auction
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for SpotPairDeployAuctionStatus {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.spot_pair_deploy_auction_status"])?;
        Ok(Self {
            auction: DeployAuction::from_value(parsed.value(), "")?,
        })
    }
}

pub fn parse_spot_pair_deploy_auction_status(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, SpotPairDeployAuctionStatus), InfoError> {
    parse_family(
        "official.info.spot_pair_deploy_auction_status",
        raw,
        context,
        DEPLOY_AUCTION_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenDetails {
    name: String,
    token_id: Option<AssetId>,
    max_supply: Decimal,
    total_supply: Decimal,
    circulating_supply: Decimal,
    sz_decimals: u64,
    wei_decimals: u64,
    mid_px: Decimal,
    mark_px: Decimal,
    prev_day_px: Decimal,
    deployer: Address,
    deploy_gas: Option<Decimal>,
    deploy_time: Option<String>,
    seeded_usdc: Decimal,
    future_emissions: Decimal,
}

impl TokenDetails {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::DirectLookup
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn token_id(&self) -> Option<&AssetId> {
        self.token_id.as_ref()
    }

    #[must_use]
    pub const fn max_supply(&self) -> Decimal {
        self.max_supply
    }

    #[must_use]
    pub const fn total_supply(&self) -> Decimal {
        self.total_supply
    }

    #[must_use]
    pub const fn circulating_supply(&self) -> Decimal {
        self.circulating_supply
    }

    #[must_use]
    pub const fn sz_decimals(&self) -> u64 {
        self.sz_decimals
    }

    #[must_use]
    pub const fn wei_decimals(&self) -> u64 {
        self.wei_decimals
    }

    #[must_use]
    pub const fn mid_px(&self) -> Decimal {
        self.mid_px
    }

    #[must_use]
    pub const fn mark_px(&self) -> Decimal {
        self.mark_px
    }

    #[must_use]
    pub const fn prev_day_px(&self) -> Decimal {
        self.prev_day_px
    }

    #[must_use]
    pub const fn deployer(&self) -> Address {
        self.deployer
    }

    #[must_use]
    pub const fn deploy_gas(&self) -> Option<Decimal> {
        self.deploy_gas
    }

    #[must_use]
    pub fn deploy_time(&self) -> Option<&str> {
        self.deploy_time.as_deref()
    }

    #[must_use]
    pub const fn seeded_usdc(&self) -> Decimal {
        self.seeded_usdc
    }

    #[must_use]
    pub const fn future_emissions(&self) -> Decimal {
        self.future_emissions
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for TokenDetails {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.token_details"])?;
        let object = require_object(parsed.value(), "")?;
        Ok(Self {
            name: require_str(object, "", "name")?.to_owned(),
            token_id: optional_str(object, "", "tokenId")?
                .map(AssetId::new)
                .transpose()
                .map_err(|_| malformed("/tokenId", "invalid token id"))?,
            max_supply: require_decimal(object, "", "maxSupply")?,
            total_supply: require_decimal(object, "", "totalSupply")?,
            circulating_supply: require_decimal(object, "", "circulatingSupply")?,
            sz_decimals: require_u64(object, "", "szDecimals")?,
            wei_decimals: require_u64(object, "", "weiDecimals")?,
            mid_px: require_decimal(object, "", "midPx")?,
            mark_px: require_decimal(object, "", "markPx")?,
            prev_day_px: require_decimal(object, "", "prevDayPx")?,
            deployer: super::decode::require_address(object, "", "deployer")?,
            deploy_gas: optional_decimal(object, "", "deployGas")?,
            deploy_time: optional_str(object, "", "deployTime")?.map(str::to_owned),
            seeded_usdc: require_decimal(object, "", "seededUsdc")?,
            future_emissions: require_decimal(object, "", "futureEmissions")?,
        })
    }
}

pub fn parse_token_details(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, TokenDetails), InfoError> {
    parse_family(
        "official.info.token_details",
        raw,
        context,
        TOKEN_DETAILS_KNOWN_FIELDS,
        &[],
    )
}
