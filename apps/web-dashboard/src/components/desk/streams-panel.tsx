import { RadioTowerIcon } from "lucide-react"

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
import type { EndpointOutcome } from "@/lib/api"
import type { ApiError } from "@/lib/contracts"
import { mapApiError } from "@/lib/fail-closed"

export function StreamsCard({
  loading,
  outcome,
}: {
  loading: boolean
  outcome: EndpointOutcome<ApiError> | undefined
}) {
  return (
    <Card size="sm">
      <CardHeader className="border-b">
        <CardTitle>Streams</CardTitle>
        <CardDescription className="font-mono">
          /v1/stream · /v1/stream/canonical-events
        </CardDescription>
      </CardHeader>
      <CardContent>
        {loading ? (
          <Skeleton className="h-24 w-full" />
        ) : (
          <StreamBody outcome={outcome} />
        )}
      </CardContent>
    </Card>
  )
}

function StreamBody({
  outcome,
}: {
  outcome: EndpointOutcome<ApiError> | undefined
}) {
  if (!outcome) {
    return null
  }
  switch (outcome.kind) {
    case "network":
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <RadioTowerIcon />
            </EmptyMedia>
            <EmptyTitle>disconnected</EmptyTitle>
            <EmptyDescription>{outcome.detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "invalid":
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <RadioTowerIcon />
            </EmptyMedia>
            <EmptyTitle>unexpected stream body</EmptyTitle>
            <EmptyDescription>
              HTTP {outcome.status} · {outcome.detail}. Live fills and charts
              are not shown.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      const unspecified = view.family === "stream_unspecified"
      return (
        <div className="flex flex-col gap-3">
          <div className="flex flex-wrap items-center gap-2">
            <ToneBadge tone={view.tone}>{view.title}</ToneBadge>
            <ToneBadge tone="neutral">{outcome.error.code}</ToneBadge>
          </div>
          {unspecified ? (
            <Empty className="border">
              <EmptyHeader>
                <EmptyMedia variant="icon">
                  <RadioTowerIcon />
                </EmptyMedia>
                <EmptyTitle>stream unspecified</EmptyTitle>
                <EmptyDescription>
                  hl.stream.v1 defines CanonicalEventBatch with next_cursor but
                  does not specify a WebSocket resume protocol. GET and
                  WebSocket upgrades fail-close with typed 501. This dashboard
                  does not invent live fills or charts.
                </EmptyDescription>
              </EmptyHeader>
            </Empty>
          ) : (
            <p className="text-xs text-muted-foreground">{view.detail}</p>
          )}
          <FieldTable
            caption="/v1/stream"
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
    case "ok":
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <RadioTowerIcon />
            </EmptyMedia>
            <EmptyTitle>unexpected 200 on /v1/stream</EmptyTitle>
            <EmptyDescription>
              The OpenAPI surface documents typed 501. This UI still refuses to
              render fills or charts.
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}
