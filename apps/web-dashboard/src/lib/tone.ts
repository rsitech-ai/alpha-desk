import {
  assertNever,
  type AuxiliarySourceHealth,
  type CaptureHealth,
  type CaptureSourceHealth,
  type HealthState,
} from "@/lib/contracts"

export type Tone = "green" | "yellow" | "red" | "neutral"

export type SourceHealth = CaptureSourceHealth | AuxiliarySourceHealth

export function healthStateTone(state: HealthState): Tone {
  switch (state) {
    case "HEALTH_STATE_GREEN":
      return "green"
    case "HEALTH_STATE_AMBER":
      return "yellow"
    case "HEALTH_STATE_RED":
      return "red"
    default:
      return assertNever(state)
  }
}

export function captureHealthTone(health: CaptureHealth): Tone {
  switch (health) {
    case "green":
      return "green"
    case "yellow":
      return "yellow"
    case "red":
      return "red"
    default:
      return assertNever(health)
  }
}

export function sourceHealthTone(health: SourceHealth): Tone {
  switch (health) {
    case "healthy":
      return "green"
    case "starting":
    case "quarantined":
      return "yellow"
    case "range-unavailable":
    case "latched":
      return "red"
    default:
      return assertNever(health)
  }
}

export function readyTone(ready: boolean): Tone {
  return ready ? "green" : "red"
}

export function toneWithoutLiveOnHttpError(status: number, tone: Tone): Tone {
  if (status === 503 && tone === "green") {
    return "red"
  }
  return tone
}
