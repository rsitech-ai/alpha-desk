/// Checked-in OpenAPI document generated from the health proto JSON fields,
/// capture-status v4 (inactive) / v5 (maintenance) required keys, optional
/// last-heartbeat throughput integers, fail-closed query budgets, frozen
/// committed source class, committed source health, auxiliary source
/// identity, auxiliary spool_records, auxiliary unarchived_records,
/// auxiliary partial_line, auxiliary cursor_epoch, auxiliary tail_cursor_epoch,
/// auxiliary durable_offset, auxiliary local_sequence,
/// auxiliary last_durable_wall_micros, auxiliary last_error_reason,
/// top-level last_error_reason, top-level failover_height, top-level failover_reason, top-level durable_height, auxiliary quarantine_reason,
/// auxiliary_sources maxItems,
/// auxiliary source_id uniqueness, auxiliary source_id sort order,
/// auxiliary source extra keys, auxiliary source health, auxiliary restart
/// reconstruction, auxiliary source qualification, core dead-letter and
/// ledger.unsupported_event reason codes, and the HTTP router. This is not a
/// production authentication, availability, or SLO contract, it does not
/// invent fills or mark sources live or qualified, and it is not a live core.
pub fn openapi_yaml() -> &'static str {
    include_str!("../../../schemas/openapi/v1/openapi.yaml")
}

pub use crate::snapshot::{
    AUXILIARY_SOURCE_HEALTH, AUXILIARY_SOURCE_QUALIFICATION, CAPTURE_SOURCE_HEALTH,
    COMMITTED_SOURCE_CLASSES, CORE_DEADLETTER_REASON_CODES, FAILOVER_REASONS,
    LEDGER_UNSUPPORTED_EVENT_REASON_CODES, MAX_AUXILIARY_SOURCES, RESTART_RECONSTRUCTION,
    is_core_deadletter_reason, is_ledger_unsupported_event_reason,
};

pub const HEALTH_JSON_FIELDS: &[&str] = &[
    "schema_version",
    "scope",
    "state",
    "reason_code",
    "observed_at_micros",
    "suppresses",
];

pub const CAPTURE_STATUS_SCHEMA_IDS: &[&str] = &[
    crate::snapshot::CAPTURE_STATUS_SCHEMA_V4,
    crate::snapshot::CAPTURE_STATUS_SCHEMA_V5,
];

pub const SNAPSHOT_UNAVAILABLE_REASON_CODES: &[&str] = &[
    crate::snapshot::SnapshotError::Missing.reason_code(),
    crate::snapshot::SnapshotError::Invalid.reason_code(),
];

pub const LAST_HEARTBEAT_THROUGHPUT_FIELDS: &[&str] =
    &["throughput_records_per_sec", "throughput_blocks_per_sec"];

pub const ROUTER_PATHS: &[&str] = &[
    "/healthz",
    "/readyz",
    "/v1/health",
    "/v1/capture/status",
    "/v1/stream",
    "/v1/stream/canonical-events",
    "/v1/openapi.yaml",
];

/// String values of `components.schemas.CoreDeadLetterReasonCode.enum`.
///
/// Returns `None` when that schema or its block `enum` is missing. Codes that
/// appear only in descriptions or other prose are not enum values, so dropping
/// a frozen code from the YAML enum fails even if the string remains in prose.
#[must_use]
pub fn core_deadletter_reason_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &["components", "schemas", "CoreDeadLetterReasonCode"],
        "enum",
    )
}

/// String values of `components.schemas.LedgerUnsupportedEventReasonCode.enum`.
///
/// Returns `None` when that schema or its block `enum` is missing. Codes that
/// appear only in descriptions or other prose are not enum values, so dropping
/// the frozen code from the YAML enum fails even if the string remains in
/// prose.
#[must_use]
pub fn ledger_unsupported_event_reason_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &["components", "schemas", "LedgerUnsupportedEventReasonCode"],
        "enum",
    )
}

/// String values of `CaptureStatusBase.properties.active_committed_source.enum`.
///
/// Returns `None` when that property or its block `enum` is missing. Values
/// that appear only in descriptions or other prose are not enum values, so
/// dropping a frozen class from the YAML enum fails even if the string
/// remains in prose. HealthAssessment.reason_code is not this field and
/// stays a free string so unknown RED is not closed out.
#[must_use]
pub fn committed_source_class_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "active_committed_source",
        ],
        "enum",
    )
}

/// String values of `CaptureStatusBase.properties.primary_source_health.enum`.
///
/// Returns `None` when that property or its block `enum` is missing. Values
/// that appear only in descriptions or other prose are not enum values, so
/// dropping a frozen health from the YAML enum fails even if the string
/// remains in prose. HealthAssessment.reason_code is not this field and
/// stays a free string so unknown RED is not closed out.
#[must_use]
pub fn capture_source_health_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "primary_source_health",
        ],
        "enum",
    )
}

/// String values of `CaptureStatusBase.properties.independent_source_health.enum`.
///
/// Optional on the wire; when documented, the YAML enum must match
/// [`CAPTURE_SOURCE_HEALTH`]. Returns `None` when that property or its
/// block `enum` is missing. HealthAssessment.reason_code stays a free
/// string so unknown RED is not closed out.
#[must_use]
pub fn independent_source_health_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "independent_source_health",
        ],
        "enum",
    )
}

/// String values of nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.health.enum`.
///
/// Optional on the wire; when documented, the YAML enum must match
/// [`AUXILIARY_SOURCE_HEALTH`]. Returns `None` when that property or its
/// block `enum` is missing. This is not [`CAPTURE_SOURCE_HEALTH`].
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn auxiliary_source_health_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "health",
        ],
        "enum",
    )
}

/// String values of nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.restart_reconstruction.enum`.
///
/// Optional on the wire; when documented, the YAML enum must match
/// [`RESTART_RECONSTRUCTION`]. Returns `None` when that property or its
/// block `enum` is missing. HealthAssessment.reason_code stays a free
/// string so unknown RED is not closed out.
#[must_use]
pub fn restart_reconstruction_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "restart_reconstruction",
        ],
        "enum",
    )
}

/// String values of nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.qualification.enum`.
///
/// Optional on the wire; when documented, the YAML enum must match
/// [`AUXILIARY_SOURCE_QUALIFICATION`]. Returns `None` when that property
/// or its block `enum` is missing. HealthAssessment.reason_code stays a
/// free string so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_qualification_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "qualification",
        ],
        "enum",
    )
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.source_id`
/// is a required free string: `type: string`, listed on `items.required`,
/// and no `$ref`, `enum`, `format`, or `pattern`. Capture writer always
/// emits `source_id`; this crate does not invent extra identity formats.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn auxiliary_source_id_is_required_string(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "source_id",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("string")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
    {
        return false;
    }
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"source_id"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.spool_records`
/// is a required u64 integer: `type: integer`, `minimum: 0`, listed on
/// `items.required`, and no `$ref`, `enum`, `format`, `pattern`, or
/// `maximum`. Capture writer always emits `spool_records` as u64; this
/// crate does not invent extra numeric bounds. HealthAssessment.reason_code
/// stays a free string so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_spool_records_is_required_u64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "spool_records",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.scalar("minimum") != Some("0")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"spool_records"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.unarchived_records`
/// is a required u64 integer: `type: integer`, `minimum: 0`, listed on
/// `items.required`, and no `$ref`, `enum`, `format`, `pattern`, or
/// `maximum`. Capture writer always emits `unarchived_records` as u64; this
/// crate does not invent extra numeric bounds. HealthAssessment.reason_code
/// stays a free string so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_unarchived_records_is_required_u64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "unarchived_records",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.scalar("minimum") != Some("0")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"unarchived_records"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.partial_line`
/// is a required boolean: `type: boolean`, listed on `items.required`, and
/// no `$ref`, `enum`, `format`, or `pattern`. Capture writer always emits
/// `partial_line` as bool; this crate does not invent extra boolean
/// encodings. HealthAssessment.reason_code stays a free string so unknown
/// RED is not closed out.
#[must_use]
pub fn auxiliary_source_partial_line_is_required_bool(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "partial_line",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("boolean")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
    {
        return false;
    }
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"partial_line"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.cursor_epoch`
/// is an optional free string: `type: string`, not listed on `items.required`,
/// and no `$ref`, `enum`, `format`, or `pattern`. Capture writer emits
/// `cursor_epoch` as a string with the durable cluster once healthy or
/// quarantined (`Option` + `skip_serializing_if`); this crate does not invent
/// extra identity formats. HealthAssessment.reason_code stays a free string
/// so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_cursor_epoch_is_optional_string(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "cursor_epoch",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("string")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"cursor_epoch"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.tail_cursor_epoch`
/// is an optional free string: `type: string`, not listed on `items.required`,
/// and no `$ref`, `enum`, `format`, or `pattern`. Capture writer emits
/// `tail_cursor_epoch` as a string (`Option` + `skip_serializing_if`); this
/// crate does not invent extra identity formats. HealthAssessment.reason_code
/// stays a free string so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_tail_cursor_epoch_is_optional_string(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "tail_cursor_epoch",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("string")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"tail_cursor_epoch"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.durable_offset`
/// is an optional u64 integer: `type: integer`, `minimum: 0`, not listed on
/// `items.required`, and no `$ref`, `enum`, `format`, `pattern`, or
/// `maximum`. Capture writer emits `durable_offset` as `Option<u64>` with
/// `skip_serializing_if` once the durable cluster is present; this crate
/// does not invent extra numeric bounds. HealthAssessment.reason_code stays
/// a free string so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_durable_offset_is_optional_u64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "durable_offset",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.scalar("minimum") != Some("0")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"durable_offset"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.local_sequence`
/// is an optional u64 integer: `type: integer`, `minimum: 0`, not listed on
/// `items.required`, and no `$ref`, `enum`, `format`, `pattern`, or
/// `maximum`. Capture writer emits `local_sequence` as `Option<u64>` with
/// `skip_serializing_if` once the durable cluster is present; this crate
/// does not invent extra numeric bounds. HealthAssessment.reason_code stays
/// a free string so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_local_sequence_is_optional_u64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "local_sequence",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.scalar("minimum") != Some("0")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"local_sequence"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.unread_bytes`
/// is an optional u64 integer: `type: integer`, `minimum: 0`, not listed on
/// `items.required`, and no `$ref`, `enum`, `format`, `pattern`, or
/// `maximum`. Capture writer emits `unread_bytes` as `Option<u64>` with
/// `skip_serializing_if`; this crate does not invent extra numeric bounds.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn auxiliary_source_unread_bytes_is_optional_u64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "unread_bytes",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.scalar("minimum") != Some("0")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"unread_bytes"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.last_durable_wall_micros`
/// is an optional i64 integer: `type: integer`, not listed on
/// `items.required`, and no `$ref`, `enum`, `format`, `pattern`,
/// `minimum`, or `maximum`. Capture writer emits `last_durable_wall_micros`
/// as `Option<i64>` with `skip_serializing_if` once the durable cluster is
/// present; this crate does not invent extra numeric bounds and does not
/// reuse the u64 `minimum: 0` freeze. HealthAssessment.reason_code stays a
/// free string so unknown RED is not closed out.
#[must_use]
pub fn auxiliary_source_last_durable_wall_micros_is_optional_i64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "last_durable_wall_micros",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("minimum")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"last_durable_wall_micros"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.quarantine_reason`
/// is an optional free string: `type: string`, not listed on `items.required`,
/// and no `$ref`, `enum`, `format`, or `pattern`. Capture writer emits
/// `quarantine_reason` as a string (`Option` + `skip_serializing_if`); this
/// crate does not invent a closed enum of quarantine reasons.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn auxiliary_source_quarantine_reason_is_optional_string(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "quarantine_reason",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("string")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"quarantine_reason"))
}

/// True when nested
/// `CaptureStatusBase.properties.auxiliary_sources.items.properties.last_error_reason`
/// is an optional free string: `type: string`, not listed on `items.required`,
/// and no `$ref`, `enum`, `format`, or `pattern`. Capture writer emits
/// `last_error_reason` as a string (`Option` + `skip_serializing_if`); this
/// crate does not invent a closed enum of error reasons.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn auxiliary_source_last_error_reason_is_optional_string(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
            "properties",
            "last_error_reason",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("string")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
        "required",
    )
    .is_some_and(|required| required.contains(&"last_error_reason"))
}

/// True when top-level `CaptureStatusBase.properties.last_error_reason` is an
/// optional free string: `type: string`, not listed on
/// `CaptureStatusBase.required`, and no `$ref`, `enum`, `format`, or
/// `pattern`. Capture writer emits top-level `last_error_reason` as a string
/// (`Option` + `skip_serializing_if`); this crate does not invent a closed
/// enum of error reasons. Nested `auxiliary_sources[].last_error_reason` is a
/// different property. HealthAssessment.reason_code stays a free string so
/// unknown RED is not closed out.
#[must_use]
pub fn capture_status_last_error_reason_is_optional_string(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "last_error_reason",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("string")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &["components", "schemas", "CaptureStatusBase"],
        "required",
    )
    .is_some_and(|required| required.contains(&"last_error_reason"))
}

/// True when top-level `CaptureStatusBase.properties.failover_height` is an
/// optional u64 integer: `type: integer`, `minimum: 0`, not listed on
/// `CaptureStatusBase.required`, and no `$ref`, `enum`, `format`, `pattern`,
/// or `maximum`. Capture writer emits top-level `failover_height` as
/// `Option<u64>` with `skip_serializing_if`; this crate does not invent extra
/// numeric bounds. HealthAssessment.reason_code stays a free string so
/// unknown RED is not closed out.
#[must_use]
pub fn capture_status_failover_height_is_optional_u64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "failover_height",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.scalar("minimum") != Some("0")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &["components", "schemas", "CaptureStatusBase"],
        "required",
    )
    .is_some_and(|required| required.contains(&"failover_height"))
}

/// String values of `CaptureStatusBase.properties.failover_reason.enum`.
///
/// Optional on the wire; when documented, the YAML enum must match
/// [`FAILOVER_REASONS`]. Returns `None` when that property or its
/// block `enum` is missing. Values that appear only in descriptions or
/// other prose are not enum values, so dropping the frozen kebab-case
/// name from the YAML enum fails even if the string remains in prose.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn capture_status_failover_reason_openapi_enum(document: &str) -> Option<Vec<&str>> {
    yaml_string_sequence(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "failover_reason",
        ],
        "enum",
    )
}

/// True when top-level `CaptureStatusBase.properties.failover_reason` is an
/// optional kebab-case string enum: `type: string`, has `enum`, not listed on
/// `CaptureStatusBase.required`, and no `$ref`, `format`, `pattern`,
/// `minLength`, or `maxLength`. Capture writer emits
/// `Option<FailoverReason>` with `skip_serializing_if` and kebab-case
/// `primary-range-unavailable` only. This crate does not invent extra
/// failover reason names or copy writer charset/length onto the API.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn capture_status_failover_reason_is_optional_enum(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "failover_reason",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("string")
        || mapping.has_key("$ref")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("minLength")
        || mapping.has_key("maxLength")
        || !mapping.has_key("enum")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &["components", "schemas", "CaptureStatusBase"],
        "required",
    )
    .is_some_and(|required| required.contains(&"failover_reason"))
}

/// True when top-level `CaptureStatusBase.properties.durable_height` is an
/// optional u64 integer: `type: integer`, `minimum: 0`, not listed on
/// `CaptureStatusBase.required`, and no `$ref`, `enum`, `format`, `pattern`,
/// or `maximum`. Capture writer emits top-level `durable_height` as
/// `Option<u64>` with `skip_serializing_if`; this crate does not invent extra
/// numeric bounds. HealthAssessment.reason_code stays a free string so
/// unknown RED is not closed out.
#[must_use]
pub fn capture_status_durable_height_is_optional_u64(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "durable_height",
        ],
    ) else {
        return false;
    };
    if mapping.scalar("type") != Some("integer")
        || mapping.scalar("minimum") != Some("0")
        || mapping.has_key("$ref")
        || mapping.has_key("enum")
        || mapping.has_key("format")
        || mapping.has_key("pattern")
        || mapping.has_key("maximum")
        || mapping.has_key("exclusiveMinimum")
        || mapping.has_key("exclusiveMaximum")
    {
        return false;
    }
    !yaml_string_sequence(
        document,
        &["components", "schemas", "CaptureStatusBase"],
        "required",
    )
    .is_some_and(|required| required.contains(&"durable_height"))
}

/// True when `CaptureStatusBase.properties.auxiliary_sources` is an array
/// capped at capture writer [`MAX_AUXILIARY_SOURCES`]: `type: array`,
/// `maxItems` equal to that constant, and no `minItems` or `uniqueItems`.
/// Omitted and empty arrays stay valid. Duplicate or out-of-order present
/// `source_id` is parse `snapshot_invalid`; OpenAPI `uniqueItems` would type
/// whole-item uniqueness, which is a different rule. Present ids must be
/// strictly increasing in lexicographic string order.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn auxiliary_sources_max_items_is_writer_cap(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
        ],
    ) else {
        return false;
    };
    mapping.scalar("type") == Some("array")
        && mapping
            .scalar("maxItems")
            .is_some_and(|value| value.parse::<usize>().ok() == Some(MAX_AUXILIARY_SOURCES))
        && !mapping.has_key("minItems")
        && !mapping.has_key("uniqueItems")
}

/// True when `CaptureStatusBase.properties.auxiliary_sources.items` forbids
/// extra keys: `type: object` and `additionalProperties: false`, matching
/// HealthAssessment / CaptureMaintenance / ApiError. Present unknown nested
/// properties are parse `snapshot_invalid`. Known objects without extras
/// stay valid. CaptureStatusBase itself does not set
/// `additionalProperties: false` (top-level writer fields such as
/// `capture_backlog_records`, `oldest_pending_capture_height`,
/// `disk_free_basis_points`, and `archive_manifest_id` stay untyped).
/// Top-level `failover_height` is an optional u64. Top-level
/// `failover_reason` is an optional kebab-case enum. Top-level
/// `durable_height` is an optional u64. Top-level
/// `last_error_reason` is an optional string.
/// HealthAssessment.reason_code stays a free string so unknown RED is not
/// closed out.
#[must_use]
pub fn auxiliary_source_items_forbid_additional_properties(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "CaptureStatusBase",
            "properties",
            "auxiliary_sources",
            "items",
        ],
    ) else {
        return false;
    };
    mapping.scalar("type") == Some("object")
        && mapping.scalar("additionalProperties") == Some("false")
}

/// True when `HealthAssessment.reason_code` is a free string: `type: string`,
/// no `$ref` (including `CoreDeadLetterReasonCode` or
/// `LedgerUnsupportedEventReasonCode`), and no inline `enum`. Unknown RED
/// codes must still fail closed at serve time; the named enums are
/// documentation-only and must not close out this field.
#[must_use]
pub fn health_reason_code_is_unrestricted_string(document: &str) -> bool {
    let Some(mapping) = yaml_mapping(
        document,
        &[
            "components",
            "schemas",
            "HealthAssessment",
            "properties",
            "reason_code",
        ],
    ) else {
        return false;
    };
    mapping.scalar("type") == Some("string") && !mapping.has_key("$ref") && !mapping.has_key("enum")
}

/// Schema `$ref` for `/readyz` 200.
///
/// Must stay `HealthAssessment`. Switching this path to `ApiError` would
/// mis-document GREEN readiness. Named `components.responses` `$ref`
/// returns `None`, same as [`readyz_503_schema_ref`].
#[must_use]
pub fn readyz_200_schema_ref(document: &str) -> Option<&str> {
    path_response_schema_ref(document, "/readyz", "200")
}

/// Schema `$ref` for `/readyz` 503.
///
/// Returns `None` when that status is a named `components.responses` `$ref`
/// (for example `Unavailable`) instead of an inline JSON schema. Switching
/// the body back to `ApiError` while the handler still writes `hl.health.v1`
/// fails the freeze that requires `HealthAssessment`.
#[must_use]
pub fn readyz_503_schema_ref(document: &str) -> Option<&str> {
    path_response_schema_ref(document, "/readyz", "503")
}

/// Named `components.responses` `$ref` for `/v1/health` 503.
///
/// Must stay `Unavailable`. Shared `Unavailable` remaining `ApiError` does
/// not pin this path: inlining `HealthAssessment` here while the handler
/// still writes `hl.api.error.v1` would otherwise pass.
#[must_use]
pub fn health_503_response_ref(document: &str) -> Option<&str> {
    path_response_named_ref(document, "/v1/health", "503")
}

/// Inline schema `$ref` for `/v1/health` 503.
///
/// Frozen shape is a named `Unavailable` `$ref`, so this is `None`. Inlining
/// `HealthAssessment` while the handler still returns `hl.api.error.v1`
/// fails the freeze.
#[must_use]
pub fn health_503_schema_ref(document: &str) -> Option<&str> {
    path_response_schema_ref(document, "/v1/health", "503")
}

/// Schema `$ref` for the shared `Unavailable` response (`/v1/health` 503).
///
/// Must stay `ApiError`. Retargeting this named response at health would
/// silently retype `/v1/health` 503.
#[must_use]
pub fn unavailable_response_schema_ref(document: &str) -> Option<&str> {
    yaml_mapping(
        document,
        &[
            "components",
            "responses",
            "Unavailable",
            "content",
            "application/json",
            "schema",
        ],
    )?
    .scalar("$ref")
}

/// Frozen `/readyz` 200 path description after YAML folded-block join.
///
/// Exact path-item equality so substring `"GREEN-only"` without
/// `"present and valid"` cannot pass after the prose is rewritten.
/// Schema `$ref` stays `HealthAssessment`; this string is the
/// description pin, not a whole-document search.
pub const READYZ_200_DESCRIPTION: &str = concat!(
    "Aggregate is HEALTH_STATE_GREEN. Readiness is GREEN-only and is ",
    "not implied by /v1/health 200. AMBER, including lag, is 503.",
);

/// Folded or inline `/readyz` 200 description.
///
/// Frozen by exact equality with [`READYZ_200_DESCRIPTION`] so 200 cannot
/// keep `"GREEN-only"` (or restore `"present and valid"`) while the path
/// description drifts.
#[must_use]
pub fn readyz_200_description(document: &str) -> Option<String> {
    path_response_description(document, "/readyz", "200")
}

/// Frozen `/readyz` 503 path description after YAML folded-block join.
///
/// Exact path-item equality so substring `"Not ApiError"` plus
/// `HealthAssessment` or `hl.health.v1` cannot pass after the prose is
/// rewritten. Schema `$ref` stays `HealthAssessment`; this string is the
/// description pin, not a whole-document search. `/v1/health` 503 stays
/// named `Unavailable` / `ApiError`.
pub const READYZ_503_DESCRIPTION: &str = concat!(
    "Aggregate is not GREEN. Body is hl.health.v1 HealthAssessment, ",
    "including typed AMBER lag. Not ApiError. Canonical snapshot ",
    "validity is /v1/health 200; readiness is GREEN-only. This is ",
    "not Stage 2 PASS.",
);

/// Folded or inline `/readyz` 503 description.
///
/// Frozen by exact equality with [`READYZ_503_DESCRIPTION`] so 503 cannot
/// be documented as `ApiError` while the body stays `hl.health.v1`.
#[must_use]
pub fn readyz_503_description(document: &str) -> Option<String> {
    path_response_description(document, "/readyz", "503")
}

/// Frozen `/readyz` GET operation description after YAML folded-block join.
///
/// Exact path-item equality so substring `"not ApiError"` cannot pass after
/// the operation prose is rewritten. Response 200/503 descriptions stay
/// their own pins; this string is `paths./readyz.get.description`, not a
/// whole-document search.
pub const READYZ_GET_DESCRIPTION: &str = concat!(
    "GREEN-only readiness as hl.health.v1. 200 is HEALTH_STATE_GREEN; ",
    "non-GREEN including typed AMBER lag is 503 health, not ApiError. ",
    "A valid /v1/health snapshot is not readiness. This is not Stage 2 ",
    "PASS and not a live core.",
);

/// Folded or inline `/readyz` GET operation description.
///
/// Frozen by exact equality with [`READYZ_GET_DESCRIPTION`] so a rewrite
/// that still contains `"not ApiError"` fails if this field drifted.
#[must_use]
pub fn readyz_get_description(document: &str) -> Option<String> {
    path_operation_description(document, "/readyz")
}

fn path_response_schema_ref<'a>(document: &'a str, path: &str, status: &str) -> Option<&'a str> {
    yaml_mapping(
        document,
        &[
            "paths",
            path,
            "get",
            "responses",
            status,
            "content",
            "application/json",
            "schema",
        ],
    )?
    .scalar("$ref")
}

fn path_response_named_ref<'a>(document: &'a str, path: &str, status: &str) -> Option<&'a str> {
    yaml_mapping(document, &["paths", path, "get", "responses", status])?.scalar("$ref")
}

fn path_response_description(document: &str, path: &str, status: &str) -> Option<String> {
    yaml_mapping(document, &["paths", path, "get", "responses", status])?
        .scalar_or_folded("description")
}

fn path_operation_description(document: &str, path: &str) -> Option<String> {
    yaml_mapping(document, &["paths", path, "get"])?.scalar_or_folded("description")
}

struct YamlMapping<'a> {
    lines: Vec<&'a str>,
    begin: usize,
    end: usize,
    key_indent: usize,
}

impl<'a> YamlMapping<'a> {
    fn has_key(&self, key: &str) -> bool {
        self.find_key(key).is_some()
    }

    fn scalar(&self, key: &str) -> Option<&'a str> {
        let (index, _) = self.find_key(key)?;
        mapping_inline_value(self.lines[index].trim())
    }

    fn scalar_or_folded(&self, key: &str) -> Option<String> {
        let (index, children_end) = self.find_key(key)?;
        if let Some(value) = mapping_inline_value(self.lines[index].trim()) {
            return Some(value.to_owned());
        }
        mapping_block_indicator(self.lines[index].trim())?;
        folded_block(&self.lines, index + 1, children_end)
    }

    fn find_key(&self, key: &str) -> Option<(usize, usize)> {
        find_mapping_key(&self.lines, self.begin, self.end, self.key_indent, key)
    }
}

fn yaml_string_sequence<'a>(
    document: &'a str,
    path: &[&str],
    sequence_key: &str,
) -> Option<Vec<&'a str>> {
    let mapping = yaml_mapping(document, path)?;
    let (index, children_end) = mapping.find_key(sequence_key)?;
    if mapping_inline_value(mapping.lines[index].trim()).is_some() {
        return None;
    }
    let children_begin = index + 1;
    let child_indent = min_significant_indent(&mapping.lines, children_begin, children_end)?;
    let mut values = Vec::new();
    for line in &mapping.lines[children_begin..children_end] {
        let Some((indent, trimmed)) = significant_line(line) else {
            continue;
        };
        if indent != child_indent {
            continue;
        }
        values.push(sequence_scalar(trimmed)?);
    }
    Some(values)
}

fn yaml_mapping<'a>(document: &'a str, path: &[&str]) -> Option<YamlMapping<'a>> {
    let lines: Vec<&str> = document.lines().collect();
    let mut begin = 0;
    let mut end = lines.len();
    let mut key_indent = 0;
    for key in path {
        let (index, children_end) = find_mapping_key(&lines, begin, end, key_indent, key)?;
        if mapping_inline_value(lines[index].trim()).is_some() {
            return None;
        }
        begin = index + 1;
        end = children_end;
        key_indent = min_significant_indent(&lines, begin, end)?;
    }
    Some(YamlMapping {
        lines,
        begin,
        end,
        key_indent,
    })
}

fn find_mapping_key(
    lines: &[&str],
    begin: usize,
    end: usize,
    key_indent: usize,
    key: &str,
) -> Option<(usize, usize)> {
    let mut index = begin;
    while index < end {
        let Some((indent, trimmed)) = significant_line(lines[index]) else {
            index += 1;
            continue;
        };
        if indent < key_indent {
            break;
        }
        if indent > key_indent {
            index += 1;
            continue;
        }
        if mapping_key(trimmed) != Some(key) {
            index += 1;
            continue;
        }
        let mut children_end = index + 1;
        while children_end < end {
            if let Some((child_indent, _)) = significant_line(lines[children_end])
                && child_indent <= key_indent
            {
                break;
            }
            children_end += 1;
        }
        return Some((index, children_end));
    }
    None
}

fn min_significant_indent(lines: &[&str], begin: usize, end: usize) -> Option<usize> {
    lines[begin..end]
        .iter()
        .filter_map(|line| significant_line(line).map(|(indent, _)| indent))
        .min()
}

fn significant_line(line: &str) -> Option<(usize, &str)> {
    let indent = line.len() - line.trim_start_matches(' ').len();
    let trimmed = line[indent..].trim_end();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        None
    } else {
        Some((indent, trimmed))
    }
}

fn mapping_key(trimmed: &str) -> Option<&str> {
    if trimmed.starts_with('-') {
        return None;
    }
    let (key, _) = trimmed.split_once(':')?;
    let key = unquote(key);
    if is_simple_yaml_key(key) {
        Some(key)
    } else {
        None
    }
}

fn mapping_inline_value(trimmed: &str) -> Option<&str> {
    let (key, rest) = trimmed.split_once(':')?;
    if !is_simple_yaml_key(unquote(key)) {
        return None;
    }
    let value = rest.trim();
    if value.is_empty() || matches!(value, ">" | "|" | ">-" | "|-" | ">+" | "|+") {
        None
    } else {
        Some(unquote(value))
    }
}

fn sequence_scalar(trimmed: &str) -> Option<&str> {
    let value = trimmed.strip_prefix("- ")?.trim();
    if value.is_empty() {
        None
    } else {
        Some(unquote(value))
    }
}

fn mapping_block_indicator(trimmed: &str) -> Option<&str> {
    let (_, rest) = trimmed.split_once(':')?;
    let value = rest.trim();
    matches!(value, ">" | "|" | ">-" | "|-" | ">+" | "|+").then_some(value)
}

fn folded_block(lines: &[&str], begin: usize, end: usize) -> Option<String> {
    let mut parts = Vec::new();
    for line in &lines[begin..end] {
        let Some((_, trimmed)) = significant_line(line) else {
            continue;
        };
        parts.push(trimmed);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" "))
    }
}

fn is_simple_yaml_key(key: &str) -> bool {
    !key.is_empty()
        && key.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '$' | '/' | '.')
        })
}

fn unquote(value: &str) -> &str {
    let bytes = value.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AUXILIARY_SOURCE_HEALTH, AUXILIARY_SOURCE_QUALIFICATION, CAPTURE_SOURCE_HEALTH,
        COMMITTED_SOURCE_CLASSES, CORE_DEADLETTER_REASON_CODES, FAILOVER_REASONS,
        LEDGER_UNSUPPORTED_EVENT_REASON_CODES, READYZ_200_DESCRIPTION, READYZ_503_DESCRIPTION,
        READYZ_GET_DESCRIPTION, RESTART_RECONSTRUCTION,
        auxiliary_source_cursor_epoch_is_optional_string,
        auxiliary_source_durable_offset_is_optional_u64, auxiliary_source_health_openapi_enum,
        auxiliary_source_id_is_required_string,
        auxiliary_source_items_forbid_additional_properties,
        auxiliary_source_last_durable_wall_micros_is_optional_i64,
        auxiliary_source_last_error_reason_is_optional_string,
        auxiliary_source_local_sequence_is_optional_u64,
        auxiliary_source_partial_line_is_required_bool,
        auxiliary_source_qualification_openapi_enum,
        auxiliary_source_quarantine_reason_is_optional_string,
        auxiliary_source_spool_records_is_required_u64,
        auxiliary_source_tail_cursor_epoch_is_optional_string,
        auxiliary_source_unarchived_records_is_required_u64,
        auxiliary_source_unread_bytes_is_optional_u64, auxiliary_sources_max_items_is_writer_cap,
        capture_source_health_openapi_enum, capture_status_durable_height_is_optional_u64,
        capture_status_failover_height_is_optional_u64,
        capture_status_failover_reason_is_optional_enum,
        capture_status_failover_reason_openapi_enum,
        capture_status_last_error_reason_is_optional_string, committed_source_class_openapi_enum,
        core_deadletter_reason_openapi_enum, health_503_response_ref, health_503_schema_ref,
        health_reason_code_is_unrestricted_string, independent_source_health_openapi_enum,
        ledger_unsupported_event_reason_openapi_enum, openapi_yaml, readyz_200_description,
        readyz_200_schema_ref, readyz_503_description, readyz_503_schema_ref,
        readyz_get_description, restart_reconstruction_openapi_enum,
        unavailable_response_schema_ref,
    };

    #[test]
    fn checked_in_openapi_enum_matches_frozen_const() {
        let document = openapi_yaml();
        let values = core_deadletter_reason_openapi_enum(document)
            .expect("OpenAPI must define components.schemas.CoreDeadLetterReasonCode.enum");
        assert_eq!(values, CORE_DEADLETTER_REASON_CODES);
        let ledger_values = ledger_unsupported_event_reason_openapi_enum(document)
            .expect("OpenAPI must define components.schemas.LedgerUnsupportedEventReasonCode.enum");
        assert_eq!(ledger_values, LEDGER_UNSUPPORTED_EVENT_REASON_CODES);
        let committed_values = committed_source_class_openapi_enum(document).expect(
            "OpenAPI must define CaptureStatusBase.properties.active_committed_source.enum",
        );
        assert_eq!(committed_values, COMMITTED_SOURCE_CLASSES);
        let source_health_values = capture_source_health_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.properties.primary_source_health.enum");
        assert_eq!(source_health_values, CAPTURE_SOURCE_HEALTH);
        let independent_health_values = independent_source_health_openapi_enum(document).expect(
            "OpenAPI must define CaptureStatusBase.properties.independent_source_health.enum",
        );
        assert_eq!(independent_health_values, CAPTURE_SOURCE_HEALTH);
        let failover_reason_values = capture_status_failover_reason_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.properties.failover_reason.enum");
        assert_eq!(failover_reason_values, FAILOVER_REASONS);
        let reconstruction_values = restart_reconstruction_openapi_enum(document).expect(
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.restart_reconstruction.enum",
        );
        assert_eq!(reconstruction_values, RESTART_RECONSTRUCTION);
        let auxiliary_health_values = auxiliary_source_health_openapi_enum(document)
            .expect("OpenAPI must define CaptureStatusBase.auxiliary_sources.items.health.enum");
        assert_eq!(auxiliary_health_values, AUXILIARY_SOURCE_HEALTH);
        assert_ne!(auxiliary_health_values.as_slice(), CAPTURE_SOURCE_HEALTH);
        let qualification_values = auxiliary_source_qualification_openapi_enum(document).expect(
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.qualification.enum",
        );
        assert_eq!(qualification_values, AUXILIARY_SOURCE_QUALIFICATION);
        assert!(
            auxiliary_source_id_is_required_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.source_id as a required string"
        );
        assert!(
            auxiliary_source_spool_records_is_required_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.spool_records as a required u64 integer"
        );
        assert!(
            auxiliary_source_unarchived_records_is_required_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.unarchived_records as a required u64 integer"
        );
        assert!(
            auxiliary_source_partial_line_is_required_bool(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.partial_line as a required boolean"
        );
        assert!(
            auxiliary_source_cursor_epoch_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.cursor_epoch as an optional string"
        );
        assert!(
            auxiliary_source_tail_cursor_epoch_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.tail_cursor_epoch as an optional string"
        );
        assert!(
            auxiliary_source_durable_offset_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.durable_offset as an optional u64 integer"
        );
        assert!(
            auxiliary_source_local_sequence_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.local_sequence as an optional u64 integer"
        );
        assert!(
            auxiliary_source_unread_bytes_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.unread_bytes as an optional u64 integer"
        );
        assert!(
            auxiliary_source_last_durable_wall_micros_is_optional_i64(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.last_durable_wall_micros as an optional i64 integer"
        );
        assert!(
            auxiliary_source_quarantine_reason_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.quarantine_reason as an optional string"
        );
        assert!(
            auxiliary_source_last_error_reason_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.items.last_error_reason as an optional string"
        );
        assert!(
            capture_status_last_error_reason_is_optional_string(document),
            "OpenAPI must define CaptureStatusBase.last_error_reason as an optional string"
        );
        assert!(
            capture_status_failover_height_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.failover_height as an optional u64 integer"
        );
        assert!(
            capture_status_failover_reason_is_optional_enum(document),
            "OpenAPI must define CaptureStatusBase.failover_reason as an optional kebab-case enum"
        );
        assert!(
            capture_status_durable_height_is_optional_u64(document),
            "OpenAPI must define CaptureStatusBase.durable_height as an optional u64 integer"
        );
        assert!(
            auxiliary_source_items_forbid_additional_properties(document),
            "OpenAPI must set CaptureStatusBase.auxiliary_sources.items.additionalProperties false"
        );
        assert!(
            auxiliary_sources_max_items_is_writer_cap(document),
            "OpenAPI must define CaptureStatusBase.auxiliary_sources.maxItems as the capture writer cap"
        );
        assert!(
            document.contains("Duplicate present source_id"),
            "OpenAPI must describe source_id uniqueness without uniqueItems"
        );
        assert!(
            document.contains("strictly increasing"),
            "OpenAPI must describe source_id sort order without uniqueItems"
        );
        assert!(
            document.contains("Present unknown nested properties"),
            "OpenAPI must describe nested extra keys as snapshot_invalid"
        );
        assert!(
            document.contains("not CaptureStatusBase additionalProperties"),
            "OpenAPI must not close CaptureStatusBase extra keys in this leftover"
        );
        assert!(
            !document.contains("Sort order stays untyped"),
            "OpenAPI must not leave source_id sort order untyped"
        );
        assert!(health_reason_code_is_unrestricted_string(document));
        assert!(
            document.contains("no inline enum"),
            "OpenAPI must freeze HealthAssessment.reason_code without an inline enum"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_deadletter_enum_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        state:
          enum:
            - HEALTH_STATE_RED
        reason_code:
          type: string
          description: >
            core.deadletter_corrupt remains in prose after the YAML enum
            drops it. core.deadletter_serialization is listed here too.
    CoreDeadLetterReasonCode:
      description: >
        core.deadletter_corrupt is only in this description
      type: string
      enum:
        - core.deadletter_unsafe_path
        - core.deadletter_io
        - core.deadletter_invalid_record
        - core.deadletter_serialization
"#;
        let values = core_deadletter_reason_openapi_enum(document)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(
            values,
            &[
                "core.deadletter_unsafe_path",
                "core.deadletter_io",
                "core.deadletter_invalid_record",
                "core.deadletter_serialization",
            ]
        );
        assert!(
            !values.contains(&"core.deadletter_corrupt"),
            "prose must not count as an enum value"
        );
        assert_ne!(
            values.as_slice(),
            CORE_DEADLETTER_REASON_CODES,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_ledger_unsupported_event_enum_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
          description: >
            ledger.unsupported_event remains in prose after the YAML enum
            drops it.
    LedgerUnsupportedEventReasonCode:
      description: >
        ledger.unsupported_event is only in this description
      type: string
      enum:
        - ledger.placeholder
"#;
        let values = ledger_unsupported_event_reason_openapi_enum(document)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(values, &["ledger.placeholder"]);
        assert!(
            !values.contains(&"ledger.unsupported_event"),
            "prose must not count as an enum value"
        );
        assert_ne!(
            values.as_slice(),
            LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_committed_source_class_enum_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        active_committed_source:
          description: >
            independent-committed remains in prose after the YAML enum
            drops it.
          type: string
          enum:
            - locally-verified-committed
"#;
        let values = committed_source_class_openapi_enum(document)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(values, &["locally-verified-committed"]);
        assert!(
            !values.contains(&"independent-committed"),
            "prose must not count as an enum value"
        );
        assert_ne!(
            values.as_slice(),
            COMMITTED_SOURCE_CLASSES,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );
        assert!(health_reason_code_is_unrestricted_string(document));
    }

    #[test]
    fn prose_mention_does_not_satisfy_capture_source_health_enum_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        primary_source_health:
          description: >
            range-unavailable remains in prose after the YAML enum
            drops it.
          type: string
          enum:
            - starting
            - healthy
        independent_source_health:
          description: >
            range-unavailable remains in prose after the YAML enum
            drops it.
          type: string
          enum:
            - starting
            - healthy
"#;
        let values = capture_source_health_openapi_enum(document)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(values, &["starting", "healthy"]);
        assert!(
            !values.contains(&"range-unavailable"),
            "prose must not count as an enum value"
        );
        assert_ne!(
            values.as_slice(),
            CAPTURE_SOURCE_HEALTH,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );
        let independent = independent_source_health_openapi_enum(document)
            .expect("synthetic independent schema must still parse the YAML enum");
        assert_eq!(independent, &["starting", "healthy"]);
        assert_ne!(independent.as_slice(), CAPTURE_SOURCE_HEALTH);
        assert!(health_reason_code_is_unrestricted_string(document));
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_source_health_enum_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            latched remains in prose after the YAML enum drops it.
          type: array
          items:
            type: object
            properties:
              health:
                description: >
                  latched and range-unavailable remain in prose after the
                  YAML enum drops them.
                type: string
                enum:
                  - starting
                  - healthy
                  - quarantined
"#;
        let values = auxiliary_source_health_openapi_enum(document)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(values, &["starting", "healthy", "quarantined"]);
        assert!(
            !values.contains(&"latched"),
            "prose must not count as an enum value"
        );
        assert!(
            !values.contains(&"range-unavailable"),
            "committed range-unavailable must not count as auxiliary health"
        );
        assert_ne!(
            values.as_slice(),
            AUXILIARY_SOURCE_HEALTH,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );
        assert_ne!(values.as_slice(), CAPTURE_SOURCE_HEALTH);
        assert!(health_reason_code_is_unrestricted_string(document));
    }

    #[test]
    fn prose_mention_does_not_satisfy_restart_reconstruction_enum_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            complete remains in prose after the YAML enum drops it.
          type: array
          items:
            type: object
            properties:
              restart_reconstruction:
                description: >
                  complete remains in prose after the YAML enum drops it.
                type: string
                enum:
                  - not-required
                  - incomplete
"#;
        let values = restart_reconstruction_openapi_enum(document)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(values, &["not-required", "incomplete"]);
        assert!(
            !values.contains(&"complete"),
            "prose must not count as an enum value"
        );
        assert_ne!(
            values.as_slice(),
            RESTART_RECONSTRUCTION,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );
        assert!(health_reason_code_is_unrestricted_string(document));
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_source_qualification_enum_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            qualified remains in prose after the YAML enum drops it.
          type: array
          items:
            type: object
            properties:
              qualification:
                description: >
                  qualified remains in prose after the YAML enum drops it.
                type: string
                enum:
                  - unqualified
"#;
        let values = auxiliary_source_qualification_openapi_enum(document)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(values, &["unqualified"]);
        assert!(
            !values.contains(&"qualified"),
            "prose must not count as an enum value"
        );
        assert_ne!(
            values.as_slice(),
            AUXILIARY_SOURCE_QUALIFICATION,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );
        assert!(health_reason_code_is_unrestricted_string(document));
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_source_id_required_string_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            source_id remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            properties:
              health:
                type: string
"#;
        assert!(
            !auxiliary_source_id_is_required_string(prose_only),
            "prose mention of source_id must not satisfy the required-string freeze"
        );

        let optional_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              source_id:
                type: string
"#;
        assert!(
            !auxiliary_source_id_is_required_string(optional_string),
            "optional source_id must not satisfy the required-string freeze"
        );

        let integer_id = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
            properties:
              source_id:
                type: integer
"#;
        assert!(
            !auxiliary_source_id_is_required_string(integer_id),
            "required non-string source_id must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
            properties:
              source_id:
                type: string
                format: uuid
"#;
        assert!(
            !auxiliary_source_id_is_required_string(formatted),
            "invented source_id format must not satisfy the freeze"
        );

        let closed_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
            properties:
              source_id:
                type: string
                enum:
                  - node-misc-events
"#;
        assert!(
            !auxiliary_source_id_is_required_string(closed_enum),
            "invented source_id enum must not satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_spool_records_required_u64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            spool_records remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
            properties:
              source_id:
                type: string
"#;
        assert!(
            !auxiliary_source_spool_records_is_required_u64(prose_only),
            "prose mention of spool_records must not satisfy the required-u64 freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
            properties:
              spool_records:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_spool_records_is_required_u64(optional_integer),
            "optional spool_records must not satisfy the required-u64 freeze"
        );

        let string_count = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - spool_records
            properties:
              spool_records:
                type: string
"#;
        assert!(
            !auxiliary_source_spool_records_is_required_u64(string_count),
            "required non-integer spool_records must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - spool_records
            properties:
              spool_records:
                type: integer
                minimum: 0
                format: int64
"#;
        assert!(
            !auxiliary_source_spool_records_is_required_u64(formatted),
            "invented spool_records format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - spool_records
            properties:
              spool_records:
                type: integer
                minimum: 0
                maximum: 100
"#;
        assert!(
            !auxiliary_source_spool_records_is_required_u64(bounded),
            "invented spool_records maximum must not satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_unarchived_records_required_u64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            unarchived_records remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_unarchived_records_is_required_u64(prose_only),
            "prose mention of unarchived_records must not satisfy the required-u64 freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
            properties:
              unarchived_records:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_unarchived_records_is_required_u64(optional_integer),
            "optional unarchived_records must not satisfy the required-u64 freeze"
        );

        let string_count = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - unarchived_records
            properties:
              unarchived_records:
                type: string
"#;
        assert!(
            !auxiliary_source_unarchived_records_is_required_u64(string_count),
            "required non-integer unarchived_records must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - unarchived_records
            properties:
              unarchived_records:
                type: integer
                minimum: 0
                format: int64
"#;
        assert!(
            !auxiliary_source_unarchived_records_is_required_u64(formatted),
            "invented unarchived_records format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - unarchived_records
            properties:
              unarchived_records:
                type: integer
                minimum: 0
                maximum: 100
"#;
        assert!(
            !auxiliary_source_unarchived_records_is_required_u64(bounded),
            "invented unarchived_records maximum must not satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_partial_line_required_bool_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            partial_line remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_partial_line_is_required_bool(prose_only),
            "prose mention of partial_line must not satisfy the required-bool freeze"
        );

        let optional_boolean = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
            properties:
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_partial_line_is_required_bool(optional_boolean),
            "optional partial_line must not satisfy the required-bool freeze"
        );

        let string_flag = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - partial_line
            properties:
              partial_line:
                type: string
"#;
        assert!(
            !auxiliary_source_partial_line_is_required_bool(string_flag),
            "required non-boolean partial_line must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - partial_line
            properties:
              partial_line:
                type: boolean
                format: 0-1
"#;
        assert!(
            !auxiliary_source_partial_line_is_required_bool(formatted),
            "invented partial_line format must not satisfy the freeze"
        );

        let enumerated = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - partial_line
            properties:
              partial_line:
                type: boolean
                enum:
                  - true
                  - false
"#;
        assert!(
            !auxiliary_source_partial_line_is_required_bool(enumerated),
            "invented partial_line enum must not satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_cursor_epoch_optional_string_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            cursor_epoch remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_cursor_epoch_is_optional_string(prose_only),
            "prose mention of cursor_epoch must not satisfy the optional-string freeze"
        );

        let required_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - cursor_epoch
            properties:
              cursor_epoch:
                type: string
"#;
        assert!(
            !auxiliary_source_cursor_epoch_is_optional_string(required_string),
            "required cursor_epoch must not satisfy the optional-string freeze"
        );

        let integer_epoch = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              cursor_epoch:
                type: integer
"#;
        assert!(
            !auxiliary_source_cursor_epoch_is_optional_string(integer_epoch),
            "optional non-string cursor_epoch must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              cursor_epoch:
                type: string
                format: uuid
"#;
        assert!(
            !auxiliary_source_cursor_epoch_is_optional_string(formatted),
            "invented cursor_epoch format must not satisfy the freeze"
        );

        let closed_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              cursor_epoch:
                type: string
                enum:
                  - node-file-v1:epoch
"#;
        assert!(
            !auxiliary_source_cursor_epoch_is_optional_string(closed_enum),
            "invented cursor_epoch enum must not satisfy the freeze"
        );

        let optional_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              cursor_epoch:
                type: string
"#;
        assert!(
            auxiliary_source_cursor_epoch_is_optional_string(optional_string),
            "optional string cursor_epoch must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_tail_cursor_epoch_optional_string_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            tail_cursor_epoch remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_tail_cursor_epoch_is_optional_string(prose_only),
            "prose mention of tail_cursor_epoch must not satisfy the optional-string freeze"
        );

        let required_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - tail_cursor_epoch
            properties:
              tail_cursor_epoch:
                type: string
"#;
        assert!(
            !auxiliary_source_tail_cursor_epoch_is_optional_string(required_string),
            "required tail_cursor_epoch must not satisfy the optional-string freeze"
        );

        let integer_epoch = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              tail_cursor_epoch:
                type: integer
"#;
        assert!(
            !auxiliary_source_tail_cursor_epoch_is_optional_string(integer_epoch),
            "optional non-string tail_cursor_epoch must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              tail_cursor_epoch:
                type: string
                format: uuid
"#;
        assert!(
            !auxiliary_source_tail_cursor_epoch_is_optional_string(formatted),
            "invented tail_cursor_epoch format must not satisfy the freeze"
        );

        let closed_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              tail_cursor_epoch:
                type: string
                enum:
                  - node-file-v1:epoch
"#;
        assert!(
            !auxiliary_source_tail_cursor_epoch_is_optional_string(closed_enum),
            "invented tail_cursor_epoch enum must not satisfy the freeze"
        );

        let optional_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              tail_cursor_epoch:
                type: string
"#;
        assert!(
            auxiliary_source_tail_cursor_epoch_is_optional_string(optional_string),
            "optional string tail_cursor_epoch must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_durable_offset_optional_u64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            durable_offset remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_durable_offset_is_optional_u64(prose_only),
            "prose mention of durable_offset must not satisfy the optional-u64 freeze"
        );

        let required_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - durable_offset
            properties:
              durable_offset:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_durable_offset_is_optional_u64(required_integer),
            "required durable_offset must not satisfy the optional-u64 freeze"
        );

        let string_offset = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              durable_offset:
                type: string
"#;
        assert!(
            !auxiliary_source_durable_offset_is_optional_u64(string_offset),
            "optional non-integer durable_offset must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              durable_offset:
                type: integer
                minimum: 0
                format: int64
"#;
        assert!(
            !auxiliary_source_durable_offset_is_optional_u64(formatted),
            "invented durable_offset format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              durable_offset:
                type: integer
                minimum: 0
                maximum: 100
"#;
        assert!(
            !auxiliary_source_durable_offset_is_optional_u64(bounded),
            "invented durable_offset maximum must not satisfy the freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              durable_offset:
                type: integer
                minimum: 0
"#;
        assert!(
            auxiliary_source_durable_offset_is_optional_u64(optional_integer),
            "optional u64 durable_offset must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_local_sequence_optional_u64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            local_sequence remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_local_sequence_is_optional_u64(prose_only),
            "prose mention of local_sequence must not satisfy the optional-u64 freeze"
        );

        let required_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - local_sequence
            properties:
              local_sequence:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_local_sequence_is_optional_u64(required_integer),
            "required local_sequence must not satisfy the optional-u64 freeze"
        );

        let string_sequence = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              local_sequence:
                type: string
"#;
        assert!(
            !auxiliary_source_local_sequence_is_optional_u64(string_sequence),
            "optional non-integer local_sequence must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              local_sequence:
                type: integer
                minimum: 0
                format: int64
"#;
        assert!(
            !auxiliary_source_local_sequence_is_optional_u64(formatted),
            "invented local_sequence format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              local_sequence:
                type: integer
                minimum: 0
                maximum: 100
"#;
        assert!(
            !auxiliary_source_local_sequence_is_optional_u64(bounded),
            "invented local_sequence maximum must not satisfy the freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              local_sequence:
                type: integer
                minimum: 0
"#;
        assert!(
            auxiliary_source_local_sequence_is_optional_u64(optional_integer),
            "optional u64 local_sequence must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_unread_bytes_optional_u64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            unread_bytes remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_unread_bytes_is_optional_u64(prose_only),
            "prose mention of unread_bytes must not satisfy the optional-u64 freeze"
        );

        let required_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - unread_bytes
            properties:
              unread_bytes:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_unread_bytes_is_optional_u64(required_integer),
            "required unread_bytes must not satisfy the optional-u64 freeze"
        );

        let string_unread = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              unread_bytes:
                type: string
"#;
        assert!(
            !auxiliary_source_unread_bytes_is_optional_u64(string_unread),
            "optional non-integer unread_bytes must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              unread_bytes:
                type: integer
                minimum: 0
                format: int64
"#;
        assert!(
            !auxiliary_source_unread_bytes_is_optional_u64(formatted),
            "invented unread_bytes format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              unread_bytes:
                type: integer
                minimum: 0
                maximum: 100
"#;
        assert!(
            !auxiliary_source_unread_bytes_is_optional_u64(bounded),
            "invented unread_bytes maximum must not satisfy the freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              unread_bytes:
                type: integer
                minimum: 0
"#;
        assert!(
            auxiliary_source_unread_bytes_is_optional_u64(optional_integer),
            "optional u64 unread_bytes must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_last_durable_wall_micros_optional_i64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            last_durable_wall_micros remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_last_durable_wall_micros_is_optional_i64(prose_only),
            "prose mention of last_durable_wall_micros must not satisfy the optional-i64 freeze"
        );

        let required_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - last_durable_wall_micros
            properties:
              last_durable_wall_micros:
                type: integer
"#;
        assert!(
            !auxiliary_source_last_durable_wall_micros_is_optional_i64(required_integer),
            "required last_durable_wall_micros must not satisfy the optional-i64 freeze"
        );

        let string_wall = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_durable_wall_micros:
                type: string
"#;
        assert!(
            !auxiliary_source_last_durable_wall_micros_is_optional_i64(string_wall),
            "optional non-integer last_durable_wall_micros must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_durable_wall_micros:
                type: integer
                format: int64
"#;
        assert!(
            !auxiliary_source_last_durable_wall_micros_is_optional_i64(formatted),
            "invented last_durable_wall_micros format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_durable_wall_micros:
                type: integer
                maximum: 100
"#;
        assert!(
            !auxiliary_source_last_durable_wall_micros_is_optional_i64(bounded),
            "invented last_durable_wall_micros maximum must not satisfy the freeze"
        );

        let copied_u64_minimum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_durable_wall_micros:
                type: integer
                minimum: 0
"#;
        assert!(
            !auxiliary_source_last_durable_wall_micros_is_optional_i64(copied_u64_minimum),
            "copied u64 minimum: 0 must not satisfy the optional-i64 freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              last_durable_wall_micros:
                type: integer
"#;
        assert!(
            auxiliary_source_last_durable_wall_micros_is_optional_i64(optional_integer),
            "optional i64 last_durable_wall_micros must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_quarantine_reason_optional_string_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            quarantine_reason remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_quarantine_reason_is_optional_string(prose_only),
            "prose mention of quarantine_reason must not satisfy the optional-string freeze"
        );

        let required_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - quarantine_reason
            properties:
              quarantine_reason:
                type: string
"#;
        assert!(
            !auxiliary_source_quarantine_reason_is_optional_string(required_string),
            "required quarantine_reason must not satisfy the optional-string freeze"
        );

        let integer_reason = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              quarantine_reason:
                type: integer
"#;
        assert!(
            !auxiliary_source_quarantine_reason_is_optional_string(integer_reason),
            "optional non-string quarantine_reason must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              quarantine_reason:
                type: string
                format: uuid
"#;
        assert!(
            !auxiliary_source_quarantine_reason_is_optional_string(formatted),
            "invented quarantine_reason format must not satisfy the freeze"
        );

        let closed_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              quarantine_reason:
                type: string
                enum:
                  - source.schema_drift
"#;
        assert!(
            !auxiliary_source_quarantine_reason_is_optional_string(closed_enum),
            "invented quarantine_reason enum must not satisfy the freeze"
        );

        let optional_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              quarantine_reason:
                type: string
"#;
        assert!(
            auxiliary_source_quarantine_reason_is_optional_string(optional_string),
            "optional string quarantine_reason must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_last_error_reason_optional_string_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            last_error_reason remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              source_id:
                type: string
              spool_records:
                type: integer
                minimum: 0
              unarchived_records:
                type: integer
                minimum: 0
              partial_line:
                type: boolean
"#;
        assert!(
            !auxiliary_source_last_error_reason_is_optional_string(prose_only),
            "prose mention of last_error_reason must not satisfy the optional-string freeze"
        );

        let required_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - last_error_reason
            properties:
              last_error_reason:
                type: string
"#;
        assert!(
            !auxiliary_source_last_error_reason_is_optional_string(required_string),
            "required last_error_reason must not satisfy the optional-string freeze"
        );

        let integer_reason = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_error_reason:
                type: integer
"#;
        assert!(
            !auxiliary_source_last_error_reason_is_optional_string(integer_reason),
            "optional non-string last_error_reason must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_error_reason:
                type: string
                format: uuid
"#;
        assert!(
            !auxiliary_source_last_error_reason_is_optional_string(formatted),
            "invented last_error_reason format must not satisfy the freeze"
        );

        let closed_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_error_reason:
                type: string
                enum:
                  - source.temporary_disconnect
"#;
        assert!(
            !auxiliary_source_last_error_reason_is_optional_string(closed_enum),
            "invented last_error_reason enum must not satisfy the freeze"
        );

        let optional_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            required:
              - source_id
              - spool_records
              - unarchived_records
              - partial_line
            properties:
              last_error_reason:
                type: string
"#;
        assert!(
            auxiliary_source_last_error_reason_is_optional_string(optional_string),
            "optional string last_error_reason must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_top_level_last_error_reason_optional_string_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        pending_blocks:
          description: >
            last_error_reason remains in prose after the YAML property drops it.
          type: integer
          minimum: 0
"#;
        assert!(
            !capture_status_last_error_reason_is_optional_string(prose_only),
            "prose mention of last_error_reason must not satisfy the optional-string freeze"
        );

        let nested_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - pending_blocks
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              last_error_reason:
                type: string
"#;
        assert!(
            !capture_status_last_error_reason_is_optional_string(nested_only),
            "nested last_error_reason must not satisfy the top-level optional-string freeze"
        );

        let required_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - last_error_reason
      properties:
        last_error_reason:
          type: string
"#;
        assert!(
            !capture_status_last_error_reason_is_optional_string(required_string),
            "required last_error_reason must not satisfy the optional-string freeze"
        );

        let integer_reason = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        last_error_reason:
          type: integer
"#;
        assert!(
            !capture_status_last_error_reason_is_optional_string(integer_reason),
            "optional non-string last_error_reason must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        last_error_reason:
          type: string
          format: uuid
"#;
        assert!(
            !capture_status_last_error_reason_is_optional_string(formatted),
            "invented last_error_reason format must not satisfy the freeze"
        );

        let closed_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        last_error_reason:
          type: string
          enum:
            - capture_bus.unavailable
"#;
        assert!(
            !capture_status_last_error_reason_is_optional_string(closed_enum),
            "invented last_error_reason enum must not satisfy the freeze"
        );

        let optional_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - schema_version
        - pending_blocks
      properties:
        last_error_reason:
          type: string
"#;
        assert!(
            capture_status_last_error_reason_is_optional_string(optional_string),
            "optional string last_error_reason must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_top_level_failover_height_optional_u64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        pending_blocks:
          description: >
            failover_height remains in prose after the YAML property drops it.
          type: integer
          minimum: 0
"#;
        assert!(
            !capture_status_failover_height_is_optional_u64(prose_only),
            "prose mention of failover_height must not satisfy the optional-u64 freeze"
        );

        let nested_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - pending_blocks
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              failover_height:
                type: integer
                minimum: 0
"#;
        assert!(
            !capture_status_failover_height_is_optional_u64(nested_only),
            "nested failover_height must not satisfy the top-level optional-u64 freeze"
        );

        let required_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - failover_height
      properties:
        failover_height:
          type: integer
          minimum: 0
"#;
        assert!(
            !capture_status_failover_height_is_optional_u64(required_integer),
            "required failover_height must not satisfy the optional-u64 freeze"
        );

        let string_height = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        failover_height:
          type: string
"#;
        assert!(
            !capture_status_failover_height_is_optional_u64(string_height),
            "optional non-integer failover_height must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        failover_height:
          type: integer
          minimum: 0
          format: int64
"#;
        assert!(
            !capture_status_failover_height_is_optional_u64(formatted),
            "invented failover_height format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        failover_height:
          type: integer
          minimum: 0
          maximum: 100
"#;
        assert!(
            !capture_status_failover_height_is_optional_u64(bounded),
            "invented failover_height maximum must not satisfy the freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - schema_version
        - pending_blocks
      properties:
        failover_height:
          type: integer
          minimum: 0
"#;
        assert!(
            capture_status_failover_height_is_optional_u64(optional_integer),
            "optional u64 failover_height must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_top_level_failover_reason_optional_enum_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        pending_blocks:
          description: >
            failover_reason remains in prose after the YAML property drops it.
          type: integer
          minimum: 0
"#;
        assert!(
            capture_status_failover_reason_openapi_enum(prose_only).is_none(),
            "prose mention of failover_reason must not satisfy the enum freeze"
        );
        assert!(
            !capture_status_failover_reason_is_optional_enum(prose_only),
            "prose mention of failover_reason must not satisfy the optional-enum freeze"
        );

        let nested_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - pending_blocks
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              failover_reason:
                type: string
                enum:
                  - primary-range-unavailable
"#;
        assert!(
            capture_status_failover_reason_openapi_enum(nested_only).is_none(),
            "nested failover_reason must not satisfy the top-level enum freeze"
        );
        assert!(
            !capture_status_failover_reason_is_optional_enum(nested_only),
            "nested failover_reason must not satisfy the top-level optional-enum freeze"
        );

        let required_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - failover_reason
      properties:
        failover_reason:
          type: string
          enum:
            - primary-range-unavailable
"#;
        assert_eq!(
            capture_status_failover_reason_openapi_enum(required_enum).as_deref(),
            Some(FAILOVER_REASONS)
        );
        assert!(
            !capture_status_failover_reason_is_optional_enum(required_enum),
            "required failover_reason must not satisfy the optional-enum freeze"
        );

        let free_string = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        failover_reason:
          type: string
"#;
        assert!(
            capture_status_failover_reason_openapi_enum(free_string).is_none(),
            "optional free-string failover_reason must not satisfy the enum freeze"
        );
        assert!(
            !capture_status_failover_reason_is_optional_enum(free_string),
            "optional free-string failover_reason must not satisfy the optional-enum freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        failover_reason:
          type: string
          format: uuid
          enum:
            - primary-range-unavailable
"#;
        assert_eq!(
            capture_status_failover_reason_openapi_enum(formatted).as_deref(),
            Some(FAILOVER_REASONS)
        );
        assert!(
            !capture_status_failover_reason_is_optional_enum(formatted),
            "invented failover_reason format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        failover_reason:
          type: string
          minLength: 1
          enum:
            - primary-range-unavailable
"#;
        assert_eq!(
            capture_status_failover_reason_openapi_enum(bounded).as_deref(),
            Some(FAILOVER_REASONS)
        );
        assert!(
            !capture_status_failover_reason_is_optional_enum(bounded),
            "invented failover_reason minLength must not satisfy the freeze"
        );

        let shrunk = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        failover_reason:
          description: >
            primary-range-unavailable remains in prose after the YAML enum
            drops it.
          type: string
          enum:
            - range-unavailable
"#;
        let values = capture_status_failover_reason_openapi_enum(shrunk)
            .expect("synthetic schema must still parse the YAML enum");
        assert_eq!(values, &["range-unavailable"]);
        assert!(
            !values.contains(&"primary-range-unavailable"),
            "prose must not count as an enum value"
        );
        assert_ne!(
            values.as_slice(),
            FAILOVER_REASONS,
            "shrinking the YAML enum without shrinking the const must fail the freeze"
        );

        let optional_enum = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - schema_version
        - pending_blocks
      properties:
        failover_reason:
          type: string
          enum:
            - primary-range-unavailable
"#;
        assert_eq!(
            capture_status_failover_reason_openapi_enum(optional_enum).as_deref(),
            Some(FAILOVER_REASONS)
        );
        assert!(
            capture_status_failover_reason_is_optional_enum(optional_enum),
            "optional kebab-case failover_reason enum must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_top_level_durable_height_optional_u64_freeze() {
        let prose_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        pending_blocks:
          description: >
            durable_height remains in prose after the YAML property drops it.
          type: integer
          minimum: 0
"#;
        assert!(
            !capture_status_durable_height_is_optional_u64(prose_only),
            "prose mention of durable_height must not satisfy the optional-u64 freeze"
        );

        let nested_only = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - pending_blocks
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            properties:
              durable_offset:
                type: integer
                minimum: 0
              durable_height:
                type: integer
                minimum: 0
"#;
        assert!(
            !capture_status_durable_height_is_optional_u64(nested_only),
            "nested durable_height or durable_offset must not satisfy the top-level optional-u64 freeze"
        );

        let required_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - durable_height
      properties:
        durable_height:
          type: integer
          minimum: 0
"#;
        assert!(
            !capture_status_durable_height_is_optional_u64(required_integer),
            "required durable_height must not satisfy the optional-u64 freeze"
        );

        let string_height = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        durable_height:
          type: string
"#;
        assert!(
            !capture_status_durable_height_is_optional_u64(string_height),
            "optional non-integer durable_height must not satisfy the freeze"
        );

        let formatted = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        durable_height:
          type: integer
          minimum: 0
          format: int64
"#;
        assert!(
            !capture_status_durable_height_is_optional_u64(formatted),
            "invented durable_height format must not satisfy the freeze"
        );

        let bounded = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      properties:
        durable_height:
          type: integer
          minimum: 0
          maximum: 100
"#;
        assert!(
            !capture_status_durable_height_is_optional_u64(bounded),
            "invented durable_height maximum must not satisfy the freeze"
        );

        let optional_integer = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
    CaptureStatusBase:
      required:
        - schema_version
        - pending_blocks
      properties:
        durable_height:
          type: integer
          minimum: 0
"#;
        assert!(
            capture_status_durable_height_is_optional_u64(optional_integer),
            "optional u64 durable_height must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_sources_max_items_freeze() {
        let prose_only = r#"
components:
  schemas:
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          description: >
            maxItems 16 remains in prose after the YAML property drops it.
          type: array
          items:
            type: object
"#;
        assert!(
            !auxiliary_sources_max_items_is_writer_cap(prose_only),
            "prose mention of maxItems 16 must not satisfy the writer-cap freeze"
        );

        let wrong_cap = r#"
components:
  schemas:
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          maxItems: 17
          items:
            type: object
"#;
        assert!(
            !auxiliary_sources_max_items_is_writer_cap(wrong_cap),
            "invented maxItems must not satisfy the writer-cap freeze"
        );

        let min_items = r#"
components:
  schemas:
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          maxItems: 16
          minItems: 1
          items:
            type: object
"#;
        assert!(
            !auxiliary_sources_max_items_is_writer_cap(min_items),
            "minItems must not satisfy the writer-cap freeze; empty arrays stay valid"
        );

        let unique_items = r#"
components:
  schemas:
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          maxItems: 16
          uniqueItems: true
          items:
            type: object
"#;
        assert!(
            !auxiliary_sources_max_items_is_writer_cap(unique_items),
            "uniqueItems must not satisfy the writer-cap freeze; uniqueness is on source_id, not the whole item"
        );

        let writer_cap = r#"
components:
  schemas:
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          maxItems: 16
          items:
            type: object
"#;
        assert!(
            auxiliary_sources_max_items_is_writer_cap(writer_cap),
            "array maxItems matching the capture writer cap must satisfy the freeze"
        );
    }

    #[test]
    fn prose_mention_does_not_satisfy_auxiliary_source_additional_properties_freeze() {
        let prose_only = r#"
components:
  schemas:
    CaptureStatusBase:
      description: >
        additionalProperties false remains in prose after the YAML key drops it.
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
"#;
        assert!(
            !auxiliary_source_items_forbid_additional_properties(prose_only),
            "prose mention of additionalProperties false must not satisfy the freeze"
        );

        let allowed = r#"
components:
  schemas:
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            additionalProperties: true
"#;
        assert!(
            !auxiliary_source_items_forbid_additional_properties(allowed),
            "additionalProperties true must not satisfy the freeze"
        );

        let base_only = r#"
components:
  schemas:
    CaptureStatusBase:
      type: object
      additionalProperties: false
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
"#;
        assert!(
            !auxiliary_source_items_forbid_additional_properties(base_only),
            "CaptureStatusBase additionalProperties must not satisfy the nested items freeze"
        );

        let forbidden = r#"
components:
  schemas:
    CaptureStatusBase:
      properties:
        auxiliary_sources:
          type: array
          items:
            type: object
            additionalProperties: false
"#;
        assert!(
            auxiliary_source_items_forbid_additional_properties(forbidden),
            "items additionalProperties false must satisfy the freeze"
        );
    }

    #[test]
    fn reason_code_ref_to_closed_enum_fails_unrestricted_string_freeze() {
        let document = r##"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          $ref: "#/components/schemas/LedgerUnsupportedEventReasonCode"
    LedgerUnsupportedEventReasonCode:
      type: string
      enum:
        - ledger.unsupported_event
"##;
        assert!(
            !health_reason_code_is_unrestricted_string(document),
            "$ref of a closed enum into reason_code must fail so unknown RED stays typed"
        );
        let values = ledger_unsupported_event_reason_openapi_enum(document)
            .expect("closed enum remaining in components must still parse");
        assert_eq!(values, LEDGER_UNSUPPORTED_EVENT_REASON_CODES);
    }

    #[test]
    fn reason_code_inline_enum_fails_unrestricted_string_freeze() {
        let document = r#"
components:
  schemas:
    HealthAssessment:
      properties:
        reason_code:
          type: string
          enum:
            - ledger.unsupported_event
            - healthy
"#;
        assert!(
            !health_reason_code_is_unrestricted_string(document),
            "inline enum on reason_code must fail so unknown RED stays typed"
        );
    }

    #[test]
    fn readyz_503_openapi_is_health_not_api_error() {
        let document = openapi_yaml();
        assert_eq!(
            readyz_503_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "/readyz 503 must document hl.health.v1, not ApiError"
        );
        assert_eq!(
            health_503_response_ref(document),
            Some("#/components/responses/Unavailable"),
            "/v1/health 503 must $ref named Unavailable, not inline HealthAssessment"
        );
        assert!(
            health_503_schema_ref(document).is_none(),
            "/v1/health 503 must stay a named Unavailable $ref, not an inline schema"
        );
        assert_ne!(
            health_503_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "/v1/health 503 must not switch to HealthAssessment while the handler returns hl.api.error.v1"
        );
        assert_eq!(
            unavailable_response_schema_ref(document),
            Some("#/components/schemas/ApiError"),
            "shared Unavailable must stay ApiError for /v1/health 503"
        );
        assert_eq!(
            readyz_200_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "/readyz 200 must document hl.health.v1, not ApiError"
        );
        assert_eq!(
            readyz_200_description(document).as_deref(),
            Some(READYZ_200_DESCRIPTION),
            "/readyz 200 path description must stay GREEN-only by exact equality"
        );
        assert_eq!(
            readyz_503_description(document).as_deref(),
            Some(READYZ_503_DESCRIPTION),
            "/readyz 503 path description must stay health-not-ApiError by exact equality"
        );
        assert_eq!(
            readyz_get_description(document).as_deref(),
            Some(READYZ_GET_DESCRIPTION),
            "/readyz GET operation description must stay health-not-ApiError by exact equality"
        );
    }

    #[test]
    fn named_unavailable_ref_does_not_count_as_readyz_health_schema() {
        let document = r##"
paths:
  /readyz:
    get:
      responses:
        "503":
          $ref: "#/components/responses/Unavailable"
components:
  responses:
    Unavailable:
      content:
        application/json:
          schema:
            $ref: "#/components/schemas/ApiError"
"##;
        assert!(
            readyz_503_schema_ref(document).is_none(),
            "named Unavailable $ref must not satisfy the /readyz 503 health freeze"
        );
        assert_eq!(
            unavailable_response_schema_ref(document),
            Some("#/components/schemas/ApiError")
        );
    }

    #[test]
    fn inlined_readyz_503_api_error_fails_health_schema_freeze() {
        let document = r##"
paths:
  /readyz:
    get:
      responses:
        "503":
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/ApiError"
"##;
        assert_eq!(
            readyz_503_schema_ref(document),
            Some("#/components/schemas/ApiError")
        );
        assert_ne!(
            readyz_503_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "inlining ApiError on /readyz 503 must fail the health freeze"
        );
    }

    #[test]
    fn inlined_health_503_health_assessment_fails_unavailable_path_freeze() {
        let document = r##"
paths:
  /v1/health:
    get:
      responses:
        "503":
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/HealthAssessment"
components:
  responses:
    Unavailable:
      content:
        application/json:
          schema:
            $ref: "#/components/schemas/ApiError"
"##;
        assert_eq!(
            unavailable_response_schema_ref(document),
            Some("#/components/schemas/ApiError"),
            "shared Unavailable staying ApiError must not hide a /v1/health 503 path switch"
        );
        assert!(
            health_503_response_ref(document).is_none(),
            "inlined HealthAssessment must not satisfy the named Unavailable path freeze"
        );
        assert_ne!(
            health_503_response_ref(document),
            Some("#/components/responses/Unavailable"),
            "inlining HealthAssessment on /v1/health 503 must fail the Unavailable path freeze"
        );
        assert_eq!(
            health_503_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "inlining HealthAssessment on /v1/health 503 must fail while the handler returns hl.api.error.v1"
        );
    }

    #[test]
    fn readyz_503_api_error_prose_fails_health_not_api_error_freeze() {
        let document = r##"
paths:
  /readyz:
    get:
      responses:
        "503":
          description: >
            Aggregate is not GREEN. Body is ApiError.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/HealthAssessment"
"##;
        assert_eq!(
            readyz_503_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "schema freeze must not hide a /readyz 503 description rewrite"
        );
        assert_ne!(
            readyz_503_description(document).as_deref(),
            Some(READYZ_503_DESCRIPTION),
            "rewriting /readyz 503 prose to ApiError must fail the path description freeze"
        );
    }

    #[test]
    fn readyz_503_substring_prose_fails_exact_path_description_freeze() {
        let document = r##"
paths:
  /readyz:
    get:
      responses:
        "503":
          description: >
            Aggregate is not GREEN. Body is hl.health.v1 HealthAssessment,
            including typed AMBER lag. Not ApiError. Canonical snapshot
            validity is /v1/health 200; readiness is GREEN-only. This is
            not Stage 2 PASS. Invented extra claim.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/HealthAssessment"
"##;
        assert_eq!(
            readyz_503_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "schema freeze must not hide a /readyz 503 description rewrite"
        );
        let description = readyz_503_description(document).expect("/readyz 503 description");
        assert!(
            description.contains("Not ApiError")
                && (description.contains("HealthAssessment")
                    || description.contains("hl.health.v1")),
            "fixture must still satisfy the old substring checks, got {description}"
        );
        assert_ne!(
            description.as_str(),
            READYZ_503_DESCRIPTION,
            "substring health-not-ApiError prose must not satisfy the exact path freeze, got {description}"
        );
    }

    #[test]
    fn inlined_readyz_200_api_error_fails_health_schema_freeze() {
        let document = r##"
paths:
  /readyz:
    get:
      responses:
        "200":
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/ApiError"
"##;
        assert_eq!(
            readyz_200_schema_ref(document),
            Some("#/components/schemas/ApiError")
        );
        assert_ne!(
            readyz_200_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "inlining ApiError on /readyz 200 must fail the health freeze"
        );
    }

    #[test]
    fn readyz_200_present_and_valid_prose_fails_exact_path_description_freeze() {
        let document = r##"
paths:
  /readyz:
    get:
      responses:
        "200":
          description: >
            Aggregate is HEALTH_STATE_GREEN. Readiness is GREEN-only and is
            not implied by /v1/health 200. AMBER, including lag, is 503.
            Snapshots are present and valid.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/HealthAssessment"
"##;
        assert_eq!(
            readyz_200_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "schema freeze must not hide a /readyz 200 description rewrite"
        );
        let description = readyz_200_description(document).expect("/readyz 200 description");
        assert!(
            description.contains("GREEN-only") && description.contains("present and valid"),
            "fixture must still contain the old substring tokens, got {description}"
        );
        assert_ne!(
            description.as_str(),
            READYZ_200_DESCRIPTION,
            "GREEN-only plus present-and-valid rewrite must fail the exact path freeze, got {description}"
        );
    }

    #[test]
    fn readyz_200_substring_prose_fails_exact_path_description_freeze() {
        let document = r##"
paths:
  /readyz:
    get:
      responses:
        "200":
          description: >
            Aggregate is HEALTH_STATE_GREEN. Readiness is GREEN-only and is
            not implied by /v1/health 200. AMBER, including lag, is 503.
            Invented extra claim.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/HealthAssessment"
"##;
        assert_eq!(
            readyz_200_schema_ref(document),
            Some("#/components/schemas/HealthAssessment"),
            "schema freeze must not hide a /readyz 200 description rewrite"
        );
        let description = readyz_200_description(document).expect("/readyz 200 description");
        assert!(
            description.contains("GREEN-only") && !description.contains("present and valid"),
            "fixture must still satisfy the old substring checks, got {description}"
        );
        assert_ne!(
            description.as_str(),
            READYZ_200_DESCRIPTION,
            "substring GREEN-only prose must not satisfy the exact path freeze, got {description}"
        );
    }

    #[test]
    fn readyz_get_substring_prose_fails_exact_operation_description_freeze() {
        let document = r##"
paths:
  /readyz:
    get:
      description: >
        GREEN-only readiness as hl.health.v1. 200 is HEALTH_STATE_GREEN;
        non-GREEN including typed AMBER lag is 503 health, not ApiError.
        Invented extra claim.
      responses:
        "200":
          description: >
            Aggregate is HEALTH_STATE_GREEN. Readiness is GREEN-only and is
            not implied by /v1/health 200. AMBER, including lag, is 503.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/HealthAssessment"
        "503":
          description: >
            Aggregate is not GREEN. Body is hl.health.v1 HealthAssessment,
            including typed AMBER lag. Not ApiError. Canonical snapshot
            validity is /v1/health 200; readiness is GREEN-only. This is
            not Stage 2 PASS.
          content:
            application/json:
              schema:
                $ref: "#/components/schemas/HealthAssessment"
"##;
        assert_eq!(
            readyz_200_description(document).as_deref(),
            Some(READYZ_200_DESCRIPTION),
            "200 path freeze must not hide a /readyz GET description rewrite"
        );
        assert_eq!(
            readyz_503_description(document).as_deref(),
            Some(READYZ_503_DESCRIPTION),
            "503 path freeze must not hide a /readyz GET description rewrite"
        );
        let description = readyz_get_description(document).expect("/readyz GET description");
        assert!(
            description.contains("not ApiError"),
            "fixture must still satisfy the old substring check, got {description}"
        );
        assert_ne!(
            description.as_str(),
            READYZ_GET_DESCRIPTION,
            "substring not-ApiError prose must not satisfy the exact GET freeze, got {description}"
        );
    }
}
