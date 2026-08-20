import { ActivityIcon, UnplugIcon } from "lucide-react"

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert"
import {
  Empty,
  EmptyDescription,
  EmptyHeader,
  EmptyMedia,
  EmptyTitle,
} from "@/components/ui/empty"
import { Skeleton } from "@/components/ui/skeleton"

export type ConnectionKind =
  | "loading"
  | "disconnected"
  | "unauthorized"
  | "degraded"
  | "unavailable"
  | "polling"

export function ConnectionBanner({
  kind,
  detail,
}: {
  kind: ConnectionKind
  detail: string
}) {
  if (kind === "loading") {
    return (
      <div className="flex flex-col gap-2">
        <Skeleton className="h-12 w-full" />
      </div>
    )
  }
  if (kind === "disconnected") {
    return (
      <Empty className="border">
        <EmptyHeader>
          <EmptyMedia variant="icon">
            <UnplugIcon />
          </EmptyMedia>
          <EmptyTitle>disconnected</EmptyTitle>
          <EmptyDescription>
            hl-api is unreachable at the Vite proxy target. Start{" "}
            <span className="font-mono">hl-api run</span> on 127.0.0.1:8788.
            {detail ? ` ${detail}` : ""}
          </EmptyDescription>
        </EmptyHeader>
      </Empty>
    )
  }
  if (kind === "polling") {
    return (
      <Alert>
        <ActivityIcon />
        <AlertTitle className="font-mono">polling</AlertTitle>
        <AlertDescription>{detail}</AlertDescription>
      </Alert>
    )
  }

  const variant =
    kind === "unauthorized" || kind === "unavailable"
      ? "destructive"
      : "default"
  const title =
    kind === "unauthorized"
      ? "unauthorized"
      : kind === "unavailable"
        ? "data_unavailable"
        : "degraded"

  return (
    <Alert variant={variant}>
      <ActivityIcon />
      <AlertTitle className="font-mono">{title}</AlertTitle>
      <AlertDescription>{detail}</AlertDescription>
    </Alert>
  )
}
