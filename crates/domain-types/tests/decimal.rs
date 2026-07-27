use domain_types::{
    AnalyticFloat, Decimal, MAX_DECIMAL_SCALE, Price, ProbabilityPpm, Quantity, RoundingMode,
    ValueError,
};
use proptest::prelude::*;
use std::str::FromStr;

#[test]
fn price_parses_at_metadata_scale_and_formats_without_float() {
    let price = Price::parse_at_scale("12345.6789", 6).unwrap();
    assert_eq!(price.raw(), 12_345_678_900);
    assert_eq!(price.scale(), 6);
    assert_eq!(price.to_string(), "12345.678900");
}

#[test]
fn parsing_rejects_precision_beyond_metadata_scale() {
    let error = Quantity::parse_at_scale("1.0000001", 6).unwrap_err();
    assert_eq!(error, ValueError::ExcessPrecision { allowed: 6 });
}

#[test]
fn downscaling_requires_an_explicit_rounding_mode() {
    let price = Price::from_str("1.005").unwrap();
    assert_eq!(
        price
            .rescale(2, RoundingMode::NearestTiesToEven)
            .unwrap()
            .to_string(),
        "1.00"
    );
    assert_eq!(
        price.rescale(2, RoundingMode::Ceiling).unwrap().to_string(),
        "1.01"
    );
}

#[test]
fn analytical_conversion_retains_source_scale() {
    let converted: AnalyticFloat = Price::parse_at_scale("12.34", 4)
        .unwrap()
        .to_analytic_float();
    assert_eq!(converted.source_scale, 4);
    assert!((converted.value - 12.34).abs() < f64::EPSILON);
}

#[test]
fn probability_is_bounded_and_scales_integers_exactly() {
    assert!(ProbabilityPpm::from_ppm(1_000_001).is_err());
    let half = ProbabilityPpm::from_ppm(500_000).unwrap();
    assert_eq!(half.checked_scale_i128_toward_zero(101).unwrap(), 50);
}

#[test]
fn decimal_rejects_malformed_syntax() {
    for input in ["+", "-", ".", ".1", "1.", "1.2.3", " 1", "1 ", "1e3", "--1"] {
        assert_eq!(
            Decimal::from_str(input),
            Err(ValueError::Invalid),
            "{input}"
        );
    }
    assert_eq!(Decimal::from_str(""), Err(ValueError::Empty));
}

#[test]
fn decimal_rejects_scales_outside_the_supported_range() {
    assert_eq!(
        Decimal::from_raw(1, MAX_DECIMAL_SCALE + 1),
        Err(ValueError::ScaleOutOfRange {
            scale: MAX_DECIMAL_SCALE + 1,
            maximum: MAX_DECIMAL_SCALE,
        })
    );
    let too_precise = format!("0.{}", "1".repeat(usize::from(MAX_DECIMAL_SCALE) + 1));
    assert!(matches!(
        Decimal::from_str(&too_precise),
        Err(ValueError::ScaleOutOfRange { .. })
    ));
}

#[test]
fn checked_arithmetic_rejects_scale_mismatches_and_every_overflow_path() {
    let unit = Decimal::from_raw(1, 0).unwrap();
    let fractional = Decimal::from_raw(1, 1).unwrap();
    assert_eq!(
        unit.checked_add(fractional),
        Err(ValueError::ScaleMismatch { left: 0, right: 1 })
    );
    assert_eq!(
        unit.checked_sub(fractional),
        Err(ValueError::ScaleMismatch { left: 0, right: 1 })
    );
    assert_eq!(
        Decimal::from_raw(i128::MAX, 0).unwrap().checked_add(unit),
        Err(ValueError::Overflow)
    );
    assert_eq!(
        Decimal::from_raw(i128::MIN, 0).unwrap().checked_sub(unit),
        Err(ValueError::Overflow)
    );
    assert_eq!(
        Decimal::from_raw(i128::MAX, 0).unwrap().checked_mul(
            Decimal::from_raw(2, 0).unwrap(),
            0,
            RoundingMode::TowardZero
        ),
        Err(ValueError::Overflow)
    );
    assert_eq!(
        Decimal::from_raw(i128::MAX, 0)
            .unwrap()
            .rescale(1, RoundingMode::TowardZero),
        Err(ValueError::Overflow)
    );
    assert_eq!(
        Decimal::from_raw(i128::MIN, 0).unwrap().checked_div(
            Decimal::from_raw(-1, 0).unwrap(),
            0,
            RoundingMode::TowardZero
        ),
        Err(ValueError::Overflow)
    );
}

#[test]
fn rounding_handles_signed_floor_ceiling_and_ties_to_even() {
    let negative = Decimal::from_str("-1.5").unwrap();
    assert_eq!(
        negative
            .rescale(0, RoundingMode::TowardZero)
            .unwrap()
            .to_string(),
        "-1"
    );
    assert_eq!(
        negative
            .rescale(0, RoundingMode::Floor)
            .unwrap()
            .to_string(),
        "-2"
    );
    assert_eq!(
        negative
            .rescale(0, RoundingMode::Ceiling)
            .unwrap()
            .to_string(),
        "-1"
    );
    assert_eq!(
        negative
            .rescale(0, RoundingMode::NearestTiesToEven)
            .unwrap()
            .to_string(),
        "-2"
    );
    assert_eq!(
        Decimal::from_str("2.5")
            .unwrap()
            .rescale(0, RoundingMode::NearestTiesToEven)
            .unwrap()
            .to_string(),
        "2"
    );
    assert_eq!(
        Decimal::from_str("3.5")
            .unwrap()
            .rescale(0, RoundingMode::NearestTiesToEven)
            .unwrap()
            .to_string(),
        "4"
    );
    assert_eq!(
        Decimal::from_str("-2.5")
            .unwrap()
            .rescale(0, RoundingMode::NearestTiesToEven)
            .unwrap()
            .to_string(),
        "-2"
    );
    assert_eq!(
        Decimal::from_str("-3.5")
            .unwrap()
            .rescale(0, RoundingMode::NearestTiesToEven)
            .unwrap()
            .to_string(),
        "-4"
    );
}

#[test]
fn division_rejects_zero_and_rounds_with_negative_denominators() {
    let one = Decimal::from_raw(1, 0).unwrap();
    assert_eq!(
        one.checked_div(
            Decimal::from_raw(0, 0).unwrap(),
            2,
            RoundingMode::TowardZero
        ),
        Err(ValueError::DivisionByZero)
    );
    let value = Decimal::from_raw(1, 0).unwrap();
    let negative_two = Decimal::from_raw(-2, 0).unwrap();
    assert_eq!(
        value
            .checked_div(negative_two, 0, RoundingMode::Floor)
            .unwrap()
            .to_string(),
        "-1"
    );
    assert_eq!(
        value
            .checked_div(negative_two, 0, RoundingMode::Ceiling)
            .unwrap()
            .to_string(),
        "0"
    );
}

#[test]
fn decimal_and_newtype_serde_round_trips_preserve_scale() {
    let decimal = Decimal::from_raw(-12_340, 3).unwrap();
    let encoded = serde_json::to_string(&decimal).unwrap();
    assert_eq!(encoded, "\"-12.340\"");
    assert_eq!(serde_json::from_str::<Decimal>(&encoded).unwrap(), decimal);

    let price = Price::parse_at_scale("7.5", 4).unwrap();
    let encoded = serde_json::to_string(&price).unwrap();
    assert_eq!(encoded, "\"7.5000\"");
    assert_eq!(serde_json::from_str::<Price>(&encoded).unwrap(), price);
}

#[test]
fn probability_scaling_avoids_intermediate_overflow() {
    assert_eq!(
        ProbabilityPpm::ONE.checked_scale_i128_toward_zero(i128::MAX),
        Ok(i128::MAX)
    );
    assert_eq!(
        ProbabilityPpm::from_ppm(999_999)
            .unwrap()
            .checked_scale_i128_toward_zero(i128::MAX),
        Ok(170_141_013_319_285_771_262_455_572_028_580_389_842)
    );
}

proptest! {
    #[test]
    fn parse_display_and_serde_round_trip_for_supported_raw_values(raw in any::<i64>(), scale in 0_u8..=18) {
        let decimal = Decimal::from_raw(i128::from(raw), scale).unwrap();
        let parsed = Decimal::from_str(&decimal.to_string()).unwrap();
        prop_assert_eq!(parsed, decimal);
        let encoded = serde_json::to_string(&decimal).unwrap();
        let decoded: Decimal = serde_json::from_str(&encoded).unwrap();
        prop_assert_eq!(decoded, decimal);
    }

    #[test]
    fn upscaling_then_downscaling_restores_the_original_value(raw in -1_000_000_i64..=1_000_000, scale in 0_u8..=12, delta in 0_u8..=12) {
        prop_assume!(u16::from(scale) + u16::from(delta) <= u16::from(MAX_DECIMAL_SCALE));
        let original = Decimal::from_raw(i128::from(raw), scale).unwrap();
        let expanded = original.rescale(scale + delta, RoundingMode::TowardZero).unwrap();
        let restored = expanded.rescale(scale, RoundingMode::TowardZero).unwrap();
        prop_assert_eq!(restored, original);
    }

    #[test]
    fn probability_construction_accepts_exactly_the_closed_ppm_range(value in any::<u32>()) {
        let actual = ProbabilityPpm::from_ppm(value);
        prop_assert_eq!(actual.is_ok(), value <= 1_000_000);
    }
}
