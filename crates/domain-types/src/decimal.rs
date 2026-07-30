use num_bigint::BigInt;
use num_traits::{Signed, ToPrimitive, Zero};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::{
    cmp::Ordering,
    fmt,
    hash::{Hash, Hasher},
    str::FromStr,
};
use thiserror::Error;

pub const MAX_DECIMAL_SCALE: u8 = 38;
pub const MAX_NOTIONAL_SCALE: u8 = 76;
pub const MAX_NOTIONAL_COEFFICIENT_BITS: u64 = 512;
pub const MAX_NOTIONAL_DECIMAL_DIGITS: usize = 155;
pub const MAX_NOTIONAL_WIRE_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalyticFloat {
    pub value: f64,
    pub source_scale: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoundingMode {
    TowardZero,
    Floor,
    Ceiling,
    NearestTiesToEven,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ValueError {
    #[error("empty decimal")]
    Empty,
    #[error("invalid decimal")]
    Invalid,
    #[error("decimal has more than {allowed} fractional digits")]
    ExcessPrecision { allowed: u8 },
    #[error("scale {scale} exceeds maximum {maximum}")]
    ScaleOutOfRange { scale: u8, maximum: u8 },
    #[error("scales differ: left={left}, right={right}")]
    ScaleMismatch { left: u8, right: u8 },
    #[error("value is outside the permitted range")]
    OutOfRange,
    #[error("division by zero")]
    DivisionByZero,
    #[error("fixed-point arithmetic overflow")]
    Overflow,
}

#[derive(Debug, Clone, Copy)]
pub struct Decimal {
    raw: i128,
    scale: u8,
}

impl Decimal {
    pub fn from_raw(raw: i128, scale: u8) -> Result<Self, ValueError> {
        validate_scale(scale)?;
        Ok(Self { raw, scale })
    }

    pub const fn raw(self) -> i128 {
        self.raw
    }

    pub const fn scale(self) -> u8 {
        self.scale
    }

    pub fn parse_at_scale(input: &str, scale: u8) -> Result<Self, ValueError> {
        validate_scale(scale)?;
        let parsed = Self::from_str(input)?;
        if parsed.scale > scale {
            return Err(ValueError::ExcessPrecision { allowed: scale });
        }
        parsed.rescale(scale, RoundingMode::TowardZero)
    }

    pub fn checked_add(self, rhs: Self) -> Result<Self, ValueError> {
        self.require_same_scale(rhs)?;
        Self::from_raw(
            self.raw.checked_add(rhs.raw).ok_or(ValueError::Overflow)?,
            self.scale,
        )
    }

    pub fn checked_sub(self, rhs: Self) -> Result<Self, ValueError> {
        self.require_same_scale(rhs)?;
        Self::from_raw(
            self.raw.checked_sub(rhs.raw).ok_or(ValueError::Overflow)?,
            self.scale,
        )
    }

    pub fn checked_mul(
        self,
        rhs: Self,
        output_scale: u8,
        rounding: RoundingMode,
    ) -> Result<Self, ValueError> {
        validate_scale(output_scale)?;
        let product = BigInt::from(self.raw) * BigInt::from(rhs.raw);
        let product_scale = u16::from(self.scale) + u16::from(rhs.scale);
        let raw = rescale_big(product, product_scale, u16::from(output_scale), rounding)?;
        Self::from_raw(big_to_i128(raw)?, output_scale)
    }

    pub fn checked_div(
        self,
        rhs: Self,
        output_scale: u8,
        rounding: RoundingMode,
    ) -> Result<Self, ValueError> {
        validate_scale(output_scale)?;
        if rhs.raw == 0 {
            return Err(ValueError::DivisionByZero);
        }
        let shift = i32::from(output_scale) + i32::from(rhs.scale) - i32::from(self.scale);
        let mut numerator = BigInt::from(self.raw);
        let mut denominator = BigInt::from(rhs.raw);
        if shift >= 0 {
            numerator *= pow10_big(u32::try_from(shift).map_err(|_| ValueError::Overflow)?);
        } else {
            denominator *= pow10_big(shift.unsigned_abs());
        }
        let raw = div_round_big(numerator, denominator, rounding)?;
        Self::from_raw(big_to_i128(raw)?, output_scale)
    }

    pub fn rescale(self, target_scale: u8, rounding: RoundingMode) -> Result<Self, ValueError> {
        validate_scale(target_scale)?;
        let raw = rescale_big(
            BigInt::from(self.raw),
            u16::from(self.scale),
            u16::from(target_scale),
            rounding,
        )?;
        Self::from_raw(big_to_i128(raw)?, target_scale)
    }

    #[allow(clippy::cast_precision_loss)]
    pub fn to_analytic_float(self) -> AnalyticFloat {
        AnalyticFloat {
            value: self.raw as f64 / 10_f64.powi(i32::from(self.scale)),
            source_scale: self.scale,
        }
    }

    fn require_same_scale(self, rhs: Self) -> Result<(), ValueError> {
        if self.scale == rhs.scale {
            Ok(())
        } else {
            Err(ValueError::ScaleMismatch {
                left: self.scale,
                right: rhs.scale,
            })
        }
    }

    fn canonical_parts(self) -> (i128, u8) {
        if self.raw == 0 {
            return (0, 0);
        }

        let mut raw = self.raw;
        let mut scale = self.scale;
        while scale > 0 && raw % 10 == 0 {
            raw /= 10;
            scale -= 1;
        }
        (raw, scale)
    }

    fn numeric_cmp(self, other: Self) -> Ordering {
        let left = BigInt::from(self.raw) * pow10_big(u32::from(other.scale));
        let right = BigInt::from(other.raw) * pow10_big(u32::from(self.scale));
        left.cmp(&right)
    }
}

impl PartialEq for Decimal {
    fn eq(&self, other: &Self) -> bool {
        self.canonical_parts() == other.canonical_parts()
    }
}

impl Eq for Decimal {}

impl PartialOrd for Decimal {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Decimal {
    fn cmp(&self, other: &Self) -> Ordering {
        self.numeric_cmp(*other)
    }
}

impl Hash for Decimal {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.canonical_parts().hash(state);
    }
}

fn validate_scale(scale: u8) -> Result<(), ValueError> {
    if scale > MAX_DECIMAL_SCALE {
        Err(ValueError::ScaleOutOfRange {
            scale,
            maximum: MAX_DECIMAL_SCALE,
        })
    } else {
        Ok(())
    }
}

fn pow10_big(exponent: u32) -> BigInt {
    let mut value = BigInt::from(1_u8);
    for _ in 0..exponent {
        value *= 10_u8;
    }
    value
}

fn rescale_big(
    raw: BigInt,
    source_scale: u16,
    target_scale: u16,
    rounding: RoundingMode,
) -> Result<BigInt, ValueError> {
    match target_scale.cmp(&source_scale) {
        Ordering::Equal => Ok(raw),
        Ordering::Greater => Ok(raw * pow10_big(u32::from(target_scale - source_scale))),
        Ordering::Less => div_round_big(
            raw,
            pow10_big(u32::from(source_scale - target_scale)),
            rounding,
        ),
    }
}

fn div_round_big(
    numerator: BigInt,
    denominator: BigInt,
    mode: RoundingMode,
) -> Result<BigInt, ValueError> {
    if denominator.is_zero() {
        return Err(ValueError::DivisionByZero);
    }
    let quotient = &numerator / &denominator;
    let remainder = &numerator % &denominator;
    if remainder.is_zero() || mode == RoundingMode::TowardZero {
        return Ok(quotient);
    }

    let same_sign = numerator.is_negative() == denominator.is_negative();
    let step = if same_sign {
        BigInt::from(1_u8)
    } else {
        BigInt::from(-1_i8)
    };
    match mode {
        RoundingMode::TowardZero => Ok(quotient),
        RoundingMode::Floor if same_sign => Ok(quotient),
        RoundingMode::Floor => Ok(quotient - BigInt::from(1_u8)),
        RoundingMode::Ceiling if same_sign => Ok(quotient + BigInt::from(1_u8)),
        RoundingMode::Ceiling => Ok(quotient),
        RoundingMode::NearestTiesToEven => {
            let twice_remainder = remainder.abs() * 2_u8;
            let divisor = denominator.abs();
            if twice_remainder < divisor
                || (twice_remainder == divisor && (&quotient % 2_u8).is_zero())
            {
                Ok(quotient)
            } else {
                Ok(quotient + step)
            }
        }
    }
}

fn big_to_i128(value: BigInt) -> Result<i128, ValueError> {
    value.to_i128().ok_or(ValueError::Overflow)
}

impl FromStr for Decimal {
    type Err = ValueError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.is_empty() {
            return Err(ValueError::Empty);
        }
        let (negative, unsigned) = if let Some(rest) = input.strip_prefix('-') {
            (true, rest)
        } else if let Some(rest) = input.strip_prefix('+') {
            (false, rest)
        } else {
            (false, input)
        };
        if unsigned.is_empty() {
            return Err(ValueError::Invalid);
        }

        let mut parts = unsigned.split('.');
        let whole = parts.next().ok_or(ValueError::Invalid)?;
        let fraction = match parts.next() {
            Some(value) if !value.is_empty() => value,
            Some(_) => return Err(ValueError::Invalid),
            None => "",
        };
        if parts.next().is_some()
            || whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || !fraction.bytes().all(|byte| byte.is_ascii_digit())
        {
            return Err(ValueError::Invalid);
        }

        let scale = u8::try_from(fraction.len()).map_err(|_| ValueError::ScaleOutOfRange {
            scale: u8::MAX,
            maximum: MAX_DECIMAL_SCALE,
        })?;
        validate_scale(scale)?;
        let factor = 10_u128
            .checked_pow(u32::from(scale))
            .ok_or(ValueError::Overflow)?;
        let whole = whole.parse::<u128>().map_err(|_| ValueError::Overflow)?;
        let fractional = if fraction.is_empty() {
            0
        } else {
            fraction.parse::<u128>().map_err(|_| ValueError::Overflow)?
        };
        let magnitude = whole
            .checked_mul(factor)
            .and_then(|value| value.checked_add(fractional))
            .ok_or(ValueError::Overflow)?;
        let positive_limit = i128::MAX as u128;
        let negative_limit = positive_limit + 1;
        let raw = if negative {
            if magnitude > negative_limit {
                return Err(ValueError::Overflow);
            }
            if magnitude == negative_limit {
                i128::MIN
            } else {
                -(magnitude as i128)
            }
        } else {
            if magnitude > positive_limit {
                return Err(ValueError::Overflow);
            }
            magnitude as i128
        };
        Self::from_raw(raw, scale)
    }
}

impl fmt::Display for Decimal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let factor = 10_u128.pow(u32::from(self.scale));
        let absolute = self.raw.unsigned_abs();
        let whole = absolute / factor;
        let fraction = absolute % factor;
        if self.raw < 0 {
            write!(formatter, "-")?;
        }
        if self.scale == 0 {
            write!(formatter, "{whole}")
        } else {
            write!(
                formatter,
                "{whole}.{fraction:0width$}",
                width = usize::from(self.scale)
            )
        }
    }
}

impl Serialize for Decimal {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Decimal {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

macro_rules! decimal_newtype {
    ($name:ident) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(Decimal);

        impl $name {
            pub fn from_raw(raw: i128, scale: u8) -> Result<Self, ValueError> {
                Decimal::from_raw(raw, scale).map(Self)
            }

            pub fn parse_at_scale(input: &str, scale: u8) -> Result<Self, ValueError> {
                Decimal::parse_at_scale(input, scale).map(Self)
            }

            pub const fn raw(self) -> i128 {
                self.0.raw()
            }

            pub const fn scale(self) -> u8 {
                self.0.scale()
            }

            pub fn checked_add(self, rhs: Self) -> Result<Self, ValueError> {
                self.0.checked_add(rhs.0).map(Self)
            }

            pub fn checked_sub(self, rhs: Self) -> Result<Self, ValueError> {
                self.0.checked_sub(rhs.0).map(Self)
            }

            pub fn rescale(self, scale: u8, rounding: RoundingMode) -> Result<Self, ValueError> {
                self.0.rescale(scale, rounding).map(Self)
            }

            pub fn to_analytic_float(self) -> AnalyticFloat {
                self.0.to_analytic_float()
            }
        }

        impl FromStr for $name {
            type Err = ValueError;

            fn from_str(input: &str) -> Result<Self, Self::Err> {
                input.parse::<Decimal>().map(Self)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(formatter)
            }
        }
    };
}

decimal_newtype!(Price);
decimal_newtype!(Quantity);
decimal_newtype!(PositionQuantity);
decimal_newtype!(QuoteAmount);
decimal_newtype!(BaseAmount);
decimal_newtype!(UsdAmount);
decimal_newtype!(FundingRate);
decimal_newtype!(FeeRate);
decimal_newtype!(Leverage);
decimal_newtype!(MarginRatio);
decimal_newtype!(BasisPoints);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ExactQuoteNotional {
    coefficient: BigInt,
    scale: u8,
}

impl ExactQuoteNotional {
    pub fn from_coefficient(coefficient: BigInt, scale: u8) -> Result<Self, ValueError> {
        validate_notional_scale(scale)?;
        let (coefficient, scale) = canonicalize_notional(coefficient, scale);
        validate_notional_coefficient(&coefficient)?;
        Ok(Self { coefficient, scale })
    }

    pub fn checked_product(price: Price, quantity: Quantity) -> Result<Self, ValueError> {
        let coefficient = BigInt::from(price.raw()) * BigInt::from(quantity.raw());
        let scale = price
            .scale()
            .checked_add(quantity.scale())
            .ok_or(ValueError::Overflow)?;
        Self::from_coefficient(coefficient, scale)
    }

    pub const fn scale(&self) -> u8 {
        self.scale
    }

    pub const fn coefficient(&self) -> &BigInt {
        &self.coefficient
    }

    pub fn checked_add(&self, rhs: &Self) -> Result<Self, ValueError> {
        let target_scale = self.scale.max(rhs.scale);
        let left = self.coefficient_at_scale(target_scale);
        let right = rhs.coefficient_at_scale(target_scale);
        Self::from_coefficient(left + right, target_scale)
    }

    pub fn checked_sub(&self, rhs: &Self) -> Result<Self, ValueError> {
        let target_scale = self.scale.max(rhs.scale);
        let left = self.coefficient_at_scale(target_scale);
        let right = rhs.coefficient_at_scale(target_scale);
        Self::from_coefficient(left - right, target_scale)
    }

    fn coefficient_at_scale(&self, target_scale: u8) -> BigInt {
        debug_assert!(target_scale >= self.scale);
        &self.coefficient * pow10_big(u32::from(target_scale - self.scale))
    }
}

impl FromStr for ExactQuoteNotional {
    type Err = ValueError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        if input.len() > MAX_NOTIONAL_WIRE_BYTES {
            return Err(ValueError::OutOfRange);
        }
        if input.is_empty() {
            return Err(ValueError::Invalid);
        }

        let (negative, unsigned) = if let Some(rest) = input.strip_prefix('-') {
            (true, rest)
        } else if input.starts_with('+') {
            return Err(ValueError::Invalid);
        } else {
            (false, input)
        };
        if unsigned.is_empty() {
            return Err(ValueError::Invalid);
        }

        let mut parts = unsigned.split('.');
        let whole = parts.next().ok_or(ValueError::Invalid)?;
        let fraction = parts.next();
        if parts.next().is_some()
            || whole.is_empty()
            || !whole.bytes().all(|byte| byte.is_ascii_digit())
            || whole.len() > 1 && whole.starts_with('0')
        {
            return Err(ValueError::Invalid);
        }
        let fraction = match fraction {
            Some(value)
                if !value.is_empty()
                    && value.bytes().all(|byte| byte.is_ascii_digit())
                    && !value.ends_with('0') =>
            {
                value
            }
            Some(_) => return Err(ValueError::Invalid),
            None => "",
        };
        if negative && whole == "0" && fraction.is_empty() {
            return Err(ValueError::Invalid);
        }

        let digit_count = whole
            .len()
            .checked_add(fraction.len())
            .ok_or(ValueError::OutOfRange)?;
        if digit_count > MAX_NOTIONAL_DECIMAL_DIGITS {
            return Err(ValueError::OutOfRange);
        }
        let scale = u8::try_from(fraction.len()).map_err(|_| ValueError::OutOfRange)?;
        validate_notional_scale(scale)?;

        let mut digits = String::with_capacity(digit_count);
        digits.push_str(whole);
        digits.push_str(fraction);
        let mut coefficient =
            BigInt::parse_bytes(digits.as_bytes(), 10).ok_or(ValueError::Invalid)?;
        if negative {
            coefficient = -coefficient;
        }
        Self::from_coefficient(coefficient, scale)
    }
}

impl fmt::Display for ExactQuoteNotional {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.coefficient.is_negative() {
            formatter.write_str("-")?;
        }
        let digits = self.coefficient.abs().to_str_radix(10);
        let scale = usize::from(self.scale);
        if scale == 0 {
            return formatter.write_str(&digits);
        }
        if digits.len() <= scale {
            formatter.write_str("0.")?;
            for _ in 0..(scale - digits.len()) {
                formatter.write_str("0")?;
            }
            formatter.write_str(&digits)
        } else {
            let split = digits.len() - scale;
            formatter.write_str(&digits[..split])?;
            formatter.write_str(".")?;
            formatter.write_str(&digits[split..])
        }
    }
}

impl PartialOrd for ExactQuoteNotional {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for ExactQuoteNotional {
    fn cmp(&self, other: &Self) -> Ordering {
        let target_scale = self.scale.max(other.scale);
        self.coefficient_at_scale(target_scale)
            .cmp(&other.coefficient_at_scale(target_scale))
    }
}

impl Serialize for ExactQuoteNotional {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct ExactQuoteNotionalVisitor;

impl serde::de::Visitor<'_> for ExactQuoteNotionalVisitor {
    type Value = ExactQuoteNotional;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded canonical exact quote notional string")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        value.parse().map_err(E::custom)
    }

    fn visit_borrowed_str<E>(self, value: &'_ str) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.visit_str(value)
    }
}

impl<'de> Deserialize<'de> for ExactQuoteNotional {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_str(ExactQuoteNotionalVisitor)
    }
}

fn validate_notional_scale(scale: u8) -> Result<(), ValueError> {
    if scale > MAX_NOTIONAL_SCALE {
        Err(ValueError::ScaleOutOfRange {
            scale,
            maximum: MAX_NOTIONAL_SCALE,
        })
    } else {
        Ok(())
    }
}

fn validate_notional_coefficient(coefficient: &BigInt) -> Result<(), ValueError> {
    if coefficient.bits() > MAX_NOTIONAL_COEFFICIENT_BITS {
        Err(ValueError::OutOfRange)
    } else {
        Ok(())
    }
}

fn canonicalize_notional(mut coefficient: BigInt, mut scale: u8) -> (BigInt, u8) {
    if coefficient.is_zero() {
        return (BigInt::ZERO, 0);
    }
    while scale > 0 && (&coefficient % 10_u8).is_zero() {
        coefficient /= 10_u8;
        scale -= 1;
    }
    (coefficient, scale)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(transparent)]
pub struct ProbabilityPpm(u32);

impl ProbabilityPpm {
    pub const ZERO: Self = Self(0);
    pub const ONE: Self = Self(1_000_000);

    pub fn from_ppm(value: u32) -> Result<Self, ValueError> {
        if value <= 1_000_000 {
            Ok(Self(value))
        } else {
            Err(ValueError::OutOfRange)
        }
    }

    pub const fn ppm(self) -> u32 {
        self.0
    }

    pub fn checked_scale_i128_toward_zero(self, value: i128) -> Result<i128, ValueError> {
        let divisor = 1_000_000_i128;
        let probability = i128::from(self.0);
        let whole = value.checked_div(divisor).ok_or(ValueError::Overflow)?;
        let remainder = value.checked_rem(divisor).ok_or(ValueError::Overflow)?;
        let scaled_whole = whole.checked_mul(probability).ok_or(ValueError::Overflow)?;
        let scaled_remainder = remainder
            .checked_mul(probability)
            .and_then(|product| product.checked_div(divisor))
            .ok_or(ValueError::Overflow)?;
        scaled_whole
            .checked_add(scaled_remainder)
            .ok_or(ValueError::Overflow)
    }
}

impl<'de> Deserialize<'de> for ProbabilityPpm {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::from_ppm(u32::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}
