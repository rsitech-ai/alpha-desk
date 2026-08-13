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
  HEALTH_FIELD_ORDER,
  HEALTH_SCHEMA_VERSION,
  assertNever,
  type HealthAssessment,
} from "@/lib/contracts"
import { mapApiError } from "@/lib/fail-closed"
import { formatUnixMicros } from "@/lib/format"

export function DataHealthCard({
  loading,
  canonical,
  healthz,
  readyz,
}: {
  loading: boolean
  canonical: EndpointOutcome<HealthAssessment> | undefined
  healthz: EndpointOutcome<HealthAssessment> | undefined
  readyz: EndpointOutcome<HealthAssessment> | undefined
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
  outcome: EndpointOutcome<HealthAssessment> | undefined
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
      const assessment = outcome.data
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
      return (
        <div className="flex flex-col gap-2">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <p className="font-mono text-xs text-muted-foreground">{title}</p>
            <div className="flex flex-wrap items-center gap-2">
              <ToneBadge
                tone={toneWithoutLiveOnHttpError(
                  outcome.status,
                  healthStateTone(assessment.state)
                )}
              >
                {assessment.state}
              </ToneBadge>
              {outcome.status === 503 ? (
                <ToneBadge tone="red">HTTP 503</ToneBadge>
              ) : null}
            </div>
          </div>
          <FieldTable caption={title} rows={rows} />
        </div>
      )
    }
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}
