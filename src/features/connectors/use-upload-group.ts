"use client"

import { useRefreshable } from "@/hooks/use-refreshable"
import type { QueueFile } from "@/lib/upload-queue"
import { columnSpecOf, groupProblem } from "@/lib/uploads"
import { uploadService } from "@/services"
import type {
  UploadMismatch,
  UploadParseOptions,
  UploadPreviewColumn,
} from "@/services/contracts/uploads"

/** One file after the first, as the Check step lists it. */
export type GroupFile = {
  readonly key: string
  readonly name: string
  readonly size: number
  readonly state: QueueFile["state"]
  /** Its upload id, once the API has it all. */
  readonly uploadId: string | null
  /** What differs from the first file, when it does; `null` for a file that matches or was not compared yet. */
  readonly mismatch: UploadMismatch | null
  /** The comparison that covers this file has come back. */
  readonly compared: boolean
}

type Settled = { key: string; mismatches: UploadMismatch[] | null }

/**
 * The other files of a load of several files into one table (`DATA-17` slice 5, decision 4):
 * which of them have arrived, and which cannot load with the first one.
 *
 * The comparison is asked of the API (`POST /api/uploads/{id}/compare`) under the reading and the
 * column types in force on the Check step, so the types the other files must fit are the person's,
 * and it is asked again when the files, the reading or a column's type change. A change of a
 * column's NAME does not ask again: only types decide, and the API refuses a list with a bad
 * name, which the screen would only repeat. The previous answer stays on screen while a new
 * one is read (`useRefreshable`), and Load stays off until the answer is for what is in force.
 *
 * `ready` is what the page may go on with: no file is still being sent, none was found to differ,
 * and the comparison (when there are files to compare) has come back for the files and settings now
 * in force. A failed comparison is not "no mismatch": it keeps Load off and says why.
 */
export function useUploadGroup({
  primaryId,
  others,
  reading,
  sheet,
  columns,
  columnsValid,
}: {
  readonly primaryId: string | null
  /** The files after the first that were not refused, in the order picked. */
  readonly others: readonly QueueFile[]
  /** The settings in force on the Check step (the preview's `using`), when it has settled. */
  readonly reading: UploadParseOptions | null
  readonly sheet: string | undefined
  readonly columns: readonly UploadPreviewColumn[]
  /** The column list has no problem the screen can see, so the API would not refuse it. */
  readonly columnsValid: boolean
}) {
  const arrived = others.filter((file) => file.state === "received" && file.result !== null)
  const ids = arrived.map((file) => file.result?.id ?? "")
  const pending = others.length - arrived.length
  const typeKey = columnsValid
    ? columns.map((c) => `${c.type}/${c.dateFormat ?? ""}/${c.timezone ?? ""}`).join(",")
    : "guess"
  const key = `${primaryId ?? ""}|${ids.join(",")}|${JSON.stringify(reading ?? null)}|${sheet ?? ""}|${typeKey}`
  const wanted = primaryId !== null && ids.length > 0 && reading !== null

  const state = useRefreshable<Settled>(async (signal) => {
    if (!wanted) return { key, mismatches: null }
    const answer = await uploadService.compare(
      primaryId,
      {
        alsoUploads: ids,
        options: { encoding: reading.encoding, delimiter: reading.delimiter, headerRow: reading.headerRow, ...(sheet !== undefined ? { sheet } : {}) },
        ...(columnsValid && columns.length > 0 ? { columns: columns.map(columnSpecOf) } : {}),
      },
      signal
    )
    return { key, mismatches: answer.mismatches }
  }, key)

  const settled = state.data !== null && state.data.key === key ? state.data.mismatches : null
  // Whatever the last answer said stays on screen while a new one is read; it is only trusted
  // for `ready` when it is for what is in force now.
  const shown = settled ?? state.data?.mismatches ?? null

  const files: GroupFile[] = others.map((file) => {
    const uploadId = file.result?.id ?? null
    return {
      key: file.key,
      name: file.name,
      size: file.size,
      state: file.state,
      uploadId,
      mismatch: uploadId === null ? null : (shown?.find((m) => m.uploadId === uploadId) ?? null),
      compared: settled !== null && uploadId !== null,
    }
  })

  const mismatched = settled === null ? 0 : settled.length
  const checking = wanted && settled === null && state.error === null
  const error = wanted && settled === null ? state.error : null
  const ready = pending === 0 && mismatched === 0 && (!wanted || (settled !== null && state.error === null))
  const problem = groupProblem({ pending, mismatched })

  return {
    /** The files after the first. */
    files,
    /** Their upload ids, in order: what the load is told (`alsoUploads`). */
    ids,
    /** How many are still being sent. */
    pending,
    mismatched,
    checking,
    error,
    ready,
    /** Why Load is off because of the other files, or `null`. */
    problem,
    retry: state.refresh,
  }
}

export type UploadGroupView = ReturnType<typeof useUploadGroup>
