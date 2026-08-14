import { readFileSync } from "node:fs"
import { dirname, join } from "node:path"
import { fileURLToPath } from "node:url"

import { describe, expect, it } from "vitest"

import type { FeedState } from "@/hooks/use-hl-api"
import type { DeskFeed, EndpointOutcome } from "@/lib/api"
import {
  AUXILIARY_SOURCE_FIELD_ORDER,
  AUXILIARY_SOURCE_HEALTH,
  CAPTURE_HEALTH_NOT_READY_REASONS,
  CAPTURE_SOURCE_HEALTH,
  CAPTURE_STATUS_FIELD_ORDER,
  COMMITTED_SOURCE_CLASS,
  CORE_DEADLETTER_REASONS,
  FAILOVER_REASONS,
  LEDGER_UNSUPPORTED_EVENT_REASONS,
  RESTART_RECONSTRUCTION,
  lastHeartbeatThroughput,
  parseCaptureHealth,
  parseCaptureStatus,
  parseCoreHealth,
  parseCoreStatus,
  parseHealthAssessment,
  type ApiError,
  type HealthBody,
} from "@/lib/contracts"
import { deriveConnection } from "@/lib/derive-connection"
import {
  LEFTOVER_V4_OMITTED_DETAIL,
  captureHealthIsFailClosed,
  captureHealthObservedReason,
  captureHealthOmittedReasonUnready,
  captureHealthWatchView,
  classifyHttpBody,
  coreHealthIsFailClosed,
  coreHealthWatchView,
  familyOf,
  healthBodyIsFailClosed,
  leftoverV4LaneKind,
  mapApiError,
  type ProbeOutcome,
} from "@/lib/fail-closed"

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
    expect(view.family).not.toBe("ledger_unsupported_event")
    expect(view.family).not.toBe("capture_health_not_ready")
    expect(view.family).toBe("data_unavailable")
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
    expect(view.title).not.toMatch(/ready/i)
  })

  it.each(LEDGER_UNSUPPORTED_EVENT_REASONS)(
    "maps 503 %s as typed consume-poison fail-closed, not generic data_unavailable",
    (reason_code) => {
      const view = mapApiError(503, errorBody("data_unavailable", reason_code))
      expect(view.family).toBe("ledger_unsupported_event")
      expect(view.family).not.toBe("data_unavailable")
      expect(view.family).not.toBe("core_deadletter")
      expect(view.family).not.toBe("capture_health_not_ready")
      expect(view.title).toBe("503 ledger unsupported event")
      expect(view.title).not.toBe("503 data unavailable")
      expect(view.tone).toBe("red")
      expect(view.tone).not.toBe("green")
      expect(view.detail).toMatch(/action-bearing or poison/)
      expect(view.detail).not.toMatch(/live-qualified|Stage 6|Stage PASS/i)
      expect(view.reasonCode).toBe(reason_code)
      expect(familyOf(503, errorBody("data_unavailable", reason_code))).toBe(
        "ledger_unsupported_event"
      )
    }
  )

  it("does not treat unknown ledger.* as ready or as ledger_unsupported_event", () => {
    const view = mapApiError(
      503,
      errorBody("data_unavailable", "ledger.unspecified_future")
    )
    expect(view.family).not.toBe("ledger_unsupported_event")
    expect(view.family).not.toBe("core_deadletter")
    expect(view.family).not.toBe("capture_health_not_ready")
    expect(view.family).toBe("data_unavailable")
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
    expect(view.title).not.toMatch(/ready/i)
  })

  it.each(CAPTURE_HEALTH_NOT_READY_REASONS)(
    "maps 503 %s as typed leftover v4 / not live-ready capture healthz, not generic data_unavailable",
    (reason_code) => {
      const view = mapApiError(503, errorBody("data_unavailable", reason_code))
      expect(view.family).toBe("capture_health_not_ready")
      expect(view.family).not.toBe("data_unavailable")
      expect(view.family).not.toBe("core_deadletter")
      expect(view.family).not.toBe("ledger_unsupported_event")
      expect(view.title).toBe("503 capture health not ready")
      expect(view.title).not.toBe("503 data unavailable")
      expect(view.tone).toBe("red")
      expect(view.tone).not.toBe("green")
      expect(view.detail).toMatch(/leftover v4 or not live-ready/)
      expect(view.detail).not.toMatch(/Stage 6|Stage PASS/i)
      expect(view.reasonCode).toBe(reason_code)
      expect(familyOf(503, errorBody("data_unavailable", reason_code))).toBe(
        "capture_health_not_ready"
      )
    }
  )

  it("does not treat unknown capture_health.* as ready or as capture_health_not_ready", () => {
    const view = mapApiError(
      503,
      errorBody("data_unavailable", "capture_health.unspecified_future")
    )
    expect(view.family).not.toBe("capture_health_not_ready")
    expect(view.family).not.toBe("core_deadletter")
    expect(view.family).not.toBe("ledger_unsupported_event")
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
  it("allowlists this web parse's known CaptureStatus fields, not OpenAPI Base", () => {
    expect(CAPTURE_STATUS_FIELD_ORDER).toContain("failover_reason")
    expect(CAPTURE_STATUS_FIELD_ORDER).toContain("throughput_records_per_sec")
    expect(CAPTURE_STATUS_FIELD_ORDER).toContain("throughput_blocks_per_sec")
    expect(CAPTURE_STATUS_FIELD_ORDER).not.toContain("maintenance")
    expect(CAPTURE_STATUS_FIELD_ORDER).not.toContain("restart_reconstruction")
  })

  it("fail-closes present unknown top-level keys as invalid, not recorded extras", () => {
    for (const extra of ["fills", "invented", "adapter"] as const) {
      const parsed = parseCaptureStatus(v4Status({ [extra]: true }))
      expect(parsed.ok).toBe(false)
      if (parsed.ok) {
        return
      }
      expect(parsed.detail).toBe(`unknown capture status field: ${extra}`)
    }

    const both = parseCaptureStatus(
      v4Status({
        later_unknown: "ignored-as-qualification",
        px: 1.5,
      })
    )
    expect(both.ok).toBe(false)
    if (both.ok) {
      return
    }
    expect(both.detail).toBe("unknown capture status fields: later_unknown, px")
  })

  it("fail-closes top-level restart_reconstruction and maintenance as extras", () => {
    const restart = parseCaptureStatus(
      v4Status({ restart_reconstruction: "incomplete" })
    )
    expect(restart.ok).toBe(false)
    if (restart.ok) {
      return
    }
    expect(restart.detail).toBe(
      "unknown capture status field: restart_reconstruction"
    )

    const maintenance = parseCaptureStatus(
      v4Status({
        schema_version: "hl.capture.status.v5",
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
      })
    )
    expect(maintenance.ok).toBe(false)
    if (maintenance.ok) {
      return
    }
    expect(maintenance.detail).toBe("unknown capture status field: maintenance")
  })

  it("parses a known payload without extras, including v5 without maintenance", () => {
    const v4 = parseCaptureStatus(v4Status())
    expect(v4.ok).toBe(true)
    if (!v4.ok) {
      return
    }
    expect(v4.value.schema_version).toBe("hl.capture.status.v4")
    expect(v4.value.health).toBe("red")
    expect(v4.value.ready).toBe(false)
    expect(v4.value.extra_fields).toEqual({})

    const v5 = parseCaptureStatus(
      v4Status({ schema_version: "hl.capture.status.v5" })
    )
    expect(v5.ok).toBe(true)
    if (!v5.ok) {
      return
    }
    expect(v5.value.schema_version).toBe("hl.capture.status.v5")
    expect(v5.value.extra_fields).toEqual({})
  })

  it("allowlists this web parse's known auxiliary-source fields", () => {
    expect(AUXILIARY_SOURCE_FIELD_ORDER).toEqual([
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
    ])
    expect(AUXILIARY_SOURCE_FIELD_ORDER).toContain("restart_reconstruction")
    expect(AUXILIARY_SOURCE_FIELD_ORDER).not.toContain("maintenance")
  })

  it("fail-closes present unknown nested auxiliary keys as invalid, not recorded extras", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        schema_version: "hl.capture.status.v5",
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
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toBe(
      "auxiliary_sources[0] unknown field: future_aux_flag"
    )

    const both = parseCaptureStatus(
      v4Status({
        auxiliary_sources: [
          {
            source_id: "node-line-a",
            health: "starting",
            qualification: "unqualified",
            spool_records: 0,
            unarchived_records: 0,
            partial_line: false,
            later_aux: true,
            fills: 1,
          },
        ],
      })
    )
    expect(both.ok).toBe(false)
    if (both.ok) {
      return
    }
    expect(both.detail).toBe(
      "auxiliary_sources[0] unknown fields: fills, later_aux"
    )
  })

  it("parses a known auxiliary source without extras", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        schema_version: "hl.capture.status.v5",
        auxiliary_sources: [
          {
            source_id: "node-line-a",
            health: "starting",
            qualification: "unqualified",
            spool_records: 0,
            unarchived_records: 0,
            partial_line: false,
            restart_reconstruction: "complete",
          },
        ],
      })
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.schema_version).toBe("hl.capture.status.v5")
    expect(parsed.value.extra_fields).toEqual({})
    const aux = parsed.value.auxiliary_sources?.[0]
    expect(aux?.source_id).toBe("node-line-a")
    expect(aux?.qualification).toBe("unqualified")
    expect(aux?.restart_reconstruction).toBe("complete")
    expect(aux?.extra_fields).toEqual({})
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
    const mapped = lastHeartbeatThroughput(missing.value.extra_fields)
    expect(mapped.ok).toBe(true)
    if (!mapped.ok) {
      return
    }
    expect(mapped.value.throughput_records_per_sec).toBeUndefined()
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
})

describe("parseCaptureStatus source health", () => {
  it("accepts every constructible committed source health", () => {
    for (const health of CAPTURE_SOURCE_HEALTH) {
      const parsed = parseCaptureStatus(
        v4Status({ primary_source_health: health })
      )
      expect(parsed.ok).toBe(true)
      if (!parsed.ok) {
        return
      }
      expect(parsed.value.primary_source_health).toBe(health)
    }
  })

  it("fail-closes unknown primary_source_health as invalid, not a quiet chip", () => {
    const parsed = parseCaptureStatus(
      v4Status({ primary_source_health: "degraded" })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(/primary_source_health must be one of/)
    expect(parsed.detail).toMatch(/starting/)
    expect(parsed.detail).toMatch(/healthy/)
    expect(parsed.detail).toMatch(/range-unavailable/)
    expect(parsed.detail).not.toMatch(/degraded/)
  })

  it("fail-closes auxiliary health strings on the committed source field", () => {
    const parsed = parseCaptureStatus(
      v4Status({ primary_source_health: "latched" })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(/primary_source_health must be one of/)
  })

  it("accepts every constructible independent source health and rejects unknown", () => {
    for (const health of CAPTURE_SOURCE_HEALTH) {
      const parsed = parseCaptureStatus(
        v4Status({ independent_source_health: health })
      )
      expect(parsed.ok).toBe(true)
      if (!parsed.ok) {
        return
      }
      expect(parsed.value.independent_source_health).toBe(health)
    }

    const unknown = parseCaptureStatus(
      v4Status({ independent_source_health: "latched" })
    )
    expect(unknown.ok).toBe(false)
    if (unknown.ok) {
      return
    }
    expect(unknown.detail).toMatch(/independent_source_health must be one of/)
  })

  it("still fail-closes unknown auxiliary source health", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        auxiliary_sources: [
          {
            source_id: "node-line-a",
            health: "range-unavailable",
            qualification: "unqualified",
            spool_records: 0,
            unarchived_records: 0,
            partial_line: false,
          },
        ],
      })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(
      /auxiliary_sources\[0\]\.health must be one of/
    )
    expect(AUXILIARY_SOURCE_HEALTH).toEqual([
      "starting",
      "healthy",
      "quarantined",
      "latched",
    ])
  })
})

describe("parseCaptureStatus committed source class", () => {
  it("accepts every constructible committed source class", () => {
    expect(COMMITTED_SOURCE_CLASS).toEqual([
      "locally-verified-committed",
      "independent-committed",
    ])
    for (const source of COMMITTED_SOURCE_CLASS) {
      const parsed = parseCaptureStatus(
        v4Status({ active_committed_source: source })
      )
      expect(parsed.ok).toBe(true)
      if (!parsed.ok) {
        return
      }
      expect(parsed.value.active_committed_source).toBe(source)
    }
  })

  it("fail-closes unknown active_committed_source as invalid, not a quiet chip", () => {
    const parsed = parseCaptureStatus(
      v4Status({ active_committed_source: "primary" })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(/active_committed_source must be one of/)
    expect(parsed.detail).toMatch(/locally-verified-committed/)
    expect(parsed.detail).toMatch(/independent-committed/)
    expect(parsed.detail).not.toMatch(/primary/)
  })

  it("fail-closes empty active_committed_source instead of admitting a free string", () => {
    const parsed = parseCaptureStatus(
      v4Status({ active_committed_source: "" })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(/active_committed_source must be one of/)
  })
})

describe("parseCaptureStatus last-heartbeat throughput", () => {
  it("accepts known integer throughput including 0", () => {
    for (const throughput_records_per_sec of [0, 3]) {
      const parsed = parseCaptureStatus(
        v4Status({ throughput_records_per_sec })
      )
      expect(parsed.ok).toBe(true)
      if (!parsed.ok) {
        return
      }
      expect(parsed.value.throughput_records_per_sec).toBe(
        throughput_records_per_sec
      )
      expect(parsed.value.throughput_blocks_per_sec).toBeUndefined()
    }
    for (const throughput_blocks_per_sec of [0, 1]) {
      const parsed = parseCaptureStatus(
        v4Status({ throughput_blocks_per_sec })
      )
      expect(parsed.ok).toBe(true)
      if (!parsed.ok) {
        return
      }
      expect(parsed.value.throughput_blocks_per_sec).toBe(
        throughput_blocks_per_sec
      )
      expect(parsed.value.throughput_records_per_sec).toBeUndefined()
    }
  })

  it("keeps omitted and null throughput omitted", () => {
    const omitted = parseCaptureStatus(v4Status())
    expect(omitted.ok).toBe(true)
    if (!omitted.ok) {
      return
    }
    expect(omitted.value.throughput_records_per_sec).toBeUndefined()
    expect(omitted.value.throughput_blocks_per_sec).toBeUndefined()

    const nulled = parseCaptureStatus(
      v4Status({
        throughput_records_per_sec: null,
        throughput_blocks_per_sec: null,
      })
    )
    expect(nulled.ok).toBe(true)
    if (!nulled.ok) {
      return
    }
    expect(nulled.value.throughput_records_per_sec).toBeUndefined()
    expect(nulled.value.throughput_blocks_per_sec).toBeUndefined()
  })

  it("fail-closes present non-integer throughput as invalid, not a dropped rate", () => {
    for (const throughput_records_per_sec of [
      "0",
      true,
      { not: "a-u64" },
      ["not-a-u64"],
    ]) {
      const parsed = parseCaptureStatus(
        v4Status({
          throughput_records_per_sec,
          throughput_blocks_per_sec: 1,
          later_unknown: "still-ignored",
        })
      )
      expect(parsed.ok).toBe(false)
      if (parsed.ok) {
        return
      }
      expect(parsed.detail).toBe(
        "throughput_records_per_sec must be a non-negative integer"
      )
    }
    for (const throughput_blocks_per_sec of [
      "fast",
      false,
      { not: "a-u64" },
      ["not-a-u64"],
    ]) {
      const parsed = parseCaptureStatus(
        v4Status({
          throughput_records_per_sec: 7,
          throughput_blocks_per_sec,
        })
      )
      expect(parsed.ok).toBe(false)
      if (parsed.ok) {
        return
      }
      expect(parsed.detail).toBe(
        "throughput_blocks_per_sec must be a non-negative integer"
      )
    }
  })

  it("fail-closes negative throughput instead of dropping to undefined", () => {
    const records = parseCaptureStatus(
      v4Status({ throughput_records_per_sec: -1 })
    )
    expect(records.ok).toBe(false)
    if (records.ok) {
      return
    }
    expect(records.detail).toBe(
      "throughput_records_per_sec must be a non-negative integer"
    )

    const blocks = parseCaptureStatus(
      v4Status({ throughput_blocks_per_sec: -1 })
    )
    expect(blocks.ok).toBe(false)
    if (blocks.ok) {
      return
    }
    expect(blocks.detail).toBe(
      "throughput_blocks_per_sec must be a non-negative integer"
    )
  })

  it("fail-closes fractional throughput instead of dropping to undefined", () => {
    const records = parseCaptureStatus(
      v4Status({ throughput_records_per_sec: 1.5 })
    )
    expect(records.ok).toBe(false)
    if (records.ok) {
      return
    }
    expect(records.detail).toBe(
      "throughput_records_per_sec must be a non-negative integer"
    )

    const blocks = parseCaptureStatus(
      v4Status({ throughput_blocks_per_sec: 0.5 })
    )
    expect(blocks.ok).toBe(false)
    if (blocks.ok) {
      return
    }
    expect(blocks.detail).toBe(
      "throughput_blocks_per_sec must be a non-negative integer"
    )
  })
})

describe("parseCaptureStatus failover reason", () => {
  it("accepts every constructible failover reason", () => {
    expect(FAILOVER_REASONS).toEqual(["primary-range-unavailable"])
    for (const reason of FAILOVER_REASONS) {
      const parsed = parseCaptureStatus(
        v4Status({ failover_reason: reason })
      )
      expect(parsed.ok).toBe(true)
      if (!parsed.ok) {
        return
      }
      expect(parsed.value.failover_reason).toBe(reason)
      expect(parsed.value.failover_height).toBeUndefined()
    }
  })

  it("keeps omitted and null failover_reason omitted", () => {
    const omitted = parseCaptureStatus(v4Status())
    expect(omitted.ok).toBe(true)
    if (!omitted.ok) {
      return
    }
    expect(omitted.value.failover_reason).toBeUndefined()
    expect(omitted.value.failover_height).toBeUndefined()

    const nulled = parseCaptureStatus(v4Status({ failover_reason: null }))
    expect(nulled.ok).toBe(true)
    if (!nulled.ok) {
      return
    }
    expect(nulled.value.failover_reason).toBeUndefined()
    expect(nulled.value.failover_height).toBeUndefined()
  })

  it("fail-closes unknown failover_reason as invalid, not a quiet chip", () => {
    const parsed = parseCaptureStatus(
      v4Status({ failover_reason: "manual-failover" })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toBe(
      "failover_reason must be one of primary-range-unavailable"
    )
  })

  it("fail-closes empty failover_reason instead of admitting a free string", () => {
    const parsed = parseCaptureStatus(v4Status({ failover_reason: "" }))
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(/failover_reason must be one of/)
  })

  it("fail-closes malformed failover_reason instead of admitting a free string", () => {
    const parsed = parseCaptureStatus(v4Status({ failover_reason: 12 }))
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(/failover_reason must be one of/)
  })
})

describe("parseCaptureStatus restart reconstruction", () => {
  function auxiliarySource(
    overrides: Record<string, unknown> = {}
  ): Record<string, unknown> {
    return {
      source_id: "node-line-a",
      health: "starting",
      qualification: "unqualified",
      spool_records: 0,
      unarchived_records: 0,
      partial_line: false,
      ...overrides,
    }
  }

  it("accepts every constructible restart reconstruction", () => {
    expect(RESTART_RECONSTRUCTION).toEqual([
      "not-required",
      "incomplete",
      "complete",
    ])
    for (const reconstruction of RESTART_RECONSTRUCTION) {
      const parsed = parseCaptureStatus(
        v4Status({
          auxiliary_sources: [
            auxiliarySource({ restart_reconstruction: reconstruction }),
          ],
        })
      )
      expect(parsed.ok).toBe(true)
      if (!parsed.ok) {
        return
      }
      expect(parsed.value.auxiliary_sources?.[0]?.restart_reconstruction).toBe(
        reconstruction
      )
    }
  })

  it("keeps omitted and null restart_reconstruction omitted", () => {
    const omitted = parseCaptureStatus(
      v4Status({ auxiliary_sources: [auxiliarySource()] })
    )
    expect(omitted.ok).toBe(true)
    if (!omitted.ok) {
      return
    }
    expect(
      omitted.value.auxiliary_sources?.[0]?.restart_reconstruction
    ).toBeUndefined()

    const nulled = parseCaptureStatus(
      v4Status({
        auxiliary_sources: [auxiliarySource({ restart_reconstruction: null })],
      })
    )
    expect(nulled.ok).toBe(true)
    if (!nulled.ok) {
      return
    }
    expect(
      nulled.value.auxiliary_sources?.[0]?.restart_reconstruction
    ).toBeUndefined()
  })

  it("fail-closes unknown restart_reconstruction as invalid, not ignored", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        auxiliary_sources: [
          auxiliarySource({ restart_reconstruction: "NotRequired" }),
        ],
      })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toBe(
      "auxiliary_sources[0].restart_reconstruction must be one of not-required, incomplete, complete"
    )
  })

  it("fail-closes malformed restart_reconstruction instead of ignoring it", () => {
    const parsed = parseCaptureStatus(
      v4Status({
        auxiliary_sources: [
          auxiliarySource({ restart_reconstruction: 12 }),
        ],
      })
    )
    expect(parsed.ok).toBe(false)
    if (parsed.ok) {
      return
    }
    expect(parsed.detail).toMatch(
      /auxiliary_sources\[0\]\.restart_reconstruction must be one of/
    )
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
    expect(outcome.view.family).not.toBe("ledger_unsupported_event")
    expect(outcome.view.family).not.toBe("capture_health_not_ready")
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
    expect(outcome.view.family).not.toBe("ledger_unsupported_event")
    expect(outcome.view.family).not.toBe("capture_health_not_ready")
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

describe("hl-core ledger.unsupported_event consume poison", () => {
  it.each(LEDGER_UNSUPPORTED_EVENT_REASONS)(
    "classifies /healthz 503 %s as typed fail-closed, not invalid or ready",
    (reason_code) => {
      const outcome = classifyHttpBody(503, coreHealth503(reason_code))
      expect(outcome.kind).toBe("observed")
      if (outcome.kind !== "observed") {
        return
      }
      expect(outcome.view.family).toBe("ledger_unsupported_event")
      expect(outcome.view.family).not.toBe("data_unavailable")
      expect(outcome.view.family).not.toBe("core_deadletter")
      expect(outcome.view.httpStatus).toBe(503)
      expect(outcome.view.reasonCode).toBe(reason_code)
      expect(outcome.view.tone).toBe("red")
      expect(outcome.view.tone).not.toBe("green")
      expect(outcome.view.title).toBe("503 ledger unsupported event")
      expect(outcome.view.title).not.toBe("503 data unavailable")
      expect(outcome.view.detail).toMatch(/action-bearing or poison/)
      expect(outcome.view.detail).not.toMatch(/live-qualified|Stage PASS/i)
    }
  )

  it.each(LEDGER_UNSUPPORTED_EVENT_REASONS)(
    "classifies /status HTTP 200 with fail_closed_reason %s as typed 503, not ready",
    (reason_code) => {
      const outcome = classifyHttpBody(200, coreStatusFailClosed(reason_code))
      expect(outcome.kind).toBe("observed")
      if (outcome.kind !== "observed") {
        return
      }
      expect(outcome.view.family).toBe("ledger_unsupported_event")
      expect(outcome.view.family).not.toBe("data_unavailable")
      expect(outcome.view.reasonCode).toBe(reason_code)
      expect(outcome.view.tone).toBe("red")
      expect(outcome.view.tone).not.toBe("green")
      expect(outcome.view.title).toBe("503 ledger unsupported event")
      expect(outcome.view.title).not.toBe("503 data unavailable")
      expect(outcome.view.detail).toMatch(/fail_closed_reason/)
    }
  )

  it("keeps omitted last_applied_watermark omitted, not 0, on ledger unsupported-event status", () => {
    const parsed = parseCoreStatus(
      coreStatusFailClosed("ledger.unsupported_event")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.fail_closed_reason).toBe("ledger.unsupported_event")
    expect(parsed.value.ready).toBe(false)
    expect(parsed.value.last_applied_watermark).toBeUndefined()
    expect(parsed.value.live_qualified).toBe(false)
    expect(parsed.value.stage_2_qualified).toBe(false)

    const missingThroughput = parseCaptureStatus(v4Status())
    expect(missingThroughput.ok).toBe(true)
    if (!missingThroughput.ok) {
      return
    }
    expect(missingThroughput.value.throughput_records_per_sec).toBeUndefined()
    expect(missingThroughput.value.throughput_blocks_per_sec).toBeUndefined()
  })

  it("fail-closes unknown ledger reason codes instead of showing ready", () => {
    const outcome = classifyHttpBody(
      503,
      coreHealth503("ledger.unspecified_future")
    )
    expect(outcome.kind).toBe("observed")
    if (outcome.kind !== "observed") {
      return
    }
    expect(outcome.view.family).not.toBe("ledger_unsupported_event")
    expect(outcome.view.family).not.toBe("capture_health_not_ready")
    expect(outcome.view.family).toBe("data_unavailable")
    expect(outcome.view.tone).toBe("red")
    expect(outcome.view.tone).not.toBe("green")
    expect(outcome.view.title).not.toMatch(/ready/i)
  })

  it("fail-closes /status HTTP 200 unknown ledger.* as red data_unavailable, not typed consume-poison", () => {
    const outcome = classifyHttpBody(
      200,
      coreStatusFailClosed("ledger.unspecified_future")
    )
    expect(outcome.kind).toBe("observed")
    if (outcome.kind !== "observed") {
      return
    }
    expect(outcome.view.family).toBe("data_unavailable")
    expect(outcome.view.family).not.toBe("ledger_unsupported_event")
    expect(outcome.view.family).not.toBe("core_deadletter")
    expect(outcome.view.family).not.toBe("capture_health_not_ready")
    expect(outcome.view.httpStatus).toBe(200)
    expect(outcome.view.reasonCode).toBe("ledger.unspecified_future")
    expect(outcome.view.tone).toBe("red")
    expect(outcome.view.tone).not.toBe("green")
    expect(outcome.view.title).not.toMatch(/ready/i)
    expect(outcome.view.title).not.toBe("503 ledger unsupported event")
  })
})

function captureHealth503(
  reason_code: string,
  overrides: Record<string, unknown> = {}
): Record<string, unknown> {
  return {
    schema_version: "hl.capture.health.v1",
    ok: false,
    reason_code,
    ready: false,
    ...overrides,
  }
}

describe("capture leftover v4 / not live-ready healthz", () => {
  it.each(CAPTURE_HEALTH_NOT_READY_REASONS)(
    "classifies leftover v4 /healthz 503 %s as typed fail-closed, not invalid or ready",
    (reason_code) => {
      const outcome = classifyHttpBody(503, captureHealth503(reason_code))
      expect(outcome.kind).toBe("observed")
      if (outcome.kind !== "observed") {
        return
      }
      expect(outcome.view.family).toBe("capture_health_not_ready")
      expect(outcome.view.family).not.toBe("data_unavailable")
      expect(outcome.view.family).not.toBe("core_deadletter")
      expect(outcome.view.family).not.toBe("ledger_unsupported_event")
      expect(outcome.view.httpStatus).toBe(503)
      expect(outcome.view.reasonCode).toBe(reason_code)
      expect(outcome.view.tone).toBe("red")
      expect(outcome.view.tone).not.toBe("green")
      expect(outcome.view.title).toBe("503 capture health not ready")
      expect(outcome.view.title).not.toBe("503 data unavailable")
      expect(outcome.view.detail).toMatch(/leftover v4 or not live-ready/)
      expect(outcome.view.detail).not.toMatch(/Stage 6|Stage PASS/i)
      expect(captureHealthObservedReason(reason_code)).toBe(reason_code)
    }
  )

  it.each(CAPTURE_HEALTH_NOT_READY_REASONS)(
    "classifies valid not-ready v5 /healthz 503 %s as typed fail-closed, not generic data_unavailable",
    (reason_code) => {
      const outcome = classifyHttpBody(503, captureHealth503(reason_code))
      expect(outcome.kind).toBe("observed")
      if (outcome.kind !== "observed") {
        return
      }
      expect(outcome.view.family).toBe("capture_health_not_ready")
      expect(outcome.view.family).not.toBe("data_unavailable")
      expect(outcome.view.tone).toBe("red")
      expect(outcome.view.tone).not.toBe("green")
      expect(outcome.view.title).toBe("503 capture health not ready")

      const notReadyV5 = parseCaptureStatus(
        v4Status({
          schema_version: "hl.capture.status.v5",
          health: "green",
          ready: false,
        })
      )
      expect(notReadyV5.ok).toBe(true)
      if (!notReadyV5.ok) {
        return
      }
      expect(notReadyV5.value.schema_version).toBe("hl.capture.status.v5")
      expect(notReadyV5.value.ready).toBe(false)
      expect(notReadyV5.value.throughput_records_per_sec).toBeUndefined()
      expect(notReadyV5.value.throughput_blocks_per_sec).toBeUndefined()
    }
  )

  it("keeps omitted last-heartbeat throughput omitted, not 0, beside leftover v4 healthz", () => {
    const parsed = parseCaptureHealth(
      captureHealth503("capture_health.not_ready")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.ok).toBe(false)
    expect(parsed.value.ready).toBe(false)
    expect(parsed.value.reason_code).toBe("capture_health.not_ready")
    expect(parsed.value.health).toBeUndefined()

    const leftoverV4 = parseCaptureStatus(v4Status())
    expect(leftoverV4.ok).toBe(true)
    if (!leftoverV4.ok) {
      return
    }
    expect(leftoverV4.value.schema_version).toBe("hl.capture.status.v4")
    expect(leftoverV4.value.ready).toBe(false)
    expect(leftoverV4.value.throughput_records_per_sec).toBeUndefined()
    expect(leftoverV4.value.throughput_blocks_per_sec).toBeUndefined()
  })

  it("omits ready on capture healthz status-read errors instead of inventing false or 0", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: false,
      reason_code: "capture_status.serialization",
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.ready).toBeUndefined()
    expect(parsed.value.health).toBeUndefined()
    expect(parsed.value.reason_code).toBe("capture_status.serialization")

    const outcome = classifyHttpBody(503, {
      schema_version: "hl.capture.health.v1",
      ok: false,
      reason_code: "capture_status.serialization",
    })
    expect(outcome.kind).toBe("observed")
    if (outcome.kind !== "observed") {
      return
    }
    expect(outcome.view.family).toBe("data_unavailable")
    expect(outcome.view.family).not.toBe("capture_health_not_ready")
    expect(outcome.view.tone).toBe("red")
    expect(outcome.view.tone).not.toBe("green")
    expect(outcome.view.title).not.toMatch(/ready/i)
  })

  it("fail-closes omitted capture healthz reason_code as unknown data_unavailable, not leftover v4", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.reason_code).toBeUndefined()
    expect(parsed.value.reason_code).not.toBe("capture_health.not_ready")
    expect(parsed.value.health).toBeUndefined()
    expect(captureHealthObservedReason(parsed.value.reason_code)).toBe(
      "data_unavailable"
    )
    expect(captureHealthObservedReason(parsed.value.reason_code)).not.toBe(
      "capture_health.not_ready"
    )

    const outcome = classifyHttpBody(503, {
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
    })
    expect(outcome.kind).toBe("observed")
    if (outcome.kind !== "observed") {
      return
    }
    expect(outcome.view.family).toBe("data_unavailable")
    expect(outcome.view.family).not.toBe("capture_health_not_ready")
    expect(outcome.view.family).not.toBe("core_deadletter")
    expect(outcome.view.family).not.toBe("ledger_unsupported_event")
    expect(outcome.view.reasonCode).toBe("data_unavailable")
    expect(outcome.view.reasonCode).not.toBe("capture_health.not_ready")
    expect(outcome.view.tone).toBe("red")
    expect(outcome.view.tone).not.toBe("green")
    expect(outcome.view.title).toBe("503 data unavailable")
    expect(outcome.view.title).not.toBe("503 capture health not ready")
    expect(outcome.view.title).not.toMatch(/ready/i)
    expect(outcome.view.detail).not.toMatch(/leftover v4/)
    expect(outcome.view.detail).not.toMatch(/Stage 6|Stage PASS/i)

    const nullReason = classifyHttpBody(503, {
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
      reason_code: null,
    })
    expect(nullReason.kind).toBe("observed")
    if (nullReason.kind === "observed") {
      expect(nullReason.view.family).toBe("data_unavailable")
      expect(nullReason.view.family).not.toBe("capture_health_not_ready")
      expect(nullReason.view.reasonCode).not.toBe("capture_health.not_ready")
      expect(nullReason.view.tone).toBe("red")
      expect(nullReason.view.tone).not.toBe("green")
    }

    const leftoverV4 = parseCaptureStatus(v4Status())
    expect(leftoverV4.ok).toBe(true)
    if (!leftoverV4.ok) {
      return
    }
    expect(leftoverV4.value.throughput_records_per_sec).toBeUndefined()
    expect(leftoverV4.value.throughput_blocks_per_sec).toBeUndefined()
  })

  it("keeps the leftover-v4 fail-closed lane unknown when healthz omits reason_code, not typed leftover-v4 and not a quiet not_observed", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(captureHealthOmittedReasonUnready(503, parsed.value)).toBe(true)
    expect(leftoverV4LaneKind(503, parsed.value)).toBe("unknown_omitted")
    expect(leftoverV4LaneKind(503, parsed.value)).not.toBe("typed")
    expect(leftoverV4LaneKind(503, parsed.value)).not.toBe("not_observed")
    expect(LEFTOVER_V4_OMITTED_DETAIL).toMatch(/omitted reason_code/)
    expect(LEFTOVER_V4_OMITTED_DETAIL).toMatch(/not typed/)
    expect(LEFTOVER_V4_OMITTED_DETAIL).toMatch(/Unknown data_unavailable/)
    expect(LEFTOVER_V4_OMITTED_DETAIL).toMatch(/not ready/)
    expect(LEFTOVER_V4_OMITTED_DETAIL).not.toMatch(/was not returned this poll/)
    expect(parsed.value.reason_code).not.toBe("capture_health.not_ready")

    const nullParsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
      reason_code: null,
    })
    expect(nullParsed.ok).toBe(true)
    if (!nullParsed.ok) {
      return
    }
    expect(nullParsed.value.reason_code).toBeUndefined()
    expect(leftoverV4LaneKind(503, nullParsed.value)).toBe("unknown_omitted")
    expect(leftoverV4LaneKind(503, nullParsed.value)).not.toBe("typed")
    expect(captureHealthObservedReason(nullParsed.value.reason_code)).toBe(
      "data_unavailable"
    )
    expect(captureHealthObservedReason(nullParsed.value.reason_code)).not.toBe(
      "capture_health.not_ready"
    )
  })

  it("keeps present capture_health.not_ready typed on the leftover-v4 lane", () => {
    const parsed = parseCaptureHealth(
      captureHealth503("capture_health.not_ready")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(captureHealthOmittedReasonUnready(503, parsed.value)).toBe(false)
    expect(leftoverV4LaneKind(503, parsed.value)).toBe("typed")
    expect(leftoverV4LaneKind(503, parsed.value)).not.toBe("unknown_omitted")
    expect(leftoverV4LaneKind(503, parsed.value)).not.toBe("not_observed")
  })

  it("does not treat ready capture healthz with omitted reason as leftover-v4 or as unknown omitted", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: true,
      health: "green",
      ready: true,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(captureHealthOmittedReasonUnready(200, parsed.value)).toBe(false)
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("not_observed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("typed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("unknown_omitted")
  })

  it("fail-closes unknown capture_health.* /healthz codes instead of showing ready", () => {
    const outcome = classifyHttpBody(
      503,
      captureHealth503("capture_health.unspecified_future")
    )
    expect(outcome.kind).toBe("observed")
    if (outcome.kind !== "observed") {
      return
    }
    expect(outcome.view.family).not.toBe("capture_health_not_ready")
    expect(outcome.view.family).not.toBe("core_deadletter")
    expect(outcome.view.family).not.toBe("ledger_unsupported_event")
    expect(outcome.view.family).toBe("data_unavailable")
    expect(outcome.view.tone).toBe("red")
    expect(outcome.view.tone).not.toBe("green")
    expect(outcome.view.title).not.toMatch(/ready/i)

    const parsed = parseCaptureHealth(
      captureHealth503("capture_health.unspecified_future")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(leftoverV4LaneKind(503, parsed.value)).toBe("not_observed")
    expect(leftoverV4LaneKind(503, parsed.value)).not.toBe("typed")
    expect(leftoverV4LaneKind(503, parsed.value)).not.toBe("unknown_omitted")
  })

  it("does not treat leftover v4 /status HTTP 200 as typed capture healthz or as a PASS", () => {
    const outcome = classifyHttpBody(200, v4Status())
    expect(outcome.kind).toBe("not_observed")
    if (outcome.kind === "not_observed") {
      expect(outcome.detail).toMatch(/Not a Stage PASS/)
    }
  })

  it("parses ready capture /healthz without inventing a not-ready reason or painting a PASS", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: true,
      health: "green",
      ready: true,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.reason_code).toBeUndefined()
    expect(parsed.value.ready).toBe(true)
    expect(parsed.value.health).toBe("green")

    const classified = classifyHttpBody(200, {
      schema_version: "hl.capture.health.v1",
      ok: true,
      health: "green",
      ready: true,
    })
    expect(classified.kind).toBe("not_observed")
    if (classified.kind === "not_observed") {
      expect(classified.detail).toMatch(/Not a Stage PASS/)
      expect(classified.detail).not.toMatch(/live-qualified/i)
    }
  })

  it("shows capture healthz HTTP 200 unready omitted reason as red unknown, not leftover-v4 and not ok", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.reason_code).toBeUndefined()
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("unknown_omitted")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("typed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("not_observed")

    const view = captureHealthWatchView(200, parsed.value)
    expect(view).toBeDefined()
    if (view === undefined) {
      return
    }
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
    expect(view.family).toBe("data_unavailable")
    expect(view.family).not.toBe("capture_health_not_ready")
    expect(view.reasonCode).toBe("data_unavailable")
    expect(view.reasonCode).not.toBe("capture_health.not_ready")
    expect(view.title).not.toMatch(/ready/i)
    expect(view.title).not.toBe("503 capture health not ready")

    const classified = classifyHttpBody(200, {
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
    })
    expect(classified.kind).toBe("observed")
    if (classified.kind !== "observed") {
      return
    }
    expect(classified.view.httpStatus).toBe(200)
    expect(classified.view.tone).toBe("red")
    expect(classified.view.tone).not.toBe("green")
    expect(classified.view.family).toBe("data_unavailable")
    expect(classified.view.family).not.toBe("capture_health_not_ready")
    expect(classified.view.reasonCode).not.toBe("capture_health.not_ready")

    const leftoverV4 = parseCaptureStatus(v4Status())
    expect(leftoverV4.ok).toBe(true)
    if (!leftoverV4.ok) {
      return
    }
    expect(leftoverV4.value.throughput_records_per_sec).toBeUndefined()
    expect(leftoverV4.value.throughput_blocks_per_sec).toBeUndefined()
  })

  it("keeps present capture_health.not_ready typed on leftover-v4 when healthz is HTTP 200", () => {
    const parsed = parseCaptureHealth(
      captureHealth503("capture_health.not_ready")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("typed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("unknown_omitted")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("not_observed")

    const view = captureHealthWatchView(200, parsed.value)
    expect(view).toBeDefined()
    if (view === undefined) {
      return
    }
    expect(view.family).toBe("capture_health_not_ready")
    expect(view.reasonCode).toBe("capture_health.not_ready")
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
  })

  it("fail-closes unknown capture_health.* on HTTP 200 as generic red, not leftover-v4", () => {
    const parsed = parseCaptureHealth(
      captureHealth503("capture_health.unspecified_future")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("not_observed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("typed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("unknown_omitted")

    const view = captureHealthWatchView(200, parsed.value)
    expect(view).toBeDefined()
    if (view === undefined) {
      return
    }
    expect(view.family).toBe("data_unavailable")
    expect(view.family).not.toBe("capture_health_not_ready")
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
    expect(view.title).not.toMatch(/ready/i)

    const classified = classifyHttpBody(
      200,
      captureHealth503("capture_health.unspecified_future")
    )
    expect(classified.kind).toBe("observed")
    if (classified.kind !== "observed") {
      return
    }
    expect(classified.view.httpStatus).toBe(200)
    expect(classified.view.family).toBe("data_unavailable")
    expect(classified.view.family).not.toBe("capture_health_not_ready")
    expect(classified.view.tone).toBe("red")
  })

  it("does not treat HTTP 200 ready omitted-reason capture healthz as fail-closed leftover-v4", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: true,
      health: "green",
      ready: true,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(captureHealthWatchView(200, parsed.value)).toBeUndefined()
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("not_observed")
  })

  it("shows core health HTTP 200 unready as red, not ok", () => {
    const parsed = parseCoreHealth({
      schema_version: "hl.core.health.v1",
      ok: false,
      ready: false,
      reason_code: null,
      live_qualified: false,
      stage_2_qualified: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    const view = coreHealthWatchView(200, parsed.value)
    expect(view).toBeDefined()
    if (view === undefined) {
      return
    }
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
    expect(view.family).not.toBe("capture_health_not_ready")
    expect(view.reasonCode).not.toBe("capture_health.not_ready")
    expect(view.title).not.toMatch(/ready/i)

    const classified = classifyHttpBody(200, {
      schema_version: "hl.core.health.v1",
      ok: false,
      ready: false,
      reason_code: null,
      live_qualified: false,
      stage_2_qualified: false,
    })
    expect(classified.kind).toBe("observed")
    if (classified.kind !== "observed") {
      return
    }
    expect(classified.view.httpStatus).toBe(200)
    expect(classified.view.tone).toBe("red")
    expect(classified.view.tone).not.toBe("green")
  })
})

describe("banner shares fail-closed helpers with chips", () => {
  const unusedProbe: ProbeOutcome = {
    kind: "not_observed",
    status: 200,
    detail: "fixture — not a PASS",
  }

  function leftoverGreen(): HealthBody {
    return {
      schema_version: "hl.health.v1",
      scope: "canonical",
      state: "HEALTH_STATE_GREEN",
      reason_code: "healthy",
      observed_at_micros: 1,
      suppresses: [],
    }
  }

  function okOutcome<T>(status: number, data: T): EndpointOutcome<T> {
    return { kind: "ok", status, data, raw: data }
  }

  function feedWithOnlyHealthz(
    healthz: EndpointOutcome<HealthBody>
  ): DeskFeed {
    const capture = parseCaptureStatus(
      v4Status({ health: "green", ready: true })
    )
    if (!capture.ok) {
      throw new Error(capture.detail)
    }
    const green = leftoverGreen()
    return {
      fetchedAt: 1,
      healthz,
      readyz: okOutcome(200, green),
      canonicalHealth: okOutcome(200, green),
      captureStatus: okOutcome(200, capture.value),
      stream: {
        kind: "http-error",
        status: 501,
        error: errorBody("not_implemented", "stream.websocket_unspecified"),
      },
      invalidQuery: unusedProbe,
      queryBudget: unusedProbe,
    }
  }

  function readyFromHealthz(healthz: EndpointOutcome<HealthBody>): FeedState {
    return { phase: "ready", feed: feedWithOnlyHealthz(healthz) }
  }

  function bannerFromHealthz(status: number, body: HealthBody) {
    return deriveConnection(readyFromHealthz(okOutcome(status, body)))
  }

  it("pins the App banner to deriveConnection, which includes /healthz in the production arrays", () => {
    const here = dirname(fileURLToPath(import.meta.url))
    const app = readFileSync(join(here, "../App.tsx"), "utf8")
    const wiring = readFileSync(join(here, "derive-connection.ts"), "utf8")
    expect(app).toContain("deriveConnection(state)")
    expect(wiring).toContain("[feed.healthz, feed.readyz, feed.canonicalHealth]")
    expect(wiring).toContain("isTypedCoreFailClosed(feed.healthz)")
  })

  it("degrades the banner for HTTP 200 unready omitted capture /healthz while chips stay red unknown", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: false,
      ready: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(parsed.value.reason_code).toBeUndefined()
    expect(captureHealthIsFailClosed(200, parsed.value)).toBe(true)
    expect(healthBodyIsFailClosed(200, parsed.value)).toBe(true)

    const feed = feedWithOnlyHealthz(okOutcome(200, parsed.value))
    const connection = deriveConnection({ phase: "ready", feed })
    expect(connection.kind).toBe("degraded")
    expect(connection.kind).not.toBe("polling")
    expect(connection.kind).not.toBe("unavailable")
    expect(connection.detail).not.toMatch(/capture_health\.not_ready/)
    expect(feed.captureStatus.kind).toBe("ok")
    if (feed.captureStatus.kind === "ok") {
      expect(feed.captureStatus.data.throughput_records_per_sec).toBeUndefined()
      expect(feed.captureStatus.data.throughput_blocks_per_sec).toBeUndefined()
      expect(feed.captureStatus.data.throughput_records_per_sec).not.toBe(0)
      expect(feed.captureStatus.data.throughput_blocks_per_sec).not.toBe(0)
    }

    const view = captureHealthWatchView(200, parsed.value)
    expect(view).toBeDefined()
    if (view === undefined) {
      return
    }
    expect(view.tone).toBe("red")
    expect(view.tone).not.toBe("green")
    expect(view.family).toBe("data_unavailable")
    expect(view.family).not.toBe("capture_health_not_ready")
    expect(view.reasonCode).toBe("data_unavailable")
    expect(view.reasonCode).not.toBe("capture_health.not_ready")
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("unknown_omitted")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("typed")
  })

  it("keeps present leftover-v4 capture_health.not_ready typed on chips; banner is unavailable not degraded", () => {
    const parsed = parseCaptureHealth(
      captureHealth503("capture_health.not_ready")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("typed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("unknown_omitted")
    expect(healthBodyIsFailClosed(200, parsed.value)).toBe(true)
    const view = captureHealthWatchView(200, parsed.value)
    expect(view?.family).toBe("capture_health_not_ready")
    expect(view?.reasonCode).toBe("capture_health.not_ready")
    expect(view?.tone).toBe("red")

    const connection = bannerFromHealthz(200, parsed.value)
    expect(connection.kind).toBe("unavailable")
    expect(connection.kind).not.toBe("degraded")
    expect(connection.kind).not.toBe("polling")
    expect(connection.detail).toMatch(/capture_health\.not_ready/)
  })

  it("fail-closes unknown capture_health.* as generic red on chips and degraded on the banner", () => {
    const parsed = parseCaptureHealth(
      captureHealth503("capture_health.unspecified_future")
    )
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("not_observed")
    expect(leftoverV4LaneKind(200, parsed.value)).not.toBe("typed")
    expect(healthBodyIsFailClosed(200, parsed.value)).toBe(true)
    const connection = bannerFromHealthz(200, parsed.value)
    expect(connection.kind).toBe("degraded")
    expect(connection.kind).not.toBe("unavailable")
    expect(connection.detail).not.toMatch(/capture_health\.not_ready/)
    const view = captureHealthWatchView(200, parsed.value)
    expect(view?.family).toBe("data_unavailable")
    expect(view?.family).not.toBe("capture_health_not_ready")
    expect(view?.tone).toBe("red")
    expect(view?.title).not.toMatch(/ready/i)
  })

  it("does not degrade the banner for HTTP 200 ready omitted-reason capture healthz", () => {
    const parsed = parseCaptureHealth({
      schema_version: "hl.capture.health.v1",
      ok: true,
      health: "green",
      ready: true,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(captureHealthWatchView(200, parsed.value)).toBeUndefined()
    expect(captureHealthIsFailClosed(200, parsed.value)).toBe(false)
    expect(healthBodyIsFailClosed(200, parsed.value)).toBe(false)
    expect(bannerFromHealthz(200, parsed.value).kind).toBe("polling")
    expect(leftoverV4LaneKind(200, parsed.value)).toBe("not_observed")
  })

  it("degrades the banner for core health HTTP 200 unready on /healthz with the same helper as chips", () => {
    const parsed = parseCoreHealth({
      schema_version: "hl.core.health.v1",
      ok: false,
      ready: false,
      reason_code: null,
      live_qualified: false,
      stage_2_qualified: false,
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(coreHealthIsFailClosed(200, parsed.value)).toBe(true)
    expect(healthBodyIsFailClosed(200, parsed.value)).toBe(true)
    const connection = bannerFromHealthz(200, parsed.value)
    expect(connection.kind).toBe("degraded")
    expect(connection.kind).not.toBe("polling")
    expect(connection.detail).not.toMatch(/capture_health\.not_ready/)
    const view = coreHealthWatchView(200, parsed.value)
    expect(view?.tone).toBe("red")
    expect(view?.reasonCode).not.toBe("capture_health.not_ready")
  })

  it("does not treat leftover hl.health.v1 GREEN HTTP 200 as banner-degraded or as a PASS", () => {
    const parsed = parseHealthAssessment({
      schema_version: "hl.health.v1",
      scope: "canonical",
      state: "HEALTH_STATE_GREEN",
      reason_code: "healthy",
      observed_at_micros: 1,
      suppresses: [],
    })
    expect(parsed.ok).toBe(true)
    if (!parsed.ok) {
      return
    }
    expect(healthBodyIsFailClosed(200, parsed.value)).toBe(false)
    expect(bannerFromHealthz(200, parsed.value).kind).toBe("polling")
  })
})
