import { useEffect, useState } from "react"

import { POLL_INTERVAL_MS, fetchDeskFeed, type DeskFeed } from "@/lib/api"

export type FeedState =
  | { phase: "loading" }
  | { phase: "ready"; feed: DeskFeed }
  | { phase: "disconnected"; detail: string; feed?: DeskFeed }

export function useHlApi(): FeedState {
  const [state, setState] = useState<FeedState>({ phase: "loading" })

  useEffect(() => {
    let cancelled = false
    let timer = 0
    let controller = new AbortController()

    const tick = async () => {
      controller.abort()
      controller = new AbortController()
      try {
        const feed = await fetchDeskFeed(controller.signal)
        if (cancelled) {
          return
        }
        if (feed.healthz.kind === "network") {
          setState({
            phase: "disconnected",
            detail: feed.healthz.detail,
            feed,
          })
        } else {
          setState({ phase: "ready", feed })
        }
      } catch (error) {
        if (cancelled || controller.signal.aborted) {
          return
        }
        setState({
          phase: "disconnected",
          detail: error instanceof Error ? error.message : "poll failed",
        })
      }
      if (!cancelled) {
        timer = window.setTimeout(() => {
          void tick()
        }, POLL_INTERVAL_MS)
      }
    }

    void tick()

    return () => {
      cancelled = true
      controller.abort()
      window.clearTimeout(timer)
    }
  }, [])

  return state
}
