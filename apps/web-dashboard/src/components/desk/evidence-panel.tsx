import { FileSearchIcon, WavesIcon } from "lucide-react"

import { Button } from "@/components/ui/button"
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
import { ScrollArea } from "@/components/ui/scroll-area"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet"
import { Skeleton } from "@/components/ui/skeleton"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { FieldTable } from "@/components/desk/field-table"
import { ToneBadge } from "@/components/desk/chips"
import { sourceHealthTone } from "@/lib/tone"
import type { EndpointOutcome } from "@/lib/api"
import {
  AUXILIARY_SOURCE_FIELD_ORDER,
  CAPTURE_STATUS_FIELD_ORDER,
  isRecord,
  type AuxiliarySourceStatus,
  type CaptureStatus,
} from "@/lib/contracts"
import { mapApiError } from "@/lib/fail-closed"

export function EvidenceCard({
  loading,
  outcome,
}: {
  loading: boolean
  outcome: EndpointOutcome<CaptureStatus> | undefined
}) {
  return (
    <Card size="sm" className="h-full">
      <CardHeader className="border-b">
        <CardTitle>Evidence</CardTitle>
        <CardDescription className="font-mono">
          status drill-down · auxiliary_sources
        </CardDescription>
      </CardHeader>
      <CardContent className="flex flex-col gap-4">
        {loading ? (
          <Skeleton className="h-40 w-full" />
        ) : (
          <EvidenceBody outcome={outcome} />
        )}
      </CardContent>
    </Card>
  )
}

function EvidenceBody({
  outcome,
}: {
  outcome: EndpointOutcome<CaptureStatus> | undefined
}) {
  if (!outcome) {
    return null
  }
  switch (outcome.kind) {
    case "network":
    case "invalid":
      return (
        <Empty>
          <EmptyHeader>
            <EmptyMedia variant="icon">
              <FileSearchIcon />
            </EmptyMedia>
            <EmptyTitle>no validated snapshot</EmptyTitle>
            <EmptyDescription>
              {outcome.kind === "network" ? outcome.detail : outcome.detail}
            </EmptyDescription>
          </EmptyHeader>
        </Empty>
      )
    case "http-error": {
      const view = mapApiError(outcome.status, outcome.error)
      return (
        <div className="flex flex-col gap-2">
          <ToneBadge tone={view.tone}>{view.title}</ToneBadge>
          <p className="text-xs text-muted-foreground">{view.detail}</p>
          <FieldTable
            caption="hl.api.error.v1"
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
      return <EvidenceDetail status={outcome.data} raw={outcome.raw} />
    default: {
      const _exhaustive: never = outcome
      return _exhaustive
    }
  }
}

function EvidenceDetail({
  status,
  raw,
}: {
  status: CaptureStatus
  raw: unknown
}) {
  const record = isRecord(raw)
    ? raw
    : (status as unknown as Record<string, unknown>)
  const known = new Set<string>(CAPTURE_STATUS_FIELD_ORDER)
  const rows = CAPTURE_STATUS_FIELD_ORDER.map((field) => ({
    field,
    value: record[field],
    omitted: !(field in record) || record[field] === undefined,
  }))
  const extras = Object.keys(record)
    .filter((key) => !known.has(key))
    .sort()
    .map((field) => ({ field, value: record[field], omitted: false }))

  return (
    <div className="flex flex-col gap-4">
      <Sheet>
        <SheetTrigger
          render={
            <Button variant="outline" size="sm">
              <FileSearchIcon data-icon="inline-start" />
              Full snapshot
            </Button>
          }
        />
        <SheetContent side="right" className="data-[side=right]:sm:max-w-2xl">
          <SheetHeader>
            <SheetTitle>hl.capture.status.v4</SheetTitle>
            <SheetDescription>
              Validated fields in contract order. Extra keys are listed after
              the frozen set. This is not production qualification.
            </SheetDescription>
          </SheetHeader>
          <ScrollArea className="h-full px-4 pb-4">
            <FieldTable rows={[...rows, ...extras]} caption="capture status" />
          </ScrollArea>
        </SheetContent>
      </Sheet>
      <AuxiliarySection sources={status.auxiliary_sources} />
    </div>
  )
}

function AuxiliarySection({
  sources,
}: {
  sources: AuxiliarySourceStatus[] | undefined
}) {
  if (sources === undefined || sources.length === 0) {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <WavesIcon />
          </EmptyMedia>
          <EmptyTitle>auxiliary_sources omitted</EmptyTitle>
          <EmptyDescription>
            The V4 contract omits auxiliary_sources when no node-line adapters
            are enabled. Auxiliary qualification is not live-source
            qualification. This UI does not invent fills or charts.
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }

  return (
    <div className="flex flex-col gap-2">
      <p className="font-mono text-[11px] tracking-wide text-muted-foreground uppercase">
        auxiliary_sources ({sources.length}) · aux qualification is not
        live-qualified
      </p>
      <Table>
        <TableHeader>
          <TableRow>
            <TableHead className="font-mono text-[11px]">source_id</TableHead>
            <TableHead className="font-mono text-[11px]">health</TableHead>
            <TableHead className="font-mono text-[11px]">
              qualification
            </TableHead>
            <TableHead className="font-mono text-[11px]">
              spool_records
            </TableHead>
            <TableHead className="font-mono text-[11px]">
              unarchived_records
            </TableHead>
            <TableHead className="font-mono text-[11px]">
              unread_bytes
            </TableHead>
            <TableHead className="font-mono text-[11px]">
              partial_line
            </TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {sources.map((source) => (
            <TableRow key={source.source_id}>
              <TableCell className="font-mono text-xs">
                {source.source_id}
              </TableCell>
              <TableCell>
                <ToneBadge tone={sourceHealthTone(source.health)}>
                  {source.health}
                </ToneBadge>
              </TableCell>
              <TableCell>
                <ToneBadge tone="neutral">{source.qualification}</ToneBadge>
              </TableCell>
              <TableCell className="font-mono text-xs tabular-nums">
                {source.spool_records}
              </TableCell>
              <TableCell className="font-mono text-xs tabular-nums">
                {source.unarchived_records}
              </TableCell>
              <TableCell className="font-mono text-xs tabular-nums">
                {source.unread_bytes === undefined
                  ? "omitted"
                  : source.unread_bytes}
              </TableCell>
              <TableCell className="font-mono text-xs">
                {String(source.partial_line)}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
      {sources.map((source) => (
        <details
          key={`${source.source_id}-detail`}
          className="rounded-lg border p-3"
        >
          <summary className="cursor-pointer font-mono text-xs">
            {source.source_id} fields
          </summary>
          <FieldTable
            caption={source.source_id}
            rows={[
              ...AUXILIARY_SOURCE_FIELD_ORDER.map((field) => {
                const record = source as unknown as Record<string, unknown>
                return {
                  field,
                  value: record[field],
                  omitted: record[field] === undefined,
                }
              }),
              ...Object.keys(source.extra_fields)
                .sort()
                .map((field) => ({
                  field,
                  value: source.extra_fields[field],
                  omitted: false,
                })),
            ]}
          />
        </details>
      ))}
    </div>
  )
}
