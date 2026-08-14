export const HEALTH_SCHEMA_VERSION = "hl.health.v1" as const
export const CORE_HEALTH_SCHEMA_VERSION = "hl.core.health.v1" as const
export const CORE_STATUS_SCHEMA_VERSION = "hl.core.status.v1" as const
export const CAPTURE_HEALTH_SCHEMA_VERSION = "hl.capture.health.v1" as const
export const CAPTURE_STATUS_SCHEMA_VERSION = "hl.capture.status.v4" as const
export const CAPTURE_STATUS_SCHEMA_V5 = "hl.capture.status.v5" as const
export const API_ERROR_SCHEMA_VERSION = "hl.api.error.v1" as const

export type CoreDeadletterReason =
  | "core.deadletter_unsafe_path"
  | "core.deadletter_io"
  | "core.deadletter_invalid_record"
  | "core.deadletter_serialization"
  | "core.deadletter_corrupt"

export const CORE_DEADLETTER_REASONS = [
  "core.deadletter_unsafe_path",
  "core.deadletter_io",
  "core.deadletter_invalid_record",
  "core.deadletter_serialization",
  "core.deadletter_corrupt",
] as const satisfies readonly CoreDeadletterReason[]

export type LedgerUnsupportedEventReason = "ledger.unsupported_event"

export const LEDGER_UNSUPPORTED_EVENT_REASONS = [
  "ledger.unsupported_event",
] as const satisfies readonly LedgerUnsupportedEventReason[]

export type CaptureHealthNotReadyReason = "capture_health.not_ready"

export const CAPTURE_HEALTH_NOT_READY_REASONS = [
  "capture_health.not_ready",
] as const satisfies readonly CaptureHealthNotReadyReason[]

export type CaptureStatusSchema =
  typeof CAPTURE_STATUS_SCHEMA_VERSION | typeof CAPTURE_STATUS_SCHEMA_V5

export type HealthState =
  "HEALTH_STATE_GREEN" | "HEALTH_STATE_AMBER" | "HEALTH_STATE_RED"

export type CaptureHealth = "green" | "yellow" | "red"

export type CaptureSourceHealth = "starting" | "healthy" | "range-unavailable"

export const CAPTURE_SOURCE_HEALTH = [
  "starting",
  "healthy",
  "range-unavailable",
] as const satisfies readonly CaptureSourceHealth[]

export type AuxiliarySourceHealth =
  "starting" | "healthy" | "quarantined" | "latched"

export const AUXILIARY_SOURCE_HEALTH = [
  "starting",
  "healthy",
  "quarantined",
  "latched",
] as const satisfies readonly AuxiliarySourceHealth[]

export type AuxiliaryQualification = "unqualified" | "qualified"

export interface HealthAssessment {
  schema_version: typeof HEALTH_SCHEMA_VERSION
  scope: string
  state: HealthState
  reason_code: string
  observed_at_micros: number
  suppresses: string[]
}

export interface ApiError {
  schema_version: typeof API_ERROR_SCHEMA_VERSION
  code: string
  reason_code: string
}

export interface CoreHealth {
  schema_version: typeof CORE_HEALTH_SCHEMA_VERSION
  ok: boolean
  ready: boolean
  reason_code: string | null
  live_qualified: boolean
  stage_2_qualified: boolean
}

export interface CaptureHealthBody {
  schema_version: typeof CAPTURE_HEALTH_SCHEMA_VERSION
  ok: boolean
  health?: CaptureHealth
  ready?: boolean
  reason_code?: string
}

export interface CoreStatus {
  schema_version: typeof CORE_STATUS_SCHEMA_VERSION
  ready: boolean
  last_applied_watermark?: number
  fail_closed_reason?: string
  live_qualified: boolean
  stage_2_qualified: boolean
}

export type HealthBody = HealthAssessment | CoreHealth | CaptureHealthBody

export const CORE_HEALTH_FIELD_ORDER = [
  "schema_version",
  "ok",
  "ready",
  "reason_code",
  "live_qualified",
  "stage_2_qualified",
] as const

export const CORE_STATUS_FIELD_ORDER = [
  "schema_version",
  "ready",
  "last_applied_watermark",
  "fail_closed_reason",
  "live_qualified",
  "stage_2_qualified",
] as const

export const CAPTURE_HEALTH_FIELD_ORDER = [
  "schema_version",
  "ok",
  "health",
  "ready",
  "reason_code",
] as const

export interface AuxiliarySourceStatus {
  source_id: string
  health: AuxiliarySourceHealth
  qualification: AuxiliaryQualification
  cursor_epoch?: string
  tail_cursor_epoch?: string
  durable_offset?: number
  local_sequence?: number
  spool_records: number
  unarchived_records: number
  unread_bytes?: number
  partial_line: boolean
  last_durable_wall_micros?: number
  quarantine_reason?: string
  last_error_reason?: string
  restart_reconstruction?: string
  extra_fields: Record<string, unknown>
}

export interface CaptureStatus {
  schema_version: CaptureStatusSchema
  snapshot_at_micros: number
  build_id: string
  chain_id: string
  health: CaptureHealth
  ready: boolean
  last_error_reason?: string
  active_committed_source: string
  primary_source_health: CaptureSourceHealth
  independent_source_health?: CaptureSourceHealth
  failover_height?: number
  failover_reason?: string
  durable_height?: number
  pending_blocks: number
  capture_backlog_records?: number
  oldest_pending_capture_height?: number
  disk_free_basis_points?: number
  archive_manifest_id?: string
  auxiliary_sources?: AuxiliarySourceStatus[]
  throughput_records_per_sec?: number
  throughput_blocks_per_sec?: number
  extra_fields: Record<string, unknown>
}

export const LAST_HEARTBEAT_THROUGHPUT_FIELDS = [
  "throughput_records_per_sec",
  "throughput_blocks_per_sec",
] as const

export interface LastHeartbeatThroughput {
  throughput_records_per_sec?: number
  throughput_blocks_per_sec?: number
}

export const CAPTURE_STATUS_FIELD_ORDER = [
  "schema_version",
  "snapshot_at_micros",
  "build_id",
  "chain_id",
  "health",
  "ready",
  "last_error_reason",
  "active_committed_source",
  "primary_source_health",
  "independent_source_health",
  "failover_height",
  "failover_reason",
  "durable_height",
  "pending_blocks",
  "capture_backlog_records",
  "oldest_pending_capture_height",
  "disk_free_basis_points",
  "archive_manifest_id",
  "auxiliary_sources",
] as const

export const HEALTH_FIELD_ORDER = [
  "schema_version",
  "scope",
  "state",
  "reason_code",
  "observed_at_micros",
  "suppresses",
] as const

export const AUXILIARY_SOURCE_FIELD_ORDER = [
  "source_id",
  "health",
  "qualification",
  "cursor_epoch",
  "tail_cursor_epoch",
  "durable_offset",
  "local_sequence",
  "spool_records",
  "unarchived_records",
  "unread_bytes",
  "partial_line",
  "last_durable_wall_micros",
  "quarantine_reason",
  "last_error_reason",
  "restart_reconstruction",
] as const

export type ParseResult<T> =
  { ok: true; value: T } | { ok: false; detail: string }

export function parseHealthAssessment(
  value: unknown
): ParseResult<HealthAssessment> {
  if (!isRecord(value)) {
    return { ok: false, detail: "health body is not an object" }
  }
  const schema_version = requireConst(
    value,
    "schema_version",
    HEALTH_SCHEMA_VERSION
  )
  if (!schema_version.ok) {
    return schema_version
  }
  const scope = requireNonEmptyString(value, "scope")
  if (!scope.ok) {
    return scope
  }
  const state = requireEnum(value, "state", HEALTH_STATES)
  if (!state.ok) {
    return state
  }
  const reason_code = requireNonEmptyString(value, "reason_code")
  if (!reason_code.ok) {
    return reason_code
  }
  const observed_at_micros = requireNonNegativeInt(value, "observed_at_micros")
  if (!observed_at_micros.ok) {
    return observed_at_micros
  }
  const suppresses = requireStringArray(value, "suppresses")
  if (!suppresses.ok) {
    return suppresses
  }
  return {
    ok: true,
    value: {
      schema_version: schema_version.value,
      scope: scope.value,
      state: state.value,
      reason_code: reason_code.value,
      observed_at_micros: observed_at_micros.value,
      suppresses: suppresses.value,
    },
  }
}

export function parseApiError(value: unknown): ParseResult<ApiError> {
  if (!isRecord(value)) {
    return { ok: false, detail: "error body is not an object" }
  }
  const schema_version = requireConst(
    value,
    "schema_version",
    API_ERROR_SCHEMA_VERSION
  )
  if (!schema_version.ok) {
    return schema_version
  }
  const code = requireNonEmptyString(value, "code")
  if (!code.ok) {
    return code
  }
  const reason_code = requireNonEmptyString(value, "reason_code")
  if (!reason_code.ok) {
    return reason_code
  }
  return {
    ok: true,
    value: {
      schema_version: schema_version.value,
      code: code.value,
      reason_code: reason_code.value,
    },
  }
}

export function asCoreDeadletterReason(
  value: string
): CoreDeadletterReason | undefined {
  switch (value) {
    case "core.deadletter_unsafe_path":
    case "core.deadletter_io":
    case "core.deadletter_invalid_record":
    case "core.deadletter_serialization":
    case "core.deadletter_corrupt":
      return value
    default:
      return undefined
  }
}

export function asLedgerUnsupportedEventReason(
  value: string
): LedgerUnsupportedEventReason | undefined {
  switch (value) {
    case "ledger.unsupported_event":
      return value
    default:
      return undefined
  }
}

export function asCaptureHealthNotReadyReason(
  value: string
): CaptureHealthNotReadyReason | undefined {
  switch (value) {
    case "capture_health.not_ready":
      return value
    default:
      return undefined
  }
}

export function asTypedCoreFailClosedReason(
  value: string
):
  | CoreDeadletterReason
  | LedgerUnsupportedEventReason
  | CaptureHealthNotReadyReason
  | undefined {
  return (
    asCoreDeadletterReason(value) ??
    asLedgerUnsupportedEventReason(value) ??
    asCaptureHealthNotReadyReason(value)
  )
}

export function healthReasonCode(body: HealthBody): string | undefined {
  switch (body.schema_version) {
    case HEALTH_SCHEMA_VERSION:
      return body.reason_code
    case CORE_HEALTH_SCHEMA_VERSION:
      return body.reason_code === null ? undefined : body.reason_code
    case CAPTURE_HEALTH_SCHEMA_VERSION:
      return body.reason_code
    default:
      return assertNever(body)
  }
}

export function parseCoreHealth(value: unknown): ParseResult<CoreHealth> {
  if (!isRecord(value)) {
    return { ok: false, detail: "core health body is not an object" }
  }
  const schema_version = requireConst(
    value,
    "schema_version",
    CORE_HEALTH_SCHEMA_VERSION
  )
  if (!schema_version.ok) {
    return schema_version
  }
  const ok = requireBool(value, "ok")
  if (!ok.ok) {
    return ok
  }
  const ready = requireBool(value, "ready")
  if (!ready.ok) {
    return ready
  }
  const reason_code = requireNullOrNonEmptyString(value, "reason_code")
  if (!reason_code.ok) {
    return reason_code
  }
  const live_qualified = requireBool(value, "live_qualified")
  if (!live_qualified.ok) {
    return live_qualified
  }
  const stage_2_qualified = requireBool(value, "stage_2_qualified")
  if (!stage_2_qualified.ok) {
    return stage_2_qualified
  }
  return {
    ok: true,
    value: {
      schema_version: schema_version.value,
      ok: ok.value,
      ready: ready.value,
      reason_code: reason_code.value,
      live_qualified: live_qualified.value,
      stage_2_qualified: stage_2_qualified.value,
    },
  }
}

export function parseCaptureHealth(
  value: unknown
): ParseResult<CaptureHealthBody> {
  if (!isRecord(value)) {
    return { ok: false, detail: "capture health body is not an object" }
  }
  const schema_version = requireConst(
    value,
    "schema_version",
    CAPTURE_HEALTH_SCHEMA_VERSION
  )
  if (!schema_version.ok) {
    return schema_version
  }
  const ok = requireBool(value, "ok")
  if (!ok.ok) {
    return ok
  }
  const health = optionalEnum(value, "health", CAPTURE_HEALTH)
  if (!health.ok) {
    return health
  }
  const ready = optionalBool(value, "ready")
  if (!ready.ok) {
    return ready
  }
  const reason_code = optionalNonEmptyString(value, "reason_code")
  if (!reason_code.ok) {
    return reason_code
  }
  return {
    ok: true,
    value: {
      schema_version: schema_version.value,
      ok: ok.value,
      health: health.value,
      ready: ready.value,
      reason_code: reason_code.value,
    },
  }
}

export function parseCoreStatus(value: unknown): ParseResult<CoreStatus> {
  if (!isRecord(value)) {
    return { ok: false, detail: "core status body is not an object" }
  }
  const schema_version = requireConst(
    value,
    "schema_version",
    CORE_STATUS_SCHEMA_VERSION
  )
  if (!schema_version.ok) {
    return schema_version
  }
  const ready = requireBool(value, "ready")
  if (!ready.ok) {
    return ready
  }
  const last_applied_watermark = optionalNonNegativeInt(
    value,
    "last_applied_watermark"
  )
  if (!last_applied_watermark.ok) {
    return last_applied_watermark
  }
  const fail_closed_reason = optionalNonEmptyString(value, "fail_closed_reason")
  if (!fail_closed_reason.ok) {
    return fail_closed_reason
  }
  const live_qualified = requireBool(value, "live_qualified")
  if (!live_qualified.ok) {
    return live_qualified
  }
  const stage_2_qualified = requireBool(value, "stage_2_qualified")
  if (!stage_2_qualified.ok) {
    return stage_2_qualified
  }
  return {
    ok: true,
    value: {
      schema_version: schema_version.value,
      ready: ready.value,
      last_applied_watermark: last_applied_watermark.value,
      fail_closed_reason: fail_closed_reason.value,
      live_qualified: live_qualified.value,
      stage_2_qualified: stage_2_qualified.value,
    },
  }
}

export function parseCaptureStatus(value: unknown): ParseResult<CaptureStatus> {
  if (!isRecord(value)) {
    return { ok: false, detail: "capture status body is not an object" }
  }
  const schema_version = requireCaptureSchema(value)
  if (!schema_version.ok) {
    return schema_version
  }
  const snapshot_at_micros = requireNonNegativeInt(value, "snapshot_at_micros")
  if (!snapshot_at_micros.ok) {
    return snapshot_at_micros
  }
  const build_id = requireNonEmptyString(value, "build_id")
  if (!build_id.ok) {
    return build_id
  }
  const chain_id = requireNonEmptyString(value, "chain_id")
  if (!chain_id.ok) {
    return chain_id
  }
  const health = requireEnum(value, "health", CAPTURE_HEALTH)
  if (!health.ok) {
    return health
  }
  const ready = requireBool(value, "ready")
  if (!ready.ok) {
    return ready
  }
  const active_committed_source = requireNonEmptyString(
    value,
    "active_committed_source"
  )
  if (!active_committed_source.ok) {
    return active_committed_source
  }
  const primary_source_health = requireEnum(
    value,
    "primary_source_health",
    CAPTURE_SOURCE_HEALTH
  )
  if (!primary_source_health.ok) {
    return primary_source_health
  }
  const pending_blocks = requireNonNegativeInt(value, "pending_blocks")
  if (!pending_blocks.ok) {
    return pending_blocks
  }

  const last_error_reason = optionalNonEmptyString(value, "last_error_reason")
  if (!last_error_reason.ok) {
    return last_error_reason
  }
  const independent_source_health = optionalEnum(
    value,
    "independent_source_health",
    CAPTURE_SOURCE_HEALTH
  )
  if (!independent_source_health.ok) {
    return independent_source_health
  }
  const failover_height = optionalNonNegativeInt(value, "failover_height")
  if (!failover_height.ok) {
    return failover_height
  }
  const failover_reason = optionalNonEmptyString(value, "failover_reason")
  if (!failover_reason.ok) {
    return failover_reason
  }
  const durable_height = optionalNonNegativeInt(value, "durable_height")
  if (!durable_height.ok) {
    return durable_height
  }
  const capture_backlog_records = optionalNonNegativeInt(
    value,
    "capture_backlog_records"
  )
  if (!capture_backlog_records.ok) {
    return capture_backlog_records
  }
  const oldest_pending_capture_height = optionalNonNegativeInt(
    value,
    "oldest_pending_capture_height"
  )
  if (!oldest_pending_capture_height.ok) {
    return oldest_pending_capture_height
  }
  const disk_free_basis_points = optionalNonNegativeInt(
    value,
    "disk_free_basis_points"
  )
  if (!disk_free_basis_points.ok) {
    return disk_free_basis_points
  }
  const archive_manifest_id = optionalNonEmptyString(
    value,
    "archive_manifest_id"
  )
  if (!archive_manifest_id.ok) {
    return archive_manifest_id
  }
  const auxiliary_sources = optionalAuxiliarySources(value)
  if (!auxiliary_sources.ok) {
    return auxiliary_sources
  }

  const extras = collectExtraFields(value, CAPTURE_STATUS_FIELD_ORDER)
  const throughput = lastHeartbeatThroughput(extras)
  return {
    ok: true,
    value: {
      schema_version: schema_version.value,
      snapshot_at_micros: snapshot_at_micros.value,
      build_id: build_id.value,
      chain_id: chain_id.value,
      health: health.value,
      ready: ready.value,
      last_error_reason: last_error_reason.value,
      active_committed_source: active_committed_source.value,
      primary_source_health: primary_source_health.value,
      independent_source_health: independent_source_health.value,
      failover_height: failover_height.value,
      failover_reason: failover_reason.value,
      durable_height: durable_height.value,
      pending_blocks: pending_blocks.value,
      capture_backlog_records: capture_backlog_records.value,
      oldest_pending_capture_height: oldest_pending_capture_height.value,
      disk_free_basis_points: disk_free_basis_points.value,
      archive_manifest_id: archive_manifest_id.value,
      auxiliary_sources: auxiliary_sources.value,
      throughput_records_per_sec: throughput.throughput_records_per_sec,
      throughput_blocks_per_sec: throughput.throughput_blocks_per_sec,
      extra_fields: extrasWithoutMappedThroughput(extras, throughput),
    },
  }
}

export function lastHeartbeatThroughput(
  extras: Record<string, unknown>
): LastHeartbeatThroughput {
  return {
    throughput_records_per_sec: lastHeartbeatRate(
      extras.throughput_records_per_sec
    ),
    throughput_blocks_per_sec: lastHeartbeatRate(
      extras.throughput_blocks_per_sec
    ),
  }
}

export function lastHeartbeatRate(value: unknown): number | undefined {
  if (
    typeof value === "number" &&
    Number.isInteger(value) &&
    Number.isSafeInteger(value) &&
    value >= 0
  ) {
    return value
  }
  return undefined
}

function extrasWithoutMappedThroughput(
  extras: Record<string, unknown>,
  throughput: LastHeartbeatThroughput
): Record<string, unknown> {
  const rest: Record<string, unknown> = { ...extras }
  for (const field of LAST_HEARTBEAT_THROUGHPUT_FIELDS) {
    if (throughput[field] !== undefined) {
      delete rest[field]
    }
  }
  return rest
}

export function assertNever(value: never): never {
  throw new Error(`unhandled variant: ${String(value)}`)
}

const HEALTH_STATES = [
  "HEALTH_STATE_GREEN",
  "HEALTH_STATE_AMBER",
  "HEALTH_STATE_RED",
] as const satisfies readonly HealthState[]

const CAPTURE_HEALTH = [
  "green",
  "yellow",
  "red",
] as const satisfies readonly CaptureHealth[]

const AUXILIARY_QUALIFICATION = [
  "unqualified",
  "qualified",
] as const satisfies readonly AuxiliaryQualification[]

export function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

function requireCaptureSchema(
  object: Record<string, unknown>
): ParseResult<CaptureStatusSchema> {
  const value = object.schema_version
  if (
    value === CAPTURE_STATUS_SCHEMA_VERSION ||
    value === CAPTURE_STATUS_SCHEMA_V5
  ) {
    return { ok: true, value }
  }
  return {
    ok: false,
    detail: `schema_version must be ${CAPTURE_STATUS_SCHEMA_VERSION} or ${CAPTURE_STATUS_SCHEMA_V5}`,
  }
}

function collectExtraFields(
  object: Record<string, unknown>,
  known: readonly string[]
): Record<string, unknown> {
  const allowed = new Set(known)
  const extras: Record<string, unknown> = {}
  for (const [key, fieldValue] of Object.entries(object)) {
    if (!allowed.has(key)) {
      extras[key] = fieldValue
    }
  }
  return extras
}

function optionalIgnoredString(
  object: Record<string, unknown>,
  field: string
): string | undefined {
  const value = object[field]
  if (typeof value === "string" && value.length > 0) {
    return value
  }
  return undefined
}

function requireConst<T extends string>(
  object: Record<string, unknown>,
  field: string,
  expected: T
): ParseResult<T> {
  const value = object[field]
  if (value !== expected) {
    return {
      ok: false,
      detail: `${field} must be ${expected}`,
    }
  }
  return { ok: true, value: expected }
}

function requireNonEmptyString(
  object: Record<string, unknown>,
  field: string
): ParseResult<string> {
  const value = object[field]
  if (typeof value !== "string" || value.length === 0) {
    return { ok: false, detail: `${field} must be a non-empty string` }
  }
  return { ok: true, value }
}

function optionalNonEmptyString(
  object: Record<string, unknown>,
  field: string
): ParseResult<string | undefined> {
  if (!(field in object) || object[field] === null) {
    return { ok: true, value: undefined }
  }
  return requireNonEmptyString(object, field)
}

function requireNullOrNonEmptyString(
  object: Record<string, unknown>,
  field: string
): ParseResult<string | null> {
  if (!(field in object)) {
    return { ok: false, detail: `${field} must be a string or null` }
  }
  if (object[field] === null) {
    return { ok: true, value: null }
  }
  return requireNonEmptyString(object, field)
}

function requireBool(
  object: Record<string, unknown>,
  field: string
): ParseResult<boolean> {
  const value = object[field]
  if (typeof value !== "boolean") {
    return { ok: false, detail: `${field} must be a boolean` }
  }
  return { ok: true, value }
}

function requireNonNegativeInt(
  object: Record<string, unknown>,
  field: string
): ParseResult<number> {
  const value = object[field]
  if (
    typeof value !== "number" ||
    !Number.isInteger(value) ||
    !Number.isSafeInteger(value) ||
    value < 0
  ) {
    return {
      ok: false,
      detail: `${field} must be a non-negative integer`,
    }
  }
  return { ok: true, value }
}

function optionalNonNegativeInt(
  object: Record<string, unknown>,
  field: string
): ParseResult<number | undefined> {
  if (!(field in object) || object[field] === null) {
    return { ok: true, value: undefined }
  }
  return requireNonNegativeInt(object, field)
}

function optionalBool(
  object: Record<string, unknown>,
  field: string
): ParseResult<boolean | undefined> {
  if (!(field in object) || object[field] === null) {
    return { ok: true, value: undefined }
  }
  return requireBool(object, field)
}

function optionalEnum<T extends string>(
  object: Record<string, unknown>,
  field: string,
  allowed: readonly T[]
): ParseResult<T | undefined> {
  if (!(field in object) || object[field] === null) {
    return { ok: true, value: undefined }
  }
  return requireEnum(object, field, allowed)
}

function requireEnum<T extends string>(
  object: Record<string, unknown>,
  field: string,
  allowed: readonly T[]
): ParseResult<T> {
  const value = object[field]
  if (typeof value !== "string" || !allowed.includes(value as T)) {
    return {
      ok: false,
      detail: `${field} must be one of ${allowed.join(", ")}`,
    }
  }
  return { ok: true, value: value as T }
}

function requireStringArray(
  object: Record<string, unknown>,
  field: string
): ParseResult<string[]> {
  const value = object[field]
  if (!Array.isArray(value) || value.some((item) => typeof item !== "string")) {
    return { ok: false, detail: `${field} must be a string array` }
  }
  return { ok: true, value }
}

function optionalAuxiliarySources(
  object: Record<string, unknown>
): ParseResult<AuxiliarySourceStatus[] | undefined> {
  if (!("auxiliary_sources" in object) || object.auxiliary_sources === null) {
    return { ok: true, value: undefined }
  }
  const value = object.auxiliary_sources
  if (!Array.isArray(value)) {
    return { ok: false, detail: "auxiliary_sources must be an array" }
  }
  const parsed: AuxiliarySourceStatus[] = []
  for (const [index, item] of value.entries()) {
    const result = parseAuxiliarySource(item, index)
    if (!result.ok) {
      return result
    }
    parsed.push(result.value)
  }
  return { ok: true, value: parsed }
}

function parseAuxiliarySource(
  value: unknown,
  index: number
): ParseResult<AuxiliarySourceStatus> {
  if (!isRecord(value)) {
    return {
      ok: false,
      detail: `auxiliary_sources[${index}] is not an object`,
    }
  }
  const prefix = `auxiliary_sources[${index}]`
  const source_id = requireNonEmptyString(value, "source_id")
  if (!source_id.ok) {
    return { ok: false, detail: `${prefix}.${source_id.detail}` }
  }
  const health = requireEnum(value, "health", AUXILIARY_SOURCE_HEALTH)
  if (!health.ok) {
    return { ok: false, detail: `${prefix}.${health.detail}` }
  }
  const qualification = requireEnum(
    value,
    "qualification",
    AUXILIARY_QUALIFICATION
  )
  if (!qualification.ok) {
    return { ok: false, detail: `${prefix}.${qualification.detail}` }
  }
  const spool_records = requireNonNegativeInt(value, "spool_records")
  if (!spool_records.ok) {
    return { ok: false, detail: `${prefix}.${spool_records.detail}` }
  }
  const unarchived_records = requireNonNegativeInt(value, "unarchived_records")
  if (!unarchived_records.ok) {
    return { ok: false, detail: `${prefix}.${unarchived_records.detail}` }
  }
  const partial_line = requireBool(value, "partial_line")
  if (!partial_line.ok) {
    return { ok: false, detail: `${prefix}.${partial_line.detail}` }
  }
  const cursor_epoch = optionalNonEmptyString(value, "cursor_epoch")
  if (!cursor_epoch.ok) {
    return { ok: false, detail: `${prefix}.${cursor_epoch.detail}` }
  }
  const tail_cursor_epoch = optionalNonEmptyString(value, "tail_cursor_epoch")
  if (!tail_cursor_epoch.ok) {
    return { ok: false, detail: `${prefix}.${tail_cursor_epoch.detail}` }
  }
  const durable_offset = optionalNonNegativeInt(value, "durable_offset")
  if (!durable_offset.ok) {
    return { ok: false, detail: `${prefix}.${durable_offset.detail}` }
  }
  const local_sequence = optionalNonNegativeInt(value, "local_sequence")
  if (!local_sequence.ok) {
    return { ok: false, detail: `${prefix}.${local_sequence.detail}` }
  }
  const unread_bytes = optionalNonNegativeInt(value, "unread_bytes")
  if (!unread_bytes.ok) {
    return { ok: false, detail: `${prefix}.${unread_bytes.detail}` }
  }
  const last_durable_wall_micros = optionalNonNegativeInt(
    value,
    "last_durable_wall_micros"
  )
  if (!last_durable_wall_micros.ok) {
    return { ok: false, detail: `${prefix}.${last_durable_wall_micros.detail}` }
  }
  const quarantine_reason = optionalNonEmptyString(value, "quarantine_reason")
  if (!quarantine_reason.ok) {
    return { ok: false, detail: `${prefix}.${quarantine_reason.detail}` }
  }
  const last_error_reason = optionalNonEmptyString(value, "last_error_reason")
  if (!last_error_reason.ok) {
    return { ok: false, detail: `${prefix}.${last_error_reason.detail}` }
  }
  return {
    ok: true,
    value: {
      source_id: source_id.value,
      health: health.value,
      qualification: qualification.value,
      cursor_epoch: cursor_epoch.value,
      tail_cursor_epoch: tail_cursor_epoch.value,
      durable_offset: durable_offset.value,
      local_sequence: local_sequence.value,
      spool_records: spool_records.value,
      unarchived_records: unarchived_records.value,
      unread_bytes: unread_bytes.value,
      partial_line: partial_line.value,
      last_durable_wall_micros: last_durable_wall_micros.value,
      quarantine_reason: quarantine_reason.value,
      last_error_reason: last_error_reason.value,
      restart_reconstruction: optionalIgnoredString(
        value,
        "restart_reconstruction"
      ),
      extra_fields: collectExtraFields(value, AUXILIARY_SOURCE_FIELD_ORDER),
    },
  }
}
