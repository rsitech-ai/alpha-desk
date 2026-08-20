import type { ReactNode } from "react"

import { Badge } from "@/components/ui/badge"
import { cn } from "@/lib/utils"
import type { Tone } from "@/lib/tone"

export function ToneBadge({
  tone,
  children,
}: {
  tone: Tone
  children: ReactNode
}) {
  return (
    <Badge
      variant={tone === "red" ? "destructive" : "outline"}
      className={cn(
        "font-mono tracking-wide uppercase",
        tone === "green" &&
          "border-health-green/40 bg-health-green/10 text-health-green",
        tone === "yellow" &&
          "border-health-yellow/40 bg-health-yellow/10 text-health-yellow",
        tone === "red" && "border-destructive/40",
        tone === "neutral" && "text-muted-foreground"
      )}
    >
      {children}
    </Badge>
  )
}

export function LiveDot({ pulse }: { pulse: boolean }) {
  return (
    <span
      className={cn(
        "inline-block size-1.5 rounded-full bg-muted-foreground",
        pulse && "desk-live-dot"
      )}
      aria-hidden="true"
    />
  )
}
