"use client"

import * as React from "react"
import Link from "next/link"
import { usePathname, useSearchParams } from "next/navigation"
import { FormStepLayout, type FormStep } from "@/components/patterns/form-step-layout"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { folderOfTable } from "@/lib/upload-folders"
import { isUploadedTable, suggestTableName, tableNameProblem } from "@/lib/uploads"
import { uploadService } from "@/services"
import type { IngestUploadInput, Upload, UploadFolder, UploadLoadMode } from "@/services/contracts/uploads"
import { UploadCheckStep, useUploadPreview } from "./upload-check-step"
import { UploadFolderField } from "./upload-folder-field"
import { FileStep } from "./upload-file-step"
import { DuplicateNotice, LoadFailure } from "./upload-parts"
import { UploadReviewStep } from "./upload-review-step"
import { UploadRunView } from "./upload-run-view"
import { UploadTableStep } from "./upload-table-step"
import { useColumnChoices } from "./use-column-choices"
import { useUploadGroup } from "./use-upload-group"
import { useUploadQueue } from "./use-upload-queue"
import { columnSpecOf } from "@/lib/uploads"

const STEPS: FormStep[] = [
  { id: "file", label: "File", description: "Choose CSV, TSV, JSON, Avro, Excel or Parquet files" },
  { id: "check", label: "Check", description: "How the file is read" },
  { id: "table", label: "Table", description: "Name the raw table" },
  { id: "review", label: "Review", description: "Load the files" },
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
 * The upload page, `/connectors/upload`: four steps (File, Check, Table,
 * Review), then the result of the load. Plan T11 of
 * `docs/superpowers/plans/2026-10-02-upload-file.md`; the labels are those of
 * the checklist in `docs/core/features/upload-file.md`.
 *
 * With `?id=` (the list's "Load", or a refresh) the upload exists already, so
 * the first step is skipped. Everything the API refuses is shown where the
 * person is, in the API's own sentence.
 *
 * Files are sent in pieces from the moment they are picked (`DATA-17`, the
 * upload queue), up to ten at a time. The next steps show the first file that
 * arrives; the others load WITH it into the one table when their columns match
 * (`DATA-17` slice 5, decision 4): the Check step lists each with the comparison,
 * Load stays off until a file whose columns differ is removed, and Review and the
 * run view say it is one load of N files.
 */
function UploadWizard({ initial, pollMs }: { readonly initial: Upload | null; readonly pollMs?: number }) {
  const pathname = usePathname()
  const [upload, setUpload] = React.useState<Upload | null>(initial)
  const [duplicate, setDuplicate] = React.useState<Upload | null>(null)
  const [leftOut, setLeftOut] = React.useState(0)
  const [step, setStep] = React.useState(initial ? CHECK_STEP : FILE_STEP)
  const [run, setRun] = React.useState<Upload | null>(initial?.status === "ingesting" ? initial : null)
  const [tableName, setTableName] = React.useState(
    () => initial?.bronzeTable ?? (initial ? suggestTableName(initial.originalFilename) : "")
  )
  const [mode, setMode] = React.useState<UploadLoadMode>("replace")
  // The folder the person picked (`DATA-35`): `null` is "no folder" on purpose, `undefined` is
  // "not touched", when the table's own folder applies.
  const [folderChoice, setFolderChoice] = React.useState<string | null | undefined>(undefined)
  // Folders made on this step, shown before the list is read again.
  const [madeFolders, setMadeFolders] = React.useState<readonly UploadFolder[]>([])
  // The files a started load took with the first, kept for "Try again" and the run view.
  const [together, setTogether] = React.useState<readonly { id: string; name: string }[] | null>(null)

  const view = useUploadPreview(run === null && step !== FILE_STEP ? (upload?.id ?? null) : null)
  const choices = useColumnChoices(run === null && step !== FILE_STEP ? (upload?.id ?? null) : null, view)
  // The tenant's uploads, to know whether the table named was created by one
  // (only then is there a choice to make). If the list cannot be read, no
  // choice is offered and the load replaces, as it does for a new table.
  const uploads = useService((s) => uploadService.list(s), [])
  // The tenant's folders for the picker. If they cannot be read, no picker is offered and the load
  // names no folder.
  const folderList = useService((s) => uploadService.listFolders(s), [])
  const queue = useUploadQueue()
  // The file the next steps carry on with: the first one that was not refused.
  const primary = queue.files.find((f) => f.state !== "refused") ?? null
  const ingest = useServiceAction((signal, id: string, input: IngestUploadInput) =>
    uploadService.ingest(id, input, signal)
  )

  // The files after the first that are part of this load: not refused, and not the first itself.
  const others = queue.files.filter((f) => f.state !== "refused" && f.key !== primary?.key && f.result?.id !== upload?.id)
  const group = useUploadGroup({
    primaryId: run === null && step !== FILE_STEP ? (upload?.id ?? null) : null,
    others,
    reading: view.settled?.using ?? null,
    sheet: view.settled?.workbook?.sheet,
    columns: choices.columns,
    columnsValid: choices.problems.length === 0,
  })

  const offersMode = uploads.status === "success" && isUploadedTable(tableName, uploads.data)
  const effectiveMode: UploadLoadMode = offersMode ? mode : "replace"
  const nameProblem = tableName === "" ? null : tableNameProblem(tableName)
  // `DATA-35` decision 2: a table an earlier upload loaded offers the folder that upload sits in.
  const tableFolder = offersMode && uploads.status === "success" ? folderOfTable(tableName, uploads.data) : undefined
  // The folder the picker shows: the one the person chose, or the table's own folder
  // while the picker is untouched. `undefined` is "no folder of the table's either".
  const folderId = folderChoice !== undefined ? folderChoice : tableFolder
  const folders =
    folderList.status === "success"
      ? [...folderList.data, ...madeFolders.filter((made) => !folderList.data.some((f) => f.id === made.id))]
      : null
  // Adding to a table that exists: its names and types are the columns' (`DATA-18` decision 9).
  const { setTable } = view
  React.useEffect(() => {
    setTable(offersMode && effectiveMode === "append" ? tableName : null)
  }, [offersMode, effectiveMode, tableName, setTable])

  function reset() {
    setUpload(null)
    setDuplicate(null)
    setLeftOut(0)
    queue.clear()
    setStep(FILE_STEP)
    setRun(null)
    setTableName("")
    setMode("replace")
    setFolderChoice(undefined)
    setTogether(null)
    view.reset()
    ingest.reset()
    setAddress(pathname, null)
  }

  function changeStep(next: number) {
    if (step === FILE_STEP && next > FILE_STEP && upload === null) {
      // Only a file the API has assembled and accepted can be checked.
      if (primary === null || primary.result === null) return
      const { duplicateOf, ...stored } = primary.result
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
      // `DATA-18`: the columns as chosen, with what to do with a value that does not fit.
      ...(choices.columns.length > 0 ? { columns: choices.columns.map(columnSpecOf), onBadValue: choices.onBadValue } : {}),
      // `DATA-17` slice 5: the other files of the load. The API compares their columns again.
      ...(group.ids.length > 0 ? { alsoUploads: group.ids } : {}),
      // `DATA-35`: the folder the Table step shows is the one the load sends, `null`
      // included — the API reads `null` as "no folder" and an absent field as "the
      // table's remembered folder" (`PR #259 review BLOCKER 1`). A picker that could
      // not be read sends no folder at all, so the table's remembered folder applies.
      ...(folders !== null ? { folderId: folderId ?? null } : {}),
    })
    if (started !== null) {
      setTogether(group.files.filter((f) => f.uploadId !== null).map((f) => ({ id: f.uploadId ?? "", name: f.name })))
      setRun(started.upload)
    }
  }

  if (run !== null) {
    // A page opened with `?id=` on a load of several files learns them from the list: the files
    // that were loaded with this one point at it (`loadedWith`).
    const loadedWith =
      together ??
      (uploads.status === "success"
        ? uploads.data.filter((u) => u.loadedWith === run.id).map((u) => ({ id: u.id, name: u.originalFilename }))
        : [])
    return (
      <UploadRunView
        initial={run}
        together={loadedWith}
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
      ? upload !== null || primary?.state === "received"
      : step === CHECK_STEP
        ? view.ready && choices.ready && group.ready
        : step === TABLE_STEP
          ? tableName !== "" && nameProblem === null
          : view.ready && choices.ready && group.ready && tableName !== "" && nameProblem === null

  const notices =
    upload !== null && step !== FILE_STEP ? (
      <div className="space-y-3">
        {duplicate !== null ? <DuplicateNotice earlier={duplicate} /> : null}
        {group.pending > 0 ? (
          <p className="rounded-lg border border-border bg-muted/30 px-3 py-2 text-sm">
            Keep this page open until {group.pending === 1 ? "the other file arrives" : "the other files arrive"}: they
            load with the first.
          </p>
        ) : null}
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
      // Below `lg` the pattern's grid has one implicit `auto` track, whose
      // minimum is the widest content's min-content (a nowrap preview table, a
      // long file name), so the page scrolled sideways at phone width. A
      // `minmax(0, 1fr)` track lets the content shrink and scroll in its own
      // frame; from `lg` the pattern's own two columns apply.
      className="grid-cols-[minmax(0,1fr)]"
      currentIndex={step}
      onStepChange={changeStep}
      canProceed={canProceed}
      onSubmit={() => void load()}
      submitLabel="Load"
      submitting={ingest.status === "pending"}
      submittingLabel="Starting…"
    >
      {notices}
      {step === FILE_STEP ? (
        <FileStep
          files={queue.files}
          onPick={(picked) => setLeftOut(queue.add(picked))}
          onRemove={queue.remove}
          leftOut={leftOut}
          sent={upload}
          onChooseAnother={reset}
        />
      ) : null}
      {step === CHECK_STEP && upload !== null ? (
        <UploadCheckStep view={view} choices={choices} first={upload} group={group} onRemoveFile={queue.remove} />
      ) : null}
      {step === TABLE_STEP ? (
        <UploadTableStep
          tableName={tableName}
          onTableName={setTableName}
          offersMode={offersMode}
          mode={mode}
          onMode={setMode}
          folderField={
            folders !== null ? (
              <UploadFolderField
                folders={folders}
                value={folderId ?? null}
                onChange={setFolderChoice}
                onCreated={(made) => setMadeFolders((prev) => [...prev, made])}
              />
            ) : null
          }
        />
      ) : null}
      {step === STEPS.length - 1 && upload !== null ? (
        <UploadReviewStep
          upload={upload}
          view={view}
          tableName={tableName}
          offersMode={offersMode}
          mode={effectiveMode}
          columns={choices.columns}
          onBadValue={choices.onBadValue}
          error={ingest.error}
          otherFiles={group.files.map((f) => f.name)}
        />
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
        description="Bring CSV, TSV, JSON, Avro, Excel or Parquet files, or a gzip or zip of them, in as raw tables. Files with the same columns can load into one table together. Check how the first one is read, name the table, then load."
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
