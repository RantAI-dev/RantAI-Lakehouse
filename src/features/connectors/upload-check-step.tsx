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
  archiveProblem,
  delimiterLabel,
  encodingLabel,
  hasFixedDialect,
  headerRowDisplay,
  headerRowFromDisplay,
} from "@/lib/uploads"
import { formatBytes } from "@/lib/format"
import { uploadService } from "@/services"
import type { Upload, UploadArchive, UploadParseOptions, UploadPreview } from "@/services/contracts/uploads"
import type { ServiceError } from "@/services/errors"
import { UploadFilesList } from "./upload-files-list"
import { Refusal, UploadNotice } from "./upload-parts"
import { UploadPreviewGrid } from "./upload-preview-grid"
import { UploadBadValuesPanel, UploadColumnsEditor } from "./upload-columns"
import type { ColumnChoices } from "./use-column-choices"
import type { UploadGroupView } from "./use-upload-group"

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
  // A table the file will be added to: its names and types become the columns' (`DATA-18` decision 9).
  const [table, setTable] = React.useState<string | null>(null)
  const key = `${uploadId ?? ""}|${table ?? ""}|${overrides.encoding ?? ""}|${JSON.stringify(overrides.delimiter ?? "")}|${overrides.headerRow ?? ""}|${JSON.stringify(overrides.sheet ?? "")}`

  const state = useRefreshable<Settled>(async (signal) => {
    if (uploadId === null) return { key, preview: null }
    return {
      key,
      preview: await uploadService.preview(uploadId, table === null ? overrides : { ...overrides, table }, signal),
    }
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
    setTable,
    reset: () => {
      setOverrides({})
      setHeaderText(null)
      setTable(null)
    },
    /**
     * Only a preview of the settings in force, with at least one column, lets the person
     * go on; and a zip whose files do not all have the first file's columns does not load.
     */
    ready: settled !== null && settled.columns.length > 0 && headerProblem === null && archiveProblem(settled.archive) === null,
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
 *
 * With files after the first (`DATA-17` slice 5) the step also lists them, each with the mark of
 * the comparison or the columns that differ, and a way to take it out (`UploadFilesList`); the
 * preview and the column list stay the first file's.
 */
export function UploadCheckStep({
  view,
  choices,
  first,
  group,
  onRemoveFile,
}: {
  readonly view: UploadPreviewView
  readonly choices: ColumnChoices
  /** The first file of the load, whose preview this is. */
  readonly first: Upload
  readonly group: UploadGroupView
  readonly onRemoveFile: (key: string) => void
}) {
  const { shown, settled } = view
  // SEC-17: a refusal of the reading itself (400, such as "The file has more
  // than 1,000 columns.") with nothing shown yet keeps the controls on screen,
  // because a wrong delimiter guess is what the person must be able to
  // change. Any other failure with nothing to show has no controls to offer.
  const refusedReading = shown === null && view.error !== null && view.error.code === "invalid_request"
  if (shown === null && !refusedReading) {
    if (view.error !== null) return <PreviewFailure error={view.error} onRetry={view.retry} />
    return <LoadingSkeleton rows={3} />
  }

  const workbook = shown?.workbook
  const parquet = shown?.parquet
  const layout = shown?.layout
  // JSON, JSON Lines and Avro: the keys or fields are the header, as the footer is for Parquet.
  const fixedDialect = parquet !== undefined || hasFixedDialect(layout)
  // SEC-17: with no preview yet, the first option says the service detects
  // the value, and it is not one to choose.
  const undecided = { value: "", label: "Detected by the service", disabled: true }
  const encodings =
    shown === null
      ? [undecided, ...UPLOAD_ENCODINGS]
      : UPLOAD_ENCODINGS.some((e) => e.value === view.values.encoding)
        ? UPLOAD_ENCODINGS
        : [...UPLOAD_ENCODINGS, { value: view.values.encoding as "utf-8", label: encodingLabel(view.values.encoding) }]
  const delimiters =
    shown === null
      ? [undecided, ...UPLOAD_DELIMITERS]
      : UPLOAD_DELIMITERS.some((d) => d.value === view.values.delimiter)
        ? UPLOAD_DELIMITERS
        : [...UPLOAD_DELIMITERS, { value: view.values.delimiter, label: delimiterLabel(view.values.delimiter) }]

  return (
    <div className="space-y-4">
      <UploadFilesList first={first} group={group} onRemove={onRemoveFile} />
      {parquet ? (
        // A Parquet file has one table and its own column names, so there is
        // no sheet to pick, no encoding or delimiter to get wrong and no
        // header row to find: the three controls would be decoration.
        <UploadNotice>
          This is a Parquet file, so there is nothing to read here: its column names are the header, and each
          column starts with the type the file declares for it. You can change a name or a type below.
        </UploadNotice>
      ) : hasFixedDialect(layout) ? (
        // `DATA-17` slice 4: the same for JSON, JSON Lines and Avro.
        <UploadNotice>
          {layout?.format === "avro"
            ? "This is an Avro file, so there is nothing to read here: its field names are the header, and each column starts with the type the schema declares for it."
            : `This is ${layout?.format === "jsonl" ? "a JSON Lines" : "a JSON"} file, so there is nothing to read here: the keys of the objects are the header, a key an object lacks is left empty, and each column starts with a type guessed from its values.`}{" "}
          {layout?.format === "avro"
            ? "A record, array or map is kept as JSON text in one column. You can change a name or a type below."
            : "A value that is an object or a list is kept as JSON text in one column. You can change a name or a type below."}
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
                <option key={e.value} value={e.value} disabled={e.disabled}>
                  {e.label}
                </option>
              ))}
            </select>
            {shown !== null ? <Detected text={encodingLabel(shown.detected.encoding)} /> : null}
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
                <option key={d.value} value={d.value} disabled={d.disabled}>
                  {d.label}
                </option>
              ))}
            </select>
            {shown !== null ? <Detected text={delimiterLabel(shown.detected.delimiter)} /> : null}
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
              (shown === null
                ? "Counts every row from the top of the file, blank ones included, starting at 1."
                : `Detected: ${headerRowDisplay(shown.detected.headerRow)}. ${
                    workbook
                      ? "Counted from the sheet's first filled cell, blank rows included."
                      : "Counted from the top of the file, blank rows included."
                  }`)}
          </p>
        </div>
      </div>
      )}
      {layout?.compression === "gzip" ? (
        <UploadNotice>
          This file is compressed with gzip. It is read as it is once unpacked, and what it unpacks to counts toward
          the 2 GB limit.
        </UploadNotice>
      ) : null}
      {shown?.archive ? <UploadArchiveList files={shown.archive.files} problem={archiveProblem(shown.archive)} /> : null}
      {workbook ? (
        <UploadNotice>
          Numbers are read as the cell holds them, without its display format (1234.5, not 1,234.50), and dates as
          2025-09-24, with the time (2025-09-24 13:30:00) when the cell has one. A formula loads the result the file
          stored, and a merged cell only its top-left value.
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

      {shown === null ? null : shown.columns.length === 0 ? (
        <p role="alert" className="rounded-lg border border-border bg-muted/40 px-3 py-2 text-sm">
          {fixedDialect
            ? "This file has no columns."
            : "The header row has no columns: it is past the part of the file that was read, or it is an empty line. Choose another header row to go on."}
        </p>
      ) : (
        <>
          <UploadPreviewGrid preview={shown} stale={settled === null} />
          {settled !== null && choices.columns.length > 0 ? (
            <>
              <UploadColumnsEditor choices={choices} sampledRows={settled.sampledRows} />
              <UploadBadValuesPanel choices={choices} sampledRows={settled.sampledRows} />
            </>
          ) : null}
        </>
      )}
    </div>
  )
}

/**
 * The files of a zip as rows (`DATA-17` slice 4): name, size once unpacked and how many columns
 * each has. The first one is what the preview and the column list below show; a file whose
 * columns differ from it is marked, and Next stays off until the zip is uploaded again.
 */
function UploadArchiveList({
  files,
  problem,
}: {
  readonly files: UploadArchive["files"]
  readonly problem: string | null
}) {
  return (
    <div className="space-y-2">
      <ul aria-label="Files in the zip" className="divide-y divide-border overflow-hidden rounded-lg border border-border text-sm">
        {files.map((file, index) => (
          <li key={`${index}-${file.name}`} className="flex flex-wrap items-center gap-x-3 gap-y-1 px-3 py-2">
            <span className="min-w-0 flex-1 basis-40 truncate font-medium" title={file.name}>
              {file.name}
            </span>
            <span className="text-xs text-muted-foreground">
              {formatBytes(file.sizeBytes)} · {file.columns} {file.columns === 1 ? "column" : "columns"}
            </span>
            {file.matchesFirst ? (
              index === 0 ? <span className="text-xs text-muted-foreground">Shown below</span> : null
            ) : (
              <span className="text-xs font-medium text-destructive">Different columns</span>
            )}
          </li>
        ))}
      </ul>
      {problem !== null ? (
        <p role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          {problem}
        </p>
      ) : null}
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
