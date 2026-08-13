import { assertNever, parseApiError, type ApiError } from "@/lib/contracts"
import type { Tone } from "@/lib/tone"

export type FailClosedFamily =
  | "snapshot_missing"
  | "snapshot_invalid"
  | "query_budget"
  | "invalid_query"
  | "stream_unspecified"
  | "unauthorized"
  | "data_unavailable"
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
    title: titleOf(family, status),
    detail: detailOf(family, error),
    tone: toneOf(family),
  }
}

export function classifyHttpBody(status: number, body: unknown): ProbeOutcome {
  const parsed = parseApiError(body)
  if (parsed.ok) {
    return { kind: "observed", view: mapApiError(status, parsed.value) }
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
  if (status === 503 || error.code === "data_unavailable") {
    return "data_unavailable"
  }
  return "typed_other"
}

function titleOf(family: FailClosedFamily, status: number): string {
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
    case "typed_other":
      return `HTTP ${status}`
    default:
      return assertNever(family)
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
    case "typed_other":
      return `${error.code} · ${error.reason_code}`
    default:
      return assertNever(family)
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
    case "typed_other":
      return "red"
    default:
      return assertNever(family)
  }
}
