use stage_gate::canonical::{CanonicalErrorCode, canonicalize_json_str};

#[test]
fn duplicate_json_object_names_are_rejected_before_canonicalization() {
    let error = canonicalize_json_str(r#"{"stage":"zero","stage":"one"}"#)
        .expect_err("duplicate property names must fail closed");

    assert_eq!(error.code(), CanonicalErrorCode::DuplicateProperty);
}

#[test]
fn rfc_8785_sample_is_serialized_exactly() {
    let source = concat!(
        r#"{"numbers":[333333333.33333329,1E30,4.50,2e-3,"#,
        r#"0.000000000000000000000000001],"string":"\u20ac$\u000F\n"#,
        r#"A'\u0042\u0022\u005c\\\"\/","literals":[null,true,false]}"#,
    );
    let expected = concat!(
        r#"{"literals":[null,true,false],"numbers":[333333333.3333333,"#,
        r#"1e+30,4.5,0.002,1e-27],"string":"€$\u000f\nA'B\"\\\\\"/"}"#,
    );

    assert_eq!(
        canonicalize_json_str(source).expect("RFC 8785 sample must canonicalize"),
        expected.as_bytes()
    );
}

#[test]
fn child_objects_are_sorted_recursively_without_reordering_arrays() {
    let source = r#"{"z":{"b":1,"a":2},"a":[{"d":4,"c":3},2,1]}"#;
    let expected = br#"{"a":[{"c":3,"d":4},2,1],"z":{"a":2,"b":1}}"#;

    assert_eq!(
        canonicalize_json_str(source).expect("nested objects must canonicalize"),
        expected
    );
}

#[test]
fn property_names_follow_rfc_8785_utf16_code_unit_order() {
    let source = concat!(
        r#"{"\ufb33":"Hebrew Letter Dalet With Dagesh","#,
        r#""\ud83d\ude00":"Emoji: Grinning Face","\u20ac":"Euro Sign","#,
        r#""\u00f6":"Latin Small Letter O With Diaeresis","#,
        r#""\u0080":"Control","1":"One","\r":"Carriage Return"}"#,
    );
    let expected = concat!(
        "{\"\\r\":\"Carriage Return\",\"1\":\"One\",\"\u{0080}\":\"Control\",",
        r#""ö":"Latin Small Letter O With Diaeresis","€":"Euro Sign","#,
        r#""😀":"Emoji: Grinning Face","דּ":"Hebrew Letter Dalet With Dagesh"}"#,
    );

    assert_eq!(
        canonicalize_json_str(source).expect("UTF-16 property order must canonicalize"),
        expected.as_bytes()
    );
}

#[test]
fn non_finite_and_out_of_range_numbers_are_rejected() {
    for source in ["[NaN]", "[Infinity]", "[-Infinity]", "[1e400]"] {
        let error = canonicalize_json_str(source)
            .expect_err("non-finite or out-of-range numbers must fail closed");
        assert_eq!(error.code(), CanonicalErrorCode::InvalidJson, "{source}");
    }
}

#[test]
fn numbers_follow_ecmascript_threshold_and_negative_zero_rules() {
    let source = "[-0,0.000001,0.0000001,100000000000000000000,1e21]";
    let expected = b"[0,0.000001,1e-7,100000000000000000000,1e+21]";

    assert_eq!(
        canonicalize_json_str(source).expect("finite JSON numbers must canonicalize"),
        expected
    );
}
