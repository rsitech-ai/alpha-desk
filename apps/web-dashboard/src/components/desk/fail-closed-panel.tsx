import { ShieldAlertIcon } from "lucide-react"

import { ToneBadge } from "@/components/desk/chips"
import { FieldTable } from "@/components/desk/field-table"
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"
import {
  INVALID_QUERY_PROBE_PATH,
  QUERY_BUDGET_PROBE_PATH,
  STREAM_PATH,
  type DeskFeed,
  type EndpointOutcome,
} from "@/lib/api"
import {
  API_ERROR_SCHEMA_VERSION,
  healthReasonCode,
  type ApiError,
  type CaptureStatus,
  type HealthBody,
} from "@/lib/contracts"
import {
  mapApiError,
  type FailClosedFamily,
  type FailClosedView,
} from "@/lib/fail-closed"

export function FailClosedCard({
  loading,
  feed,
}: {
  loading: boolean
  feed: DeskFeed | undefined
}) {
  return (
    <Card size="sm">
      <CardHeader className="border-b">
        <CardTitle>Fail-closed API states</CardTitle>
        <CardDescription>
          Typed 503 / 429 / 400 / 501 from hl-api, including hl-core
          dead-letter, ledger.unsupported_event consume-poison, and
          capture_health.not_ready leftover v4 / not live-ready capture healthz.
          Empty is not green. This is not Stage 6 and not live-qualified.
        </CardDescription>
      </CardHeader>
      <CardContent>
        {loading || !feed ? (
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-7">
            <Skeleton className="h-36 w-full" />
            <Skeleton className="h-36 w-full" />
            <Skeleton className="h-36 w-full" />
            <Skeleton className="h-36 w-full" />
            <Skeleton className="h-36 w-full" />
            <Skeleton className="h-36 w-full" />
            <Skeleton className="h-36 w-full" />
          </div>
        ) : (
          <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-7">
            <Lane
              label="503 capture status"
              path="/v1/capture/status"
              expected="snapshot_missing"
              body={captureLane(feed.captureStatus)}
            />
            <Lane
              label="503 capture health not ready"
              path="/healthz"
              expected="capture_health_not_ready"
              body={captureHealthNotReadyLane(feed)}
            />
            <Lane
              label="503 core dead-letter"
              path="/healthz · /status"
              expected="core_deadletter"
              body={deadletterLane(feed)}
            />
            <Lane
              label="503 ledger unsupported event"
              path="/healthz · /status"
              expected="ledger_unsupported_event"
              body={ledgerUnsupportedLane(feed)}
            />
            <Lane
              label="400 invalid query"
              path={INVALID_QUERY_PROBE_PATH}
              expected="invalid_query"
              body={feed.invalidQuery}
            />
            <Lane
              label="429 query budget"
              path={QUERY_BUDGET_PROBE_PATH}
              expected="query_budget"
              body={budgetLane(feed)}
            />
            <Lane
              label="501 streams"
              path={STREAM_PATH}
              expected="stream_unspecified"
              body={streamLane(feed.stream)}
            />
          </div>
        )}
      </CardContent>
    </Card>
  )
}

function Lane({
  label,
  path,
  expected,
  body,
}: {
  label: string
  path: string
  expected: FailClosedFamily
  body: LaneBody
}) {
  return (
    <div className="flex flex-col gap-3 rounded-lg border p-3">
      <div className="flex flex-col gap-1">
        <p className="font-mono text-[11px] tracking-wide text-muted-foreground uppercase">
          {label}
        </p>
        <p className="font-mono text-[11px] text-muted-foreground">{path}</p>
      </div>
      <LaneContent expected={expected} body={body} />
    </div>
  )
}

type LaneBody =
  | { kind: "observed"; view: FailClosedView }
  | { kind: "not_observed"; status?: number; detail: string }
  | { kind: "invalid"; status: number; detail: string }
  | { kind: "network"; detail: string }

function LaneContent({
  expected,
  body,
}: {
  expected: FailClosedFamily
  body: LaneBody
}) {
  switch (body.kind) {
    case "network":
      return (
        <Empty className="py-2">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ShieldAlertIcon />
            </EmptyMedia>
            <EmptyTitle>disconnected</EmptyTitle>
            <EmptyDescription>{body.detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "invalid":
      return (
        <Empty className="py-2">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <ShieldAlertIcon />
            </EmptyMedia>
            <EmptyTitle>untyped HTTP {body.status}</EmptyTitle>
            <EmptyDescription>{body.detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "not_observed":
      return (
        <div className="flex flex-col gap-2">
          <ToneBadge tone="neutral">not observed</ToneBadge>
          <p className="text-xs text-muted-foreground">
            {body.status === undefined
              ? body.detail
              : `HTTP ${body.status} · ${body.detail}`}{" "}
            Expected family {expected}. Not a PASS.
          </p>
        </div>
      )
    case "observed": {
      const matched = body.view.family === expected
      return (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center gap-2">
            <ToneBadge tone={body.view.tone}>{body.view.title}</ToneBadge>
            {matched ? null : (
              <ToneBadge tone="neutral">{body.view.family}</ToneBadge>
            )}
          </div>
          <p className="text-xs text-muted-foreground">{body.view.detail}</p>
          <FieldTable
            caption={body.view.title}
            rows={[
              {
                field: "http_status",
                value: body.view.httpStatus,
                omitted: false,
              },
              { field: "code", value: body.view.code, omitted: false },
              {
                field: "reason_code",
                value: body.view.reasonCode,
                omitted: false,
              },
            ]}
          />
        </div>
      )
    }
    default: {
      const _exhaustive: never = body
      return _exhaustive
    }
  }
}

function captureLane(outcome: EndpointOutcome<CaptureStatus>): LaneBody {
  switch (outcome.kind) {
    case "network":
      return outcome
    case "invalid":
      return {
        kind: "invalid",
        status: outcome.status,
        detail: outcome.detail,
      }
    case "http-error":
      return {
        kind: "observed",
        view: mapApiError(outcome.status, outcome.error),
      }
    case "ok":
      return {
        kind: "not_observed",
        status: outcome.status,
        detail:
          "snapshot present this poll. Missing-status 503 was not returned.",
      }
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function budgetLane(feed: DeskFeed): LaneBody {
  const live = liveBudgetView(feed)
  if (live) {
    return { kind: "observed", view: live }
  }
  return feed.queryBudget
}

function deadletterLane(feed: DeskFeed): LaneBody {
  const live = liveTypedCoreView(feed, "core_deadletter")
  if (live) {
    return { kind: "observed", view: live }
  }
  return {
    kind: "not_observed",
    detail:
      "hl-core dead-letter fail-closed was not returned this poll. Not a PASS.",
  }
}

function ledgerUnsupportedLane(feed: DeskFeed): LaneBody {
  const live = liveTypedCoreView(feed, "ledger_unsupported_event")
  if (live) {
    return { kind: "observed", view: live }
  }
  return {
    kind: "not_observed",
    detail:
      "hl-core ledger.unsupported_event consume-poison was not returned this poll. Not a PASS.",
  }
}

function captureHealthNotReadyLane(feed: DeskFeed): LaneBody {
  const live = liveTypedCoreView(feed, "capture_health_not_ready")
  if (live) {
    return { kind: "observed", view: live }
  }
  return {
    kind: "not_observed",
    detail:
      "capture /healthz leftover v4 / not live-ready was not returned this poll. Not a PASS.",
  }
}

function liveTypedCoreView(
  feed: DeskFeed,
  family: FailClosedFamily
): FailClosedView | undefined {
  const healthOutcomes: EndpointOutcome<HealthBody>[] = [
    feed.healthz,
    feed.readyz,
    feed.canonicalHealth,
  ]
  for (const outcome of healthOutcomes) {
    const view = typedViewFromHealth(outcome, family)
    if (view) {
      return view
    }
  }
  if (feed.captureStatus.kind === "http-error") {
    const view = mapApiError(
      feed.captureStatus.status,
      feed.captureStatus.error
    )
    if (view.family === family) {
      return view
    }
  }
  return undefined
}

function typedViewFromHealth(
  outcome: EndpointOutcome<HealthBody>,
  family: FailClosedFamily
): FailClosedView | undefined {
  if (outcome.kind === "http-error") {
    const view = mapApiError(outcome.status, outcome.error)
    return view.family === family ? view : undefined
  }
  if (outcome.kind !== "ok") {
    return undefined
  }
  const reason_code = healthReasonCode(outcome.data)
  if (reason_code === undefined) {
    return undefined
  }
  const view = mapApiError(outcome.status, {
    schema_version: API_ERROR_SCHEMA_VERSION,
    code: "data_unavailable",
    reason_code,
  })
  return view.family === family ? view : undefined
}

function liveBudgetView(feed: DeskFeed): FailClosedView | undefined {
  const outcomes = [
    feed.healthz,
    feed.readyz,
    feed.canonicalHealth,
    feed.captureStatus,
    feed.stream,
  ]
  for (const outcome of outcomes) {
    if (outcome.kind === "http-error" && outcome.status === 429) {
      return mapApiError(outcome.status, outcome.error)
    }
  }
  return undefined
}

function streamLane(outcome: EndpointOutcome<ApiError>): LaneBody {
  switch (outcome.kind) {
    case "network":
      return outcome
    case "invalid":
      return {
        kind: "invalid",
        status: outcome.status,
        detail: outcome.detail,
      }
    case "http-error":
      return {
        kind: "observed",
        view: mapApiError(outcome.status, outcome.error),
      }
    case "ok":
      return {
        kind: "not_observed",
        status: outcome.status,
        detail:
          "unexpected 200 on /v1/stream. Fills and charts are still not shown.",
      }
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}
