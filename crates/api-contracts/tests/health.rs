use api_contracts::{HealthCodecError, WireHealthAssessment, WireHealthState};

#[test]
fn health_assessment_round_trips_through_the_v1_proto() {
    let assessment = WireHealthAssessment::try_new(
        "canonical",
        WireHealthState::Amber,
        "lag",
        17,
        ["signal:common"],
    )
    .expect("valid assessment");

    let encoded = assessment.encode_to_vec();
    let decoded = WireHealthAssessment::decode(&encoded).expect("proto must decode");

    assert_eq!(decoded, assessment);
    assert_eq!(decoded.state.proto_name(), "HEALTH_STATE_AMBER");
}

#[test]
fn unspecified_or_invalid_health_state_fail_closed() {
    assert_eq!(
        WireHealthState::parse("HEALTH_STATE_UNSPECIFIED"),
        Err(HealthCodecError::Invalid {
            reason: "health state must not be HEALTH_STATE_UNSPECIFIED".to_owned(),
        })
    );
    assert!(WireHealthAssessment::decode(&[0x10, 0x00]).is_err());
}

#[test]
fn empty_identifiers_and_negative_times_are_rejected() {
    assert!(
        WireHealthAssessment::try_new("", WireHealthState::Green, "healthy", 0, [] as [&str; 0])
            .is_err()
    );
    assert!(
        WireHealthAssessment::try_new(
            "canonical",
            WireHealthState::Red,
            " gap",
            0,
            [] as [&str; 0]
        )
        .is_err()
    );
    assert!(
        WireHealthAssessment::try_new(
            "canonical",
            WireHealthState::Red,
            "gap",
            -1,
            [] as [&str; 0]
        )
        .is_err()
    );
}
