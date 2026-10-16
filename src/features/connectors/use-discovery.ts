"use client"

import * as React from "react"
import { useServiceAction } from "@/hooks/use-service"
import { runDiscovery } from "@/lib/connectors/run-discovery"
import { connectorService } from "@/services"
import type { DiscoveryAnswer, DiscoveryColumn, DiscoveryRequest } from "@/services/contracts/connectors"

/**
 * One listing of a connector's source (`SRC-13`): `run(request)` asks, waits for the
 * orchestrator when the source is one only it reads, and resolves with the answer.
 * `waitedMs` is `null` while nothing waits on the orchestrator and the milliseconds
 * waited since it was asked while something does, for the progress line (D1).
 *
 * `pollMs` is for tests; the default is `DISCOVERY_POLL_MS`.
 */
export function useDiscoveryList(connectorId: string, pollMs?: number) {
  const [waitedMs, setWaitedMs] = React.useState<number | null>(null)
  const action = useServiceAction(async (signal, request: DiscoveryRequest) => {
    try {
      return await runDiscovery({
        kind: "list",
        start: () => connectorService.startDiscovery(connectorId, "list", request, signal),
        get: () => connectorService.getDiscovery(connectorId, "list", signal),
        signal,
        pollMs,
        onWaiting: setWaitedMs,
      })
    } finally {
      setWaitedMs(null)
    }
  })
  return { ...action, waitedMs }
}

/** What is known about one object's columns. */
export type PreviewEntry =
  | { status: "loading" }
  | { status: "done"; columns: DiscoveryColumn[]; primaryKey: string[] }
  | { status: "failed"; reason: string }

export type ColumnPreviews = {
  entries: ReadonlyMap<string, PreviewEntry>
  /**
   * Read the columns of `name` unless they are known or on their way. `force` reads them
   * again after a failure. Safe to call on every render: a name is asked for once.
   */
  ensure: (name: string, force?: boolean) => void
}

const PREVIEW_FAILED = "The columns could not be read."

/**
 * The columns (and key) of the objects of one connector, read through the preview route
 * on demand (`SRC-13` decisions D6 and D9: names first, columns when a table is opened).
 *
 * The API keeps ONE preview answer per connector (the latest wins), so two previews in
 * flight at once would replace each other's answer on the orchestrator's kinds. The reads
 * therefore run one after another, in the order asked.
 *
 * It keeps its own map rather than using `useServiceAction` because it holds many objects'
 * answers at once, keyed by name; `runDiscovery` and the service are still the only way
 * it reaches the API.
 */
export function useColumnPreviews(connectorId: string, pollMs?: number): ColumnPreviews {
  const [entries, setEntries] = React.useState<ReadonlyMap<string, PreviewEntry>>(() => new Map())
  const known = React.useRef<Map<string, PreviewEntry>>(new Map())
  const queue = React.useRef<Promise<void>>(Promise.resolve())
  const controller = React.useRef<AbortController>(new AbortController())

  React.useEffect(() => {
    const own = new AbortController()
    controller.current = own
    return () => own.abort()
  }, [])

  const set = React.useCallback((name: string, entry: PreviewEntry) => {
    known.current.set(name, entry)
    setEntries(new Map(known.current))
  }, [])

  const ensure = React.useCallback(
    (name: string, force = false) => {
      const have = known.current.get(name)
      if (have && (have.status !== "failed" || !force)) return
      set(name, { status: "loading" })
      const signal = controller.current.signal
      queue.current = queue.current.then(async () => {
        if (signal.aborted) return
        try {
          const answer: DiscoveryAnswer = await runDiscovery({
            kind: "preview",
            start: () => connectorService.startDiscovery(connectorId, "preview", { object: name }, signal),
            get: () => connectorService.getDiscovery(connectorId, "preview", signal),
            signal,
            pollMs,
          })
          if (signal.aborted) return
          set(
            name,
            answer.state === "done"
              ? { status: "done", columns: answer.columns ?? [], primaryKey: answer.primaryKey ?? [] }
              : { status: "failed", reason: answer.reason ?? PREVIEW_FAILED }
          )
        } catch {
          // An aborted read (the page went away) or a refused call: the entry says the columns are
          // not known, in words of ours; the call's own error text is not shown here.
          if (!signal.aborted) set(name, { status: "failed", reason: PREVIEW_FAILED })
        }
      })
    },
    [connectorId, pollMs, set]
  )

  return React.useMemo(() => ({ entries, ensure }), [entries, ensure])
}
