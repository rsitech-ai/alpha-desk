import {
  parseApiError,
  parseCaptureStatus,
  parseHealthAssessment,
  type ApiError,
  type CaptureStatus,
  type HealthAssessment,
} from "@/lib/contracts"
import { classifyHttpBody, type ProbeOutcome } from "@/lib/fail-closed"

export const DEFAULT_HL_API_ORIGIN = "http://127.0.0.1:8788"
export const POLL_INTERVAL_MS = 1_000
export const INVALID_QUERY_PROBE_PATH = "/v1/health?offset=1"
export const QUERY_BUDGET_PROBE_PATH = "/v1/health?limit=999999"
export const STREAM_PATH = "/v1/stream"

export const HL_API_ORIGIN =
  import.meta.env.VITE_HL_API_ORIGIN ?? DEFAULT_HL_API_ORIGIN

export type EndpointOutcome<T> =
  | { kind: "ok"; status: number; data: T; raw: unknown }
  | { kind: "http-error"; status: number; error: ApiError }
  | { kind: "invalid"; status: number; detail: string }
  | { kind: "network"; detail: string }

export interface DeskFeed {
  fetchedAt: number
  healthz: EndpointOutcome<HealthAssessment>
  readyz: EndpointOutcome<HealthAssessment>
  canonicalHealth: EndpointOutcome<HealthAssessment>
  captureStatus: EndpointOutcome<CaptureStatus>
  stream: EndpointOutcome<ApiError>
  invalidQuery: ProbeOutcome
  queryBudget: ProbeOutcome
}

export async function fetchDeskFeed(signal: AbortSignal): Promise<DeskFeed> {
  const fetchedAt = Date.now()
  const [
    healthz,
    readyz,
    canonicalHealth,
    captureStatus,
    stream,
    invalidQuery,
    queryBudget,
  ] = await Promise.all([
    fetchHealth("/healthz", signal),
    fetchHealth("/readyz", signal),
    fetchHealth("/v1/health", signal),
    fetchCapture("/v1/capture/status", signal),
    fetchExpectedStream(STREAM_PATH, signal),
    fetchProbe(INVALID_QUERY_PROBE_PATH, signal),
    fetchProbe(QUERY_BUDGET_PROBE_PATH, signal),
  ])
  return {
    fetchedAt,
    healthz,
    readyz,
    canonicalHealth,
    captureStatus,
    stream,
    invalidQuery,
    queryBudget,
  }
}

function requestHeaders(): HeadersInit {
  const headers: Record<string, string> = { accept: "application/json" }
  const bearer = import.meta.env.VITE_HL_API_BEARER
  if (typeof bearer === "string" && bearer.length > 0) {
    headers.authorization = `Bearer ${bearer}`
  }
  return headers
}

async function fetchJson(
  path: string,
  signal: AbortSignal
): Promise<
  | { kind: "network"; detail: string }
  | { kind: "body"; status: number; value: unknown }
> {
  let response: Response
  try {
    response = await fetch(path, {
      method: "GET",
      headers: requestHeaders(),
      signal,
      cache: "no-store",
    })
  } catch (error) {
    if (signal.aborted) {
      throw error
    }
    return {
      kind: "network",
      detail: error instanceof Error ? error.message : "fetch failed",
    }
  }
  let value: unknown
  try {
    value = await response.json()
  } catch {
    return {
      kind: "body",
      status: response.status,
      value: { parse: "response is not JSON" },
    }
  }
  return { kind: "body", status: response.status, value }
}

async function fetchHealth(
  path: string,
  signal: AbortSignal
): Promise<EndpointOutcome<HealthAssessment>> {
  const result = await fetchJson(path, signal)
  if (result.kind === "network") {
    return result
  }
  if (result.status === 401 || result.status === 501) {
    return asHttpError(result.status, result.value)
  }
  if (result.status === 503) {
    const health = parseHealthAssessment(result.value)
    if (health.ok) {
      return {
        kind: "ok",
        status: result.status,
        data: health.value,
        raw: result.value,
      }
    }
    return asHttpError(result.status, result.value)
  }
  if (result.status === 200) {
    const health = parseHealthAssessment(result.value)
    if (!health.ok) {
      return { kind: "invalid", status: result.status, detail: health.detail }
    }
    return {
      kind: "ok",
      status: result.status,
      data: health.value,
      raw: result.value,
    }
  }
  return asHttpError(result.status, result.value)
}

async function fetchCapture(
  path: string,
  signal: AbortSignal
): Promise<EndpointOutcome<CaptureStatus>> {
  const result = await fetchJson(path, signal)
  if (result.kind === "network") {
    return result
  }
  if (result.status === 200) {
    const parsed = parseCaptureStatus(result.value)
    if (!parsed.ok) {
      return { kind: "invalid", status: result.status, detail: parsed.detail }
    }
    return {
      kind: "ok",
      status: result.status,
      data: parsed.value,
      raw: result.value,
    }
  }
  return asHttpError(result.status, result.value)
}

async function fetchExpectedStream(
  path: string,
  signal: AbortSignal
): Promise<EndpointOutcome<ApiError>> {
  const result = await fetchJson(path, signal)
  if (result.kind === "network") {
    return result
  }
  const error = parseApiError(result.value)
  if (error.ok) {
    return { kind: "http-error", status: result.status, error: error.value }
  }
  if (result.status === 200) {
    return {
      kind: "invalid",
      status: result.status,
      detail: "stream returned 200; this UI does not invent fills or charts",
    }
  }
  return {
    kind: "invalid",
    status: result.status,
    detail: error.detail,
  }
}

async function fetchProbe(
  path: string,
  signal: AbortSignal
): Promise<ProbeOutcome> {
  const result = await fetchJson(path, signal)
  if (result.kind === "network") {
    return result
  }
  return classifyHttpBody(result.status, result.value)
}

function asHttpError(status: number, value: unknown): EndpointOutcome<never> {
  const parsed = parseApiError(value)
  if (!parsed.ok) {
    return { kind: "invalid", status, detail: parsed.detail }
  }
  return { kind: "http-error", status, error: parsed.value }
}
