"use client"

import * as React from "react"
import { uploadService } from "@/services"
import type { Upload } from "@/services/contracts/uploads"
import type { ServiceError } from "@/services/errors"
import { toServiceError } from "@/services/errors"

/** How long a typed name waits for the next key before its uploads are asked for. */
export const TABLE_LOOKUP_DELAY_MS = 400

export type TableUploadsState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "known"; uploads: readonly Upload[] }
  | { status: "error"; error: ServiceError }

/**
 * The uploads that loaded into `table`, asked from the API by its exact name
 * (`DATA-47`, QA `LH-DATA-35-03`): the Table step needs them to know whether the
 * name is a table an earlier upload created — however old — and which folder it
 * sits in.
 *
 * Not `useService`: `useService` has no pause and no per-key abort, and a name is
 * typed a key at a time. This is `useColumnChoices`'s shape — a timer, an
 * `AbortController`, and an aborted request ignored — because the same problem
 * is being solved: one request after the last key, never one per key.
 *
 * `delayMs` is 0 for a name the page set itself (the suggestion from the file
 * name, or the table of an upload opened with `?id=`), so that answer is
 * normally there before the person reaches the Table step, and 400 for a name a
 * person typed.
 *
 * An answer and a failure belong to ONE ask, not to a name (review round 1 R2,
 * R3): the effect bumps a counter for every request it schedules, and only the
 * ask of the moment may answer. So a name asked again — after "Upload another
 * file", after "Check again", or a name typed back — is `checking` until its own
 * answer arrives, and an answer or failure from before that ask can neither
 * stand in for it (a slow lookup must not read as "a new table", which loads
 * with Replace) nor outlive it (a check that succeeds after one that failed
 * replaces the failure). The page keeps Load off while the state is `checking`.
 */
export function useTableUploads(table: string | null, delayMs: number) {
  const [answer, setAnswer] = React.useState<{
    ask: number
    table: string
    uploads: readonly Upload[]
  } | null>(null)
  const [failure, setFailure] = React.useState<{
    ask: number
    table: string
    error: ServiceError
  } | null>(null)
  /** The ask of the moment: the last request this effect scheduled. */
  const [ask, setAsk] = React.useState(0)
  // A ref, so the id is known when the request is scheduled, before its answer.
  const lastAsk = React.useRef(0)
  const [retried, setRetried] = React.useState(0)

  React.useEffect(() => {
    // Every scheduled request — the `table === null` gap between a reset and the
    // next name included — makes what came before stale.
    lastAsk.current += 1
    const scheduled = lastAsk.current
    setAsk(scheduled)
    if (table === null) return
    const controller = new AbortController()
    const timer = window.setTimeout(() => {
      uploadService
        .list(controller.signal, { table })
        .then((uploads) => setAnswer({ ask: scheduled, table, uploads }))
        .catch((err: unknown) => {
          const error = toServiceError(err)
          if (error.code !== "aborted") setFailure({ ask: scheduled, table, error })
        })
    }, delayMs)
    return () => {
      window.clearTimeout(timer)
      controller.abort()
    }
  }, [table, delayMs, retried])

  const state: TableUploadsState =
    table === null
      ? { status: "idle" }
      : failure !== null && failure.ask === ask && failure.table === table
        ? { status: "error", error: failure.error }
        : answer !== null && answer.ask === ask && answer.table === table
          ? { status: "known", uploads: answer.uploads }
          : { status: "checking" }

  return {
    state,
    /**
     * Ask again for the same name, after a failure (`Check again`). It is
     * `checking`, not the answer kept from before that failure, until it answers.
     */
    retry: () => setRetried((count) => count + 1),
  }
}
