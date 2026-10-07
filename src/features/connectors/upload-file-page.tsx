"use client"

import * as React from "react"
import Link from "next/link"
import { usePathname, useSearchParams } from "next/navigation"
import { FormReviewSummary } from "@/components/patterns/form-review-summary"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatBytes, formatDateTime } from "@/lib/format"
import {
  TABLE_NAME_RULE,
  delimiterLabel,
  encodingLabel,
  fileProblem,
  headerRowDisplay,
  isUploadedTable,
  suggestTableName,
  tableNameProblem,
} from "@/lib/uploads"
import { uploadService } from "@/services"
import type { IngestUploadInput, Upload, UploadLoadMode } from "@/services/contracts/uploads"
import { UploadCheckStep, useUploadPreview } from "./upload-check-step"
import { DuplicateNotice, LoadFailure, Refusal } from "./upload-parts"
import { UploadRunView } from "./upload-run-view"

const STEPS: FormStep[] = [
  { id: "file", label: "File", description: "Choose a CSV, TSV or Excel file" },
  { id: "check", label: "Check", description: "How the file is read" },
  { id: "table", label: "Table", description: "Name the raw table" },
  { id: "review", label: "Review", description: "Load the file" },
]

const FILE_STEP = 0
const CHECK_STEP = 1
const TABLE_STEP = 2

/**
 * Keeps the address in step with the upload a person is working on, so a
 * refresh lands back on it (`?id=`) and not on an empty first step. The
 * History API rather than the router: no server round trip, and Next keeps
 * `useSearchParams` in step with it (as `asset-detail-tabs.tsx` does).
 */
function setAddress(pathname: string, id: string | null) {
  window.history.replaceState(null, "", id === null ? pathname : `${pathname}?id=${encodeURIComponent(id)}`)
}

/**
 * Step 1: a real, labelled file control (reachable by keyboard) and a drop
 * area that sets the same file. A file over 50 MB, or one whose name says it is
 * a workbook the API does not read (not `.xls` or `.xlsx`) or another binary,
 * is refused here with the reason, and nothing is sent.
 */
function FileStep({
  file,
  onFile,
  sent,
  onChooseAnother,
  refusal,
}: {
  readonly file: File | null
  readonly onFile: (file: File | null) => void
  /** The upload already stored from this page, when there is one. */
  readonly sent: Upload | null
  readonly onChooseAnother: () => void
  readonly refusal: React.ReactNode
}) {
  const [dragging, setDragging] = React.useState(false)
  const [several, setSeveral] = React.useState(false)
  const problem = file ? fileProblem(file.name, file.size) : null

  function take(files: FileList | null) {
    setSeveral((files?.length ?? 0) > 1)
    onFile(files && files.length > 0 ? files[0] : null)
  }

  if (sent !== null) {
    return (
      <div className="space-y-3">
        <p className="text-sm">
          <span className="font-medium">{sent.originalFilename}</span> ({formatBytes(sent.sizeBytes)}) is stored.
          Press Next to check how it is read.
        </p>
        <Button type="button" variant="outline" size="sm" onClick={onChooseAnother}>
          Choose a different file
        </Button>
      </div>
    )
  }

  return (
    <div className="space-y-3">
      <div
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault()
          setDragging(false)
          take(e.dataTransfer.files)
        }}
        className={`space-y-3 rounded-lg border border-dashed p-6 ${dragging ? "border-primary bg-primary/5" : "border-border bg-muted/20"}`}
      >
        <p className="text-sm text-muted-foreground">
          Drop a CSV, TSV or Excel file here, or choose one. Delimited text files (CSV, TSV) and Excel
          workbooks (.xls, .xlsx) up to 50 MB are accepted. Of a workbook one sheet is loaded, chosen on the
          next step. Other kinds of file (.xlsm, .xlsb, .ods, archives) are refused with the reason.
        </p>
        <div className="space-y-1.5">
          <Label htmlFor="upload-file-input">Choose a file</Label>
          {/* `accept` only steers the picker; drag and drop is not limited by it, and the checks above decide. */}
          <Input
            id="upload-file-input"
            type="file"
            accept=".csv,.tsv,.txt,.xls,.xlsx,text/csv,text/tab-separated-values,text/plain,application/vnd.ms-excel,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            onChange={(e) => take(e.target.files)}
          />
        </div>
      </div>
      {file ? (
        <p className="text-sm">
          <span className="font-medium">{file.name}</span> ({formatBytes(file.size)})
        </p>
      ) : null}
      {several ? (
        <p className="text-sm text-muted-foreground">One file at a time: the first one was taken.</p>
      ) : null}
      {problem ? (
        <p role="alert" className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          {problem}
        </p>
      ) : null}
      {refusal}
    </div>
  )
}

/**
 * The upload page, `/connectors/upload`: four steps (File, Check, Table,
 * Review), then the result of the load. Plan T11 of
 * `docs/superpowers/plans/2026-10-02-upload-file.md`; the labels are those of
 * the checklist in `docs/core/features/upload-file.md`.
 *
 * With `?id=` (the list's "Load", or a refresh) the upload exists already, so
 * the first step is skipped. Everything the API refuses is shown where the
 * person is, in the API's own sentence.
 */
function UploadWizard({ initial, pollMs }: { readonly initial: Upload | null; readonly pollMs?: number }) {
  const pathname = usePathname()
  const [upload, setUpload] = React.useState<Upload | null>(initial)
  const [duplicate, setDuplicate] = React.useState<Upload | null>(null)
  const [file, setFile] = React.useState<File | null>(null)
  const [step, setStep] = React.useState(initial ? CHECK_STEP : FILE_STEP)
  const [run, setRun] = React.useState<Upload | null>(initial?.status === "ingesting" ? initial : null)
  const [tableName, setTableName] = React.useState(
    () => initial?.bronzeTable ?? (initial ? suggestTableName(initial.originalFilename) : "")
  )
  const [mode, setMode] = React.useState<UploadLoadMode>("replace")

  const view = useUploadPreview(run === null && step !== FILE_STEP ? (upload?.id ?? null) : null)
  // The tenant's uploads, to know whether the table named was created by one
  // (only then is there a choice to make). If the list cannot be read, no
  // choice is offered and the load replaces, as it does for a new table.
  const uploads = useService((s) => uploadService.list(s), [])
  const send = useServiceAction((signal, f: File) => uploadService.create(f, signal))
  const ingest = useServiceAction((signal, id: string, input: IngestUploadInput) =>
    uploadService.ingest(id, input, signal)
  )

  const offersMode = uploads.status === "success" && isUploadedTable(tableName, uploads.data)
  const effectiveMode: UploadLoadMode = offersMode ? mode : "replace"
  const nameProblem = tableName === "" ? null : tableNameProblem(tableName)

  function reset() {
    setUpload(null)
    setDuplicate(null)
    setFile(null)
    setStep(FILE_STEP)
    setRun(null)
    setTableName("")
    setMode("replace")
    view.reset()
    send.reset()
    ingest.reset()
    setAddress(pathname, null)
  }

  async function changeStep(next: number) {
    if (step === FILE_STEP && next > FILE_STEP && upload === null) {
      if (file === null) return
      const created = await send.run(file)
      if (created === null) return
      const { duplicateOf, ...stored } = created
      setUpload(stored)
      setDuplicate(duplicateOf ?? null)
      setTableName(suggestTableName(stored.originalFilename))
      view.reset()
      setAddress(pathname, stored.id)
      setStep(CHECK_STEP)
      return
    }
    setStep(next)
  }

  async function load() {
    if (upload === null || view.settled === null) return
    const { encoding, delimiter, headerRow } = view.settled.using
    // For a workbook the sheet read by the preview on screen is the one
    // loaded; the API reads UTF-8 and commas for it whatever is sent here.
    const sheet = view.settled.workbook?.sheet
    const started = await ingest.run(upload.id, {
      encoding,
      delimiter,
      headerRow,
      ...(sheet !== undefined ? { sheet } : {}),
      bronzeTable: tableName,
      mode: effectiveMode,
    })
    if (started !== null) setRun(started.upload)
  }

  if (run !== null) {
    return (
      <UploadRunView
        initial={run}
        pollMs={pollMs}
        onReset={reset}
        onChangeSettings={(latest) => {
          setUpload(latest)
          setRun(null)
          setStep(CHECK_STEP)
        }}
      />
    )
  }

  const canProceed =
    step === FILE_STEP
      ? upload !== null || (file !== null && fileProblem(file.name, file.size) === null)
      : step === CHECK_STEP
        ? view.ready
        : step === TABLE_STEP
          ? tableName !== "" && nameProblem === null
          : view.ready && tableName !== "" && nameProblem === null

  const using = view.settled?.using
  const notices =
    upload !== null && step !== FILE_STEP ? (
      <div className="space-y-3">
        {duplicate !== null ? <DuplicateNotice earlier={duplicate} /> : null}
        {upload.status === "failed" ? (
          <div className="rounded-lg border border-border bg-muted/30 px-3 py-2 space-y-1">
            <p className="text-sm font-medium">The last load of this file failed.</p>
            <LoadFailure upload={upload} />
          </div>
        ) : null}
      </div>
    ) : null

  return (
    <FormStepLayout
      steps={STEPS}
      currentIndex={step}
      onStepChange={(next) => void changeStep(next)}
      canProceed={canProceed}
      onSubmit={() => void load()}
      submitLabel="Load"
      submitting={send.status === "pending" || ingest.status === "pending"}
      submittingLabel={send.status === "pending" ? "Sending…" : "Starting…"}
    >
      {notices}
      {step === FILE_STEP ? (
        <FileStep
          file={file}
          onFile={(next) => {
            send.reset()
            setFile(next)
          }}
          sent={upload}
          onChooseAnother={reset}
          refusal={send.error !== null ? <Refusal error={send.error} /> : null}
        />
      ) : null}
      {step === CHECK_STEP ? <UploadCheckStep view={view} /> : null}
      {step === TABLE_STEP ? (
        <div className="space-y-5">
          <div className="space-y-1.5">
            <Label htmlFor="upload-table-name">Table name</Label>
            <Input
              id="upload-table-name"
              value={tableName}
              autoComplete="off"
              spellCheck={false}
              aria-invalid={nameProblem !== null}
              aria-describedby="upload-table-name-note"
              onChange={(e) => setTableName(e.target.value)}
            />
            <p
              id="upload-table-name-note"
              className={nameProblem ? "text-xs text-destructive" : "text-xs text-muted-foreground"}
            >
              {nameProblem ?? TABLE_NAME_RULE}
            </p>
          </div>
          {offersMode ? (
            <fieldset className="space-y-2">
              <legend className="text-sm font-medium">
                {tableName} was created by an earlier upload. What should happen to its rows?
              </legend>
              {(
                [
                  { value: "replace", label: "Replace its rows" },
                  { value: "append", label: "Add to its rows" },
                ] as const
              ).map((option) => (
                <label key={option.value} className="flex items-center gap-2 text-sm">
                  <input
                    type="radio"
                    name="upload-load-mode"
                    value={option.value}
                    checked={mode === option.value}
                    onChange={() => setMode(option.value)}
                  />
                  {option.label}
                </label>
              ))}
            </fieldset>
          ) : null}
          <p className="text-sm text-muted-foreground">
            Every column is stored as text. Numbers and dates have to be converted afterwards.
          </p>
        </div>
      ) : null}
      {step === STEPS.length - 1 && upload !== null ? (
        <div className="space-y-4">
          <FormReviewSummary
            sections={[
              {
                title: "File",
                items: [
                  { label: "Name", value: upload.originalFilename },
                  { label: "Size", value: formatBytes(upload.sizeBytes) },
                  { label: "Uploaded", value: formatDateTime(upload.createdAt) },
                ],
              },
              {
                title: "How it is read",
                items: [
                  ...(view.settled?.workbook
                    ? [{ label: "Sheet", value: view.settled.workbook.sheet }]
                    : [
                        { label: "Encoding", value: using ? encodingLabel(using.encoding) : "" },
                        { label: "Delimiter", value: using ? delimiterLabel(using.delimiter) : "" },
                      ]),
                  { label: "Header row", value: using ? String(headerRowDisplay(using.headerRow)) : "" },
                  { label: "Columns", value: view.settled ? String(view.settled.columns.length) : "" },
                ],
              },
              {
                title: "Table",
                items: [
                  { label: "Table name", value: tableName },
                  ...(offersMode
                    ? [
                        {
                          label: "Rows already in it",
                          value: effectiveMode === "append" ? "Add to its rows" : "Replace its rows",
                        },
                      ]
                    : []),
                  { label: "Column types", value: "Text" },
                ],
              },
            ]}
          />
          {ingest.error !== null ? <Refusal error={ingest.error} /> : null}
        </div>
      ) : null}
    </FormStepLayout>
  )
}

/** `?id=`: reads the upload first. A refusal (not found, no permission) goes through the existing error state. */
function ExistingUpload({ id, pollMs }: { readonly id: string; readonly pollMs?: number }) {
  const state = useService((s) => uploadService.get(id, s), [id])
  if (state.status === "loading") return <LoadingSkeleton rows={3} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  return <UploadWizard initial={state.data} pollMs={pollMs} />
}

export function UploadFilePage({ pollMs }: { readonly pollMs?: number }) {
  const searchParams = useSearchParams()
  // Read once: the page writes `?id=` itself once a file is stored, and that
  // must not turn a wizard that is working into one that starts over.
  const [initialId] = React.useState(() => searchParams.get("id"))

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Upload file"
        description="Bring a CSV or TSV file in as a raw table. Check how it is read, name the table, then load it."
        actions={
          <Button variant="outline" size="sm" render={<Link href="/connectors" />}>
            Cancel
          </Button>
        }
      />
      {initialId !== null && initialId !== "" ? (
        <ExistingUpload id={initialId} pollMs={pollMs} />
      ) : (
        <UploadWizard initial={null} pollMs={pollMs} />
      )}
    </div>
  )
}
