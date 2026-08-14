import {
  API_ERROR_SCHEMA_VERSION,
  asCaptureHealthNotReadyReason,
  asCoreDeadletterReason,
  asLedgerUnsupportedEventReason,
  assertNever,
  parseApiError,
  parseCaptureHealth,
  parseCoreHealth,
  parseCoreStatus,
  type ApiError,
  type CaptureHealthBody,
  type CoreHealth,
  type CoreStatus,
} from "@/lib/contracts"
import type { Tone } from "@/lib/tone"

export type FailClosedFamily =
  | "snapshot_missing"
  | "snapshot_invalid"
  | "query_budget"
  | "invalid_query"
  | "stream_unspecified"
  | "unauthorized"
  | "data_unavailable"
  | "core_deadletter"
  | "ledger_unsupported_event"
  | "capture_health_not_ready"
  | "typed_other"

export interface FailClosedView {
  family: FailClosedFamily
  httpStatus: number
  code: string
  reasonCode: string
  title: string
  detail: string
  tone: Tone
}

export type ProbeOutcome =
  | { kind: "network"; detail: string }
  | { kind: "observed"; view: FailClosedView }
  | { kind: "not_observed"; status: number; detail: string }
  | { kind: "invalid"; status: number; detail: string }

export function mapApiError(status: number, error: ApiError): FailClosedView {
  const family = familyOf(status, error)
  return {
    family,
    httpStatus: status,
    code: error.code,
    reasonCode: error.reason_code,
    title: titleOf(family, status, error.reason_code),
    detail: detailOf(family, error),
    tone: toneOf(family),
  }
}

export function classifyHttpBody(status: number, body: unknown): ProbeOutcome {
  const parsed = parseApiError(body)
  if (parsed.ok) {
    return { kind: "observed", view: mapApiError(status, parsed.value) }
  }
  const coreHealth = parseCoreHealth(body)
  if (coreHealth.ok) {
    return classifyCoreHealth(status, coreHealth.value)
  }
  const coreStatus = parseCoreStatus(body)
  if (coreStatus.ok) {
    return classifyCoreStatus(status, coreStatus.value)
  }
  const captureHealth = parseCaptureHealth(body)
  if (captureHealth.ok) {
    return classifyCaptureHealth(status, captureHealth.value)
  }
  if (status === 200) {
    return {
      kind: "not_observed",
      status,
      detail:
        "HTTP 200 — this listener did not return typed hl.api.error.v1. Not a Stage PASS.",
    }
  }
  return {
    kind: "invalid",
    status,
    detail: parsed.detail,
  }
}

export function familyOf(status: number, error: ApiError): FailClosedFamily {
  if (status === 401 || error.code === "unauthorized") {
    return "unauthorized"
  }
  if (status === 501 && error.reason_code === "stream.websocket_unspecified") {
    return "stream_unspecified"
  }
  if (status === 429 || error.code === "query_budget_exceeded") {
    return "query_budget"
  }
  if (status === 400 && error.code === "invalid_query") {
    return "invalid_query"
  }
  if (status === 503 && error.reason_code === "snapshot_missing") {
    return "snapshot_missing"
  }
  if (status === 503 && error.reason_code === "snapshot_invalid") {
    return "snapshot_invalid"
  }
  if (asCoreDeadletterReason(error.reason_code)) {
    return "core_deadletter"
  }
  if (asLedgerUnsupportedEventReason(error.reason_code)) {
    return "ledger_unsupported_event"
  }
  if (asCaptureHealthNotReadyReason(error.reason_code)) {
    return "capture_health_not_ready"
  }
  if (status === 503 || error.code === "data_unavailable") {
    return "data_unavailable"
  }
  return "typed_other"
}

function classifyCoreHealth(status: number, health: CoreHealth): ProbeOutcome {
  if (health.live_qualified || health.stage_2_qualified) {
    return observedUnavailable(status, "core_status.qualification_claim")
  }
  if (health.reason_code !== null) {
    return observedUnavailable(status, health.reason_code)
  }
  if (status === 200 && health.ok && health.ready) {
    return {
      kind: "not_observed",
      status,
      detail:
        "HTTP 200 — this listener did not return typed hl.api.error.v1. Not a Stage PASS.",
    }
  }
  return observedUnavailable(status, "core_status.not_ready")
}

function classifyCoreStatus(
  status: number,
  snapshot: CoreStatus
): ProbeOutcome {
  if (snapshot.live_qualified || snapshot.stage_2_qualified) {
    return observedUnavailable(status, "core_status.qualification_claim")
  }
  if (snapshot.fail_closed_reason !== undefined) {
    return observedUnavailable(status, snapshot.fail_closed_reason)
  }
  if (status === 200 && snapshot.ready) {
    return {
      kind: "not_observed",
      status,
      detail:
        "HTTP 200 — this listener did not return typed hl.api.error.v1. Not a Stage PASS.",
    }
  }
  return observedUnavailable(status, "core_status.not_ready")
}

function classifyCaptureHealth(
  status: number,
  health: CaptureHealthBody
): ProbeOutcome {
  if (health.reason_code !== undefined) {
    return observedUnavailable(status, health.reason_code)
  }
  if (status === 200 && health.ok && health.ready === true) {
    return {
      kind: "not_observed",
      status,
      detail:
        "HTTP 200 — this listener did not return typed hl.api.error.v1. Not a Stage PASS.",
    }
  }
  return observedUnavailable(status, "capture_health.not_ready")
}

function observedUnavailable(
  status: number,
  reason_code: string
): ProbeOutcome {
  return {
    kind: "observed",
    view: mapApiError(status, {
      schema_version: API_ERROR_SCHEMA_VERSION,
      code: "data_unavailable",
      reason_code,
    }),
  }
}

function titleOf(
  family: FailClosedFamily,
  status: number,
  reasonCode: string
): string {
  switch (family) {
    case "snapshot_missing":
      return "503 snapshot missing"
    case "snapshot_invalid":
      return "503 snapshot invalid"
    case "query_budget":
      return `${status} query budget`
    case "invalid_query":
      return "400 invalid query"
    case "stream_unspecified":
      return "501 stream unspecified"
    case "unauthorized":
      return `${status} unauthorized`
    case "data_unavailable":
      return `${status} data unavailable`
    case "core_deadletter":
      return titleOfDeadletter(reasonCode)
    case "ledger_unsupported_event":
      return titleOfLedgerUnsupported(reasonCode)
    case "capture_health_not_ready":
      return titleOfCaptureHealthNotReady(reasonCode)
    case "typed_other":
      return `HTTP ${status}`
    default:
      return assertNever(family)
  }
}

function titleOfDeadletter(reasonCode: string): string {
  const reason = asCoreDeadletterReason(reasonCode)
  if (!reason) {
    return "503 data unavailable"
  }
  switch (reason) {
    case "core.deadletter_unsafe_path":
      return "503 dead-letter unsafe path"
    case "core.deadletter_io":
      return "503 dead-letter I/O"
    case "core.deadletter_invalid_record":
      return "503 dead-letter invalid record"
    case "core.deadletter_serialization":
      return "503 dead-letter serialization"
    case "core.deadletter_corrupt":
      return "503 dead-letter corrupt"
    default:
      return assertNever(reason)
  }
}

function titleOfLedgerUnsupported(reasonCode: string): string {
  const reason = asLedgerUnsupportedEventReason(reasonCode)
  if (!reason) {
    return "503 data unavailable"
  }
  switch (reason) {
    case "ledger.unsupported_event":
      return "503 ledger unsupported event"
    default:
      return assertNever(reason)
  }
}

function titleOfCaptureHealthNotReady(reasonCode: string): string {
  const reason = asCaptureHealthNotReadyReason(reasonCode)
  if (!reason) {
    return "503 data unavailable"
  }
  switch (reason) {
    case "capture_health.not_ready":
      return "503 capture health not ready"
    default:
      return assertNever(reason)
  }
}

function detailOf(family: FailClosedFamily, error: ApiError): string {
  switch (family) {
    case "snapshot_missing":
      return "Capture/canonical snapshot file is missing. No invented fills."
    case "snapshot_invalid":
      return "Snapshot failed validation. Body is not treated as healthy capture."
    case "query_budget":
      return "Query row, concurrency, or timeout budget refused the request."
    case "invalid_query":
      return "Query string is not accepted (offset, unknown parameter, or bad limit)."
    case "stream_unspecified":
      return "WebSocket resume is unspecified; hl-api fail-closes streams with typed 501."
    case "unauthorized":
      return "Bearer rejected. This is not live-source qualification."
    case "data_unavailable":
      return "Typed data_unavailable. Empty panels are not a green snapshot."
    case "core_deadletter":
      return detailOfDeadletter(error.reason_code)
    case "ledger_unsupported_event":
      return detailOfLedgerUnsupported(error.reason_code)
    case "capture_health_not_ready":
      return detailOfCaptureHealthNotReady(error.reason_code)
    case "typed_other":
      return `${error.code} · ${error.reason_code}`
    default:
      return assertNever(family)
  }
}

function detailOfDeadletter(reasonCode: string): string {
  const reason = asCoreDeadletterReason(reasonCode)
  if (!reason) {
    return `${reasonCode} · fail-closed`
  }
  switch (reason) {
    case "core.deadletter_unsafe_path":
      return "hl-core dead-letter path is unsafe. Fail-closed; not ready. /status fail_closed_reason is latched."
    case "core.deadletter_io":
      return "hl-core dead-letter I/O failed. Fail-closed; not ready. /status fail_closed_reason is latched."
    case "core.deadletter_invalid_record":
      return "hl-core dead-letter record is invalid. Fail-closed; not ready. /status fail_closed_reason is latched."
    case "core.deadletter_serialization":
      return "hl-core dead-letter record could not be serialized. Fail-closed; not ready. /status fail_closed_reason is latched."
    case "core.deadletter_corrupt":
      return "hl-core dead-letter file is truncated or corrupt. Fail-closed; not ready. /status fail_closed_reason is latched."
    default:
      return assertNever(reason)
  }
}

function detailOfLedgerUnsupported(reasonCode: string): string {
  const reason = asLedgerUnsupportedEventReason(reasonCode)
  if (!reason) {
    return `${reasonCode} · fail-closed`
  }
  switch (reason) {
    case "ledger.unsupported_event":
      return "hl-core consume rejected an action-bearing or poison event. Fail-closed; not ready. /status fail_closed_reason is latched."
    default:
      return assertNever(reason)
  }
}

function detailOfCaptureHealthNotReady(reasonCode: string): string {
  const reason = asCaptureHealthNotReadyReason(reasonCode)
  if (!reason) {
    return `${reasonCode} · fail-closed`
  }
  switch (reason) {
    case "capture_health.not_ready":
      return "Capture /healthz is leftover v4 or not live-ready. Fail-closed; not ready. Not live-source qualification."
    default:
      return assertNever(reason)
  }
}

function toneOf(family: FailClosedFamily): Tone {
  switch (family) {
    case "stream_unspecified":
    case "invalid_query":
      return "yellow"
    case "snapshot_missing":
    case "snapshot_invalid":
    case "query_budget":
    case "unauthorized":
    case "data_unavailable":
    case "core_deadletter":
    case "ledger_unsupported_event":
    case "capture_health_not_ready":
    case "typed_other":
      return "red"
    default:
      return assertNever(family)
  }
}
