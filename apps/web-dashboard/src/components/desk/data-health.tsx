import { HeartPulseIcon } from "lucide-react"

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
import { FieldTable } from "@/components/desk/field-table"
import { ToneBadge } from "@/components/desk/chips"
import { healthStateTone, toneWithoutLiveOnHttpError } from "@/lib/tone"
import type { EndpointOutcome } from "@/lib/api"
import {
  API_ERROR_SCHEMA_VERSION,
  CAPTURE_HEALTH_FIELD_ORDER,
  CAPTURE_HEALTH_SCHEMA_VERSION,
  CORE_HEALTH_FIELD_ORDER,
  CORE_HEALTH_SCHEMA_VERSION,
  HEALTH_FIELD_ORDER,
  HEALTH_SCHEMA_VERSION,
  asTypedCoreFailClosedReason,
  assertNever,
  type CaptureHealthBody,
  type CoreHealth,
  type HealthBody,
} from "@/lib/contracts"
import {
  captureHealthWatchView,
  coreHealthWatchView,
  mapApiError,
} from "@/lib/fail-closed"
import { formatUnixMicros } from "@/lib/format"

export function DataHealthCard({
  loading,
  canonical,
  healthz,
  readyz,
}: {
  loading: boolean
  canonical: EndpointOutcome<HealthBody> | undefined
  healthz: EndpointOutcome<HealthBody> | undefined
  readyz: EndpointOutcome<HealthBody> | undefined
}) {
  return (
    <Card size="sm" className="h-full">
      <CardHeader className="border-b">
        <CardTitle>Data health</CardTitle>
        <CardDescription className="font-mono">
          {HEALTH_SCHEMA_VERSION} · /v1/health · /healthz · /readyz
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {loading ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-8 w-40" />
            <Skeleton className="h-24 w-full" />
          </div>
        ) : (
          <>
            <HealthBlock title="/healthz api:process" outcome={healthz} />
            <HealthBlock title="/readyz health:aggregate" outcome={readyz} />
            <HealthBlock title="/v1/health canonical" outcome={canonical} />
          </>
        )}
      </CardContent>
    </Card>
  )
}

function HealthBlock({
  title,
  outcome,
}: {
  title: string
  outcome: EndpointOutcome<HealthBody> | undefined
}) {
  if (!outcome) {
    return null
  }
  switch (outcome.kind) {
    case "network":
      return (
        <Empty className="border py-4">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HeartPulseIcon />
            </EmptyMedia>
            <EmptyTitle>{title}</EmptyTitle>
            <EmptyDescription>disconnected · {outcome.detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "invalid":
      return (
        <Empty className="border py-4">
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <HeartPulseIcon />
            </EmptyMedia>
            <EmptyTitle>{title}</EmptyTitle>
            <EmptyDescription>
              HTTP {outcome.status} · snapshot rejected · {outcome.detail}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      return (
        <div className="flex flex-col gap-2 rounded-lg border p-3">
          <div className="flex items-center justify-between gap-2">
            <p className="font-mono text-xs text-muted-foreground">{title}</p>
            <ToneBadge tone={view.tone}>{view.title}</ToneBadge>
          </div>
          <FieldTable
            caption={title}
            rows={[
              {
                field: "schema_version",
                value: outcome.error.schema_version,
                omitted: false,
              },
              { field: "code", value: outcome.error.code, omitted: false },
              {
                field: "reason_code",
                value: outcome.error.reason_code,
                omitted: false,
              },
            ]}
          />
        </div>
      )
    }
    case "ok": {
      switch (outcome.data.schema_version) {
        case CORE_HEALTH_SCHEMA_VERSION:
          return (
            <CoreHealthBlock
              title={title}
              status={outcome.status}
              health={outcome.data}
            />
          )
        case CAPTURE_HEALTH_SCHEMA_VERSION:
          return (
            <CaptureHealthBlock
              title={title}
              status={outcome.status}
              health={outcome.data}
            />
          )
        case HEALTH_SCHEMA_VERSION: {
          const assessment = outcome.data
          const typedReason = asTypedCoreFailClosedReason(
            assessment.reason_code
          )
          const rows = HEALTH_FIELD_ORDER.map((field) => {
            switch (field) {
              case "schema_version":
                return {
                  field,
                  value: assessment.schema_version,
                  omitted: false,
                }
              case "scope":
                return { field, value: assessment.scope, omitted: false }
              case "state":
                return { field, value: assessment.state, omitted: false }
              case "reason_code":
                return { field, value: assessment.reason_code, omitted: false }
              case "observed_at_micros":
                return {
                  field,
                  value: `${assessment.observed_at_micros} · ${formatUnixMicros(assessment.observed_at_micros)}`,
                  omitted: false,
                }
              case "suppresses":
                return { field, value: assessment.suppresses, omitted: false }
              default:
                return assertNever(field)
            }
          })
          const typed =
            typedReason !== undefined
              ? mapApiError(outcome.status, {
                  schema_version: API_ERROR_SCHEMA_VERSION,
                  code: "data_unavailable",
                  reason_code: assessment.reason_code,
                })
              : undefined
          return (
            <div className="flex flex-col gap-2">
              <div className="flex flex-wrap items-center justify-between gap-2">
                <p className="font-mono text-xs text-muted-foreground">
                  {title}
                </p>
                <div className="flex flex-wrap items-center gap-2">
                  <ToneBadge
                    tone={
                      typed?.tone ??
                      toneWithoutLiveOnHttpError(
                        outcome.status,
                        healthStateTone(assessment.state)
                      )
                    }
                  >
                    {typed?.title ?? assessment.state}
                  </ToneBadge>
                  {outcome.status === 503 && typed === undefined ? (
                    <ToneBadge tone="red">HTTP 503</ToneBadge>
                  ) : null}
                </div>
              </div>
              <FieldTable caption={title} rows={rows} />
            </div>
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

function CoreHealthBlock({
  title,
  status,
  health,
}: {
  title: string
  status: number
  health: CoreHealth
}) {
  const view = coreHealthWatchView(status, health)
  const rows = CORE_HEALTH_FIELD_ORDER.map((field) => {
    switch (field) {
      case "schema_version":
        return { field, value: health.schema_version, omitted: false }
      case "ok":
        return { field, value: health.ok, omitted: false }
      case "ready":
        return { field, value: health.ready, omitted: false }
      case "reason_code":
        return {
          field,
          value: health.reason_code,
          omitted: health.reason_code === null,
        }
      case "live_qualified":
        return { field, value: health.live_qualified, omitted: false }
      case "stage_2_qualified":
        return { field, value: health.stage_2_qualified, omitted: false }
      default:
        return assertNever(field)
    }
  })
  return (
    <div className="flex flex-col gap-2 rounded-lg border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="font-mono text-xs text-muted-foreground">{title}</p>
        <ToneBadge tone={view?.tone ?? "yellow"}>
          {view?.title ?? "not live-qualified"}
        </ToneBadge>
      </div>
      {view ? (
        <p className="text-xs text-muted-foreground">{view.detail}</p>
      ) : null}
      <FieldTable caption={title} rows={rows} />
    </div>
  )
}

function CaptureHealthBlock({
  title,
  status,
  health,
}: {
  title: string
  status: number
  health: CaptureHealthBody
}) {
  const view = captureHealthWatchView(status, health)
  const rows = CAPTURE_HEALTH_FIELD_ORDER.map((field) => {
    switch (field) {
      case "schema_version":
        return { field, value: health.schema_version, omitted: false }
      case "ok":
        return { field, value: health.ok, omitted: false }
      case "health":
        return {
          field,
          value: health.health,
          omitted: health.health === undefined,
        }
      case "ready":
        return {
          field,
          value: health.ready,
          omitted: health.ready === undefined,
        }
      case "reason_code":
        return {
          field,
          value: health.reason_code,
          omitted: health.reason_code === undefined,
        }
      default:
        return assertNever(field)
    }
  })
  return (
    <div className="flex flex-col gap-2 rounded-lg border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="font-mono text-xs text-muted-foreground">{title}</p>
        <ToneBadge tone={view?.tone ?? "yellow"}>
          {view?.title ?? "not live-qualified"}
        </ToneBadge>
      </div>
      {view ? (
        <p className="text-xs text-muted-foreground">{view.detail}</p>
      ) : null}
      <FieldTable caption={title} rows={rows} />
    </div>
  )
}
