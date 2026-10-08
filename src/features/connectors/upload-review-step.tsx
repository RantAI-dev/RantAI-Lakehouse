"use client"

import * as React from "react"
import { MetadataList, type MetadataItem } from "@/components/patterns/metadata-list"
import { formatBytes, formatDateTime } from "@/lib/format"
import { delimiterLabel, encodingLabel, headerRowDisplay, rawTableFullName } from "@/lib/uploads"
import type { Upload, UploadLoadMode } from "@/services/contracts/uploads"
import type { UploadPreviewView } from "./upload-check-step"
import { Refusal } from "./upload-parts"

function Group({ title, items }: { readonly title: string; readonly items: MetadataItem[] }) {
  return (
    <section className="space-y-3 p-4">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-muted-foreground">{title}</h3>
      <MetadataList items={items} />
    </section>
  )
}

/** A value that may be long and unbroken (a file or table name): it wraps inside its cell, never widens it. */
function Wrapped({ children, mono = false }: { readonly children: React.ReactNode; readonly mono?: boolean }) {
  return <span className={mono ? "break-all font-mono text-[13px]" : "[overflow-wrap:anywhere]"}>{children}</span>
}

/**
 * Step 4: the three groups (the file, how it is read, the table) as one
 * framed summary with one label size and one value size, then one sentence
 * saying what "Load" does with exactly these settings, then the API's refusal
 * of the load when there is one. The sentence only repeats what is chosen on
 * the earlier steps; the load itself (replace or add, text columns) is the
 * API's, as the Table step already says.
 */
export function UploadReviewStep({
  upload,
  view,
  tableName,
  offersMode,
  mode,
  error,
}: {
  readonly upload: Upload
  readonly view: UploadPreviewView
  readonly tableName: string
  /** Whether the table was created by an earlier upload, so the mode is the person's choice. */
  readonly offersMode: boolean
  /** The mode that will be sent (`replace` unless the choice was offered and made). */
  readonly mode: UploadLoadMode
  readonly error: React.ComponentProps<typeof Refusal>["error"] | null
}) {
  const { settled } = view
  const using = settled?.using
  const full = rawTableFullName(tableName)
  const rowsSentence =
    mode === "append"
      ? "The rows already in the table stay, and this file's are added after them."
      : "Any rows the table already holds are replaced by this file's."

  return (
    <div className="space-y-4">
      <div className="divide-y divide-border overflow-hidden rounded-lg border border-border bg-muted/30">
        <Group
          title="File"
          items={[
            { label: "Name", value: <Wrapped>{upload.originalFilename}</Wrapped> },
            { label: "Size", value: formatBytes(upload.sizeBytes) },
            { label: "Uploaded", value: formatDateTime(upload.createdAt) },
          ]}
        />
        <Group
          title="How it is read"
          items={[
            ...(settled?.workbook
              ? [{ label: "Sheet", value: <Wrapped>{settled.workbook.sheet}</Wrapped> }]
              : [
                  { label: "Encoding", value: using ? encodingLabel(using.encoding) : "—" },
                  { label: "Delimiter", value: using ? delimiterLabel(using.delimiter) : "—" },
                ]),
            { label: "Header row", value: using ? String(headerRowDisplay(using.headerRow)) : "—" },
            { label: "Columns", value: settled ? String(settled.columns.length) : "—" },
          ]}
        />
        <Group
          title="Table"
          items={[
            { label: "Table", value: <Wrapped mono>{full}</Wrapped> },
            ...(offersMode
              ? [{ label: "Rows already in it", value: mode === "append" ? "Add to its rows" : "Replace its rows" }]
              : []),
            { label: "Column types", value: "Text" },
          ]}
        />
      </div>
      <p className="text-sm text-muted-foreground">
        Load reads <span className="break-words font-medium text-foreground">{upload.originalFilename}</span> with these
        settings and writes its rows to <span className="break-all font-mono text-foreground">{full}</span>, every column as
        text. {rowsSentence} You can follow it on the next page.
      </p>
      {error !== null ? <Refusal error={error} /> : null}
    </div>
  )
}
