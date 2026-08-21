use api_contracts::WireCanonicalEventEnvelope;
use canonical_events::{CanonicalEventEnvelope, CanonicalUpcaster, UpcastError};

fn with_schema_version(bytes: &[u8], schema_version: &str) -> Vec<u8> {
    let mut wire = WireCanonicalEventEnvelope::decode(bytes).expect("wire envelope");
    wire.schema_version = schema_version.to_owned();
    wire.encode_to_vec()
}

#[test]
fn current_v1_minor_range_is_validated_and_preserved_byte_for_byte() {
    let fixture = CanonicalEventEnvelope::fixture().unwrap();
    let original = fixture.encode_to_vec().unwrap();
    let upcaster = CanonicalUpcaster::v1();

    for schema_version in ["1.0.0", "1.0.7", "1.1.0", "1.1.7"] {
        let input = with_schema_version(&original, schema_version);
        let result = upcaster.upcast(&input).unwrap();

        assert_eq!(result.schema_version().to_string(), schema_version);
        assert_eq!(result.as_bytes(), input);
        assert_eq!(
            CanonicalEventEnvelope::decode(result.as_bytes())
                .unwrap()
                .schema_version(),
            schema_version
        );
    }
}

#[test]
fn historical_and_future_semantic_versions_fail_closed() {
    let original = CanonicalEventEnvelope::fixture()
        .unwrap()
        .encode_to_vec()
        .unwrap();
    let upcaster = CanonicalUpcaster::v1();

    for schema_version in ["0.9.0", "1.2.0", "2.0.0"] {
        let error = upcaster
            .upcast(&with_schema_version(&original, schema_version))
            .expect_err("unsupported version must fail");

        assert!(matches!(
            error,
            UpcastError::UnsupportedVersion { ref version } if version == schema_version
        ));
        assert_eq!(error.reason_code(), "canonical_upcast.unsupported_version");
    }
}

#[test]
fn malformed_version_and_envelope_have_distinct_stable_errors() {
    let original = CanonicalEventEnvelope::fixture()
        .unwrap()
        .encode_to_vec()
        .unwrap();
    let upcaster = CanonicalUpcaster::v1();

    let malformed_version = upcaster
        .upcast(&with_schema_version(&original, "v1"))
        .unwrap_err();
    assert!(matches!(
        malformed_version,
        UpcastError::MalformedVersion { .. }
    ));
    assert_eq!(
        malformed_version.reason_code(),
        "canonical_upcast.malformed_version"
    );

    let malformed_envelope = upcaster.upcast(&[0xff, 0xff]).unwrap_err();
    assert!(matches!(
        malformed_envelope,
        UpcastError::MalformedEnvelope(_)
    ));
    assert_eq!(
        malformed_envelope.reason_code(),
        "canonical_upcast.malformed_envelope"
    );
}

#[test]
fn supported_version_still_requires_a_valid_current_envelope() {
    let original = CanonicalEventEnvelope::fixture()
        .unwrap()
        .encode_to_vec()
        .unwrap();
    let mut wire = WireCanonicalEventEnvelope::decode(&original).unwrap();
    wire.payload_hash = vec![0; 32];

    let error = CanonicalUpcaster::v1()
        .upcast(&wire.encode_to_vec())
        .expect_err("upcaster must validate current payload hashes");

    assert!(matches!(error, UpcastError::InvalidCurrentEnvelope(_)));
    assert_eq!(
        error.reason_code(),
        "canonical_upcast.invalid_current_envelope"
    );
}
