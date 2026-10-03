"use client"

import { ErrorState } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { formatDateTime } from "@/lib/format"
import { REGISTRATION_FAILED_REASON, statusLabel } from "@/lib/uploads"
import type { Upload, UploadStatus } from "@/services/contracts/uploads"
import type { ServiceError } from "@/services/errors"

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

/** The same file arrived before: its name and date, and what loading it again does. */
export function DuplicateNotice({ earlier }: { readonly earlier: Upload }) {
  return (
    <div
      role="status"
      className="rounded-lg border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-foreground"
    >
      This file was uploaded before, as {earlier.originalFilename} on {formatDateTime(earlier.createdAt)}.
      {earlier.bronzeTable !== undefined ? ` It was loaded into ${earlier.bronzeTable}.` : ""} Loading it
      again with &quot;Add to its rows&quot; would add its rows a second time.
    </div>
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
