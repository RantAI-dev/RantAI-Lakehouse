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
import { Refusal, UploadNotice } from "./upload-parts"
import { UploadPreviewGrid } from "./upload-preview-grid"

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
 *
 * For an Excel workbook the preview carries `workbook`, the person chooses a
 * sheet instead of an encoding and a delimiter, and a change of sheet drops
 * the header row chosen for the previous one (another sheet has its own). For
 * a Parquet file the preview carries `parquet` and nothing here is the
 * person's to choose (see `UploadCheckStep`).
 */
export function useUploadPreview(uploadId: string | null) {
  const [overrides, setOverrides] = React.useState<Partial<UploadParseOptions>>({})
  const [headerText, setHeaderText] = React.useState<string | null>(null)
  const key = `${uploadId ?? ""}|${overrides.encoding ?? ""}|${JSON.stringify(overrides.delimiter ?? "")}|${overrides.headerRow ?? ""}|${JSON.stringify(overrides.sheet ?? "")}`

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
      sheet: overrides.sheet ?? shown?.workbook?.sheet ?? "",
    },
    setEncoding: (encoding: UploadParseOptions["encoding"]) => setOverrides((o) => ({ ...o, encoding })),
    setDelimiter: (delimiter: string) => setOverrides((o) => ({ ...o, delimiter })),
    setSheet: (sheet: string) => {
      setOverrides((o) => ({ ...o, sheet, headerRow: undefined }))
      setHeaderText(null)
    },
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

  const workbook = shown.workbook
  const parquet = shown.parquet
  const encodings = UPLOAD_ENCODINGS.some((e) => e.value === view.values.encoding)
    ? UPLOAD_ENCODINGS
    : [...UPLOAD_ENCODINGS, { value: view.values.encoding as "utf-8", label: encodingLabel(view.values.encoding) }]
  const delimiters = UPLOAD_DELIMITERS.some((d) => d.value === view.values.delimiter)
    ? UPLOAD_DELIMITERS
    : [...UPLOAD_DELIMITERS, { value: view.values.delimiter, label: delimiterLabel(view.values.delimiter) }]

  return (
    <div className="space-y-4">
      {parquet ? (
        // A Parquet file has one table and its own column names, so there is
        // no sheet to pick, no encoding or delimiter to get wrong and no
        // header row to find: the three controls would be decoration.
        <UploadNotice>
          This is a Parquet file, so there is nothing to choose here: its column names are the header, and every
          column is loaded as text. Each column shows the type the file declares for it. Numbers are written
          without a display format (1234.5), a decimal keeps its exact digits (12.50), dates are written as
          2025-09-24, and a timestamp as 2025-09-24T13:30:00, with a trailing Z when the column is in UTC.
        </UploadNotice>
      ) : (
      <div className={workbook ? "grid gap-4 sm:grid-cols-2" : "grid gap-4 sm:grid-cols-3"}>
        {workbook ? (
          <div className="space-y-1.5">
            <Label htmlFor="upload-sheet">Sheet</Label>
            <select
              id="upload-sheet"
              className={SELECT_CLASS}
              value={view.values.sheet}
              onChange={(e) => view.setSheet(e.target.value)}
            >
              {workbook.sheets.map((sheet) => (
                <option key={sheet.name} value={sheet.name}>
                  {sheet.visible ? sheet.name : `${sheet.name} (hidden)`}
                </option>
              ))}
            </select>
            <p className="text-xs text-muted-foreground">First visible sheet to begin with. Only the sheet chosen is loaded.</p>
          </div>
        ) : (
          <>
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
          </>
        )}
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
            {view.headerProblem ??
              `Detected: ${headerRowDisplay(shown.detected.headerRow)}. ${
                workbook
                  ? "Counted from the sheet's first filled cell, blank rows included."
                  : "Counted from the top of the file, blank rows included."
              }`}
          </p>
        </div>
      </div>
      )}
      {workbook ? (
        <UploadNotice>
          Every column is loaded as text. Numbers are written as the cell holds them, without its display format
          (1234.5, not 1,234.50). Dates are written as 2025-09-24, with the time (2025-09-24 13:30:00) when the
          cell has one. A formula loads the result the file stored, and a merged cell only its top-left value.
        </UploadNotice>
      ) : null}

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
          {parquet
            ? "This file has no columns."
            : "The header row has no columns: it is past the part of the file that was read, or it is an empty line. Choose another header row to go on."}
        </p>
      ) : (
        <UploadPreviewGrid preview={shown} stale={settled === null} />
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
