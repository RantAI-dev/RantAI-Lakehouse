"use client"

import * as React from "react"

import { saveBlob } from "@/lib/browser-file"
import { formatNumber } from "@/lib/format"
import { notifyFailure, notifySuccess } from "@/lib/notify"
import { queryService } from "@/services"
import type { QueryDownload, QueryDownloadFormat } from "@/services/contracts/queries"

/**
 * What a finished download says: the file that arrived, and what it holds.
 *
 * A file that was cut says so, and names the limit that cut it: a partial
 * file read as a whole one is worse than no file at all.
 */
function describeDownload(result: QueryDownload): { message: string; description?: string } {
  const message = `Downloaded ${result.fileName}`
  if (!result.cut) {
    // The engine-written file carries no row count: the server never counted
    // the rows it wrote, so the console does not invent one.
    if (result.rows === null) return { message }
    return { message, description: `The file holds ${formatNumber(result.rows)} rows.` }
  }
  if (result.cutBy === "bytes") {
    return { message, description: "The file was cut at the 5 GB limit." }
  }
  return {
    message,
    description: `The file was cut at ${formatNumber(result.rows)} rows, which is the limit of this format.`,
  }
}

/**
 * The download of a query result: the file in flight, and the outcome of the
 * last one.
 *
 * A download is a query on the engine like any other: it carries the run's
 * own id, so the Stop button reaches it there. The browser's request is
 * aborted as well, but that only ends the waiting, not the query.
 */
export function useQueryDownload() {
  const [busy, setBusy] = React.useState<QueryDownloadFormat | null>(null)
  const [done, setDone] = React.useState<QueryDownload | null>(null)
  const [error, setError] = React.useState<string | null>(null)
  // The request in flight, and the run it belongs to. Stop needs the run id
  // to end the download's query on the engine.
  const controllerRef = React.useRef<AbortController | null>(null)
  const runIdRef = React.useRef<string | null>(null)

  React.useEffect(() => () => controllerRef.current?.abort(), [])

  const start = React.useCallback(
    (id: string, format: QueryDownloadFormat, runId: string) => {
      // The download has a controller of its own: it is not a run, and
      // stopping the run must not end a file the person asked for after it.
      const controller = new AbortController()
      controllerRef.current = controller
      runIdRef.current = runId
      setBusy(format)
      setError(null)
      void (async () => {
        try {
          const result = await queryService.download(id, format, runId, controller.signal)
          // The person stopped it: nothing was downloaded, so nothing is said.
          if (controller.signal.aborted) return
          saveBlob(result.blob, result.fileName)
          setDone(result)
          const { message, description } = describeDownload(result)
          notifySuccess(message, description)
        } catch (err) {
          if (controller.signal.aborted) return
          // The server's own sentence, as it was sent: a made-up one would
          // hide what the server refused and why.
          const message = err instanceof Error ? err.message : "The download failed."
          setError(message)
          notifyFailure(message)
        } finally {
          // A download that was started after this one owns the button now.
          if (controllerRef.current !== controller) return
          controllerRef.current = null
          setBusy(null)
        }
      })()
    },
    []
  )

  const stop = React.useCallback(() => {
    controllerRef.current?.abort()
    controllerRef.current = null
    const runId = runIdRef.current
    runIdRef.current = null
    // The download's query runs on the engine, so the server is asked to end
    // it there as well. Best effort: the person asked for it to stop, and
    // the browser's request is already gone.
    if (runId) void queryService.cancel(runId).catch(() => undefined)
    setBusy(null)
  }, [])

  return { busy, done, error, start, stop }
}
