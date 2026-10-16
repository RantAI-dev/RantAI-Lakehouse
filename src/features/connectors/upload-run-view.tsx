"use client"

import * as React from "react"
import Link from "next/link"
import { Button } from "@/components/ui/button"
import { HealthTile } from "@/components/patterns/health-tile"
import { SectionCard } from "@/components/patterns/section-card"
import { useRefreshable } from "@/hooks/use-refreshable"
import { useServiceAction } from "@/hooks/use-service"
import { formatBytes, formatNumber } from "@/lib/format"
import { rawTableFullName, retryInput } from "@/lib/uploads"
import { uploadService } from "@/services"
import type { Upload } from "@/services/contracts/uploads"
import { FileMark, LoadFailure, Refusal, UploadStatusPill } from "./upload-parts"

/** How often the upload is read again after "Load", while it is loading. */
const POLL_MS = 2000

type Polled = { attempt: number; upload: Upload }

/**
 * What happens after "Load": the upload's status, refreshed while it is
 * loading, then its result. Plan T11, feature page checklist 5, 12, 17, 18.
 *
 * - Loading: refreshed every 2 s, only while the status says so; the timer
 *   goes with the component. A refresh that fails is said, and the next tick
 *   tries again, so a dropped connection is never shown as a stalled load.
 * - Loaded: the row count (or "Not measured": a missing count is never
 *   written as 0), "Open in Data Explorer" and "Upload another file".
 * - Failed: the API's reason as it is, "Change settings" and "Try again".
 *   After a failure that happened only at the catalog the rows ARE in the
 *   table, which `LoadFailure` says and `retryInput` accounts for.
 *
 * `attempt` counts the loads started from here, so a poll answer that was
 * asked for before "Try again" (a stale `failed`) is never taken for the new
 * load's.
 */
export function UploadRunView({
  initial,
  pollMs = POLL_MS,
  onChangeSettings,
  onReset,
}: {
  readonly initial: Upload
  readonly pollMs?: number
  /** Back to the Check step, with the upload as it now stands (its failure reason included). */
  readonly onChangeSettings: (upload: Upload) => void
  /** "Upload another file": back to an empty first step. */
  readonly onReset: () => void
}) {
  const [attempt, setAttempt] = React.useState(0)
  const [started, setStarted] = React.useState<Upload>(initial)
  const polled = useRefreshable<Polled>(
    async (signal) => ({ attempt, upload: await uploadService.get(initial.id, signal) }),
    `${initial.id}:${attempt}`
  )
  const answer = polled.data !== null && polled.data.attempt === attempt ? polled.data.upload : null
  const upload = answer ?? started
  const retry = useServiceAction((signal, id: string, input: NonNullable<ReturnType<typeof retryInput>>) =>
    uploadService.ingest(id, input, signal)
  )

  const loading = upload.status === "ingesting"
  const { refresh } = polled
  React.useEffect(() => {
    if (!loading) return
    const timer = window.setInterval(refresh, pollMs)
    return () => window.clearInterval(timer)
  }, [loading, refresh, pollMs])

  const input = retryInput(upload)

  async function tryAgain() {
    if (input === null) return
    const res = await retry.run(upload.id, input)
    if (res === null) return
    setStarted(res.upload)
    setAttempt((n) => n + 1)
  }

  const title =
    upload.status === "ingesting"
      ? "Load in progress"
      : upload.status === "ingested"
        ? "Load finished"
        : upload.status === "failed"
          ? "Load failed"
          : "Upload status"
  const table = upload.bronzeTable !== undefined ? rawTableFullName(upload.bronzeTable) : null

  // Status first (the pill and the file), then what happened, then what to do
  // next. Nothing sits in the card's header slot: it cannot shrink at phone
  // width, and a long file name would crush the title beside it.
  return (
    <SectionCard title={title}>
      <div className="space-y-4">
        <div className="flex flex-wrap items-center gap-x-3 gap-y-2">
          <UploadStatusPill status={upload.status} />
          <div className="flex min-w-0 flex-1 basis-48 items-center gap-2 text-sm">
            <FileMark name={upload.originalFilename} variant="inline" />
            <span className="min-w-0 truncate font-medium" title={upload.originalFilename}>
              {upload.originalFilename}
            </span>
            <span className="shrink-0 text-xs text-muted-foreground">{formatBytes(upload.sizeBytes)}</span>
          </div>
        </div>

        {loading ? (
          <p role="status" className="text-sm text-muted-foreground">
            Loading {upload.originalFilename}
            {upload.bronzeTable ? ` into ${upload.bronzeTable}` : ""}. This page updates by itself; you can also
            leave it and come back from Uploaded files on Sources.
          </p>
        ) : null}
        {loading && polled.error !== null ? (
          <p role="alert" className="text-sm text-destructive">
            The status could not be refreshed: {polled.error.message} Trying again.
          </p>
        ) : null}

        {upload.status === "ingested" ? (
          <>
            <div className="grid gap-2 sm:grid-cols-3">
              <HealthTile label="Table">
                <span className="min-w-0 break-all font-mono text-[13px]">{table ?? "—"}</span>
              </HealthTile>
              <HealthTile label="Rows">
                {upload.rows === undefined ? (
                  <span className="text-muted-foreground">Not measured</span>
                ) : (
                  <span className="tabular-nums">{formatNumber(upload.rows)}</span>
                )}
              </HealthTile>
              {upload.loadMode === "append" || upload.loadMode === "replace" ? (
                <HealthTile label="Load mode">
                  <span>{upload.loadMode === "append" ? "Add to its rows" : "Replace its rows"}</span>
                </HealthTile>
              ) : null}
            </div>
            <div className="flex flex-wrap gap-2">
              {upload.assetId !== undefined ? (
                <Button size="sm" render={<Link href={`/data/assets/${encodeURIComponent(upload.assetId)}`} />}>
                  Open in Data Explorer
                </Button>
              ) : null}
              <Button size="sm" variant="outline" onClick={onReset}>
                Upload another file
              </Button>
            </div>
          </>
        ) : null}

        {upload.status === "failed" ? (
          <>
            <div className="rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2.5">
              <p className="mb-1 text-sm font-medium">The load{table ? ` into ${table}` : ""} failed.</p>
              <LoadFailure upload={upload} />
            </div>
            {retry.error !== null ? <Refusal error={retry.error} /> : null}
            <div className="flex flex-wrap gap-2">
              <Button size="sm" disabled={input === null || retry.status === "pending"} onClick={tryAgain}>
                {retry.status === "pending" ? "Starting…" : "Try again"}
              </Button>
              <Button size="sm" variant="outline" onClick={() => onChangeSettings(upload)}>
                Change settings
              </Button>
              <Button size="sm" variant="outline" onClick={onReset}>
                Upload another file
              </Button>
            </div>
          </>
        ) : null}

        {upload.status !== "ingesting" && upload.status !== "ingested" && upload.status !== "failed" ? (
          <div className="flex flex-wrap gap-2">
            <Button size="sm" variant="outline" onClick={() => onChangeSettings(upload)}>
              Change settings
            </Button>
          </div>
        ) : null}
      </div>
    </SectionCard>
  )
}
