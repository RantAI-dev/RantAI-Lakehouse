"use client"

import type { LucideIcon } from "lucide-react"
import { FileIcon, FileSpreadsheetIcon, FileTextIcon, InfoIcon, TriangleAlertIcon } from "lucide-react"
import { ErrorState } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { formatDateTime } from "@/lib/format"
import { REGISTRATION_FAILED_REASON, statusLabel, uploadFileKind, uploadFileLabel } from "@/lib/uploads"
import type { Upload, UploadStatus } from "@/services/contracts/uploads"
import type { ServiceError } from "@/services/errors"
import { cn } from "@/lib/utils"

/**
 * Small pieces the upload screens share: the status pill, the way a refusal
 * is shown, and the two notices (a repeated file, a failed load). Plan T11 of
 * `docs/superpowers/plans/2026-10-02-upload-file.md`.
 */

const STATUS_TONE: Record<string, "neutral" | "info" | "success" | "destructive"> = {
  uploaded: "neutral",
  ingesting: "info",
  ingested: "success",
  failed: "destructive",
}

/** The upload's status, in the words of `statusLabel`; a status this version does not know is shown as received. */
export function UploadStatusPill({ status }: { readonly status: UploadStatus }) {
  return (
    <Pill tone={STATUS_TONE[status] ?? "neutral"}>
      {status === "ingesting" ? (
        <span className="size-1.5 animate-pulse rounded-full bg-current" aria-hidden />
      ) : null}
      {statusLabel(status)}
    </Pill>
  )
}

/**
 * A refusal or failure from the API, shown in place. The sentence is the
 * API's and is written to be read (principle 4: nothing upstream reaches a
 * response), so it is shown as it is, with nothing added. A missing
 * permission is the exception: it goes through the existing `ErrorState`,
 * which says so, so a person without `connector:manage` is told why and not
 * shown a bare sentence.
 */
export function Refusal({ error }: { readonly error: ServiceError }) {
  if (error.code === "permission_denied") return <ErrorState error={error} />
  return (
    <p
      role="alert"
      className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive"
    >
      {error.message}
    </p>
  )
}

/**
 * The mark beside a file name, by the format its name says: a spreadsheet
 * glyph in green for an Excel workbook, a text glyph in the primary tone for
 * CSV and TSV, the neutral document glyph for anything else, each with the
 * extension in capitals. A generic glyph, never a vendor's logo (see
 * `src/lib/connectors/brand-marks.ts`).
 *
 * Decoration only: the name beside it already shows the extension, so the
 * whole mark is `aria-hidden`. The chip is a fixed size (`tile`: the square
 * of a file row; `inline`: one text line high) and `shrink-0`, so it never
 * changes the row's height or takes room from the name.
 *
 * Contrast, by numbers: the workbook tone is `emerald-700` on the chip's
 * `emerald-500/12` in light (about 4.6:1) and `emerald-400` in dark (about
 * 6.2:1 to 7.5:1). `emerald-600`, the Pill's text tone, is 3.3:1 there, too
 * low for the 10px label.
 */
const MARK_TONE = {
  workbook: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
  delimited: "bg-primary/10 text-primary",
  other: "bg-muted text-muted-foreground",
} as const

export function FileMark({ name, variant }: { readonly name: string; readonly variant: "tile" | "inline" }) {
  const kind = uploadFileKind(name)
  const label = uploadFileLabel(name)
  const Icon = kind === "workbook" ? FileSpreadsheetIcon : kind === "delimited" ? FileTextIcon : FileIcon
  return (
    <span
      aria-hidden
      data-file-mark={kind}
      className={cn(
        "shrink-0 rounded-lg",
        MARK_TONE[kind],
        variant === "tile"
          ? "flex size-10 flex-col items-center justify-center gap-0.5"
          : "inline-flex h-5 items-center gap-1 rounded-md px-1.5"
      )}
    >
      <Icon className={variant === "tile" ? "size-4" : "size-3.5"} />
      {label !== null ? (
        <span className={cn("font-semibold leading-none tracking-wide", variant === "tile" ? "text-[9px]" : "text-[10px]")}>
          {label}
        </span>
      ) : null}
    </span>
  )
}

const NOTICE_TONE = {
  info: { box: "border-border bg-muted/30", icon: "text-muted-foreground" },
  warning: { box: "border-amber-500/30 bg-amber-500/10", icon: "text-amber-600 dark:text-amber-400" },
} as const

/**
 * A small bordered notice in the style of the connector page's test notice:
 * an icon, then the sentence, which wraps inside the box (`min-w-0`) so a
 * long file or table name never widens the page.
 */
export function UploadNotice({
  tone = "info",
  icon,
  children,
}: {
  readonly tone?: keyof typeof NOTICE_TONE
  readonly icon?: LucideIcon
  readonly children: React.ReactNode
}) {
  const Icon = icon ?? (tone === "warning" ? TriangleAlertIcon : InfoIcon)
  const { box, icon: accent } = NOTICE_TONE[tone]
  return (
    <div role="status" className={cn("flex items-start gap-2 rounded-lg border px-3 py-2", box)}>
      <Icon className={cn("mt-0.5 size-4 shrink-0", accent)} aria-hidden />
      <p className="min-w-0 break-words text-sm text-foreground">{children}</p>
    </div>
  )
}

/** The same file arrived before: its name and date, and what loading it again does. */
export function DuplicateNotice({ earlier }: { readonly earlier: Upload }) {
  return (
    <UploadNotice tone="warning">
      This file was uploaded before, as {earlier.originalFilename} on {formatDateTime(earlier.createdAt)}.
      {earlier.bronzeTable !== undefined ? ` It was loaded into ${earlier.bronzeTable}.` : ""} Loading it
      again with &quot;Add to its rows&quot; would add its rows a second time.
    </UploadNotice>
  )
}

/**
 * Why the last load failed, as the API recorded it, and, for the one reason
 * after which the rows ARE in the table, what that means for loading again.
 */
export function LoadFailure({ upload }: { readonly upload: Upload }) {
  const registrationFailed = upload.error === REGISTRATION_FAILED_REASON
  return (
    <div className="space-y-2">
      <p role="alert" className="text-sm text-destructive">
        {upload.error ?? "No reason was recorded for this failure."}
      </p>
      {registrationFailed ? (
        <p className="text-sm text-muted-foreground">
          The rows are in the table{upload.bronzeTable ? ` ${upload.bronzeTable}` : ""}. Loading the file again
          with &quot;Add to its rows&quot; would add them a second time. &quot;Try again&quot; loads with
          &quot;Replace its rows&quot;, so the table ends with this file&apos;s rows once.
        </p>
      ) : null}
    </div>
  )
}
