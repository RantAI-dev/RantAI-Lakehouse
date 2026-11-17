"use client"

import * as React from "react"
import Link from "next/link"
import { RefreshCwIcon } from "lucide-react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Switch } from "@/components/ui/switch"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { useService, useServiceAction, type ServiceState } from "@/hooks/use-service"
import { formatDateTime } from "@/lib/format"
import { freshnessLine } from "@/lib/gold-freshness"
import { READERS_SHOWN, consumerLine, consumerState } from "@/lib/gold-consumers"
import { fmtMeasured } from "@/lib/measured"
import { goldService } from "@/services"
import type {
  GoldConsumers,
  GoldExportRun,
  GoldPublication,
  GoldReadBack,
  GoldTrimSummary,
} from "@/services/contracts/gold"

function trimmedMart(assetId: string): string {
  return assetId.startsWith("serving.") ? assetId.slice("serving.".length) : assetId
}

function LastFiveRuns({ mart, runsKey }: { mart: string; runsKey: number }) {
  const state = useService<GoldExportRun[]>(
    (signal) => goldService.listExportRuns(mart, signal),
    [mart, runsKey]
  )
  if (state.status === "loading") return <LoadingSkeleton rows={3} />
  if (state.status === "error") return <ErrorState error={state.error} onRetry={state.reload} />
  const runs = state.data.slice(0, 5)
  if (runs.length === 0) return <p className="text-sm text-muted-foreground">No publish runs yet.</p>
  return (
    <Table>
      <TableHeader>
        <TableRow className="hover:bg-transparent">
          <TableHead>Status</TableHead>
          <TableHead>Time</TableHead>
          <TableHead>Rows</TableHead>
          <TableHead>Snapshot</TableHead>
        </TableRow>
      </TableHeader>
      <TableBody>
        {runs.map((r) => (
          <TableRow key={r.id}>
            <TableCell className={r.status === "failed" ? "text-destructive" : ""}>
              {r.status}
              {r.status === "failed" && r.error && (
                <div className="text-xs text-destructive/80">{r.error}</div>
              )}
            </TableCell>
            <TableCell className="text-muted-foreground">
              {formatDateTime(r.startedAt)}
            </TableCell>
            <TableCell>{fmtMeasured(r.rowsExported)}</TableCell>
            <TableCell>
              {fmtMeasured(r.snapshotId, (v) => String(v))}
            </TableCell>
          </TableRow>
        ))}
      </TableBody>
    </Table>
  )
}

/**
 * The detail after the copy count (`DATA-49`, D17): when the last trim counted
 * it, and the oldest copy that trim kept.
 *
 * The number is the trim's, and no trim follows a manual publish (the Dagster
 * `trim_gold_mart` op is the only writer of `console.gold_trim_run`), so a
 * publish after the trim is not in it. Saying when it was counted is what keeps
 * the number honest; the card must not pass it off as a live count.
 */
function copiesKeptDetail(trim: GoldTrimSummary): string {
  const parts = [
    trim.finishedAt !== "" ? `counted ${formatDateTime(trim.finishedAt)}` : null,
    trim.oldestKeptExportedAt ? `oldest ${formatDateTime(trim.oldestKeptExportedAt)}` : null,
  ].filter((part): part is string => part !== null)
  return parts.length > 0 ? ` (${parts.join("; ")})` : ""
}

const stateLabelClass = (label: string): string => {
  if (label === "Up to date") return "font-medium text-green-600 dark:text-green-400"
  if (label === "Out of date") return "font-medium text-amber-600 dark:text-amber-400"
  // QA LH-DATA-1-01 (DATA-49): a failed newest run is the state that needs a
  // person, so it reads in the destructive tone, not the muted one.
  if (label === "Last publish failed") return "font-medium text-destructive"
  return "font-medium text-muted-foreground"
}

/**
 * The "Readers (7 days)" value: who read the published table through the
 * API's Iceberg door, and how often (DATA-5 D10).
 *
 * The same three states as the Gold Exports table: the tools that read
 * it (each with its reads and its last read, five at most, then "and N
 * more"), "No reads in 7 days" for a measured zero, and "Not measured"
 * with the route's reason when outside reading is not set up.
 */
function ReadersValue({ state }: { state: ServiceState<GoldConsumers> }) {
  if (state.status === "loading") {
    return <span className="text-muted-foreground">…</span>
  }
  if (state.status === "error") {
    return <span className="text-destructive">—</span>
  }
  const readers = consumerState(state.data)
  if (readers.kind === "not_measured") {
    return (
      <span className="text-muted-foreground" title={readers.reason}>
        Not measured
      </span>
    )
  }
  if (readers.kind === "unknown") {
    return (
      <span
        className="text-muted-foreground"
        title="Outside reading is set up, but the route sent no reader list."
      >
        —
      </span>
    )
  }
  if (readers.kind === "no_reads") {
    return <span className="text-muted-foreground">No reads in 7 days</span>
  }
  const shown = readers.readers.slice(0, READERS_SHOWN)
  const rest = readers.readers.length - shown.length
  return (
    <ul className="list-inside list-disc">
      {shown.map((reader) => (
        <li key={reader.identityId}>{consumerLine(reader)}</li>
      ))}
      {rest > 0 ? <li>and {rest} more</li> : null}
    </ul>
  )
}

export function OpenFormatCard({ assetId }: { assetId: string }) {
  const mart = trimmedMart(assetId)
  const pub = useService<GoldPublication>(
    (signal) => goldService.getPublication(mart, signal),
    [mart]
  )
  // QA LH-DATA-1-02 (plan D7): the publish history is read through routes
  // that need gold:export. A person without it got a 4xx there, and the
  // console read a 401 as an expired session. So nothing is requested until
  // the publication says this person may; the card still shows its state.
  const mayReadHistory = pub.status === "success" && pub.data.canEdit
  // QA LH-DATA-1-01 (DATA-49): a mart never published answers 200 with
  // `published: false`, so nothing is caught here — a failure is a failure and
  // the row below says it could not be read, instead of passing as "never
  // published".
  const lastExport = useService<GoldReadBack | null>(async (signal) => {
    if (!mayReadHistory) return null
    return await goldService.getLastExport(mart, signal)
  }, [mart, mayReadHistory])
  const readers = useService<GoldConsumers>(
    (signal) => goldService.getConsumers(mart, signal),
    [mart]
  )
  const toggle = useServiceAction(
    (signal, enabled: boolean) => goldService.setPublication(mart, enabled, signal)
  )
  const exportNow = useServiceAction((signal) => goldService.triggerExport(mart, signal))
  // Bumped after a publish so `LastFiveRuns` reads again: its `useService` is keyed
  // on the mart alone, so a new run would otherwise stay unseen.
  const [runsKey, setRunsKey] = React.useState(0)

  async function handleToggle(checked: boolean) {
    const result = await toggle.run(checked)
    if (result !== null) {
      pub.reload()
    }
  }

  if (pub.status === "loading") return <SectionCard title="Publish in open format (Iceberg)"><LoadingSkeleton rows={2} /></SectionCard>
  if (pub.status === "error") return <SectionCard title="Publish in open format (Iceberg)"><ErrorState error={pub.error} onRetry={pub.reload} /></SectionCard>

  const p = pub.data
  const enabled = p.enabled
  const canEdit = p.canEdit
  const line = freshnessLine(p)
  const hasHistory = p.lastExportedAt !== null
  // PR slice D review D-B3: always show the freshness line and,
  // when a publish exists, the last published time and snapshot,
  // whether the switch is on or off.
  const showHistory = hasHistory && lastExport.status === "success" && lastExport.data?.published === true

  return (
    <SectionCard
      title="Publish in open format (Iceberg)"
      description="A copy in open Iceberg format that outside tools can read. Publishing adds one full copy each time; old copies stay."
      action={
        <label className="flex items-center gap-2" data-testid="open-format-switch">
          <Switch
            checked={enabled}
            disabled={!canEdit || toggle.status === "pending"}
            aria-label="Publish in open format (Iceberg)"
            onCheckedChange={(checked) => {
              void handleToggle(checked)
            }}
            data-testid="open-format-toggle"
          />
          <span className="text-sm text-muted-foreground">
            {toggle.status === "pending"
              ? "Saving…"
              : enabled
                ? "On"
                : "Off"}
          </span>
        </label>
      }
    >
      <div className="flex flex-col gap-3">
        {!canEdit && (
          <p className="text-xs text-muted-foreground">
            You do not hold the gold:export permission. Only a Platform Admin may switch publishing on or off.
          </p>
        )}
        {/* PR slice D review D-S2: render toggle error from status,
            not from the stale closure capture of toggle.error. */}
        {toggle.status === "error" && (
          <p role="alert" className="text-sm text-destructive">{toggle.error.message}</p>
        )}
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span>Status:</span>
          {/* `data-testid` so a test can name THIS status line: the
              readers row below can also read "Not measured", and the
              two must not be confused. */}
          <span className={stateLabelClass(line)} data-testid="publication-status">
            {line}
          </span>
        </div>
        {/* QA LH-DATA-1-01 (DATA-49): what failed, when, and where the last good
            copy stands — so "Last publish failed" is actionable without opening
            the history. The failure's own sentence is the API's. */}
        {p.lastFailure && (
          <>
            <p className="text-xs text-destructive" data-testid="publication-failure">
              {p.lastFailure.failedAt
                ? `Failed ${formatDateTime(p.lastFailure.failedAt)}: ${p.lastFailure.reason}`
                : `Failed: ${p.lastFailure.reason}`}
            </p>
            <p className="text-xs text-muted-foreground">
              {p.lastExportedAt
                ? `Last successful publish: ${formatDateTime(p.lastExportedAt)}.`
                : "Nothing has been published yet."}
            </p>
          </>
        )}
        {!enabled && hasHistory && (
          <p className="text-xs text-muted-foreground">
            Publishing is off; this copy is no longer updated.
          </p>
        )}
        <dl className="grid grid-cols-3 gap-1 text-sm">
          <dt className="text-muted-foreground">Readers (7 days)</dt>
          <dd className="col-span-2">
            <ReadersValue state={readers} />
          </dd>
          {showHistory && lastExport.data && (
            <>
              <dt className="text-muted-foreground">Last published</dt>
              <dd className="col-span-2">{lastExport.data.exportedAt ? formatDateTime(lastExport.data.exportedAt) : "—"}</dd>
              <dt className="text-muted-foreground">Snapshot</dt>
              <dd className="col-span-2 font-mono text-xs">
                {fmtMeasured(lastExport.data.snapshotId, (v) => String(v))}
              </dd>
            </>
          )}
          {/* QA LH-DATA-1-01 (DATA-49): the read-back failed, so the copy's time
              and snapshot cannot be shown — say that, rather than leaving the rows
              out and looking like a mart nobody published. */}
          {hasHistory && lastExport.status === "error" && (
            <>
              <dt className="text-muted-foreground">Last published</dt>
              <dd className="col-span-2 text-destructive" title={lastExport.error.message}>
                Could not be read
              </dd>
            </>
          )}
          {p.trim && (p.trim.status === "success" || p.trim.status === "nothing_to_trim") && p.trim.copiesKept !== null && (
            <>
              <dt className="text-muted-foreground">Copies kept at last trim</dt>
              <dd className="col-span-2">
                {p.trim.copiesKept}
                {copiesKeptDetail(p.trim)}
              </dd>
            </>
          )}
          {p.trim && p.trim.status === "failed" && (
            <>
              <dt className="text-muted-foreground">Trim</dt>
              <dd className="col-span-2 text-destructive">
                Trim failed{p.trim.reason ? `: ${p.trim.reason}` : ""}
              </dd>
            </>
          )}
        </dl>
        {enabled && (
          <>
            <div className="flex flex-col items-start gap-1">
              {/* PR slice D review D-S3: "Publish now" when canEdit is
                  false is confusing — a user who cannot toggle also
                  cannot export. Show it only when they can. */}
              {canEdit && (
                <Button
                  size="sm"
                  variant="outline"
                  disabled={exportNow.status === "pending"}
                  onClick={() => {
                    // QA LH-DATA-1-01 (DATA-49), review round 1 R1: a failed publish
                    // answers `lastFailure` and records a run, so the state and the run
                    // list are read again whatever the result — the card kept saying
                    // "Up to date" until the page was loaded again.
                    void exportNow.run().then((result) => {
                      pub.reload()
                      setRunsKey((key) => key + 1)
                      if (result) lastExport.reload()
                    })
                  }}
                >
                  <RefreshCwIcon data-icon="inline-start" />
                  {exportNow.status === "pending" ? "Publishing…" : "Publish now"}
                </Button>
              )}
              {exportNow.status === "error" && (
                <p role="alert" className="text-xs text-destructive">
                  {exportNow.error.message}
                </p>
              )}
            </div>
            {/* QA LH-DATA-1-02 (plan D7): the run list needs gold:export too. */}
            {canEdit && (
              <div>
                <h4 className="mb-1 text-sm font-medium">Last 5 publish runs</h4>
                <LastFiveRuns mart={mart} runsKey={runsKey} />
              </div>
            )}
            <Link
              href="/gold-exports"
              className="self-start text-sm text-primary hover:underline"
            >
              All published marts
            </Link>
          </>
        )}
      </div>
    </SectionCard>
  )
}