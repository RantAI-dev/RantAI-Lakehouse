"use client"

import * as React from "react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useRefreshable } from "@/hooks/use-refreshable"
import {
  UPLOAD_DELIMITERS,
  UPLOAD_ENCODINGS,
  delimiterLabel,
  encodingLabel,
  headerRowDisplay,
  headerRowFromDisplay,
} from "@/lib/uploads"
import { uploadService } from "@/services"
import type { UploadParseOptions, UploadPreview } from "@/services/contracts/uploads"
import type { ServiceError } from "@/services/errors"
import { Refusal } from "./upload-parts"

const SELECT_CLASS = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

const HEADER_ROW_PROBLEM = "Enter a whole number, 1 or more."

type Settled = { key: string; preview: UploadPreview | null }

/**
 * What the Check step knows: the preview of one upload under the settings the
 * person chose, which settings those are, and whether the preview on screen
 * belongs to them.
 *
 * `overrides` holds only what the person changed; a setting left out is
 * detected by the API. The preview is read again whenever one changes, and
 * the previous one stays on screen until the new one arrives (`useRefreshable`
 * does not clear), so the controls keep their place and focus.
 * `headerText` is what is typed in the header-row field: it can be wrong
 * (empty, `0`, `1.5`) without a request being made, and then `headerProblem`
 * says so and `ready` is false.
 *
 * The 1-based number a person types and the 0-based index the API takes
 * meet only in `headerRowFromDisplay` / `headerRowDisplay`.
 */
export function useUploadPreview(uploadId: string | null) {
  const [overrides, setOverrides] = React.useState<Partial<UploadParseOptions>>({})
  const [headerText, setHeaderText] = React.useState<string | null>(null)
  const key = `${uploadId ?? ""}|${overrides.encoding ?? ""}|${JSON.stringify(overrides.delimiter ?? "")}|${overrides.headerRow ?? ""}`

  const state = useRefreshable<Settled>(async (signal) => {
    if (uploadId === null) return { key, preview: null }
    return { key, preview: await uploadService.preview(uploadId, overrides, signal) }
  }, key)

  const settled = state.data !== null && state.data.key === key ? state.data.preview : null
  const shown = settled ?? state.data?.preview ?? null

  // A preview for the settings now in force: the typed text has done its
  // job, so the field follows what the API used (it also shows a detected
  // header row). An invalid text is kept: it never made a request.
  const [syncedFor, setSyncedFor] = React.useState<UploadPreview | null>(null)
  if (settled !== null && settled !== syncedFor) {
    setSyncedFor(settled)
    if (headerText !== null && headerRowFromDisplay(headerText) !== null) setHeaderText(null)
  }

  const headerField = headerText ?? (shown ? String(headerRowDisplay(shown.using.headerRow)) : "")
  const headerProblem = headerText !== null && headerRowFromDisplay(headerText) === null ? HEADER_ROW_PROBLEM : null

  return {
    shown,
    settled,
    error: state.error,
    loading: uploadId !== null && settled === null && state.error === null,
    retry: state.refresh,
    headerField,
    headerProblem,
    /** The settings in force, or the one the API detected. */
    values: {
      encoding: overrides.encoding ?? shown?.using.encoding ?? "",
      delimiter: overrides.delimiter ?? shown?.using.delimiter ?? "",
    },
    setEncoding: (encoding: UploadParseOptions["encoding"]) => setOverrides((o) => ({ ...o, encoding })),
    setDelimiter: (delimiter: string) => setOverrides((o) => ({ ...o, delimiter })),
    setHeaderText: (text: string) => {
      setHeaderText(text)
      const row = headerRowFromDisplay(text)
      if (row !== null) setOverrides((o) => ({ ...o, headerRow: row }))
    },
    reset: () => {
      setOverrides({})
      setHeaderText(null)
    },
    /** Only a preview of the settings in force, with at least one column, lets the person go on. */
    ready: settled !== null && settled.columns.length > 0 && headerProblem === null,
  }
}

export type UploadPreviewView = ReturnType<typeof useUploadPreview>

function Detected({ text }: { readonly text: string }) {
  return <p className="text-xs text-muted-foreground">Detected: {text}</p>
}

/**
 * Step 2: how the file is read, and what that makes of it. Each control says
 * what the API detected; changing one reads the preview again. A preview
 * with no columns says so and the step's "Next" stays off (`view.ready`).
 */
export function UploadCheckStep({ view }: { readonly view: UploadPreviewView }) {
  const { shown, settled } = view
  if (shown === null) {
    if (view.error !== null) return <PreviewFailure error={view.error} onRetry={view.retry} />
    return <LoadingSkeleton rows={3} />
  }

  const encodings = UPLOAD_ENCODINGS.some((e) => e.value === view.values.encoding)
    ? UPLOAD_ENCODINGS
    : [...UPLOAD_ENCODINGS, { value: view.values.encoding as "utf-8", label: encodingLabel(view.values.encoding) }]
  const delimiters = UPLOAD_DELIMITERS.some((d) => d.value === view.values.delimiter)
    ? UPLOAD_DELIMITERS
    : [...UPLOAD_DELIMITERS, { value: view.values.delimiter, label: delimiterLabel(view.values.delimiter) }]

  return (
    <div className="space-y-4">
      <div className="grid gap-4 sm:grid-cols-3">
        <div className="space-y-1.5">
          <Label htmlFor="upload-encoding">Encoding</Label>
          <select
            id="upload-encoding"
            className={SELECT_CLASS}
            value={view.values.encoding}
            onChange={(e) => view.setEncoding(e.target.value as UploadParseOptions["encoding"])}
          >
            {encodings.map((e) => (
              <option key={e.value} value={e.value}>
                {e.label}
              </option>
            ))}
          </select>
          <Detected text={encodingLabel(shown.detected.encoding)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="upload-delimiter">Delimiter</Label>
          <select
            id="upload-delimiter"
            className={SELECT_CLASS}
            value={view.values.delimiter}
            onChange={(e) => view.setDelimiter(e.target.value)}
          >
            {delimiters.map((d) => (
              <option key={d.value} value={d.value}>
                {d.label}
              </option>
            ))}
          </select>
          <Detected text={delimiterLabel(shown.detected.delimiter)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="upload-header-row">Header row</Label>
          <Input
            id="upload-header-row"
            inputMode="numeric"
            autoComplete="off"
            value={view.headerField}
            aria-invalid={view.headerProblem !== null}
            aria-describedby="upload-header-row-note"
            onChange={(e) => view.setHeaderText(e.target.value)}
          />
          <p
            id="upload-header-row-note"
            className={view.headerProblem ? "text-xs text-destructive" : "text-xs text-muted-foreground"}
          >
            {view.headerProblem ?? `Detected: ${headerRowDisplay(shown.detected.headerRow)}. Counts every row from the top of the file, blank ones included, starting at 1.`}
          </p>
        </div>
      </div>

      {settled === null && view.error === null ? (
        <p role="status" className="text-xs text-muted-foreground">
          Reading the file again with these settings…
        </p>
      ) : null}
      {settled === null && view.error !== null ? (
        <div className="space-y-2">
          <Refusal error={view.error} />
          <Button type="button" variant="outline" size="sm" onClick={view.retry}>
            Retry
          </Button>
        </div>
      ) : null}

      {shown.columns.length === 0 ? (
        <p role="alert" className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-sm">
          The header row has no columns: it is past the part of the file that was read, or it is an empty
          line. Choose another header row to go on.
        </p>
      ) : (
        <PreviewTable preview={shown} stale={settled === null} />
      )}
    </div>
  )
}

function PreviewFailure({ error, onRetry }: { readonly error: ServiceError; readonly onRetry: () => void }) {
  if (error.code === "permission_denied") return <ErrorState error={error} onRetry={onRetry} />
  return (
    <div className="space-y-2">
      <Refusal error={error} />
      <Button type="button" variant="outline" size="sm" onClick={onRetry}>
        Retry
      </Button>
    </div>
  )
}

function PreviewTable({ preview, stale }: { readonly preview: UploadPreview; readonly stale: boolean }) {
  const { columns, rows } = preview
  return (
    <div className={stale ? "space-y-2 opacity-60" : "space-y-2"}>
      <div className="overflow-x-auto rounded-md border">
        <table className="w-full text-left text-sm">
          <thead className="bg-muted/40">
            <tr>
              {columns.map((name, i) => (
                <th key={i} scope="col" className="whitespace-nowrap px-3 py-1.5 font-medium">
                  {name}
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {rows.map((row, r) => (
              <tr key={r} className="border-t border-border">
                {row.map((cell, c) => (
                  <td key={c} className="whitespace-nowrap px-3 py-1.5">
                    {cell}
                  </td>
                ))}
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      <p className="text-xs text-muted-foreground">
        {rows.length === 0
          ? "There are no rows below the header row."
          : preview.truncated
            ? `Showing the first ${rows.length} rows. The file has more.`
            : `Showing all ${rows.length} rows.`}
      </p>
    </div>
  )
}
