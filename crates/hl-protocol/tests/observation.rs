use std::time::{Duration, Instant};

use bytes::Bytes;
use domain_types::SourceId;
use hl_protocol::{
    CursorTransition, ErrorDisposition, ObservationClass, ObservationError, ParseWarning,
    ReceiveTimestamps, SourceCursor, SourceError, SourceObservation, SourceRequestContext,
};
use tokio_util::sync::CancellationToken;

fn source_id() -> SourceId {
    SourceId::new("primary-node").expect("valid source id")
}

fn valid_observation(
    class: ObservationClass,
    payload: Bytes,
    maximum: usize,
) -> Result<SourceObservation, ObservationError> {
    SourceObservation::new(
        source_id(),
        "node-v1.2.3",
        class,
        SourceCursor::new("node-session-17", 42)?,
        ReceiveTimestamps::new(1_721_000_000_000_000, 99_000)?,
        "parser-v1",
        payload,
        vec![ParseWarning::new(
            "field.defaulted",
            "optional field absent",
        )?],
        maximum,
    )
}

#[test]
fn observation_preserves_original_bytes_and_computes_content_hash_internally() {
    let payload = Bytes::from_static(br#"{"height":42}"#);
    let observation = valid_observation(ObservationClass::CommittedBlock, payload.clone(), 1024)
        .expect("valid observation");

    assert_eq!(observation.payload(), &payload);
    assert_eq!(observation.content_hash(), blake3::hash(&payload));
    assert_eq!(observation.source_id(), &source_id());
    assert_eq!(observation.source_version(), "node-v1.2.3");
    assert_eq!(observation.parser_schema_version(), "parser-v1");
    assert_eq!(observation.warnings().len(), 1);
}

#[test]
fn metadata_rejects_empty_padded_or_oversized_values() {
    for invalid in ["", " parser-v1", "parser-v1 ", " \t"] {
        let error = SourceObservation::new(
            source_id(),
            invalid,
            ObservationClass::CommittedBlock,
            SourceCursor::new("epoch", 0).expect("cursor"),
            ReceiveTimestamps::new(1, 1).expect("timestamps"),
            "parser-v1",
            Bytes::from_static(b"x"),
            Vec::new(),
            16,
        )
        .expect_err("source version must be canonical");
        assert_eq!(error.reason_code(), "observation.invalid_source_version");
    }

    let oversized = "x".repeat(257);
    let error = SourceObservation::new(
        source_id(),
        "source-v1",
        ObservationClass::CommittedBlock,
        SourceCursor::new("epoch", 0).expect("cursor"),
        ReceiveTimestamps::new(1, 1).expect("timestamps"),
        &oversized,
        Bytes::from_static(b"x"),
        Vec::new(),
        16,
    )
    .expect_err("parser version must be bounded");
    assert_eq!(
        error.reason_code(),
        "observation.invalid_parser_schema_version"
    );
}

#[test]
fn payload_limits_fail_closed_for_each_observation_class() {
    for class in ObservationClass::ALL {
        let error = valid_observation(class, Bytes::from_static(b"12345"), 4)
            .expect_err("class payload limit must be enforced");
        assert_eq!(error.reason_code(), "observation.payload_too_large");
        assert_eq!(error.observation_class(), Some(class));
    }
}

#[test]
fn empty_payload_and_zero_limit_are_rejected() {
    let empty = valid_observation(ObservationClass::Snapshot, Bytes::new(), 16)
        .expect_err("empty payload is not source evidence");
    assert_eq!(empty.reason_code(), "observation.empty_payload");

    let zero_limit = valid_observation(ObservationClass::Snapshot, Bytes::from_static(b"x"), 0)
        .expect_err("zero payload limit is invalid");
    assert_eq!(
        zero_limit.reason_code(),
        "observation.invalid_payload_limit"
    );
}

#[test]
fn cursor_regression_duplicate_advance_and_epoch_transition_are_explicit() {
    let current = SourceCursor::new("epoch-a", 18).expect("cursor");

    assert_eq!(
        SourceCursor::new("epoch-a", 18)
            .expect("cursor")
            .validate_successor_of(&current)
            .expect("duplicate delivery is valid"),
        CursorTransition::Duplicate
    );
    assert_eq!(
        SourceCursor::new("epoch-a", 19)
            .expect("cursor")
            .validate_successor_of(&current)
            .expect("advance is valid"),
        CursorTransition::Advanced { by: 1 }
    );
    assert_eq!(
        SourceCursor::new("epoch-b", 0)
            .expect("cursor")
            .validate_successor_of(&current)
            .expect("epoch changes are explicit"),
        CursorTransition::EpochChanged
    );

    let regression = SourceCursor::new("epoch-a", 17)
        .expect("cursor")
        .validate_successor_of(&current)
        .expect_err("same-epoch regression must fail");
    assert_eq!(regression.reason_code(), "observation.cursor_regression");
}

#[test]
fn cursors_timestamps_and_warnings_validate_their_boundaries() {
    assert_eq!(
        SourceCursor::new(" epoch", 0)
            .expect_err("padded epoch")
            .reason_code(),
        "observation.invalid_cursor_epoch"
    );
    assert_eq!(
        ReceiveTimestamps::new(-1, 0)
            .expect_err("negative wall time")
            .reason_code(),
        "observation.invalid_wall_timestamp"
    );
    assert_eq!(
        ParseWarning::new("", "detail")
            .expect_err("empty warning code")
            .reason_code(),
        "observation.invalid_warning_code"
    );
    assert_eq!(
        ParseWarning::new("field.defaulted", "")
            .expect_err("empty warning detail")
            .reason_code(),
        "observation.invalid_warning_detail"
    );
}

#[test]
fn source_error_reason_codes_and_dispositions_are_exhaustive() {
    let cases = [
        (
            SourceError::TemporaryDisconnect("closed".into()),
            "source.temporary_disconnect",
            ErrorDisposition::Retry,
        ),
        (
            SourceError::MalformedPayload("json".into()),
            "source.malformed_payload",
            ErrorDisposition::Quarantine,
        ),
        (
            SourceError::SchemaDrift("unknown variant".into()),
            "source.schema_drift",
            ErrorDisposition::Quarantine,
        ),
        (
            SourceError::CursorRegression,
            "source.cursor_regression",
            ErrorDisposition::Quarantine,
        ),
        (
            SourceError::Configuration("missing node path".into()),
            "source.configuration",
            ErrorDisposition::Stop,
        ),
        (
            SourceError::RangeUnavailable,
            "source.range_unavailable",
            ErrorDisposition::Quarantine,
        ),
        (
            SourceError::Cancelled,
            "source.cancelled",
            ErrorDisposition::Stop,
        ),
        (
            SourceError::BackpressureTimeout,
            "source.backpressure_timeout",
            ErrorDisposition::Retry,
        ),
    ];

    for (error, reason_code, disposition) in cases {
        assert_eq!(error.reason_code(), reason_code);
        assert_eq!(error.disposition(), disposition);
    }
}

#[test]
fn request_context_reports_cancellation_before_backpressure_deadline() {
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let cancelled =
        SourceRequestContext::new(cancellation, Instant::now() - Duration::from_secs(1))
            .check()
            .expect_err("cancellation wins");
    assert!(matches!(cancelled, SourceError::Cancelled));

    let timed_out = SourceRequestContext::new(
        CancellationToken::new(),
        Instant::now() - Duration::from_millis(1),
    )
    .check()
    .expect_err("expired deadline");
    assert!(matches!(timed_out, SourceError::BackpressureTimeout));

    SourceRequestContext::new(
        CancellationToken::new(),
        Instant::now() + Duration::from_secs(1),
    )
    .check()
    .expect("active request");
}
