use domain_types::Decimal;
use serde_json::Value;

use super::decode::{
    InfoObservationKind, expect_capability, malformed, optional_decimal, pair_entries,
    parse_family, require_bool, require_decimal, require_i64, require_object, require_object_field,
    require_str,
};
use super::{InfoError, InfoParseContext, ParsedInfoResponse};

pub const BORROW_LEND_HEALTH_NAMES: &[&str] = &["healthy", "warning", "liquidation"];

pub const ALIGNED_QUOTE_KNOWN_FIELDS: &[&str] = &[
    "/isAligned",
    "/firstAlignedTime",
    "/evmMintedSupply",
    "/dailyAmountOwed",
    "/predictedRate",
];
pub const BORROW_LEND_USER_KNOWN_FIELDS: &[&str] = &[
    "/tokenToState",
    "/tokenToState/borrow",
    "/tokenToState/borrow/basis",
    "/tokenToState/borrow/value",
    "/tokenToState/supply",
    "/tokenToState/supply/basis",
    "/tokenToState/supply/value",
    "/health",
    "/healthFactor",
];
pub const BORROW_LEND_RESERVE_KNOWN_FIELDS: &[&str] = &[
    "/borrowYearlyRate",
    "/supplyYearlyRate",
    "/balance",
    "/utilization",
    "/oraclePx",
    "/ltv",
    "/totalSupplied",
    "/totalBorrowed",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AlignedQuoteTokenInfo {
    is_aligned: bool,
    first_aligned_time_millis: i64,
    evm_minted_supply: Decimal,
    daily_amount_owed: Vec<(String, Decimal)>,
    predicted_rate: Decimal,
}

impl AlignedQuoteTokenInfo {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn is_aligned(&self) -> bool {
        self.is_aligned
    }

    #[must_use]
    pub const fn first_aligned_time_millis(&self) -> i64 {
        self.first_aligned_time_millis
    }

    #[must_use]
    pub const fn evm_minted_supply(&self) -> Decimal {
        self.evm_minted_supply
    }

    #[must_use]
    pub fn daily_amount_owed(&self) -> &[(String, Decimal)] {
        &self.daily_amount_owed
    }

    #[must_use]
    pub const fn predicted_rate(&self) -> Decimal {
        self.predicted_rate
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for Option<AlignedQuoteTokenInfo> {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.aligned_quote_token_info"])?;
        if parsed.value().is_null() {
            return Ok(None);
        }
        let object = require_object(parsed.value(), "")?;
        let owed = pair_entries(
            object
                .get("dailyAmountOwed")
                .ok_or_else(|| malformed("/dailyAmountOwed", "missing field"))?,
            "/dailyAmountOwed",
        )?
        .into_iter()
        .enumerate()
        .map(|(index, (date, amount))| {
            let path = format!("/dailyAmountOwed/{index}");
            Ok((
                date.as_str()
                    .ok_or_else(|| malformed(&format!("{path}/0"), "expected date"))?
                    .to_owned(),
                super::decode::decimal_from_value(amount, &format!("{path}/1"))?,
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Some(AlignedQuoteTokenInfo {
            is_aligned: require_bool(object, "", "isAligned")?,
            first_aligned_time_millis: require_i64(object, "", "firstAlignedTime")?,
            evm_minted_supply: require_decimal(object, "", "evmMintedSupply")?,
            daily_amount_owed: owed,
            predicted_rate: require_decimal(object, "", "predictedRate")?,
        }))
    }
}

pub fn parse_aligned_quote_token_info(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, Option<AlignedQuoteTokenInfo>), InfoError> {
    parse_family(
        "official.info.aligned_quote_token_info",
        raw,
        context,
        ALIGNED_QUOTE_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowLendSide {
    basis: Decimal,
    value: Decimal,
}

impl BorrowLendSide {
    #[must_use]
    pub const fn basis(&self) -> Decimal {
        self.basis
    }

    #[must_use]
    pub const fn value(&self) -> Decimal {
        self.value
    }

    fn from_object(object: &serde_json::Map<String, Value>, path: &str) -> Result<Self, InfoError> {
        Ok(Self {
            basis: require_decimal(object, path, "basis")?,
            value: require_decimal(object, path, "value")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowLendTokenState {
    token: u64,
    borrow: BorrowLendSide,
    supply: BorrowLendSide,
}

impl BorrowLendTokenState {
    #[must_use]
    pub const fn token(&self) -> u64 {
        self.token
    }

    #[must_use]
    pub const fn borrow(&self) -> &BorrowLendSide {
        &self.borrow
    }

    #[must_use]
    pub const fn supply(&self) -> &BorrowLendSide {
        &self.supply
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowLendUserState {
    token_to_state: Vec<BorrowLendTokenState>,
    health: String,
    health_factor: Option<Decimal>,
}

impl BorrowLendUserState {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReconciledSnapshot
    }

    #[must_use]
    pub fn token_to_state(&self) -> &[BorrowLendTokenState] {
        &self.token_to_state
    }

    #[must_use]
    pub fn health(&self) -> &str {
        &self.health
    }

    #[must_use]
    pub const fn health_factor(&self) -> Option<Decimal> {
        self.health_factor
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for BorrowLendUserState {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.borrow_lend_user_state"])?;
        let object = require_object(parsed.value(), "")?;
        let health = require_str(object, "", "health")?.to_owned();
        if !BORROW_LEND_HEALTH_NAMES.contains(&health.as_str()) {
            return Err(InfoError::UnknownStateAffectingVariant {
                path: "/health".to_owned(),
                value: health,
            });
        }
        let token_to_state = pair_entries(
            object
                .get("tokenToState")
                .ok_or_else(|| malformed("/tokenToState", "missing field"))?,
            "/tokenToState",
        )?
        .into_iter()
        .enumerate()
        .map(|(index, (token, body))| {
            let path = format!("/tokenToState/{index}");
            let body_object = require_object(body, &format!("{path}/1"))?;
            Ok(BorrowLendTokenState {
                token: super::decode::u64_from_value(token, &format!("{path}/0"))?,
                borrow: BorrowLendSide::from_object(
                    require_object_field(body_object, &format!("{path}/1"), "borrow")?,
                    &format!("{path}/1/borrow"),
                )?,
                supply: BorrowLendSide::from_object(
                    require_object_field(body_object, &format!("{path}/1"), "supply")?,
                    &format!("{path}/1/supply"),
                )?,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
        Ok(Self {
            token_to_state,
            health,
            health_factor: optional_decimal(object, "", "healthFactor")?,
        })
    }
}

pub fn parse_borrow_lend_user_state(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, BorrowLendUserState), InfoError> {
    parse_family(
        "official.info.borrow_lend_user_state",
        raw,
        context,
        BORROW_LEND_USER_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BorrowLendReserveState {
    token: Option<u64>,
    borrow_yearly_rate: Decimal,
    supply_yearly_rate: Decimal,
    balance: Decimal,
    utilization: Decimal,
    oracle_px: Decimal,
    ltv: Decimal,
    total_supplied: Decimal,
    total_borrowed: Decimal,
}

impl BorrowLendReserveState {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub const fn token(&self) -> Option<u64> {
        self.token
    }

    #[must_use]
    pub const fn borrow_yearly_rate(&self) -> Decimal {
        self.borrow_yearly_rate
    }

    #[must_use]
    pub const fn supply_yearly_rate(&self) -> Decimal {
        self.supply_yearly_rate
    }

    #[must_use]
    pub const fn balance(&self) -> Decimal {
        self.balance
    }

    #[must_use]
    pub const fn utilization(&self) -> Decimal {
        self.utilization
    }

    #[must_use]
    pub const fn oracle_px(&self) -> Decimal {
        self.oracle_px
    }

    #[must_use]
    pub const fn ltv(&self) -> Decimal {
        self.ltv
    }

    #[must_use]
    pub const fn total_supplied(&self) -> Decimal {
        self.total_supplied
    }

    #[must_use]
    pub const fn total_borrowed(&self) -> Decimal {
        self.total_borrowed
    }

    fn from_object(
        object: &serde_json::Map<String, Value>,
        path: &str,
        token: Option<u64>,
    ) -> Result<Self, InfoError> {
        Ok(Self {
            token,
            borrow_yearly_rate: require_decimal(object, path, "borrowYearlyRate")?,
            supply_yearly_rate: require_decimal(object, path, "supplyYearlyRate")?,
            balance: require_decimal(object, path, "balance")?,
            utilization: require_decimal(object, path, "utilization")?,
            oracle_px: require_decimal(object, path, "oraclePx")?,
            ltv: require_decimal(object, path, "ltv")?,
            total_supplied: require_decimal(object, path, "totalSupplied")?,
            total_borrowed: require_decimal(object, path, "totalBorrowed")?,
        })
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for BorrowLendReserveState {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.borrow_lend_reserve_state"])?;
        let object = require_object(parsed.value(), "")?;
        Self::from_object(object, "", None)
    }
}

pub fn parse_borrow_lend_reserve_state(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, BorrowLendReserveState), InfoError> {
    parse_family(
        "official.info.borrow_lend_reserve_state",
        raw,
        context,
        BORROW_LEND_RESERVE_KNOWN_FIELDS,
        &[],
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllBorrowLendReserveStates {
    reserves: Vec<BorrowLendReserveState>,
}

impl AllBorrowLendReserveStates {
    #[must_use]
    pub const fn kind(&self) -> InfoObservationKind {
        InfoObservationKind::ReferenceSnapshot
    }

    #[must_use]
    pub fn reserves(&self) -> &[BorrowLendReserveState] {
        &self.reserves
    }
}

impl TryFrom<&ParsedInfoResponse<Value>> for AllBorrowLendReserveStates {
    type Error = InfoError;

    fn try_from(parsed: &ParsedInfoResponse<Value>) -> Result<Self, Self::Error> {
        expect_capability(parsed, &["official.info.all_borrow_lend_reserve_states"])?;
        let reserves = pair_entries(parsed.value(), "")?
            .into_iter()
            .enumerate()
            .map(|(index, (token, body))| {
                let path = format!("/{index}");
                BorrowLendReserveState::from_object(
                    require_object(body, &format!("{path}/1"))?,
                    &format!("{path}/1"),
                    Some(super::decode::u64_from_value(token, &format!("{path}/0"))?),
                )
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { reserves })
    }
}

pub fn parse_all_borrow_lend_reserve_states(
    raw: &[u8],
    context: InfoParseContext,
) -> Result<(ParsedInfoResponse<Value>, AllBorrowLendReserveStates), InfoError> {
    parse_family(
        "official.info.all_borrow_lend_reserve_states",
        raw,
        context,
        BORROW_LEND_RESERVE_KNOWN_FIELDS,
        &[],
    )
}
