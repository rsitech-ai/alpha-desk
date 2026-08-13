import type { ReactNode } from "react"

import { CapturePipelineCard } from "@/components/desk/capture-pipeline"
import { LiveDot, ToneBadge } from "@/components/desk/chips"
import { captureHealthTone, healthStateTone, readyTone } from "@/lib/tone"
import {
  ConnectionBanner,
  type ConnectionKind,
} from "@/components/desk/connection-banner"
import { DataHealthCard } from "@/components/desk/data-health"
import { EvidenceCard } from "@/components/desk/evidence-panel"
import { StreamsCard } from "@/components/desk/streams-panel"
import { Separator } from "@/components/ui/separator"
import { useHlApi, type FeedState } from "@/hooks/use-hl-api"
import {
  HL_API_ORIGIN,
  POLL_INTERVAL_MS,
  type DeskFeed,
  type EndpointOutcome,
} from "@/lib/api"
import type { CaptureStatus, HealthAssessment } from "@/lib/contracts"

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
                Stage 6, not live-source qualification.
              </p>
            </div>
            <div className="flex flex-col items-end gap-2">
              <div className="flex items-center gap-2 font-mono text-xs text-muted-foreground">
                <LiveDot
                  live={
                    connection.kind === "live" || connection.kind === "degraded"
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
    feed.captureStatus.kind === "invalid"
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
    feed.canonicalHealth.data.state !== "HEALTH_STATE_GREEN"
  const readyDegraded =
    feed.readyz.kind === "ok" &&
    (feed.readyz.status === 503 ||
      feed.readyz.data.state !== "HEALTH_STATE_GREEN")
  if (captureDegraded || healthDegraded || readyDegraded) {
    return {
      kind: "degraded",
      detail: "API reachable; health, ready, or capture is not green/ready.",
    }
  }
  return { kind: "live", detail: "polls succeeding" }
}

function unavailableReason(feed: DeskFeed): string {
  if (feed.captureStatus.kind === "http-error") {
    return feed.captureStatus.error.reason_code
  }
  if (feed.canonicalHealth.kind === "http-error") {
    return feed.canonicalHealth.error.reason_code
  }
  if (feed.captureStatus.kind === "invalid") {
    return feed.captureStatus.detail
  }
  if (feed.canonicalHealth.kind === "invalid") {
    return feed.canonicalHealth.detail
  }
  return "data_unavailable"
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

function HealthChip({
  outcome,
}: {
  outcome: EndpointOutcome<HealthAssessment>
}) {
  switch (outcome.kind) {
    case "network":
      return <ToneBadge tone="red">disconnected</ToneBadge>
    case "invalid":
      return <ToneBadge tone="red">invalid</ToneBadge>
    case "http-error":
      return (
        <ToneBadge tone="red">
          {outcome.status} {outcome.error.reason_code}
        </ToneBadge>
      )
    case "ok":
      return (
        <ToneBadge tone={healthStateTone(outcome.data.state)}>
          {outcome.data.scope}
        </ToneBadge>
      )
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function ReadyChip({
  outcome,
}: {
  outcome: EndpointOutcome<HealthAssessment>
}) {
  switch (outcome.kind) {
    case "network":
      return <ToneBadge tone="red">disconnected</ToneBadge>
    case "invalid":
      return <ToneBadge tone="red">invalid</ToneBadge>
    case "http-error":
      return (
        <ToneBadge tone="red">
          {outcome.status} {outcome.error.reason_code}
        </ToneBadge>
      )
    case "ok": {
      const unready =
        outcome.status === 503 || outcome.data.state !== "HEALTH_STATE_GREEN"
      return (
        <ToneBadge tone={unready ? "red" : "green"}>
          {unready ? "unready" : "ready"}
        </ToneBadge>
      )
    }
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function CaptureChip({ outcome }: { outcome: EndpointOutcome<CaptureStatus> }) {
  switch (outcome.kind) {
    case "network":
      return <ToneBadge tone="red">disconnected</ToneBadge>
    case "invalid":
      return <ToneBadge tone="red">invalid</ToneBadge>
    case "http-error":
      return (
        <ToneBadge tone="red">
          {outcome.status} {outcome.error.reason_code}
        </ToneBadge>
      )
    case "ok":
      return (
        <span className="flex items-center gap-1">
          <ToneBadge tone={captureHealthTone(outcome.data.health)}>
            {outcome.data.health}
          </ToneBadge>
          <ToneBadge tone={readyTone(outcome.data.ready)}>
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

export default App
