/// Checked-in OpenAPI document generated from the health proto JSON fields,
/// capture-status v4 (inactive) / v5 (maintenance) required keys, optional
/// last-heartbeat throughput integers, fail-closed query budgets, frozen
/// core dead-letter and ledger.unsupported_event reason codes, and the HTTP
/// router. This is not a production authentication, availability, or SLO
/// contract, it does not invent fills or mark sources live or qualified, and
/// it is not a live core.
pub fn openapi_yaml() -> &'static str {
    include_str!("../../../schemas/openapi/v1/openapi.yaml")
}

pub use crate::snapshot::{
    CORE_DEADLETTER_REASON_CODES, LEDGER_UNSUPPORTED_EVENT_REASON_CODES, is_core_deadletter_reason,
    is_ledger_unsupported_event_reason,
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

/// Folded or inline `/readyz` 200 description.
///
/// Used so "present and valid" cannot be restored as GREEN-ready prose.
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
        CORE_DEADLETTER_REASON_CODES, LEDGER_UNSUPPORTED_EVENT_REASON_CODES,
        READYZ_503_DESCRIPTION, core_deadletter_reason_openapi_enum, health_503_response_ref,
        health_503_schema_ref, health_reason_code_is_unrestricted_string,
        ledger_unsupported_event_reason_openapi_enum, openapi_yaml, readyz_200_description,
        readyz_503_description, readyz_503_schema_ref, unavailable_response_schema_ref,
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
        let description = readyz_200_description(document).expect("/readyz 200 description");
        assert!(
            !description.contains("present and valid"),
            "/readyz 200 must not read as GREEN-ready merely because snapshots are valid, got {description}"
        );
        assert!(
            description.contains("GREEN-only"),
            "/readyz 200 must name GREEN-only readiness, got {description}"
        );
        assert_eq!(
            readyz_503_description(document).as_deref(),
            Some(READYZ_503_DESCRIPTION),
            "/readyz 503 path description must stay health-not-ApiError by exact equality"
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
}
