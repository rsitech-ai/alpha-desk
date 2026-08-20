import { describe, expect, it } from "vitest"

import {
  AUXILIARY_SOURCE_HEALTH,
  CAPTURE_SOURCE_HEALTH,
} from "@/lib/contracts"
import { sourceHealthTone, type SourceHealth, type Tone } from "@/lib/tone"

const EXPECTED_TONES: Record<SourceHealth, Tone> = {
  healthy: "green",
  starting: "yellow",
  quarantined: "yellow",
  "range-unavailable": "red",
  latched: "red",
}

describe("sourceHealthTone", () => {
  it("names every constructible committed and auxiliary source health", () => {
    const named = new Set<SourceHealth>([
      ...CAPTURE_SOURCE_HEALTH,
      ...AUXILIARY_SOURCE_HEALTH,
    ])
    expect([...named].sort()).toEqual(
      (Object.keys(EXPECTED_TONES) as SourceHealth[]).sort()
    )
    for (const health of named) {
      expect(sourceHealthTone(health)).toBe(EXPECTED_TONES[health])
    }
  })
})
