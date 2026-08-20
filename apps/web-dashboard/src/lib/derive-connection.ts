import type { ConnectionKind } from "@/components/desk/connection-banner"
import type { FeedState } from "@/hooks/use-hl-api"
import type { DeskFeed, EndpointOutcome } from "@/lib/api"
import {
  asTypedCoreFailClosedReason,
  healthReasonCode,
  type HealthBody,
} from "@/lib/contracts"
import { healthBodyIsFailClosed } from "@/lib/fail-closed"

export function deriveConnection(state: FeedState): {
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
  const healthDegraded = [feed.healthz, feed.readyz, feed.canonicalHealth].some(
    (outcome) =>
      outcome.kind === "ok" &&
      healthBodyIsFailClosed(outcome.status, outcome.data)
  )
  if (captureDegraded || healthDegraded) {
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
