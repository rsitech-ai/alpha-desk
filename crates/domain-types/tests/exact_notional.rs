use domain_types::{
    ExactQuoteNotional, MAX_NOTIONAL_COEFFICIENT_BITS, MAX_NOTIONAL_DECIMAL_DIGITS,
    MAX_NOTIONAL_SCALE, MAX_NOTIONAL_WIRE_BYTES, Price, Quantity, ValueError,
};
use num_bigint::BigInt;
use std::{collections::HashSet, str::FromStr};

#[test]
fn price_times_quantity_is_exact_and_canonical_for_every_sign() {
    let price = Price::from_raw(123, 2).unwrap();

    for (quantity, expected_coefficient, expected_scale, expected_text) in [
        (
            Quantity::from_raw(4, 3).unwrap(),
            BigInt::from(492),
            5,
            "0.00492",
        ),
        (
            Quantity::from_raw(-4, 3).unwrap(),
            BigInt::from(-492),
            5,
            "-0.00492",
        ),
        (Quantity::from_raw(0, 38).unwrap(), BigInt::from(0), 0, "0"),
    ] {
        let product = ExactQuoteNotional::checked_product(price, quantity).unwrap();
        assert_eq!(product.coefficient(), &expected_coefficient);
        assert_eq!(product.scale(), expected_scale);
        assert_eq!(product.to_string(), expected_text);
    }
}

#[test]
fn product_preserves_the_exact_scale_sum_up_to_76() {
    let product = ExactQuoteNotional::checked_product(
        Price::from_raw(1, 38).unwrap(),
        Quantity::from_raw(1, 38).unwrap(),
    )
    .unwrap();

    assert_eq!(product.coefficient(), &BigInt::from(1));
    assert_eq!(product.scale(), MAX_NOTIONAL_SCALE);
    assert_eq!(product.to_string(), format!("0.{}1", "0".repeat(75)));
}

#[test]
fn constructor_canonicalizes_zero_and_decimal_trailing_zeros() {
    let zero = ExactQuoteNotional::checked_product(
        Price::from_raw(0, 38).unwrap(),
        Quantity::from_raw(1, 38).unwrap(),
    )
    .unwrap();
    assert_eq!(zero.coefficient(), &BigInt::from(0));
    assert_eq!(zero.scale(), 0);
    assert_eq!(zero.to_string(), "0");

    let normalized = ExactQuoteNotional::checked_product(
        Price::from_raw(120, 2).unwrap(),
        Quantity::from_raw(10, 1).unwrap(),
    )
    .unwrap();
    assert_eq!(normalized.coefficient(), &BigInt::from(12));
    assert_eq!(normalized.scale(), 1);
    assert_eq!(normalized.to_string(), "1.2");
}

#[test]
fn coefficient_bound_counts_magnitude_bits_for_both_signs() {
    let bit_511 = BigInt::from(1_u8) << 510_usize;
    let bit_512 = BigInt::from(1_u8) << 511_usize;
    let bit_513 = BigInt::from(1_u8) << 512_usize;

    for coefficient in [bit_511, -bit_512.clone(), bit_512] {
        assert!(ExactQuoteNotional::from_str(&coefficient.to_string()).is_ok());
    }
    for coefficient in [bit_513.clone(), -bit_513] {
        assert_eq!(
            ExactQuoteNotional::from_str(&coefficient.to_string()),
            Err(ValueError::OutOfRange)
        );
    }
    assert_eq!(MAX_NOTIONAL_COEFFICIENT_BITS, 512);
}

#[test]
fn parsing_rejects_input_before_bigint_when_wire_or_digit_bounds_are_exceeded() {
    let over_wire = "1".repeat(MAX_NOTIONAL_WIRE_BYTES + 1);
    assert_eq!(
        ExactQuoteNotional::from_str(&over_wire),
        Err(ValueError::OutOfRange)
    );

    let over_digits = "1".repeat(MAX_NOTIONAL_DECIMAL_DIGITS + 1);
    assert_eq!(
        ExactQuoteNotional::from_str(&over_digits),
        Err(ValueError::OutOfRange)
    );

    let encoded = serde_json::to_string(&over_wire).unwrap();
    assert!(serde_json::from_str::<ExactQuoteNotional>(&encoded).is_err());
}

#[test]
fn decimal_digit_limit_accepts_a_bounded_155_digit_coefficient() {
    let boundary = format!("1{}", "0".repeat(MAX_NOTIONAL_DECIMAL_DIGITS - 1));
    let value = ExactQuoteNotional::from_str(&boundary).unwrap();
    assert_eq!(value.to_string(), boundary);
}

#[test]
fn checked_add_and_subtract_align_upward_without_rounding() {
    let left = ExactQuoteNotional::from_str("1.2").unwrap();
    let right = ExactQuoteNotional::from_str("0.03").unwrap();

    let sum = left.checked_add(&right).unwrap();
    assert_eq!(sum.coefficient(), &BigInt::from(123));
    assert_eq!(sum.scale(), 2);
    assert_eq!(sum.to_string(), "1.23");

    let difference = left.checked_sub(&right).unwrap();
    assert_eq!(difference.coefficient(), &BigInt::from(117));
    assert_eq!(difference.scale(), 2);
    assert_eq!(difference.to_string(), "1.17");

    let canonical_integer = left
        .checked_add(&ExactQuoteNotional::from_str("-0.2").unwrap())
        .unwrap();
    assert_eq!(canonical_integer.to_string(), "1");
}

#[test]
fn checked_coefficient_view_only_normalizes_upward_within_every_bound() {
    let value = ExactQuoteNotional::from_str("-1.2").unwrap();
    assert_eq!(
        value.checked_coefficient_at_scale(3),
        Ok(BigInt::from(-1_200))
    );
    assert_eq!(
        value.checked_coefficient_at_scale(0),
        Err(ValueError::DownwardExactRescale {
            source_scale: 1,
            target_scale: 0,
        })
    );
    assert_eq!(
        value.checked_coefficient_at_scale(MAX_NOTIONAL_SCALE + 1),
        Err(ValueError::ScaleOutOfRange {
            scale: MAX_NOTIONAL_SCALE + 1,
            maximum: MAX_NOTIONAL_SCALE,
        })
    );
}

#[test]
fn upward_coefficient_view_and_cross_scale_arithmetic_reject_intermediate_overflow() {
    let maximum = ExactQuoteNotional::from_str(
        &((BigInt::from(1_u8) << 512_usize) - BigInt::from(1_u8)).to_string(),
    )
    .unwrap();

    assert_eq!(
        maximum.checked_coefficient_at_scale(1),
        Err(ValueError::OutOfRange)
    );
    assert_eq!(
        maximum.checked_add(&ExactQuoteNotional::from_str("0.1").unwrap()),
        Err(ValueError::OutOfRange)
    );
}

#[test]
fn arithmetic_revalidates_the_coefficient_bound_after_each_result() {
    let maximum = (BigInt::from(1_u8) << 512_usize) - BigInt::from(1_u8);
    let positive = ExactQuoteNotional::from_str(&maximum.to_string()).unwrap();
    let negative = ExactQuoteNotional::from_str(&(-maximum).to_string()).unwrap();
    let one = ExactQuoteNotional::from_str("1").unwrap();

    assert_eq!(positive.checked_add(&one), Err(ValueError::OutOfRange));
    assert_eq!(negative.checked_sub(&one), Err(ValueError::OutOfRange));
}

#[test]
fn string_and_serde_forms_are_canonical_and_round_trip() {
    for input in ["0", "1", "-1", "0.001", "-0.001", "123.45"] {
        let value = ExactQuoteNotional::from_str(input).unwrap();
        assert_eq!(value.to_string(), input);
        let encoded = serde_json::to_string(&value).unwrap();
        assert_eq!(encoded, format!("\"{input}\""));
        assert_eq!(
            serde_json::from_str::<ExactQuoteNotional>(&encoded).unwrap(),
            value
        );
    }
    assert!(serde_json::from_str::<ExactQuoteNotional>("1").is_err());
}

#[test]
fn malformed_or_noncanonical_strings_are_rejected() {
    for input in [
        "", "+", "+1", "-", "-0", "00", "01", "-01", ".", ".1", "1.", "1.0", "0.0", "1.20", "1e3",
        " 1", "1 ", "--1",
    ] {
        assert_eq!(
            ExactQuoteNotional::from_str(input),
            Err(ValueError::Invalid),
            "{input}"
        );
    }
}

#[test]
fn equality_hashing_and_ordering_are_numeric_and_deterministic() {
    let mut values = ["1.2", "-2", "10", "0", "-1.5", "0.01", "1"]
        .map(|value| ExactQuoteNotional::from_str(value).unwrap());
    values.sort();
    assert_eq!(
        values.map(|value| value.to_string()),
        ["-2", "-1.5", "0", "0.01", "1", "1.2", "10"]
    );

    let canonical = ExactQuoteNotional::from_str("1.23")
        .unwrap()
        .checked_sub(&ExactQuoteNotional::from_str("0.03").unwrap())
        .unwrap();
    let parsed = ExactQuoteNotional::from_str("1.2").unwrap();
    assert_eq!(canonical, parsed);
    let mut set = HashSet::new();
    set.insert(canonical);
    set.insert(parsed);
    assert_eq!(set.len(), 1);
}
