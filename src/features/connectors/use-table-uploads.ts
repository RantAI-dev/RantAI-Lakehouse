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
 * person typed. An answer is used only for the name it was asked for: while the
 * kept answer's name is not the current one, the state is `checking`, and the
 * page keeps Load off rather than guessing.
 */
export function useTableUploads(table: string | null, delayMs: number) {
  const [answer, setAnswer] = React.useState<{ table: string; uploads: readonly Upload[] } | null>(null)
  const [failure, setFailure] = React.useState<{ table: string; error: ServiceError } | null>(null)
  const [asked, setAsked] = React.useState(0)

  React.useEffect(() => {
    if (table === null) return
    const controller = new AbortController()
    const timer = window.setTimeout(() => {
      uploadService
        .list(controller.signal, { table })
        .then((uploads) => setAnswer({ table, uploads }))
        .catch((err: unknown) => {
          const error = toServiceError(err)
          if (error.code !== "aborted") setFailure({ table, error })
        })
    }, delayMs)
    return () => {
      window.clearTimeout(timer)
      controller.abort()
    }
  }, [table, delayMs, asked])

  const state: TableUploadsState =
    table === null
      ? { status: "idle" }
      : failure !== null && failure.table === table
        ? { status: "error", error: failure.error }
        : answer !== null && answer.table === table
          ? { status: "known", uploads: answer.uploads }
          : { status: "checking" }

  return {
    state,
    /** Ask again for the same name, after a failure (`Check again`). */
    retry: () => {
      setFailure(null)
      setAsked((count) => count + 1)
    },
  }
}
