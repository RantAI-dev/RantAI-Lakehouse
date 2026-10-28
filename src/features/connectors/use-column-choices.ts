"use client"

import * as React from "react"
import { columnListProblems, columnSpecOf, withColumnType } from "@/lib/uploads"
import { uploadService } from "@/services"
import type {
  UploadCheck,
  UploadColumnType,
  UploadDateFormat,
  UploadOnBadValue,
  UploadPreviewColumn,
} from "@/services/contracts/uploads"
import type { ServiceError } from "@/services/errors"
import { toServiceError } from "@/services/errors"
import type { UploadPreviewView } from "./upload-check-step"

/** How long a change waits for the next one before the check is asked (the page is typed into). */
const CHECK_DELAY_MS = 400

export type CheckState =
  | { status: "idle" }
  | { status: "checking" }
  | { status: "done"; result: UploadCheck }
  | { status: "error"; error: ServiceError }

/**
 * The columns of the load as a person is shaping them (`DATA-18`): the preview's
 * `columnSpecs` with the person's edits on top, what to do with a value that does
 * not fit (**stop**, pre-selected: decision 6), and the check of the list against
 * the sample, asked after a short pause so typing a name is not a request per key.
 *
 * The edits are by position and are dropped when the preview they were made on is
 * replaced (a new reading, or a table chosen): they would belong to other columns.
 * A list with a problem the screen can see (a name that breaks the rule, a date still
 * asking "day first or month first?") is not sent to the check: the API would only
 * refuse it, and `problems` already says it. With every column text there is nothing
 * to check.
 */
export function useColumnChoices(uploadId: string | null, view: UploadPreviewView) {
  const settled = view.settled
  const specs = settled?.columnSpecs ?? null
  const [edits, setEdits] = React.useState<Record<number, Partial<UploadPreviewColumn>>>({})
  const [editsFor, setEditsFor] = React.useState<typeof specs>(null)
  const [onBadValue, setOnBadValue] = React.useState<UploadOnBadValue>("stop")
  const [check, setCheck] = React.useState<CheckState>({ status: "idle" })

  if (specs !== editsFor) {
    setEditsFor(specs)
    setEdits({})
  }

  const fixed = settled?.fromTable === true
  const columns: UploadPreviewColumn[] = React.useMemo(
    () => (specs ?? []).map((column, index) => ({ ...column, ...edits[index] })),
    [specs, edits]
  )

  const change = (index: number, patch: (column: UploadPreviewColumn) => UploadPreviewColumn) => {
    if (fixed) return
    setEdits((all) => {
      const current = { ...(specs?.[index] as UploadPreviewColumn), ...all[index] }
      return { ...all, [index]: patch(current) }
    })
  }

  const problems = React.useMemo(() => columnListProblems(columns), [columns])
  const allText = columns.every((column) => column.type === "text")
  const reading = settled?.using
  const sheet = settled?.workbook?.sheet

  React.useEffect(() => {
    if (uploadId === null || columns.length === 0 || problems.length > 0 || allText) {
      setCheck({ status: "idle" })
      return
    }
    const controller = new AbortController()
    setCheck({ status: "checking" })
    const timer = window.setTimeout(() => {
      uploadService
        .check(
          uploadId,
          {
            options: {
              ...(reading ? { encoding: reading.encoding, delimiter: reading.delimiter, headerRow: reading.headerRow } : {}),
              ...(sheet !== undefined ? { sheet } : {}),
            },
            columns: columns.map(columnSpecOf),
          },
          controller.signal
        )
        .then((result) => setCheck({ status: "done", result }))
        .catch((err: unknown) => {
          const error = toServiceError(err)
          if (error.code !== "aborted") setCheck({ status: "error", error })
        })
    }, CHECK_DELAY_MS)
    return () => {
      window.clearTimeout(timer)
      controller.abort()
    }
  }, [uploadId, columns, problems, allText, reading, sheet])

  return {
    columns,
    /** The names and types are a table's: they cannot be changed. */
    fixed,
    onBadValue,
    setOnBadValue,
    check,
    problems,
    setName: (index: number, name: string) => change(index, (c) => ({ ...c, name })),
    setType: (index: number, type: UploadColumnType) => change(index, (c) => withColumnType(c, type)),
    setDateFormat: (index: number, dateFormat: UploadDateFormat) => change(index, (c) => ({ ...c, dateFormat })),
    setTimezone: (index: number, timezone: boolean) => change(index, (c) => ({ ...c, timezone })),
    /** Whether the list may go on: nothing to fix on the screen and the API has not refused it. */
    ready: problems.length === 0 && check.status !== "error",
  }
}

export type ColumnChoices = ReturnType<typeof useColumnChoices>
