import { describe, expect, it } from "vitest"

import {
  parseCaptureStatus,
  type ApiError,
  type CaptureStatus,
} from "@/lib/contracts"
import { classifyHttpBody, familyOf, mapApiError } from "@/lib/fail-closed"

const ERROR_SCHEMA = "hl.api.error.v1" as const

function errorBody(code: string, reason_code: string): ApiError {
  return {
    schema_version: ERROR_SCHEMA,
    code,
    reason_code,
  }
}

function v4Status(
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    schema_version: "hl.capture.status.v4",
    snapshot_at_micros: 1,
    build_id: "fixture",
    chain_id: "mainnet",
    health: "red",
    ready: false,
    active_committed_source: "locally-verified-committed",
    primary_source_health: "starting",
    pending_blocks: 0,
    ...overrides,
  }
}

describe("mapApiError", () => {
  it("maps 503 snapshot_missing as first-class missing status", () => {
    const view = mapApiError(
      503,
      errorBody("data_unavailable", "snapshot_missing")
    )
    expect(view.family).toBe("snapshot_missing")
    expect(view.title).toBe("503 snapshot missing")
    expect(view.tone).toBe("red")
    expect(view.detail).not.toMatch(/live-qualified/i)
    expect(view.detail).toMatch(/No invented fills/)
  })

  it("maps 503 snapshot_invalid separately from missing", () => {
    const view = mapApiError(
      503,
      errorBody("data_unavailable", "snapshot_invalid")
    )
    expect(view.family).toBe("snapshot_invalid")
    expect(view.title).toBe("503 snapshot invalid")
  })

  it("maps 429 query_budget_exceeded", () => {
    const view = mapApiError(
      429,
      errorBody("query_budget_exceeded", "query.concurrency")
    )
    expect(view.family).toBe("query_budget")
    expect(view.title).toBe("429 query budget")
    expect(view.reasonCode).toBe("query.concurrency")
  })

  it("maps 400 query.max_rows as query budget, not invalid query", () => {
    const view = mapApiError(
      400,
      errorBody("query_budget_exceeded", "query.max_rows")
    )
    expect(view.family).toBe("query_budget")
    expect(
      familyOf(400, errorBody("query_budget_exceeded", "query.max_rows"))
    ).toBe("query_budget")
  })

  it("maps 400 invalid_query", () => {
    const view = mapApiError(
      400,
      errorBody("invalid_query", "query.offset_forbidden")
    )
    expect(view.family).toBe("invalid_query")
    expect(view.title).toBe("400 invalid query")
  })

  it("maps WS 501 stream.websocket_unspecified", () => {
    const view = mapApiError(
      501,
      errorBody("not_implemented", "stream.websocket_unspecified")
    )
    expect(view.family).toBe("stream_unspecified")
    expect(view.title).toBe("501 stream unspecified")
    expect(view.tone).toBe("yellow")
  })
})

describe("classifyHttpBody", () => {
  it("treats HTTP 200 without an error body as not observed, not a pass", () => {
    const outcome = classifyHttpBody(200, {
      schema_version: "hl.health.v1",
      scope: "canonical",
      state: "HEALTH_STATE_GREEN",
      reason_code: "healthy",
      observed_at_micros: 1,
      suppresses: [],
    })
    expect(outcome.kind).toBe("not_observed")
    if (outcome.kind === "not_observed") {
      expect(outcome.detail).toMatch(/Not a Stage PASS/)
      expect(outcome.detail).not.toMatch(/live-qualified/i)
    }
  })

  it("observes a typed 400 invalid query body", () => {
    const outcome = classifyHttpBody(
      400,
      errorBody("invalid_query", "query.offset_forbidden")
    )
    expect(outcome.kind).toBe("observed")
    if (outcome.kind === "observed") {
      expect(outcome.view.family).toBe("invalid_query")
    }
  })
})

describe("parseCaptureStatus extras", () => {
  it("keeps frozen v4 fields and records unknown extras without crashing", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        later_unknown: "ignored-as-qualification",
        px: 1.5,
      })
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    const status: CaptureStatus = parsed.value
    expect(status.health).toBe("red")
    expect(status.ready).toBe(false)
    expect(status.extra_fields.later_unknown).toBe("ignored-as-qualification")
    expect(status.extra_fields.px).toBe(1.5)
  })

  it("renders restart_reconstruction and v5 maintenance when present", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        schema_version: "hl.capture.status.v5",
        restart_reconstruction: "incomplete",
        maintenance: {
          enabled: true,
          kill_switch: false,
          health: "yellow",
          reason_code: "capture_maintenance.degraded",
          pending_pack_manifest_count: 1,
          packed_range_count: 0,
          logical_manifest_count: 2,
          physical_data_object_count: 2,
          retention_authorized: false,
        },
        auxiliary_sources: [
          {
            source_id: "node-line-a",
            health: "starting",
            qualification: "unqualified",
            spool_records: 0,
            unarchived_records: 0,
            partial_line: false,
            restart_reconstruction: "complete",
            future_aux_flag: true,
          },
        ],
      })
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.schema_version).toBe("hl.capture.status.v5")
    expect(parsed.value.extra_fields.restart_reconstruction).toBe("incomplete")
    expect(parsed.value.extra_fields.maintenance).toEqual(
      expect.objectContaining({
        enabled: true,
        health: "yellow",
        retention_authorized: false,
      })
    )
    const aux = parsed.value.auxiliary_sources?.[0]
    expect(aux?.qualification).toBe("unqualified")
    expect(aux?.restart_reconstruction).toBe("complete")
    expect(aux?.extra_fields.future_aux_flag).toBe(true)
  })

  it("ignores malformed restart_reconstruction without rejecting the snapshot", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        auxiliary_sources: [
          {
            source_id: "node-line-a",
            health: "starting",
            qualification: "unqualified",
            spool_records: 0,
            unarchived_records: 0,
            partial_line: false,
            restart_reconstruction: 12,
          },
        ],
      })
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(
      parsed.value.auxiliary_sources?.[0]?.restart_reconstruction
    ).toBeUndefined()
  })
})
