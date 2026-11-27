"use client"

import * as React from "react"
import Link from "next/link"
import { RefreshCwIcon } from "lucide-react"
import { Copyable } from "@/components/copyable"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
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
import { freshnessLine, SYNC_MODE_LABELS } from "@/lib/gold-freshness"
import { READERS_SHOWN, consumerLine, consumerState } from "@/lib/gold-consumers"
import { fmtMeasured } from "@/lib/measured"
import { goldService } from "@/services"
import type {
  GoldConsumers,
  GoldExportRun,
  GoldLatestCopyHolds,
  GoldPublication,
  GoldReadBack,
  GoldSyncSchedule,
  GoldSyncSetting,
  GoldTrimSummary,
  GoldZone,
} from "@/services/contracts/gold"
import { TimeZoneSelect, browserTimeZone } from "@/components/patterns/time-zone-select"
// Shared with the asset "Connect" dialog: both name the mart the Gold export
// routes use, not the `serving.`-prefixed catalog id.
import { martFromAssetId as trimmedMart } from "./asset-connect-snippets"

function LastFiveRuns({
  mart,
  zone,
  runsKey,
}: {
  mart: string
  zone: GoldZone
  runsKey: number
}) {
  const state = useService<GoldExportRun[]>(
    (signal) => goldService.listExportRuns(mart, signal, zone),
    [mart, zone, runsKey]
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

/**
 * The "Holds" line of the latest-copy block (`DATA-6` D4): which publish
 * the held copy came from and how many rows it carries. A success row
 * always records both; a missing fact renders as an em dash rather than
 * a guessed number (the same "never fabricated" posture as every other
 * measured value on this card).
 */
function latestHoldsLine(holds: GoldLatestCopyHolds): string {
  const when = holds.copyExportedAt ? formatDateTime(holds.copyExportedAt) : "—"
  const rows = holds.rows !== null ? `${holds.rows} rows` : "—"
  return `The copy published ${when} (${rows})`
}

/**
 * The fixed sentence for a failed latest-copy run (`DATA-6` D12): one
 * English sentence per recorded reason, the map this feature owes the
 * card. An unknown code gets the generic sentence with the code in
 * parentheses — the code is this repo's own fixed string, never
 * exception text, so showing it is honest; inventing a sentence for it
 * would not be.
 */
function latestCopyFailureText(reason: string | null): string {
  const sentences: Record<string, string> = {
    not_started: "The latest copy was not updated: the update job could not be started.",
    invalid_request: "The latest copy was not updated: the update request was malformed.",
    publication_state_unavailable:
      "The latest copy was not updated: the publishing state could not be read.",
    catalog_unavailable: "The latest copy was not updated: the data catalog could not be reached.",
    name_taken:
      "The latest copy was not updated: the latest-copy table name is already used by another table.",
    schema_incompatible:
      "The latest copy was not updated: the latest-copy table's columns no longer match the published table's.",
    copy_incomplete:
      "The latest copy was not updated: the published copy was incomplete or already removed.",
    commit_conflict: "The latest copy was not updated: another update committed at the same moment.",
    write_failed: "The latest copy was not updated: writing the new copy failed.",
  }
  if (reason === null) return "The latest copy was not updated."
  const sentence = sentences[reason]
  return sentence ?? `The latest copy was not updated. (${reason})`
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

/** `DATA-66` D5: the editor's own state, prefilled from the stored slot. */
type SyncDraft = {
  kind: "hourly" | "daily"
  hour: number
  minute: number
  timeZone: string
}

/**
 * `DATA-66` D4: when "On a schedule" is first picked, pre-fill daily
 * 04:00 (today's nightly time) in the browser's zone, falling back to
 * `UTC` when the runtime cannot say. An existing schedule keeps its own
 * slot and zone, so opening the editor never rewrites it by accident.
 */
function syncDraft(publication: GoldPublication): SyncDraft {
  const timeZone =
    publication.timeZone ?? browserTimeZone() ?? "UTC"
  if (publication.schedule?.kind === "hourly") {
    return { kind: "hourly", hour: 4, minute: publication.schedule.minute, timeZone }
  }
  if (publication.schedule?.kind === "daily") {
    return {
      kind: "daily",
      hour: publication.schedule.hour,
      minute: publication.schedule.minute,
      timeZone,
    }
  }
  return { kind: "daily", hour: 4, minute: 0, timeZone }
}

/**
 * `DATA-66` D2/D8: the "Sync" row of the publish card, with the editor for
 * "On a schedule". The two one-click modes save at once; the scheduled
 * editor prefills per D4 and saves the mode, the slot and the IANA zone in
 * one PUT. Every mode line is the feature page's own sentence, so the
 * card cannot promise a cadence the API does not run.
 */
function SyncSetting({
  mart,
  zone,
  publication,
  canEdit,
  onSaved,
}: {
  mart: string
  zone: GoldZone
  publication: GoldPublication
  canEdit: boolean
  onSaved: () => void
}) {
  const [editing, setEditing] = React.useState(false)
  const [draft, setDraft] = React.useState<SyncDraft>(() => syncDraft(publication))
  const save = useServiceAction((signal, sync: GoldSyncSetting) =>
    goldService.setPublication(mart, true, signal, zone, sync)
  )
  const saving = save.status === "pending"

  async function persist(sync: GoldSyncSetting) {
    const result = await save.run(sync)
    if (result !== null) {
      setEditing(false)
      onSaved()
    }
  }

  function pickMode(mode: string) {
    if (mode === "scheduled") {
      setDraft(syncDraft(publication))
      setEditing(true)
      return
    }
    setEditing(false)
    void persist({ syncMode: mode })
  }

  const mode = editing ? "scheduled" : publication.syncMode
  const line =
    mode === "manual"
      ? "Synced only when you press Publish now."
      : mode === "scheduled"
        ? publication.nextSyncAt
          ? `Next sync: ${formatDateTime(publication.nextSyncAt)} (${publication.timeZone ?? "UTC"}).`
          : "No next sync yet."
        : "Synced after each pipeline write, and checked every 15 seconds for other changes."

  function saveSchedule() {
    const schedule: GoldSyncSchedule =
      draft.kind === "hourly"
        ? { kind: "hourly", minute: draft.minute }
        : { kind: "daily", hour: draft.hour, minute: draft.minute }
    void persist({ syncMode: "scheduled", schedule, timeZone: draft.timeZone })
  }

  const pad = (n: number) => String(n).padStart(2, "0")

  return (
    <div className="flex flex-col gap-2" data-testid="sync-setting">
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="text-muted-foreground">Sync</span>
        <Select
          value={mode}
          items={SYNC_MODE_LABELS}
          onValueChange={(next) => {
            if (next) pickMode(next)
          }}
          disabled={!canEdit || saving}
        >
          <SelectTrigger size="sm" aria-label="Sync mode">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {Object.entries(SYNC_MODE_LABELS).map(([value, label]) => (
              <SelectItem key={value} value={value}>
                {label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
      {editing && (
        <div className="flex flex-wrap items-end gap-2" data-testid="sync-schedule-editor">
          <Select
            value={draft.kind}
            items={{ hourly: "Hourly", daily: "Daily" }}
            onValueChange={(next) => {
              if (next === "hourly" || next === "daily") setDraft({ ...draft, kind: next })
            }}
            disabled={!canEdit || saving}
          >
            <SelectTrigger size="sm" aria-label="Frequency">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="hourly">Hourly</SelectItem>
              <SelectItem value="daily">Daily</SelectItem>
            </SelectContent>
          </Select>
          {draft.kind === "hourly" ? (
            <label className="flex flex-col gap-1 text-xs text-muted-foreground">
              Minute
              <Input
                type="number"
                min={0}
                max={59}
                aria-label="Minute"
                className="w-20"
                value={draft.minute}
                disabled={!canEdit || saving}
                onChange={(e) => {
                  const minute = Number(e.target.value)
                  if (Number.isInteger(minute) && minute >= 0 && minute <= 59) {
                    setDraft({ ...draft, minute })
                  }
                }}
              />
            </label>
          ) : (
            <label className="flex flex-col gap-1 text-xs text-muted-foreground">
              Time
              <Input
                type="time"
                aria-label="Time of day"
                className="w-28"
                value={`${pad(draft.hour)}:${pad(draft.minute)}`}
                disabled={!canEdit || saving}
                onChange={(e) => {
                  const [hour, minute] = e.target.value.split(":").map(Number)
                  if (Number.isInteger(hour) && Number.isInteger(minute)) {
                    setDraft({ ...draft, hour, minute })
                  }
                }}
              />
            </label>
          )}
          <div className="w-56">
            <TimeZoneSelect
              id="sync-time-zone"
              value={draft.timeZone}
              onValueChange={(timeZone) => setDraft({ ...draft, timeZone })}
              disabled={!canEdit || saving}
            />
          </div>
          <Button size="sm" variant="outline" disabled={!canEdit || saving} onClick={saveSchedule}>
            {saving ? "Saving…" : "Save schedule"}
          </Button>
        </div>
      )}
      <p className="text-xs text-muted-foreground" data-testid="sync-line">
        {line}
      </p>
      {save.status === "error" && (
        <p role="alert" className="text-sm text-destructive">
          {save.error.message}
        </p>
      )}
    </div>
  )
}

export function OpenFormatCard({ assetId }: { assetId: string }) {
  // DATA-65: a Silver asset publishes to the `silver` Iceberg namespace;
  // every other asset (Gold `serving.*`) keeps the old gold behavior.
  const zone: GoldZone = assetId.startsWith("silver.") ? "silver" : "gold"
  const mart = trimmedMart(assetId).replace(/^silver\./, "")
  const pub = useService<GoldPublication>(
    (signal) => goldService.getPublication(mart, signal, zone),
    [mart, zone]
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
    return await goldService.getLastExport(mart, signal, zone)
  }, [mart, zone, mayReadHistory])
  const readers = useService<GoldConsumers>(
    (signal) => goldService.getConsumers(mart, signal, zone),
    [mart, zone]
  )
  const toggle = useServiceAction(
    (signal, enabled: boolean) => goldService.setPublication(mart, enabled, signal, zone)
  )
  const exportNow = useServiceAction((signal) => goldService.triggerExport(mart, signal, zone))
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
      description={`A copy in open Iceberg format that outside tools can read. This ${zone === "silver" ? "Silver" : "Gold"} asset publishes to ${zone}.${mart}; publishing adds one full copy each time and old copies stay. A second table holds only the newest copy, for tools that do not filter on export time.`}
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
        {/* DATA-66: with publishing on, the table's own sync choice. It is
            shown read-only without `canEdit`, with the same explanation
            the switch carries above. */}
        {enabled && (
          <SyncSetting
            mart={mart}
            zone={zone}
            publication={p}
            canEdit={canEdit}
            onSaved={() => pub.reload()}
          />
        )}
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
        <div className="flex flex-wrap items-center gap-2 text-sm">
          <span className="text-muted-foreground">Iceberg table:</span>
          <code data-testid="iceberg-table" className="font-mono text-xs">
            {zone}.{mart}
          </code>
        </div>
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
          {p.latestCopy && (
            <>
              {/* DATA-6 D4: the second table's name with a copy button, so a
                  tool can be pointed at it without retyping it. The name is
                  shown before the first update too — it is a fact of the
                  design, not of any run. */}
              <dt className="text-muted-foreground">Latest copy only</dt>
              <dd className="col-span-2">
                <Copyable value={p.latestCopy.table}>
                  <span className="font-mono text-xs">{p.latestCopy.table}</span>
                </Copyable>
              </dd>
              <dt className="text-muted-foreground">Holds</dt>
              <dd className="col-span-2">
                {p.latestCopy.holds ? latestHoldsLine(p.latestCopy.holds) : "Not written yet"}
              </dd>
              {/* The run's own outcome, after the holds: a failure never
                  erases the copy that is still there, so the card says both. */}
              {p.latestCopy.lastRun?.status === "failed" && (
                <>
                  <dt className="text-muted-foreground">Latest copy run</dt>
                  <dd className="col-span-2 text-destructive" data-testid="latest-copy-failure">
                    {latestCopyFailureText(p.latestCopy.lastRun.reason)}
                  </dd>
                </>
              )}
              {p.latestCopy.lastRun?.status === "skipped" && (
                <>
                  <dt className="text-muted-foreground">Latest copy run</dt>
                  <dd className="col-span-2 text-muted-foreground">
                    Not updated: publishing is off.
                  </dd>
                </>
              )}
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
                <LastFiveRuns mart={mart} zone={zone} runsKey={runsKey} />
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