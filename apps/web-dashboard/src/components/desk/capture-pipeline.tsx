import { DatabaseIcon } from "lucide-react"

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
import {
  Progress,
  ProgressLabel,
  ProgressValue,
} from "@/components/ui/progress"
import { Separator } from "@/components/ui/separator"
import { Skeleton } from "@/components/ui/skeleton"
import { ToneBadge } from "@/components/desk/chips"
import {
  captureHealthTone,
  readyTone,
  sourceHealthTone,
  toneWithoutLiveOnHttpError,
} from "@/lib/tone"
import type { EndpointOutcome } from "@/lib/api"
import {
  CAPTURE_STATUS_SCHEMA_VERSION,
  type CaptureStatus,
} from "@/lib/contracts"
import { mapApiError } from "@/lib/fail-closed"
import {
  formatDiskFree,
  formatOmitted,
  formatUnixMicros,
} from "@/lib/format"

export function CapturePipelineCard({
  loading,
  outcome,
}: {
  loading: boolean
  outcome: EndpointOutcome<CaptureStatus> | undefined
}) {
  return (
    <Card size="sm" className="h-full">
      <CardHeader className="border-b">
        <CardTitle>Capture pipeline</CardTitle>
        <CardDescription className="font-mono">
          {CAPTURE_STATUS_SCHEMA_VERSION} · /v1/capture/status
        </CardDescription>
      </CardHeader>
      <CardContent>
        {loading ? (
          <div className="flex flex-col gap-3">
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-16 w-full" />
            <Skeleton className="h-16 w-full" />
          </div>
        ) : (
          <CaptureBody outcome={outcome} />
        )}
      </CardContent>
    </Card>
  )
}

function CaptureBody({
  outcome,
}: {
  outcome: EndpointOutcome<CaptureStatus> | undefined
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
              <DatabaseIcon />
            </EmptyMedia>
            <EmptyTitle>/v1/capture/status</EmptyTitle>
            <EmptyDescription>disconnected · {outcome.detail}</EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "invalid":
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <DatabaseIcon />
            </EmptyMedia>
            <EmptyTitle>snapshot rejected</EmptyTitle>
            <EmptyDescription>
              HTTP {outcome.status} · {outcome.detail}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <DatabaseIcon />
            </EmptyMedia>
            <EmptyTitle>{view.title}</EmptyTitle>
            <EmptyDescription>
              {view.detail} {outcome.error.schema_version} ·{" "}
              {outcome.error.code} · {outcome.error.reason_code}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    }
    case "ok":
      return <Pipeline status={outcome.data} httpStatus={outcome.status} />
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function Pipeline({
  status,
  httpStatus,
}: {
  status: CaptureStatus
  httpStatus: number
}) {
  const disk = status.disk_free_basis_points
  const diskTone = toneWithoutLiveOnHttpError(
    httpStatus,
    disk === undefined
      ? "neutral"
      : disk < 1000
        ? "red"
        : disk < 2000
          ? "yellow"
          : "green"
  )

  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-center gap-2">
        <ToneBadge
          tone={toneWithoutLiveOnHttpError(
            httpStatus,
            captureHealthTone(status.health)
          )}
        >
          health={status.health}
        </ToneBadge>
        <ToneBadge
          tone={toneWithoutLiveOnHttpError(httpStatus, readyTone(status.ready))}
        >
          ready={String(status.ready)}
        </ToneBadge>
        <ToneBadge
          tone={toneWithoutLiveOnHttpError(
            httpStatus,
            sourceHealthTone(status.primary_source_health)
          )}
        >
          primary_source_health={status.primary_source_health}
        </ToneBadge>
        {httpStatus === 503 ? <ToneBadge tone="red">HTTP 503</ToneBadge> : null}
      </div>

      <div className="grid gap-3 sm:grid-cols-2 xl:grid-cols-4">
        <Metric
          field="durable_height"
          value={
            status.durable_height === undefined
              ? formatOmitted("durable_height")
              : String(status.durable_height)
          }
          omitted={status.durable_height === undefined}
        />
        <Metric field="pending_blocks" value={String(status.pending_blocks)} />
        <Metric
          field="capture_backlog_records"
          value={String(status.capture_backlog_records)}
        />
        <Metric
          field="oldest_pending_capture_height"
          value={
            status.oldest_pending_capture_height === undefined
              ? formatOmitted("oldest_pending_capture_height")
              : String(status.oldest_pending_capture_height)
          }
          omitted={status.oldest_pending_capture_height === undefined}
        />
      </div>

      <div className="flex flex-col gap-3 md:flex-row">
        <Stage
          field="active_committed_source"
          value={status.active_committed_source}
        />
        <Separator orientation="vertical" className="hidden md:block" />
        <Stage
          field="independent_source_health"
          value={
            status.independent_source_health ??
            formatOmitted("independent_source_health")
          }
          muted={status.independent_source_health === undefined}
        />
        <Separator orientation="vertical" className="hidden md:block" />
        <Stage
          field="failover_height"
          value={
            status.failover_height === undefined
              ? formatOmitted("failover_height")
              : String(status.failover_height)
          }
          muted={status.failover_height === undefined}
        />
        <Separator orientation="vertical" className="hidden md:block" />
        <Stage
          field="failover_reason"
          value={status.failover_reason ?? formatOmitted("failover_reason")}
          muted={status.failover_reason === undefined}
        />
      </div>

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-2">
          <p className="font-mono text-[11px] tracking-wide text-muted-foreground uppercase">
            disk_free_basis_points
          </p>
          <ToneBadge tone={diskTone}>
            {disk === undefined ? "not yet measured" : formatDiskFree(disk)}
          </ToneBadge>
        </div>
        {disk === undefined ? (
          <p className="text-xs text-muted-foreground">
            A missing disk value must not be interpreted as healthy.
          </p>
        ) : (
          <Progress value={Math.min(100, disk / 100)}>
            <ProgressLabel>free</ProgressLabel>
            <ProgressValue />
          </Progress>
        )}
      </div>

      <dl className="grid gap-2 text-xs sm:grid-cols-2">
        <Pair
          field="snapshot_at_micros"
          value={formatUnixMicros(status.snapshot_at_micros)}
        />
        <Pair field="build_id" value={status.build_id} />
        <Pair field="chain_id" value={status.chain_id} />
        <Pair
          field="archive_manifest_id"
          value={
            status.archive_manifest_id ?? formatOmitted("archive_manifest_id")
          }
        />
        <Pair
          field="last_error_reason"
          value={status.last_error_reason ?? formatOmitted("last_error_reason")}
        />
      </dl>
      <ExtraStatusFields
        throughputRecordsPerSec={status.throughput_records_per_sec}
        throughputBlocksPerSec={status.throughput_blocks_per_sec}
      />
    </div>
  )
}

function ExtraStatusFields({
  throughputRecordsPerSec,
  throughputBlocksPerSec,
}: {
  throughputRecordsPerSec: number | undefined
  throughputBlocksPerSec: number | undefined
}) {
  const hasThroughput =
    throughputRecordsPerSec !== undefined ||
    throughputBlocksPerSec !== undefined
  if (!hasThroughput) {
    return null
  }

  return (
    <div className="flex flex-col gap-2">
      <p className="font-mono text-[11px] tracking-wide text-muted-foreground uppercase">
        last-heartbeat throughput
      </p>
      <p className="text-xs text-muted-foreground">
        Windowed rates from the last completed status heartbeat. Unknown
        top-level keys fail parse. Not live-qualified. Missing rates are
        omitted, not invented.
      </p>
      {throughputRecordsPerSec !== undefined ? (
        <Pair
          field="throughput_records_per_sec"
          value={`${throughputRecordsPerSec} · last heartbeat`}
        />
      ) : null}
      {throughputBlocksPerSec !== undefined ? (
        <Pair
          field="throughput_blocks_per_sec"
          value={`${throughputBlocksPerSec} · last heartbeat`}
        />
      ) : null}
    </div>
  )
}

function Metric({
  field,
  value,
  omitted = false,
}: {
  field: string
  value: string
  omitted?: boolean
}) {
  return (
    <div className="flex flex-col gap-1 rounded-lg border p-3">
      <p className="font-mono text-[11px] tracking-wide text-muted-foreground uppercase">
        {field}
      </p>
      <p
        className={
          omitted
            ? "font-mono text-sm text-muted-foreground"
            : "desk-metric font-mono"
        }
      >
        {value}
      </p>
    </div>
  )
}

function Stage({
  field,
  value,
  muted = false,
}: {
  field: string
  value: string
  muted?: boolean
}) {
  return (
    <div className="flex min-w-0 flex-1 flex-col gap-1">
      <p className="font-mono text-[11px] tracking-wide text-muted-foreground uppercase">
        {field}
      </p>
      <p
        className={
          muted
            ? "truncate font-mono text-xs text-muted-foreground"
            : "truncate font-mono text-xs"
        }
      >
        {value}
      </p>
    </div>
  )
}

function Pair({ field, value }: { field: string; value: string }) {
  return (
    <div className="flex min-w-0 flex-col gap-0.5">
      <dt className="font-mono text-[11px] tracking-wide text-muted-foreground uppercase">
        {field}
      </dt>
      <dd className="truncate font-mono">{value}</dd>
    </div>
  )
}
