"use client"

import * as React from "react"
import { UploadCloudIcon, XIcon } from "lucide-react"
import { Button, buttonVariants } from "@/components/ui/button"
import { formatBytes } from "@/lib/format"
import type { QueueFile } from "@/lib/upload-queue"
import { MAX_FILES } from "@/lib/uploads"
import { cn } from "@/lib/utils"
import type { Upload } from "@/services/contracts/uploads"
import { FileMark } from "./upload-parts"

const INPUT_ID = "upload-file-input"

/** What the picker suggests. `accept` only steers it; drag and drop is not limited by it, `fileProblem` decides. */
const ACCEPT =
  ".csv,.tsv,.txt,.json,.jsonl,.ndjson,.avro,.gz,.zip,.xls,.xlsx,.parquet,text/csv,text/tab-separated-values,text/plain,application/json,application/gzip,application/zip,application/vnd.ms-excel,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet,application/vnd.apache.parquet"

/** The limits as short facts. They repeat what the API and `fileProblem` enforce; the sheet fact repeats the Check step. */
const FACTS: readonly { label: string; value: string }[] = [
  { label: "Accepted", value: "CSV, TSV, JSON, JSON Lines, Avro, .xls, .xlsx, .parquet, .gz and .zip" },
  { label: "Files", value: `Up to ${MAX_FILES} at once, sent in pieces; the first one is checked and loaded on the next steps` },
  { label: "Size", value: "Up to 50 MB in all" },
  { label: "Not accepted", value: ".xlsm, .xlsb, .ods, .tar, .7z and other archives" },
  { label: "Workbooks", value: "One sheet is loaded, chosen on the next step" },
  { label: "Parquet and Avro", value: "Each column starts with the type the file declares; a file with a binary column is refused" },
  { label: "JSON", value: "An array of objects (.json) or one object per line (.jsonl); the keys are the columns, and a nested value is kept as JSON text" },
  { label: "Compressed", value: "A .gz of one text or JSON file, or a .zip of up to 10 files of one format with the same columns; what it unpacks to counts toward the 50 MB" },
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

/** What a file in the list is doing, in words: the queue's trouble when it has one. */
function stateText(file: QueueFile): string {
  switch (file.state) {
    case "waiting":
      return "Waiting"
    case "sending":
      return "Sending"
    case "paused":
      return file.message ?? "Waiting for the connection"
    case "received":
      return "Received"
    case "refused":
      return "Not sent"
  }
}

/** A bar for `value` of `max`; the numbers are the ones the API acknowledged, never a guess. */
function ProgressBar({ name, value, max }: { readonly name: string; readonly value: number; readonly max: number }) {
  const percent = max > 0 ? Math.min(100, Math.round((value / max) * 100)) : 0
  return (
    <div
      role="progressbar"
      aria-label={`${name} sent`}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent}
      className="h-1.5 w-full overflow-hidden rounded-full bg-muted"
    >
      <div className="h-full rounded-full bg-primary transition-[width]" style={{ width: `${percent}%` }} />
    </div>
  )
}

/**
 * One file of the list: mark, name, size, a bar while it is on its way, its
 * state in words and a way to take it out. A file that was not sent shows why,
 * in the API's own sentence or the console's, as an alert.
 */
function QueueRow({ file, onRemove }: { readonly file: QueueFile; readonly onRemove: (key: string) => void }) {
  return (
    <li className="space-y-1.5 py-2.5" data-file-state={file.state}>
      <div className="flex flex-wrap items-center gap-x-3 gap-y-1.5">
        <FileMark name={file.name} variant="tile" />
        <div className="min-w-0 flex-1 basis-40">
          <p className="truncate text-sm font-medium" title={file.name}>
            {file.name}
          </p>
          <p className="text-xs text-muted-foreground">
            {formatBytes(file.size)} · {stateText(file)}
          </p>
        </div>
        <Button type="button" variant="ghost" size="sm" aria-label={`Remove ${file.name}`} onClick={() => onRemove(file.key)}>
          <XIcon aria-hidden />
        </Button>
      </div>
      {file.state === "refused" ? (
        <p
          role="alert"
          className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
        >
          {file.message}
        </p>
      ) : (
        <ProgressBar name={file.name} value={file.sentBytes} max={file.size} />
      )}
    </li>
  )
}

/**
 * Step 1: a drop area around a real, labelled `<input type="file" multiple>`.
 * The input is visually hidden but stays in the tab order and is what the
 * visible "Choose files" button labels, so the keyboard and a screen reader
 * reach the same control as a mouse; dropping files picks the same files. The
 * area takes the focus ring while the input has focus.
 *
 * Files start to be sent as soon as they are picked (`UploadQueue`), up to ten
 * at once, and each has a row with a bar that moves as the API acknowledges
 * its pieces. A file the console can refuse by itself (a name that is not a
 * supported kind, a size, the total) gets its row with the reason and is never
 * sent. The Check step carries on with the first file that is received; the
 * others are loaded from the list of uploaded files.
 */
export function FileStep({
  files,
  onPick,
  onRemove,
  leftOut,
  sent,
  onChooseAnother,
}: {
  readonly files: readonly QueueFile[]
  readonly onPick: (picked: File[]) => void
  readonly onRemove: (key: string) => void
  /** How many files of the last pick the full list left out. */
  readonly leftOut: number
  /** The upload already stored from this page, when there is one. */
  readonly sent: Upload | null
  readonly onChooseAnother: () => void
}) {
  const [dragging, setDragging] = React.useState(false)

  function take(list: FileList | null) {
    if (list !== null && list.length > 0) onPick(Array.from(list))
  }

  if (sent !== null) {
    const others = files.filter((f) => f.result?.id !== sent.id)
    return (
      <div className="space-y-3">
        <div className="rounded-xl border border-border bg-muted/20 p-4">
          <FileRow name={sent.originalFilename} sizeBytes={sent.sizeBytes}>
            <Button type="button" variant="outline" size="sm" onClick={onChooseAnother}>
              Choose different files
            </Button>
          </FileRow>
        </div>
        <p className="text-sm text-muted-foreground">
          <span className="font-medium text-foreground">{sent.originalFilename}</span> ({formatBytes(sent.sizeBytes)}) is
          stored. Press Next to check how it is read.
        </p>
        {others.length > 0 ? (
          <ul className="divide-y divide-border rounded-xl border border-border px-4" aria-label="Other files">
            {others.map((file) => (
              <QueueRow key={file.key} file={file} onRemove={onRemove} />
            ))}
          </ul>
        ) : null}
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
          // Dragging text or a link carries no file; it must not change the list.
          take(e.dataTransfer.files)
        }}
        className={cn(
          "relative rounded-xl border border-dashed p-5 transition-colors sm:p-8",
          "has-[input:focus-visible]:ring-2 has-[input:focus-visible]:ring-ring/50",
          dragging ? "border-primary bg-primary/5" : "border-border bg-muted/20"
        )}
      >
        <input
          id={INPUT_ID}
          type="file"
          multiple
          accept={ACCEPT}
          className="sr-only"
          onChange={(e) => {
            take(e.target.files)
            // The same file may be picked again after it was removed.
            e.target.value = ""
          }}
        />
        <div className="flex flex-col items-center gap-3 text-center">
          <span
            className={cn(
              "grid size-12 place-items-center rounded-full border bg-background",
              dragging ? "border-primary text-primary" : "border-border text-muted-foreground"
            )}
          >
            <UploadCloudIcon className="size-6" aria-hidden />
          </span>
          <p className="text-sm font-medium">
            {dragging ? "Release to choose these files" : "Drop files here, or choose some"}
          </p>
          <label htmlFor={INPUT_ID} className={cn(buttonVariants({ size: "sm" }), "cursor-pointer")}>
            Choose files
          </label>
        </div>
      </div>

      {files.length > 0 ? (
        <ul className="divide-y divide-border rounded-xl border border-border px-4" aria-label="Files">
          {files.map((file) => (
            <QueueRow key={file.key} file={file} onRemove={onRemove} />
          ))}
        </ul>
      ) : null}
      {leftOut > 0 ? (
        <p className="text-sm text-muted-foreground">
          {`Up to ${MAX_FILES} files at once: the first ones were taken and ${leftOut} ${leftOut === 1 ? "was" : "were"} left out.`}
        </p>
      ) : null}

      <dl className="grid gap-x-4 gap-y-1.5 text-sm sm:grid-cols-[7rem_minmax(0,1fr)]">
        {FACTS.map((fact) => (
          <div key={fact.label} className="contents">
            <dt className="text-xs font-medium text-muted-foreground sm:pt-0.5">{fact.label}</dt>
            <dd className="mb-1.5 min-w-0 break-words sm:mb-0">{fact.value}</dd>
          </div>
        ))}
      </dl>
    </div>
  )
}
