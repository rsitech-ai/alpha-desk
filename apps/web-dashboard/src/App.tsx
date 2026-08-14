import type { ReactNode } from "react"

import { CapturePipelineCard } from "@/components/desk/capture-pipeline"
import { LiveDot, ToneBadge } from "@/components/desk/chips"
import {
  captureHealthTone,
  healthStateTone,
  readyTone,
  toneWithoutLiveOnHttpError,
} from "@/lib/tone"
import {
  ConnectionBanner,
  type ConnectionKind,
} from "@/components/desk/connection-banner"
import { DataHealthCard } from "@/components/desk/data-health"
import { EvidenceCard } from "@/components/desk/evidence-panel"
import { FailClosedCard } from "@/components/desk/fail-closed-panel"
import { StreamsCard } from "@/components/desk/streams-panel"
import { Separator } from "@/components/ui/separator"
import { useHlApi, type FeedState } from "@/hooks/use-hl-api"
import {
  HL_API_ORIGIN,
  POLL_INTERVAL_MS,
  STREAM_PATH,
  type DeskFeed,
  type EndpointOutcome,
} from "@/lib/api"
import {
  API_ERROR_SCHEMA_VERSION,
  CAPTURE_HEALTH_SCHEMA_VERSION,
  CORE_HEALTH_SCHEMA_VERSION,
  HEALTH_SCHEMA_VERSION,
  asTypedCoreFailClosedReason,
  assertNever,
  healthReasonCode,
  type ApiError,
  type CaptureHealthBody,
  type CaptureStatus,
  type CoreHealth,
  type HealthBody,
} from "@/lib/contracts"
import { mapApiError } from "@/lib/fail-closed"

export function App() {
  const state = useHlApi()
  const feed = feedOf(state)
  const connection = deriveConnection(state)

  return (
    <div className="desk-shell min-h-svh">
      <header className="border-b">
        <div className="mx-auto flex max-w-[1600px] flex-col gap-4 px-6 py-5">
          <div className="flex flex-wrap items-start justify-between gap-4">
            <div className="flex flex-col gap-1">
              <p className="desk-kicker">Hyperliquid Alpha Desk</p>
              <h1 className="font-heading text-2xl tracking-tight">
                Operator watch
              </h1>
              <p className="max-w-2xl text-sm text-muted-foreground">
                Read-only research surface for hl-api. Not a trading client, not
                Stage 6, not live-source qualification, and not a Stage PASS.
              </p>
            </div>
            <div className="flex flex-col items-end gap-2">
              <div className="flex items-center gap-2 font-mono text-xs text-muted-foreground">
                <LiveDot
                  pulse={
                    connection.kind === "polling" ||
                    connection.kind === "degraded"
                  }
                />
                <span>{connection.kind}</span>
                <span>poll {POLL_INTERVAL_MS}ms</span>
              </div>
              <p className="font-mono text-[11px] text-muted-foreground">
                proxy → {HL_API_ORIGIN}
              </p>
            </div>
          </div>
          <Separator />
          <StatusStrip feed={feed} />
        </div>
      </header>
      <main className="mx-auto flex max-w-[1600px] flex-col gap-4 px-6 py-6">
        <ConnectionBanner kind={connection.kind} detail={connection.detail} />
        <FailClosedCard loading={state.phase === "loading"} feed={feed} />
        <div className="grid items-start gap-4 xl:grid-cols-12">
          <div className="xl:col-span-4">
            <DataHealthCard
              loading={state.phase === "loading"}
              healthz={feed?.healthz}
              readyz={feed?.readyz}
              canonical={feed?.canonicalHealth}
            />
          </div>
          <div className="xl:col-span-5">
            <CapturePipelineCard
              loading={state.phase === "loading"}
              outcome={feed?.captureStatus}
            />
          </div>
          <div className="xl:col-span-3">
            <EvidenceCard
              loading={state.phase === "loading"}
              outcome={feed?.captureStatus}
            />
          </div>
        </div>
        <StreamsCard
          loading={state.phase === "loading"}
          outcome={feed?.stream}
        />
      </main>
    </div>
  )
}

function feedOf(state: FeedState): DeskFeed | undefined {
  switch (state.phase) {
    case "loading":
      return undefined
    case "ready":
      return state.feed
    case "disconnected":
      return state.feed
    default: {
      const _exhaustive: never = state
      return _exhaustive
    }
  }
}

function deriveConnection(state: FeedState): {
  kind: ConnectionKind
  detail: string
} {
  if (state.phase === "loading") {
    return { kind: "loading", detail: "awaiting first hl-api poll" }
  }
  if (state.phase === "disconnected") {
    return { kind: "disconnected", detail: state.detail }
  }
  const feed = state.feed
  const unauthorized = [
    feed.healthz,
    feed.readyz,
    feed.canonicalHealth,
    feed.captureStatus,
    feed.stream,
  ].some((outcome) => outcome.kind === "http-error" && outcome.status === 401)
  if (unauthorized) {
    return {
      kind: "unauthorized",
      detail:
        "hl-api returned 401 unauthorized. Set VITE_HL_API_BEARER when auth.mode = credential.",
    }
  }
  if (
    feed.canonicalHealth.kind === "http-error" ||
    feed.captureStatus.kind === "http-error" ||
    feed.canonicalHealth.kind === "invalid" ||
    feed.captureStatus.kind === "invalid" ||
    isTypedCoreFailClosed(feed.healthz) ||
    isTypedCoreFailClosed(feed.readyz) ||
    isTypedCoreFailClosed(feed.canonicalHealth)
  ) {
    const reason = unavailableReason(feed)
    return {
      kind: "unavailable",
      detail: `Snapshot missing, invalid, or HTTP error · ${reason}. Fail-closed; no invented fills.`,
    }
  }
  const captureDegraded =
    feed.captureStatus.kind === "ok" &&
    (feed.captureStatus.data.health !== "green" ||
      !feed.captureStatus.data.ready)
  const healthDegraded =
    feed.canonicalHealth.kind === "ok" &&
    healthBodyIsDegraded(feed.canonicalHealth.status, feed.canonicalHealth.data)
  const readyDegraded =
    feed.readyz.kind === "ok" &&
    healthBodyIsDegraded(feed.readyz.status, feed.readyz.data)
  if (captureDegraded || healthDegraded || readyDegraded) {
    return {
      kind: "degraded",
      detail: "API reachable; health, ready, or capture is not green/ready.",
    }
  }
  return {
    kind: "polling",
    detail:
      "Polls succeeding. Not live-qualified, not Stage 6, no invented fills.",
  }
}

function unavailableReason(feed: DeskFeed): string {
  if (feed.captureStatus.kind === "http-error") {
    return feed.captureStatus.error.reason_code
  }
  if (feed.canonicalHealth.kind === "http-error") {
    return feed.canonicalHealth.error.reason_code
  }
  const healthReason =
    typedCoreFailClosedReason(feed.healthz) ??
    typedCoreFailClosedReason(feed.readyz) ??
    typedCoreFailClosedReason(feed.canonicalHealth)
  if (healthReason !== undefined) {
    return healthReason
  }
  if (feed.captureStatus.kind === "invalid") {
    return feed.captureStatus.detail
  }
  if (feed.canonicalHealth.kind === "invalid") {
    return feed.canonicalHealth.detail
  }
  return "data_unavailable"
}

function isTypedCoreFailClosed(outcome: EndpointOutcome<HealthBody>): boolean {
  return typedCoreFailClosedReason(outcome) !== undefined
}

function typedCoreFailClosedReason(
  outcome: EndpointOutcome<HealthBody>
): string | undefined {
  if (outcome.kind === "http-error") {
    return asTypedCoreFailClosedReason(outcome.error.reason_code)
  }
  if (outcome.kind !== "ok") {
    return undefined
  }
  const reason = healthReasonCode(outcome.data)
  if (reason === undefined) {
    return undefined
  }
  return asTypedCoreFailClosedReason(reason)
}

function healthBodyIsDegraded(status: number, body: HealthBody): boolean {
  switch (body.schema_version) {
    case CORE_HEALTH_SCHEMA_VERSION:
      return (
        status === 503 ||
        !body.ok ||
        !body.ready ||
        body.live_qualified ||
        body.stage_2_qualified ||
        body.reason_code !== null
      )
    case CAPTURE_HEALTH_SCHEMA_VERSION:
      return (
        status === 503 ||
        !body.ok ||
        body.ready !== true ||
        body.reason_code !== undefined
      )
    case HEALTH_SCHEMA_VERSION:
      return status === 503 || body.state !== "HEALTH_STATE_GREEN"
    default:
      return assertNever(body)
  }
}

function StatusStrip({ feed }: { feed: DeskFeed | undefined }) {
  if (!feed) {
    return (
      <p className="font-mono text-xs text-muted-foreground">
        waiting for /healthz
      </p>
    )
  }
  return (
    <div className="flex flex-wrap items-center gap-2">
      <Chip label="/healthz">
        <HealthChip outcome={feed.healthz} />
      </Chip>
      <Chip label="/readyz">
        <ReadyChip outcome={feed.readyz} />
      </Chip>
      <Chip label="/v1/health">
        <HealthChip outcome={feed.canonicalHealth} />
      </Chip>
      <Chip label="/v1/capture/status">
        <CaptureChip outcome={feed.captureStatus} />
      </Chip>
      <Chip label={STREAM_PATH}>
        <ErrorChip outcome={feed.stream} />
      </Chip>
    </div>
  )
}

function Chip({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="flex items-center gap-2 rounded-lg border px-2 py-1">
      <span className="font-mono text-[11px] text-muted-foreground">
        {label}
      </span>
      {children}
    </div>
  )
}

function HealthChip({ outcome }: { outcome: EndpointOutcome<HealthBody> }) {
  switch (outcome.kind) {
    case "network":
      return <ToneBadge tone="red">disconnected</ToneBadge>
    case "invalid":
      return <ToneBadge tone="red">invalid</ToneBadge>
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      return (
        <ToneBadge tone={view.tone}>
          {view.family === "core_deadletter" ||
          view.family === "ledger_unsupported_event" ||
          view.family === "capture_health_not_ready"
            ? view.title
            : `${outcome.status} ${outcome.error.reason_code}`}
        </ToneBadge>
      )
    }
    case "ok":
      switch (outcome.data.schema_version) {
        case CORE_HEALTH_SCHEMA_VERSION:
          return (
            <CoreHealthChip status={outcome.status} health={outcome.data} />
          )
        case CAPTURE_HEALTH_SCHEMA_VERSION:
          return (
            <CaptureHealthzChip status={outcome.status} health={outcome.data} />
          )
        case HEALTH_SCHEMA_VERSION: {
          const typed = asTypedCoreFailClosedReason(outcome.data.reason_code)
          if (typed !== undefined) {
            const view = mapApiError(outcome.status, {
              schema_version: API_ERROR_SCHEMA_VERSION,
              code: "data_unavailable",
              reason_code: outcome.data.reason_code,
            })
            return <ToneBadge tone={view.tone}>{view.title}</ToneBadge>
          }
          return (
            <ToneBadge
              tone={toneWithoutLiveOnHttpError(
                outcome.status,
                healthStateTone(outcome.data.state)
              )}
            >
              {outcome.status === 503
                ? `503 ${outcome.data.reason_code}`
                : outcome.data.scope}
            </ToneBadge>
          )
        }
        default: {
          const _exhaustive: never = outcome.data
          return _exhaustive
        }
      }
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function ReadyChip({ outcome }: { outcome: EndpointOutcome<HealthBody> }) {
  switch (outcome.kind) {
    case "network":
      return <ToneBadge tone="red">disconnected</ToneBadge>
    case "invalid":
      return <ToneBadge tone="red">invalid</ToneBadge>
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      return (
        <ToneBadge tone={view.tone}>
          {view.family === "core_deadletter" ||
          view.family === "ledger_unsupported_event" ||
          view.family === "capture_health_not_ready"
            ? view.title
            : `${outcome.status} ${outcome.error.reason_code}`}
        </ToneBadge>
      )
    }
    case "ok": {
      switch (outcome.data.schema_version) {
        case CORE_HEALTH_SCHEMA_VERSION: {
          const unready =
            outcome.status === 503 ||
            !outcome.data.ok ||
            !outcome.data.ready ||
            outcome.data.live_qualified ||
            outcome.data.stage_2_qualified ||
            outcome.data.reason_code !== null
          return (
            <ToneBadge tone={unready ? "red" : "yellow"}>
              {unready ? "unready" : "not live-qualified"}
            </ToneBadge>
          )
        }
        case CAPTURE_HEALTH_SCHEMA_VERSION: {
          const unready =
            outcome.status === 503 ||
            !outcome.data.ok ||
            outcome.data.ready !== true ||
            outcome.data.reason_code !== undefined
          return (
            <ToneBadge tone={unready ? "red" : "yellow"}>
              {unready ? "unready" : "not live-qualified"}
            </ToneBadge>
          )
        }
        case HEALTH_SCHEMA_VERSION: {
          const unready =
            outcome.status === 503 ||
            outcome.data.state !== "HEALTH_STATE_GREEN"
          return (
            <ToneBadge tone={unready ? "red" : "green"}>
              {unready ? "unready" : "ready"}
            </ToneBadge>
          )
        }
        default: {
          const _exhaustive: never = outcome.data
          return _exhaustive
        }
      }
    }
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function CoreHealthChip({
  status,
  health,
}: {
  status: number
  health: CoreHealth
}) {
  const failClosed =
    status === 503 ||
    !health.ok ||
    !health.ready ||
    health.live_qualified ||
    health.stage_2_qualified ||
    health.reason_code !== null
  if (!failClosed) {
    return <ToneBadge tone="yellow">not live-qualified</ToneBadge>
  }
  const view = mapApiError(status === 200 ? 503 : status, {
    schema_version: API_ERROR_SCHEMA_VERSION,
    code: "data_unavailable",
    reason_code:
      health.reason_code ??
      (health.live_qualified || health.stage_2_qualified
        ? "core_status.qualification_claim"
        : "core_status.not_ready"),
  })
  return <ToneBadge tone={view.tone}>{view.title}</ToneBadge>
}

function CaptureHealthzChip({
  status,
  health,
}: {
  status: number
  health: CaptureHealthBody
}) {
  const failClosed =
    status === 503 ||
    !health.ok ||
    health.ready !== true ||
    health.reason_code !== undefined
  if (!failClosed) {
    return <ToneBadge tone="yellow">not live-qualified</ToneBadge>
  }
  const view = mapApiError(status === 200 ? 503 : status, {
    schema_version: API_ERROR_SCHEMA_VERSION,
    code: "data_unavailable",
    reason_code: health.reason_code ?? "capture_health.not_ready",
  })
  return <ToneBadge tone={view.tone}>{view.title}</ToneBadge>
}

function CaptureChip({ outcome }: { outcome: EndpointOutcome<CaptureStatus> }) {
  switch (outcome.kind) {
    case "network":
      return <ToneBadge tone="red">disconnected</ToneBadge>
    case "invalid":
      return <ToneBadge tone="red">invalid</ToneBadge>
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      return (
        <ToneBadge tone={view.tone}>
          {view.family === "core_deadletter" ||
          view.family === "ledger_unsupported_event" ||
          view.family === "capture_health_not_ready"
            ? view.title
            : `${outcome.status} ${outcome.error.reason_code}`}
        </ToneBadge>
      )
    }
    case "ok":
      return (
        <span className="flex items-center gap-1">
          <ToneBadge
            tone={toneWithoutLiveOnHttpError(
              outcome.status,
              captureHealthTone(outcome.data.health)
            )}
          >
            {outcome.status === 503
              ? `503 ${outcome.data.health}`
              : outcome.data.health}
          </ToneBadge>
          <ToneBadge
            tone={toneWithoutLiveOnHttpError(
              outcome.status,
              readyTone(outcome.data.ready)
            )}
          >
            ready={String(outcome.data.ready)}
          </ToneBadge>
        </span>
      )
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function ErrorChip({ outcome }: { outcome: EndpointOutcome<ApiError> }) {
  switch (outcome.kind) {
    case "network":
      return <ToneBadge tone="red">disconnected</ToneBadge>
    case "invalid":
      return <ToneBadge tone="red">invalid</ToneBadge>
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      return (
        <ToneBadge tone={view.tone}>
          {outcome.status} {outcome.error.reason_code}
        </ToneBadge>
      )
    }
    case "ok":
      return <ToneBadge tone="red">unexpected 200</ToneBadge>
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

export default App
