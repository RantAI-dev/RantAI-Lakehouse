"use client"

import { useCallback, useEffect, useRef, useState } from "react"
import type { ConnectorDetail, ConnectorRun } from "@/services/contracts/connectors"

/** `SRC-46`: how often the detail is read again after a run ended, and for how long, while the run is not yet counted. */
const COUNT_POLL_MS = 10_000
const COUNT_WAIT_MAX_MS = 180_000

type CountWait = { field: "lastRunSuccessAt" | "lastRunFailureAt"; from: string | null }

/**
 * The wait of `SRC-46` D2 (`QA LH-SRC-6-43`): the connector's health follows a run only when the orchestrator's
 * sensor reports it to the API, about a minute after the run ends, and nothing else re-reads the detail. So when
 * the Ingest panel says a run ended, `reload` runs now, then every `pollMs` until the run is counted (the outcome's
 * timestamp differs from what it was when the run ended), for at most `maxMs`. A run that is never counted (paused,
 * or a missed sensor tick) just stops being asked about; a page load reads the truth. Returns the handler for the
 * panel's `onRunEnded`. A newer run ending replaces the older wait; unmounting clears it.
 *
 * `SRC-46 review round 3, BLOCKER 3`: a hook of its own so its tests can run with real timers and tiny values
 * (fake timers with stubbed fetch hung on CI).
 */
export function useRunCountWait(
  detail: ConnectorDetail | null,
  reload: () => void,
  { pollMs = COUNT_POLL_MS, maxMs = COUNT_WAIT_MAX_MS }: { pollMs?: number; maxMs?: number } = {}
): (run: ConnectorRun) => void {
  const [countWait, setCountWait] = useState<CountWait | null>(null)
  const detailRef = useRef<ConnectorDetail | null>(null)
  // `SRC-46 review round 1, BLOCKER 1`: synced in an effect, not during render (`react-hooks/refs`). The handler
  // runs from a later poll, after the commit, so it reads the detail as of the last render.
  useEffect(() => {
    detailRef.current = detail
  }, [detail])
  const onRunEnded = useCallback(
    (run: ConnectorRun) => {
      reload()
      const field = run.status === "failed" ? "lastRunFailureAt" : run.status === "succeeded" ? "lastRunSuccessAt" : null
      // Any other status (paused, a run with no end reported) reads once and does not wait for a count.
      // A new run ending replaces the older wait: the effect below restarts on the new object.
      setCountWait(field === null ? null : { field, from: detailRef.current?.[field] ?? null })
    },
    [reload]
  )
  // Counted when the outcome's timestamp differs from its value at the run's end. Derived, not set from an effect
  // (`react-hooks/set-state-in-effect`, `SRC-46 review round 1, BLOCKER 1`): a counted wait stays in state, inactive,
  // until a newer run replaces it.
  const countedAt = countWait && detail ? (detail[countWait.field] ?? null) : null
  const waiting = countWait !== null && countedAt === countWait.from ? countWait : null
  useEffect(() => {
    if (!waiting) return
    const poll = window.setInterval(reload, pollMs)
    const giveUp = window.setTimeout(() => setCountWait(null), maxMs)
    return () => {
      window.clearInterval(poll)
      window.clearTimeout(giveUp)
    }
  }, [waiting, reload, pollMs, maxMs])
  return onRunEnded
}
