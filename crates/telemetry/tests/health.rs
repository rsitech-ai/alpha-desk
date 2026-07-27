use telemetry::{HealthAssessment, HealthError, HealthState};

#[test]
fn aggregate_uses_most_severe_required_dependency() {
    let health = HealthAssessment::aggregate([
        HealthAssessment::green("primary"),
        HealthAssessment::amber("secondary", "temporarily unavailable"),
        HealthAssessment::red("book:BTC", "sequence gap"),
    ]);

    assert_eq!(health.state, HealthState::Red);
    assert!(health.suppresses("market:BTC:capacity"));
}

#[test]
fn aggregation_is_stable_and_uses_latest_observation_and_sorted_suppressions() {
    let first =
        HealthAssessment::try_amber_at("source:zeta", "lag", 17, ["signal:zeta", "signal:common"])
            .expect("literal assessment must be valid");
    let second = HealthAssessment::try_red_at(
        "book:BTC",
        "gap",
        23,
        ["signal:common", "market:BTC:signal"],
    )
    .expect("literal assessment must be valid");

    let forward = HealthAssessment::aggregate([first.clone(), second.clone()]);
    let reverse = HealthAssessment::aggregate([second, first]);

    assert_eq!(forward, reverse);
    assert_eq!(forward.observed_at_micros, 23);
    assert_eq!(forward.reason_code, "book:BTC=gap;source:zeta=lag");
    assert_eq!(
        forward.suppresses,
        vec![
            "market:BTC:capacity",
            "market:BTC:signal",
            "signal:common",
            "signal:zeta",
        ]
    );
}

#[test]
fn book_suppression_is_scoped_to_the_affected_market() {
    let health = HealthAssessment::aggregate([
        HealthAssessment::red("book:BTC", "sequence gap"),
        HealthAssessment::green("book:ETH"),
    ]);

    assert!(health.suppresses("market:BTC:capacity"));
    assert!(!health.suppresses("market:ETH:capacity"));
}

#[test]
fn explicit_apis_reject_ambiguous_identifiers_and_negative_times() {
    assert_eq!(
        HealthAssessment::try_green(""),
        Err(HealthError::EmptyIdentifier { field: "scope" })
    );
    assert_eq!(
        HealthAssessment::try_red("book:\nBTC", "gap"),
        Err(HealthError::ControlCharacter { field: "scope" })
    );
    assert_eq!(
        HealthAssessment::try_amber_at("source:primary", "lag", -1, [] as [&str; 0]),
        Err(HealthError::NegativeObservedTime)
    );
    assert_eq!(
        HealthAssessment::try_red_at("book:BTC", "gap", 0, ["market:\u{7f}:capacity"]),
        Err(HealthError::ControlCharacter {
            field: "suppression"
        })
    );
}

#[test]
fn infallible_sample_constructors_fail_closed_for_invalid_literals() {
    let invalid = HealthAssessment::green("");

    assert_eq!(invalid.state, HealthState::Red);
    assert_eq!(invalid.scope, "health:invalid");
    assert_eq!(invalid.reason_code, "invalid_scope");
    assert!(invalid.suppresses.is_empty());
}

#[test]
fn suppression_membership_is_independent_of_public_vector_order_and_duplicates() {
    let deserialized: HealthAssessment = serde_json::from_str(
        r#"{"scope":"book:BTC","state":"RED","reason_code":"gap","observed_at_micros":1,"suppresses":["z","a","a"]}"#,
    )
    .expect("public health JSON must deserialize");
    assert!(deserialized.suppresses("z"));
    assert!(deserialized.suppresses("a"));
    assert!(!deserialized.suppresses("missing"));

    let direct = HealthAssessment {
        scope: "book:ETH".to_owned(),
        state: HealthState::Red,
        reason_code: "gap".to_owned(),
        observed_at_micros: 2,
        suppresses: vec!["z".to_owned(), "a".to_owned(), "a".to_owned()],
    };
    assert!(direct.suppresses("z"));
    assert!(direct.suppresses("a"));
    assert!(!direct.suppresses("missing"));
}
