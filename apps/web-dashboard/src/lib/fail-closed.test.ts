import { describe, expect, it } from "vitest"

import {
  CORE_DEADLETTER_REASONS,
  lastHeartbeatThroughput,
  parseCaptureStatus,
  parseCoreHealth,
  parseCoreStatus,
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
    expect(view.tone).not.toBe("green")
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

  it.each([
    ["core.deadletter_unsafe_path", "503 dead-letter unsafe path"],
    ["core.deadletter_io", "503 dead-letter I/O"],
    ["core.deadletter_invalid_record", "503 dead-letter invalid record"],
    ["core.deadletter_serialization", "503 dead-letter serialization"],
    ["core.deadletter_corrupt", "503 dead-letter corrupt"],
  ] as const)(
    "maps 503 %s as typed dead-letter fail-closed, not generic data_unavailable",
    (reason_code, title) => {
      const view = mapApiError(503, errorBody("data_unavailable", reason_code))
      expect(view.family).toBe("core_deadletter")
      expect(view.family).not.toBe("data_unavailable")
      expect(view.title).toBe(title)
      expect(view.title).not.toBe("503 data unavailable")
      expect(view.tone).toBe("red")
      expect(view.tone).not.toBe("green")
      expect(view.detail).not.toMatch(/live-qualified|Stage 6|Stage PASS/i)
      expect(view.reasonCode).toBe(reason_code)
      expect(familyOf(503, errorBody("data_unavailable", reason_code))).toBe(
        "core_deadletter"
      )
    }
  )

  it("does not treat unknown core.deadletter_* as ready or as a known dead-letter family", () => {
    const view = mapApiError(
      503,
      errorBody("data_unavailable", "core.deadletter_unspecified_future")
    )
    expect(view.family).not.toBe("core_deadletter")
    expect(view.family).toBe("data_unavailable")
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
    expect(view.title).not.toMatch(/ready/i)
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

  it("maps last-heartbeat throughput extras when present without inventing missing rates", () => {
    const present = parseCaptureStatus(
      v4Status({
        throughput_records_per_sec: 3,
        throughput_blocks_per_sec: 1,
      })
    )
    expect(present.ok).toBe(true)
    if (!present.ok) {
      return
    }
    expect(present.value.throughput_records_per_sec).toBe(3)
    expect(present.value.throughput_blocks_per_sec).toBe(1)
    expect(
      present.value.extra_fields.throughput_records_per_sec
    ).toBeUndefined()
    expect(present.value.extra_fields.throughput_blocks_per_sec).toBeUndefined()

    const idle = parseCaptureStatus(
      v4Status({
        throughput_records_per_sec: 0,
        throughput_blocks_per_sec: 0,
      })
    )
    expect(idle.ok).toBe(true)
    if (!idle.ok) {
      return
    }
    expect(idle.value.throughput_records_per_sec).toBe(0)
    expect(idle.value.throughput_blocks_per_sec).toBe(0)

    const missing = parseCaptureStatus(v4Status())
    expect(missing.ok).toBe(true)
    if (!missing.ok) {
      return
    }
    expect(missing.value.throughput_records_per_sec).toBeUndefined()
    expect(missing.value.throughput_blocks_per_sec).toBeUndefined()
    expect("throughput_records_per_sec" in missing.value.extra_fields).toBe(
      false
    )
    expect("throughput_blocks_per_sec" in missing.value.extra_fields).toBe(
      false
    )
    expect(
      lastHeartbeatThroughput(missing.value.extra_fields)
        .throughput_records_per_sec
    ).toBeUndefined()
  })

  it("keeps malformed throughput extras without rejecting or inventing rates", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        throughput_records_per_sec: -1,
        throughput_blocks_per_sec: "fast",
        later_unknown: "still-ignored",
      })
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.throughput_records_per_sec).toBeUndefined()
    expect(parsed.value.throughput_blocks_per_sec).toBeUndefined()
    expect(parsed.value.extra_fields.throughput_records_per_sec).toBe(-1)
    expect(parsed.value.extra_fields.throughput_blocks_per_sec).toBe("fast")
    expect(parsed.value.extra_fields.later_unknown).toBe("still-ignored")
  })

  it("maps a single present throughput field without inventing the other", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        throughput_records_per_sec: 7,
      })
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.throughput_records_per_sec).toBe(7)
    expect(parsed.value.throughput_blocks_per_sec).toBeUndefined()
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

function coreHealth503(reason_code: string): Record<string, unknown> {
  return {
    schema_version: "hl.core.health.v1",
    ok: false,
    ready: false,
    reason_code,
    live_qualified: false,
    stage_2_qualified: false,
  }
}

function coreStatusFailClosed(
  reason: string,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    schema_version: "hl.core.status.v1",
    ready: false,
    fail_closed_reason: reason,
    live_qualified: false,
    stage_2_qualified: false,
    ...overrides,
  }
}

describe("hl-core dead-letter health and status", () => {
  it.each(CORE_DEADLETTER_REASONS)(
    "classifies /healthz 503 %s as typed fail-closed, not invalid or ready",
    (reason_code) => {
      const outcome = classifyHttpBody(503, coreHealth503(reason_code))
      expect(outcome.kind).toBe("observed")
      if (outcome.kind !== "observed") {
        return
      }
      expect(outcome.view.family).toBe("core_deadletter")
      expect(outcome.view.family).not.toBe("data_unavailable")
      expect(outcome.view.httpStatus).toBe(503)
      expect(outcome.view.reasonCode).toBe(reason_code)
      expect(outcome.view.tone).toBe("red")
      expect(outcome.view.tone).not.toBe("green")
      expect(outcome.view.title).toMatch(/^503 dead-letter /)
      expect(outcome.view.title).not.toBe("503 data unavailable")
      expect(outcome.view.detail).not.toMatch(/live-qualified|Stage PASS/i)
    }
  )

  it.each(CORE_DEADLETTER_REASONS)(
    "classifies /status HTTP 200 with fail_closed_reason %s as typed 503, not ready",
    (reason_code) => {
      const outcome = classifyHttpBody(200, coreStatusFailClosed(reason_code))
      expect(outcome.kind).toBe("observed")
      if (outcome.kind !== "observed") {
        return
      }
      expect(outcome.view.family).toBe("core_deadletter")
      expect(outcome.view.family).not.toBe("data_unavailable")
      expect(outcome.view.reasonCode).toBe(reason_code)
      expect(outcome.view.tone).toBe("red")
      expect(outcome.view.title).toMatch(/^503 dead-letter /)
      expect(outcome.view.title).not.toBe("503 data unavailable")
      expect(outcome.view.detail).toMatch(/fail_closed_reason/)
    }
  )

  it("keeps omitted fail_closed_reason and last_applied_watermark omitted, not 0", () => {
    const parsed = parseCoreStatus({
      schema_version: "hl.core.status.v1",
      ready: true,
      live_qualified: false,
      stage_2_qualified: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.fail_closed_reason).toBeUndefined()
    expect(parsed.value.last_applied_watermark).toBeUndefined()

    const classified = classifyHttpBody(200, {
      schema_version: "hl.core.status.v1",
      ready: true,
      live_qualified: false,
      stage_2_qualified: false,
    })
    expect(classified.kind).toBe("not_observed")
    if (classified.kind === "not_observed") {
      expect(classified.detail).toMatch(/Not a Stage PASS/)
      expect(classified.detail).not.toMatch(/ready and live-qualified/i)
    }
  })

  it("preserves last_applied_watermark 0 when present and still omits missing rates", () => {
    const parsed = parseCoreStatus(
      coreStatusFailClosed("core.deadletter_unsafe_path", {
        last_applied_watermark: 0,
      })
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.last_applied_watermark).toBe(0)
    expect(parsed.value.fail_closed_reason).toBe("core.deadletter_unsafe_path")

    const missingThroughput = parseCaptureStatus(v4Status())
    expect(missingThroughput.ok).toBe(true)
    if (!missingThroughput.ok) {
      return
    }
    expect(missingThroughput.value.throughput_records_per_sec).toBeUndefined()
    expect(missingThroughput.value.throughput_blocks_per_sec).toBeUndefined()
  })

  it("fail-closes unknown /healthz reason codes instead of showing ready", () => {
    const outcome = classifyHttpBody(
      503,
      coreHealth503("core.deadletter_unspecified_future")
    )
    expect(outcome.kind).toBe("observed")
    if (outcome.kind !== "observed") {
      return
    }
    expect(outcome.view.family).not.toBe("core_deadletter")
    expect(outcome.view.family).toBe("data_unavailable")
    expect(outcome.view.tone).toBe("red")
    expect(outcome.view.tone).not.toBe("green")
    expect(outcome.view.title).not.toMatch(/ready/i)
  })

  it("fail-closes qualification claims on core health rather than painting green", () => {
    const outcome = classifyHttpBody(200, {
      schema_version: "hl.core.health.v1",
      ok: true,
      ready: true,
      reason_code: null,
      live_qualified: true,
      stage_2_qualified: false,
    })
    expect(outcome.kind).toBe("observed")
    if (outcome.kind !== "observed") {
      return
    }
    expect(outcome.view.tone).toBe("red")
    expect(outcome.view.tone).not.toBe("green")
    expect(outcome.view.family).not.toBe("core_deadletter")
  })

  it("parses ready core /healthz without inventing a dead-letter reason", () => {
    const parsed = parseCoreHealth({
      schema_version: "hl.core.health.v1",
      ok: true,
      ready: true,
      reason_code: null,
      live_qualified: false,
      stage_2_qualified: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.reason_code).toBeNull()
    expect(parsed.value.ready).toBe(true)

    const classified = classifyHttpBody(200, {
      schema_version: "hl.core.health.v1",
      ok: true,
      ready: true,
      reason_code: null,
      live_qualified: false,
      stage_2_qualified: false,
    })
    expect(classified.kind).toBe("not_observed")
  })
})
