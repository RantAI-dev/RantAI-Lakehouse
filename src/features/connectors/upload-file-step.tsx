"use client"

import * as React from "react"
import { UploadCloudIcon } from "lucide-react"
import { Button, buttonVariants } from "@/components/ui/button"
import { formatBytes } from "@/lib/format"
import { fileProblem } from "@/lib/uploads"
import { cn } from "@/lib/utils"
import type { Upload } from "@/services/contracts/uploads"
import { FileMark } from "./upload-parts"

const INPUT_ID = "upload-file-input"

/** What the picker suggests. `accept` only steers it; drag and drop is not limited by it, `fileProblem` decides. */
const ACCEPT =
  ".csv,.tsv,.txt,.xls,.xlsx,.parquet,text/csv,text/tab-separated-values,text/plain,application/vnd.ms-excel,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,application/vnd.apache.parquet"

/** The limits as short facts. They repeat what the API and `fileProblem` enforce; the sheet fact repeats the Check step. */
const FACTS: readonly { label: string; value: string }[] = [
  { label: "Accepted", value: "CSV, TSV, .xls, .xlsx, .parquet" },
  { label: "Size", value: "Up to 50 MB" },
  { label: "Not accepted", value: ".xlsm, .xlsb, .ods, archives" },
  { label: "Workbooks", value: "One sheet is loaded, chosen on the next step" },
  { label: "Parquet", value: "Every column is loaded as text; a file with a binary or nested column is refused" },
]

/**
 * A file as one row: icon by kind, the name (cut with its whole as the title,
 * so a long name never widens the page), its size, and the action. The text
 * block has a basis and `min-w-0`, and the action wraps below it at phone
 * width instead of crushing the name.
 */
function FileRow({
  name,
  sizeBytes,
  children,
}: {
  readonly name: string
  readonly sizeBytes: number
  readonly children: React.ReactNode
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
      <FileMark name={name} variant="tile" />
      <div className="min-w-0 flex-1 basis-40">
        <p className="truncate text-sm font-medium" title={name}>
          {name}
        </p>
        <p className="text-xs text-muted-foreground">{formatBytes(sizeBytes)}</p>
      </div>
      <div className="shrink-0">{children}</div>
    </div>
  )
}

/**
 * Step 1: a drop area around a real, labelled `<input type="file">`. The input
 * is visually hidden but stays in the tab order and is what the visible
 * "Choose a file" button labels, so the keyboard and a screen reader reach the
 * same control as a mouse; dropping a file sets the same file. The area takes
 * the focus ring while the input has focus. A file over 50 MB, or one whose
 * name says it is a workbook the API does not read (not `.xls` or `.xlsx`) or
 * another binary, is refused here with the reason, and nothing is sent.
 */
export function FileStep({
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
        <div className="rounded-xl border border-border bg-muted/20 p-4">
          <FileRow name={sent.originalFilename} sizeBytes={sent.sizeBytes}>
            <Button type="button" variant="outline" size="sm" onClick={onChooseAnother}>
              Choose a different file
            </Button>
          </FileRow>
        </div>
        <p className="text-sm text-muted-foreground">
          <span className="font-medium text-foreground">{sent.originalFilename}</span> ({formatBytes(sent.sizeBytes)}) is
          stored. Press Next to check how it is read.
        </p>
      </div>
    )
  }

  return (
    <div className="space-y-4">
      <div
        data-dragging={dragging ? "true" : "false"}
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={(e) => {
          // Moving over a child of the area also fires `dragleave` on it.
          if (e.relatedTarget instanceof Node && e.currentTarget.contains(e.relatedTarget)) return
          setDragging(false)
        }}
        onDrop={(e) => {
          e.preventDefault()
          setDragging(false)
          // Dragging text or a link carries no file; it must not clear the file already chosen.
          if (e.dataTransfer.files.length > 0) take(e.dataTransfer.files)
        }}
        className={cn(
          "relative rounded-xl border border-dashed p-5 transition-colors sm:p-8",
          "has-[input:focus-visible]:ring-2 has-[input:focus-visible]:ring-ring/50",
          dragging ? "border-primary bg-primary/5" : "border-border bg-muted/20"
        )}
      >
        <input id={INPUT_ID} type="file" accept={ACCEPT} className="sr-only" onChange={(e) => take(e.target.files)} />
        {file ? (
          <FileRow name={file.name} sizeBytes={file.size}>
            <label htmlFor={INPUT_ID} className={cn(buttonVariants({ variant: "outline", size: "sm" }), "cursor-pointer")}>
              Choose another
            </label>
          </FileRow>
        ) : (
          <div className="flex flex-col items-center gap-3 text-center">
            <span
              className={cn(
                "grid size-12 place-items-center rounded-full border bg-background",
                dragging ? "border-primary text-primary" : "border-border text-muted-foreground"
              )}
            >
              <UploadCloudIcon className="size-6" aria-hidden />
            </span>
            <p className="text-sm font-medium">{dragging ? "Release to choose this file" : "Drop a file here, or choose one"}</p>
            <label htmlFor={INPUT_ID} className={cn(buttonVariants({ size: "sm" }), "cursor-pointer")}>
              Choose a file
            </label>
          </div>
        )}
      </div>

      <dl className="grid gap-x-4 gap-y-1.5 text-sm sm:grid-cols-[7rem_minmax(0,1fr)]">
        {FACTS.map((fact) => (
          <div key={fact.label} className="contents">
            <dt className="text-xs font-medium text-muted-foreground sm:pt-0.5">{fact.label}</dt>
            <dd className="mb-1.5 min-w-0 break-words sm:mb-0">{fact.value}</dd>
          </div>
        ))}
      </dl>

      {several ? <p className="text-sm text-muted-foreground">One file at a time: the first one was taken.</p> : null}
      {problem ? (
        <p
          role="alert"
          className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
        >
          {problem}
        </p>
      ) : null}
      {refusal}
    </div>
  )
}
