"use client"

import * as React from "react"
import Link from "next/link"
import {
  CalendarClockIcon,
  ChevronRightIcon,
  LoaderIcon,
  PlayIcon,
  PlusIcon,
  SearchIcon,
  TableIcon,
  XIcon,
} from "lucide-react"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useRefreshable } from "@/hooks/use-refreshable"
import { useService, useServiceAction } from "@/hooks/use-service"
import { backfillTriggerMessage } from "@/lib/connectors/backfill-message"
import { formatDateTime, formatDuration, formatRelativeTime, formatTimeUntil, parseTimestamp } from "@/lib/format"
import { withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { connectorService } from "@/services"
import type {
  Dial,
  IngestJobRun,
  IngestRun,
  IngestSpec,
  LoadMode,
  SourceObject,
} from "@/services/contracts/connectors"

/** A lower-case SQL-safe identifier: what a Bronze table can be called. */
const TARGET_PATTERN = /^[a-z_][a-z0-9_]*$/

function slug(text: string): string {
  return text
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "_")
    .replace(/^_+|_+$/g, "")
}

/**
 * The Bronze table a source object lands in, unless the user renames it:
 * `<connector>_<table>`. Every connector writes into the same flat `bronze`
 * namespace, so two connectors that both have an `orders` table would
 * otherwise append into one `bronze.orders`.
 *
 * Rust has the same rule: `default_bronze_target` in
 * `rust/crates/lakehouse-store/src/ingest_spec.rs` names the target of a table
 * `lakehouse-api` adds on its own (`SRC-8`, policy "apply all"). Change one,
 * change the other; both tests pin the same examples.
 */
export function defaultTarget(connectorName: string, objectName: string): string {
  const table = slug(objectName.split(".").pop() ?? objectName)
  const prefix = slug(connectorName)
  const joined = [prefix, table].filter(Boolean).join("_") || "table"
  return /^[0-9]/.test(joined) ? `t_${joined}` : joined
}

/** Why a target cannot be saved, or `null`. */
export function targetProblem(target: string, others: string[]): string | null {
  if (target === "") return "Required."
  if (!TARGET_PATTERN.test(target)) return "Lower-case letters, digits and _ only, not starting with a digit."
  if (others.includes(target)) return "Another table already lands here."
  return null
}

/**
 * The schema discovery lists by default: Postgres keeps tables in
 * `public`, SQL Server in `dbo`, and MySQL calls the database itself the
 * schema.
 */
export function defaultDiscoverSchema(dial: Dial | null | undefined): string {
  const driver = typeof dial?.driver === "string" ? dial.driver : ""
  if (driver === "mysql") return typeof dial?.database === "string" ? dial.database : ""
  if (driver === "mssql") return "dbo"
  if (driver === "oracle") return typeof dial?.user === "string" ? dial.user.toUpperCase() : ""
  return "public"
}

/** Five space-separated cron fields, or `null` when it looks usable. */
export function cronProblem(cron: string): string | null {
  const fields = cron.trim().split(/\s+/)
  if (cron.trim() === "") return "Enter a cron expression, or pick Manual only."
  if (fields.length !== 5) return "A cron expression has five fields: minute hour day month weekday."
  return null
}

// Crons are evaluated in UTC (`ingest_schedule_sensor`), so a fixed time
// says so; the next run below is shown in the viewer's own time.
const SCHEDULE_PRESETS: { value: string; label: string }[] = [
  { value: "", label: "Manual only" },
  { value: "0 * * * *", label: "Every hour" },
  { value: "0 2 * * *", label: "Every day at 02:00 UTC" },
]

/** A saved schedule as the console words it: a preset's own label, or the
 * cron itself, marked UTC. */
export function scheduleLabel(cron: string | null): string {
  if (!cron) return "Manual only"
  return SCHEDULE_PRESETS.find((p) => p.value === cron)?.label ?? `${cron} (UTC)`
}

const RUN_TONE: Record<string, "success" | "destructive" | "warning" | "neutral"> = {
  succeeded: "success",
  failed: "destructive",
  rejected: "warning",
  unsupported: "neutral",
}

type Selected = { name: string; target: string; loadMode: LoadMode; incrementalKey?: string }
type Selection = Selected[]

/** What a table gets when it is ticked: one copy of the source per run. */
const DEFAULT_LOAD_MODE: LoadMode = "replace"

const LOAD_MODE_LABEL: Record<LoadMode, string> = {
  replace: "Replace all rows",
  incremental: "Add only new rows",
  append: "Add all rows again",
}

/** What a mode does on each run, in one line under the choice. */
function loadModeHint(mode: LoadMode, cursor: string): string {
  switch (mode) {
    case "replace":
      return "The table always holds one copy of the source."
    case "incremental":
      return `The first run copies everything; later runs add rows whose ${
        cursor || "column"
      } is higher than the last one seen. A changed row is added again as a new version.`
    case "append":
      return "Every run adds the whole source on top of earlier loads, so the table grows by a full copy each time."
  }
}

/** `incremental` is pushed into a SQL query; no other adapter runs it. */
function loadModesFor(adapter: string): LoadMode[] {
  return adapter === "sql" ? ["replace", "incremental", "append"] : ["replace", "append"]
}

/** Why a table's load mode cannot be saved, or `null`. */
export function loadModeProblem(selected: { loadMode: LoadMode; incrementalKey?: string }): string | null {
  if (selected.loadMode === "incremental" && !(selected.incrementalKey ?? "").trim()) {
    return "Pick the column that marks new rows, such as an id or an updated-at time."
  }
  return null
}

function fromSpec(spec: IngestSpec): Selection {
  return spec.sourceObjects.map((o) => ({
    name: o.name,
    target: o.target,
    // Saved before load modes existed: the ingest job replaces.
    loadMode: o.loadMode ?? DEFAULT_LOAD_MODE,
    incrementalKey: o.incrementalKey,
  }))
}

/** `withModes: false` for CDC, whose tables stream and are never loaded by a run. */
function toSourceObjects(selection: Selection, withModes: boolean): SourceObject[] {
  return selection.map((s) => ({
    name: s.name,
    target: s.target,
    ...(withModes ? { loadMode: s.loadMode } : {}),
    ...(withModes && s.loadMode === "incremental" && s.incrementalKey?.trim()
      ? { incrementalKey: s.incrementalKey.trim() }
      : {}),
  }))
}

/**
 * What a connector ingests and when: the tables it copies into Bronze
 * (found through discovery, or typed by hand), the Bronze table each one
 * lands in, an optional schedule, a manual run, and the recent runs.
 *
 * A connector that has been created and tested moves no data until at
 * least one table is saved here: every run of the shared `ingest_job`
 * reads this list.
 *
 * `layout` is where it sits. "embedded" (the default) is one column with no
 * card of its own, for a page that already wraps it in one (the page shown
 * after creating a connector). "page" is the connector page's Ingest tab:
 * three cards, in two columns from `xl` up. Nothing but the arrangement
 * differs. `onTableCount` is told how many tables the connector has saved,
 * as soon as the saved spec is known and after every save, so the page can
 * show it on the tab without asking the API a second time.
 */
export function ConnectorIngestPanel({
  connectorId,
  connectorName,
  layout = "embedded",
  onTableCount,
}: {
  connectorId: string
  connectorName: string
  layout?: "page" | "embedded"
  onTableCount?: (count: number) => void
}) {
  const spec = useService((s) => connectorService.getIngestSpec(connectorId, s), [connectorId])
  // The spec a save returned, so saving does not refetch (and unmount the
  // editor, losing the tables it just found).
  const [savedSpec, setSavedSpec] = React.useState<IngestSpec | null>(null)
  const known = savedSpec ?? spec.data
  // No count for a connector with no connection saved: nothing can be
  // ingested there, which is not the same fact as "no tables picked".
  const tableCount = known?.adapter && known.ingestMode ? known.sourceObjects.length : null
  React.useEffect(() => {
    if (tableCount !== null) onTableCount?.(tableCount)
  }, [tableCount, onTableCount])

  if (spec.status === "loading") return <LoadingSkeleton rows={3} />
  if (spec.status === "error") return <ErrorState error={spec.error} onRetry={spec.reload} />
  const current = savedSpec ?? spec.data
  if (!current.adapter || !current.ingestMode) {
    return (
      <p className="text-sm text-muted-foreground">
        This connector has no connection settings saved yet, so there is nothing to ingest. Save its connection
        from{" "}
        <Link href={`/connectors/${connectorId}/edit`} className="text-primary hover:underline">
          Edit
        </Link>{" "}
        first.
      </p>
    )
  }
  return (
    <IngestEditor
      connectorId={connectorId}
      connectorName={connectorName}
      spec={current}
      layout={layout}
      onSaved={setSavedSpec}
    />
  )
}

function IngestEditor({
  connectorId,
  connectorName,
  spec,
  layout,
  onSaved,
}: {
  connectorId: string
  connectorName: string
  spec: IngestSpec
  layout: "page" | "embedded"
  onSaved: (spec: IngestSpec) => void
}) {
  const adapter = spec.adapter ?? ""
  const isCdc = adapter === "cdc"
  const isStream = adapter === "kafka"
  const canDiscover = adapter === "sql" || adapter === "cdc"

  const saved = React.useMemo(() => fromSpec(spec), [spec])
  const [selection, setSelection] = React.useState<Selection>(saved)
  const savedCron = spec.scheduleCron ?? ""
  const [cronChoice, setCronChoice] = React.useState<string>(
    SCHEDULE_PRESETS.some((p) => p.value === savedCron) ? savedCron : "custom"
  )
  const [customCron, setCustomCron] = React.useState(
    SCHEDULE_PRESETS.some((p) => p.value === savedCron) ? "" : savedCron
  )
  const cron = cronChoice === "custom" ? customCron.trim() : cronChoice

  const [schema, setSchema] = React.useState(() => defaultDiscoverSchema(spec.dial))
  const [filter, setFilter] = React.useState("")
  const [manualName, setManualName] = React.useState("")

  const discover = useServiceAction((signal, id: string, s: string) =>
    connectorService.discoverConnector(id, s, signal)
  )
  const save = useServiceAction(
    withNotify({ success: "Tables and schedule saved", error: "Could not save tables" }, (signal) =>
      connectorService.setIngestSpec(
        connectorId,
        {
          adapter,
          ingestMode: spec.ingestMode ?? "batch",
          dial: spec.dial,
          sourceObjects: toSourceObjects(selection, !isCdc),
          ...(cron && !isCdc ? { scheduleCron: cron } : {}),
        },
        signal
      )
    )
  )

  const selectedNames = new Set(selection.map((s) => s.name))
  const problems = selection.map((s, i) =>
    targetProblem(
      s.target,
      selection.filter((_, j) => j !== i).map((o) => o.target)
    )
  )
  const modeProblems = selection.map((s) => (isCdc ? null : loadModeProblem(s)))
  const scheduleProblem = !isCdc && cronChoice === "custom" ? cronProblem(customCron) : null
  const dirty =
    JSON.stringify(toSourceObjects(selection, !isCdc)) !== JSON.stringify(toSourceObjects(saved, !isCdc)) ||
    (!isCdc && cron !== savedCron)
  const canSave =
    dirty && problems.every((p) => p === null) && modeProblems.every((p) => p === null) && scheduleProblem === null

  function toggle(name: string) {
    setSelection((current) =>
      current.some((s) => s.name === name)
        ? current.filter((s) => s.name !== name)
        : [...current, { name, target: defaultTarget(connectorName, name), loadMode: DEFAULT_LOAD_MODE }]
    )
  }

  function change(name: string, patch: Partial<Selected>) {
    setSelection((current) => current.map((s) => (s.name === name ? { ...s, ...patch } : s)))
  }

  function addManual() {
    const name = manualName.trim()
    if (!name || selectedNames.has(name)) return
    setSelection((current) => [
      ...current,
      { name, target: defaultTarget(connectorName, name), loadMode: DEFAULT_LOAD_MODE },
    ])
    setManualName("")
  }

  const found = discover.data?.supported ? discover.data.objects : []
  // The columns of the tables found so far, to pick a cursor column from.
  const columnsOf = new Map(found.map((o) => [o.name, o.columns.map((c) => c.name)]))
  const needle = filter.trim().toLowerCase()
  const shown = needle ? found.filter((o) => o.name.toLowerCase().includes(needle)) : found
  const allShownSelected = shown.length > 0 && shown.every((o) => selectedNames.has(o.name))

  const page = layout === "page"
  const tablesIntro = isStream
    ? "A Kafka connector reads its topic; each run takes one micro-batch."
    : "Each table here lands in its own Bronze table. A run replaces its rows, or adds to them, as chosen per table. Nothing is ingested until at least one table is saved."

  const cdcNotes = (
    <>
      {isCdc ? (
        <p className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          {backfillTriggerMessage(typeof spec.dial.driver === "string" ? spec.dial.driver : "cdc")}
        </p>
      ) : null}
      {isCdc && saved[0] ? <DebeziumPanel connectorId={connectorId} table={saved[0].name} /> : null}
    </>
  )

  const tablesBody = !isStream ? (
    <>
      {/* Selected tables: the list that is saved. */}
      <div className="space-y-2">
        <p className="text-xs font-medium text-muted-foreground">
          Selected · {selection.length} {selection.length === 1 ? "table" : "tables"}
        </p>
        {selection.length === 0 ? (
          <p className="rounded-md border border-dashed border-border px-3 py-4 text-center text-xs text-muted-foreground">
            No tables selected yet.{canDiscover ? " Find tables below and tick the ones to copy." : ""}
          </p>
        ) : (
          <ul className="divide-y divide-border rounded-md border border-border">
            {selection.map((s, i) => (
              <li key={s.name} className="space-y-1.5 px-3 py-2">
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate font-mono text-xs" title={s.name}>
                    {s.name}
                  </span>
                  <button
                    type="button"
                    aria-label={`Remove ${s.name}`}
                    onClick={() => toggle(s.name)}
                    className="rounded p-0.5 text-muted-foreground outline-none hover:bg-muted hover:text-foreground focus-visible:ring-[3px] focus-visible:ring-ring/50"
                  >
                    <XIcon className="size-3.5" />
                  </button>
                </div>
                <div className="flex items-center gap-1.5">
                  <span className="shrink-0 text-xs text-muted-foreground">→ bronze.</span>
                  <Input
                    aria-label={`Bronze table for ${s.name}`}
                    value={s.target}
                    onChange={(e) => change(s.name, { target: e.target.value })}
                    className={cn("h-7 font-mono text-xs", page && "max-w-xs")}
                    aria-invalid={problems[i] !== null}
                    autoComplete="off"
                  />
                </div>
                {problems[i] ? <p className="text-xs text-destructive">{problems[i]}</p> : null}
                {!isCdc ? (
                  <LoadModeFields
                    selected={s}
                    modes={loadModesFor(adapter)}
                    columns={columnsOf.get(s.name) ?? null}
                    problem={modeProblems[i]}
                    onChange={(patch) => change(s.name, patch)}
                  />
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </div>

      {/* Finding tables. */}
      {canDiscover ? (
        <div className="space-y-2 rounded-md border border-border p-3">
          <div className="flex items-end gap-2">
            <div className={cn("min-w-0 flex-1 space-y-1", page && "max-w-xs")}>
              <Label htmlFor="discover-schema" className="text-xs">
                Schema
              </Label>
              <Input
                id="discover-schema"
                value={schema}
                onChange={(e) => setSchema(e.target.value)}
                className="h-8"
                autoComplete="off"
              />
            </div>
            <Button
              type="button"
              size="sm"
              variant="outline"
              disabled={discover.status === "pending" || schema.trim() === ""}
              onClick={() => discover.run(connectorId, schema.trim())}
            >
              <SearchIcon data-icon="inline-start" />
              {discover.status === "pending" ? "Finding…" : "Find tables"}
            </Button>
          </div>
          {discover.status === "error" ? <ErrorState error={discover.error} /> : null}
          {discover.data && !discover.data.supported ? (
            <p className="text-xs text-muted-foreground">Cannot list tables · {discover.data.reason}</p>
          ) : null}
          {discover.data?.supported && found.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              No tables in schema &ldquo;{schema.trim()}&rdquo;. Check the schema name.
            </p>
          ) : null}
          {found.length > 0 ? (
            <div className="space-y-2">
              <div className="flex items-center gap-2">
                {found.length > 8 ? (
                  <Input
                    aria-label="Filter tables"
                    value={filter}
                    onChange={(e) => setFilter(e.target.value)}
                    placeholder={`Filter ${found.length} tables`}
                    className={cn("h-7 text-xs", page && "max-w-xs")}
                    autoComplete="off"
                  />
                ) : (
                  <span className="flex-1 text-xs text-muted-foreground">{found.length} tables found</span>
                )}
                <Button
                  type="button"
                  size="xs"
                  variant="ghost"
                  className="shrink-0"
                  onClick={() =>
                    setSelection((current) => {
                      if (allShownSelected) return current.filter((s) => !shown.some((o) => o.name === s.name))
                      const missing = shown.filter((o) => !current.some((s) => s.name === o.name))
                      return [
                        ...current,
                        ...missing.map((o) => ({
                          name: o.name,
                          target: defaultTarget(connectorName, o.name),
                          loadMode: DEFAULT_LOAD_MODE,
                        })),
                      ]
                    })
                  }
                >
                  {allShownSelected ? "Clear shown" : "Select shown"}
                </Button>
              </div>
              <ul className="max-h-64 space-y-0.5 overflow-y-auto">
                {shown.map((object) => {
                  const id = `discover-${object.name}`
                  return (
                    <li key={object.name}>
                      <label
                        htmlFor={id}
                        className="flex cursor-pointer items-center gap-2 rounded px-1.5 py-1 text-sm hover:bg-muted/50"
                      >
                        <input
                          id={id}
                          type="checkbox"
                          checked={selectedNames.has(object.name)}
                          onChange={() => toggle(object.name)}
                        />
                        <span className="min-w-0 flex-1 truncate font-mono text-xs">{object.name}</span>
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {object.columns.length} cols
                        </span>
                      </label>
                    </li>
                  )
                })}
              </ul>
            </div>
          ) : null}
        </div>
      ) : (
        <div className="space-y-1.5">
          <Label htmlFor="manual-object" className="text-xs">
            Add by name
          </Label>
          <div className={cn("flex gap-2", page && "max-w-md")}>
            <Input
              id="manual-object"
              value={manualName}
              onChange={(e) => setManualName(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault()
                  addManual()
                }
              }}
              placeholder={adapter === "sftp" || adapter === "files" ? "orders.csv" : "object name"}
              className="h-8 font-mono text-xs"
              autoComplete="off"
            />
            <Button type="button" size="sm" variant="outline" disabled={!manualName.trim()} onClick={addManual}>
              <PlusIcon data-icon="inline-start" />
              Add
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            This connector type cannot list its objects, so type each one as the source names it.
          </p>
        </div>
      )}
    </>
  ) : null

  // Schedule. CDC streams on its own and has none.
  const scheduleBlock = !isCdc ? (
    <div className="space-y-1.5">
      <Label htmlFor="ingest-schedule" className={page ? "sr-only" : "text-xs"}>
        Schedule
      </Label>
      <select
        id="ingest-schedule"
        className={cn("h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm", page && "max-w-xs")}
        value={cronChoice}
        onChange={(e) => setCronChoice(e.target.value)}
      >
        {SCHEDULE_PRESETS.map((p) => (
          <option key={p.label} value={p.value}>
            {p.label}
          </option>
        ))}
        <option value="custom">Custom cron…</option>
      </select>
      {cronChoice === "custom" ? (
        <Input
          aria-label="Cron expression"
          value={customCron}
          onChange={(e) => setCustomCron(e.target.value)}
          placeholder="30 1 * * 1-5"
          className={cn("h-8 font-mono text-xs", page && "max-w-xs")}
          autoComplete="off"
        />
      ) : null}
      {scheduleProblem ? (
        <p className="text-xs text-destructive">{scheduleProblem}</p>
      ) : cronChoice === "custom" ? (
        <p className="text-xs text-muted-foreground">Minute, hour, day, month, weekday — in UTC.</p>
      ) : null}
      {cron !== savedCron ? (
        cron ? (
          <p className="text-xs text-muted-foreground">Runs on this schedule within a minute of saving.</p>
        ) : null
      ) : spec.nextRunAt ? (
        <p className="flex items-center gap-1.5 text-xs text-muted-foreground">
          <CalendarClockIcon className="size-3.5 shrink-0" />
          <span>
            Next run{" "}
            <span className="font-medium text-foreground">{formatDateTime(spec.nextRunAt)}</span> your time
            ({formatTimeUntil(spec.nextRunAt)})
          </span>
        </p>
      ) : null}
    </div>
  ) : null

  const saveRow = (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" size="sm" disabled={!canSave || save.status === "pending"} onClick={async () => {
            const result = await save.run()
            if (result) onSaved(result)
          }}>
          {save.status === "pending" ? "Saving…" : "Save tables and schedule"}
        </Button>
        {dirty ? (
          <Button type="button" size="sm" variant="ghost" onClick={() => {
            setSelection(saved)
            setCronChoice(SCHEDULE_PRESETS.some((p) => p.value === savedCron) ? savedCron : "custom")
            setCustomCron(SCHEDULE_PRESETS.some((p) => p.value === savedCron) ? "" : savedCron)
          }}>
            Discard changes
          </Button>
        ) : null}
      </div>
      {save.status === "error" ? <p className="text-xs text-destructive">{save.error.message}</p> : null}
    </>
  )

  const runs = (
    <IngestRuns
      layout={layout}
      connectorId={connectorId}
      tableCount={saved.length}
      blockedReason={
        dirty
          ? "Save your changes before running."
          : !isStream && saved.length === 0
            ? "Save at least one table to run."
            : null
      }
    />
  )

  if (page) {
    // Cards, two columns once the page is wide: what is ingested and when on
    // the left, what has run on the right. The one Save button sits in the
    // Schedule card and saves both; CDC has no schedule, so there it ends the
    // tables card.
    return (
      <div className="grid items-start gap-3 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
        <div className="flex min-w-0 flex-col gap-3">
          <SectionCard size="sm" title="Tables to ingest" description={isStream ? undefined : tablesIntro}>
            <div className="space-y-4">
              {isStream ? <p className="text-sm text-muted-foreground">{tablesIntro}</p> : null}
              {cdcNotes}
              {tablesBody}
              {isCdc ? saveRow : null}
            </div>
          </SectionCard>
          {!isCdc ? (
            <SectionCard size="sm" title="Schedule">
              <div className="space-y-4">
                {scheduleBlock}
                {saveRow}
              </div>
            </SectionCard>
          ) : null}
        </div>
        {runs}
      </div>
    )
  }

  return (
    <div className="space-y-5">
      <div className="space-y-1">
        <h3 className="flex items-center gap-2 text-sm font-medium">
          <TableIcon className="size-4 text-muted-foreground" />
          Tables to ingest
        </h3>
        <p className="text-xs text-muted-foreground">{tablesIntro}</p>
      </div>
      {cdcNotes}
      {tablesBody}
      {scheduleBlock}
      {saveRow}
      {runs}
    </div>
  )
}

/**
 * What a run does with one table's rows: replace them, add only new ones
 * (by a cursor column), or add everything again. The cursor is picked from
 * the table's columns once it has been found, and typed otherwise.
 */
function LoadModeFields({
  selected,
  modes,
  columns,
  problem,
  onChange,
}: {
  selected: Selected
  modes: LoadMode[]
  columns: string[] | null
  problem: string | null
  onChange: (patch: Partial<Selected>) => void
}) {
  const cursor = selected.incrementalKey ?? ""
  // A saved mode this adapter no longer offers still shows, rather than
  // silently reading as another one.
  const offered = modes.includes(selected.loadMode) ? modes : [...modes, selected.loadMode]
  const fieldClass = "h-7 rounded-lg border border-input bg-transparent px-2 text-xs"
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="shrink-0 text-xs text-muted-foreground">Each run</span>
        <select
          aria-label={`Load mode for ${selected.name}`}
          className={fieldClass}
          value={selected.loadMode}
          onChange={(e) => onChange({ loadMode: e.target.value as LoadMode })}
        >
          {offered.map((mode) => (
            <option key={mode} value={mode}>
              {LOAD_MODE_LABEL[mode]}
            </option>
          ))}
        </select>
        {selected.loadMode === "incremental" ? (
          <>
            <span className="shrink-0 text-xs text-muted-foreground">by</span>
            {columns && columns.length > 0 ? (
              <select
                aria-label={`New-row column for ${selected.name}`}
                className={`${fieldClass} min-w-0 flex-1 font-mono`}
                value={cursor}
                aria-invalid={problem !== null}
                onChange={(e) => onChange({ incrementalKey: e.target.value })}
              >
                <option value="">Pick a column…</option>
                {(columns.includes(cursor) || cursor === "" ? columns : [cursor, ...columns]).map((column) => (
                  <option key={column} value={column}>
                    {column}
                  </option>
                ))}
              </select>
            ) : (
              <Input
                aria-label={`New-row column for ${selected.name}`}
                value={cursor}
                onChange={(e) => onChange({ incrementalKey: e.target.value })}
                placeholder="updated_at"
                className="h-7 min-w-0 flex-1 font-mono text-xs"
                aria-invalid={problem !== null}
                autoComplete="off"
              />
            )}
          </>
        ) : null}
      </div>
      <p className={problem ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>
        {problem ?? loadModeHint(selected.loadMode, cursor.trim())}
      </p>
    </div>
  )
}

/** How often a run in progress is checked on. */
const POLL_MS = 3_000

/** How long a run just launched is waited for before Dagster lists it. */
const LAUNCH_GRACE_MS = 60_000

export const RUN_STATUS_LABEL: Record<IngestJobRun["status"], string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
  cancelled: "Cancelled",
  unknown: "Unknown",
}

export const RUN_STATUS_TONE: Record<IngestJobRun["status"], "success" | "destructive" | "warning" | "neutral" | "info"> = {
  queued: "neutral",
  running: "info",
  completed: "success",
  failed: "destructive",
  cancelled: "warning",
  unknown: "neutral",
}

export function isActive(run: IngestJobRun): boolean {
  return run.status === "queued" || run.status === "running"
}

/** Epoch milliseconds, or `null`. The per-table results carry
 * microseconds, which not every engine's `Date` parses. */
function toMillis(iso: string | null): number | null {
  if (!iso) return null
  const t = parseTimestamp(iso.replace(/(\.\d{3})\d+/, "$1")).getTime()
  return Number.isNaN(t) ? null : t
}

export type RunGroup = { run: IngestJobRun | null; results: IngestRun[] }

/**
 * Each run (newest first) with the table results recorded while it ran,
 * in the order the tables were loaded. A result is matched to a run by
 * time, since the results do not carry a run id; runs of one connector
 * never overlap (the API refuses a second one). Results older than every
 * listed run come last, under `run: null`.
 */
export function groupResultsByRun(runs: IngestJobRun[], results: IngestRun[]): RunGroup[] {
  const slack = 2_000
  const groups: RunGroup[] = runs.map((run) => ({ run, results: [] }))
  const earlier: IngestRun[] = []
  for (const result of results) {
    const at = toMillis(result.startedAt)
    const group =
      at === null
        ? undefined
        : groups.find(({ run }) => {
            const start = toMillis(run?.startedAt ?? null)
            if (start === null) return false
            const end = toMillis(run?.endedAt ?? null) ?? Number.POSITIVE_INFINITY
            return at >= start - slack && at <= end + slack
          })
    ;(group ? group.results : earlier).push(result)
  }
  const byTime = (x: IngestRun, y: IngestRun) => (toMillis(x.startedAt) ?? 0) - (toMillis(y.startedAt) ?? 0)
  for (const group of groups) group.results.sort(byTime)
  earlier.sort((x, y) => byTime(y, x))
  return earlier.length > 0 ? [...groups, { run: null, results: earlier }] : groups
}

/**
 * The connector's runs, each with the tables it loaded, and a Run now
 * that stays disabled while a run is going: Bronze is append-only, so a
 * second run would copy every table in again. While a run is going, both
 * lists are checked every few seconds.
 */
function IngestRuns({
  layout,
  connectorId,
  tableCount,
  blockedReason,
}: {
  layout: "page" | "embedded"
  connectorId: string
  tableCount: number
  blockedReason: string | null
}) {
  const jobRuns = useRefreshable((s) => connectorService.listIngestJobRuns(connectorId, s), connectorId)
  const results = useRefreshable((s) => connectorService.listIngestRuns(connectorId, s), connectorId)
  const runNow = useServiceAction((signal, id: string) => connectorService.runIngest(id, signal))
  // A run just launched, until Dagster lists it.
  const [launchedRunId, setLaunchedRunId] = React.useState<string | null>(null)

  const runs = jobRuns.data ?? []
  const activeRun = runs.find(isActive) ?? null
  const awaitingLaunch = launchedRunId !== null && !runs.some((r) => r.runId === launchedRunId)
  const watching = activeRun !== null || awaitingLaunch
  const { refresh: refreshRuns } = jobRuns
  const { refresh: refreshResults } = results

  React.useEffect(() => {
    if (!watching) return
    const timer = window.setInterval(() => {
      refreshRuns()
      refreshResults()
    }, POLL_MS)
    return () => window.clearInterval(timer)
  }, [watching, refreshRuns, refreshResults])

  // Stop waiting for a launched run Dagster never lists.
  React.useEffect(() => {
    if (!launchedRunId) return
    const timer = window.setTimeout(() => setLaunchedRunId(null), LAUNCH_GRACE_MS)
    return () => window.clearTimeout(timer)
  }, [launchedRunId])

  // A run that just finished: one last read, so its final tables show.
  const wasWatching = React.useRef(false)
  React.useEffect(() => {
    if (wasWatching.current && !watching) refreshResults()
    wasWatching.current = watching
  }, [watching, refreshResults])

  const groups = groupResultsByRun(runs, results.data ?? [])

  const runNowButton = (
    <Button
      type="button"
      size="sm"
      variant="outline"
      disabled={runNow.status === "pending" || watching || blockedReason !== null}
      onClick={async () => {
        const result = await runNow.run(connectorId)
        if (result?.runId) setLaunchedRunId(result.runId)
        refreshRuns()
      }}
    >
      <PlayIcon data-icon="inline-start" />
      {runNow.status === "pending" ? "Starting…" : "Run now"}
    </Button>
  )

  const body = (
    <>
      {blockedReason && !watching ? <p className="text-xs text-muted-foreground">{blockedReason}</p> : null}
      {activeRun ? (
        <p className="flex items-center gap-1.5 text-xs text-primary">
          <LoaderIcon className="size-3.5 animate-spin" />
          Run {activeRun.runId.slice(0, 8)} is {activeRun.status}. Tables appear below as each one is loaded.
        </p>
      ) : awaitingLaunch ? (
        <p className="flex items-center gap-1.5 text-xs text-primary">
          <LoaderIcon className="size-3.5 animate-spin" />
          Starting run {launchedRunId?.slice(0, 8)}…
        </p>
      ) : null}
      {runNow.status === "error" ? <p className="text-xs text-destructive">{runNow.error.message}</p> : null}
      {runNow.data?.supported === false ? (
        <p className="text-xs text-muted-foreground">Not runnable · {runNow.data.reason}</p>
      ) : null}

      {jobRuns.error ? (
        <ErrorState error={jobRuns.error} onRetry={refreshRuns} />
      ) : jobRuns.data === null || (results.data === null && results.error === null) ? (
        <LoadingSkeleton rows={2} />
      ) : groups.length === 0 ? (
        <p className="text-xs text-muted-foreground">No runs yet.</p>
      ) : (
        <div className="space-y-2">
          {results.error ? (
            <p className="text-xs text-destructive">Table results could not be loaded · {results.error.message}</p>
          ) : null}
          {groups.map((group, index) => (
            <RunGroupCard
              key={group.run?.runId ?? "earlier"}
              layout={layout}
              group={group}
              tableCount={tableCount}
              defaultOpen={index === 0}
            />
          ))}
        </div>
      )}
    </>
  )

  if (layout === "page") {
    return (
      <SectionCard size="sm" title="Runs" action={runNowButton}>
        <div className="space-y-3">{body}</div>
      </SectionCard>
    )
  }

  return (
    <div className="space-y-3 border-t border-border pt-4">
      <div className="flex items-center justify-between gap-2">
        <p className="text-xs font-medium text-muted-foreground">Runs</p>
        {runNowButton}
      </div>
      {body}
    </div>
  )
}

function RunGroupCard({
  layout,
  group,
  tableCount,
  defaultOpen,
}: {
  layout: "page" | "embedded"
  group: RunGroup
  tableCount: number
  defaultOpen: boolean
}) {
  const { run, results } = group
  const loaded = results.filter((r) => r.status === "succeeded")
  const notLoaded = results.length - loaded.length
  const rows = loaded.reduce((sum, r) => sum + (r.rows ?? 0), 0)
  const start = toMillis(run?.startedAt ?? null)
  const end = toMillis(run?.endedAt ?? null)

  const summary = !run
    ? `${results.length} older ${results.length === 1 ? "result" : "results"}`
    : isActive(run)
      ? `${results.length} of ${tableCount} tables so far`
      : results.length === 0
        ? "No table reached"
        : `${loaded.length} of ${results.length} tables · ${rows.toLocaleString()} rows${
            notLoaded > 0 ? ` · ${notLoaded} not loaded` : ""
          }`

  const status = run ? (
    <Pill tone={RUN_STATUS_TONE[run.status]}>{RUN_STATUS_LABEL[run.status]}</Pill>
  ) : (
    <Pill tone="neutral">Earlier</Pill>
  )
  const runId = run ? <span className="font-mono text-xs">{run.runId.slice(0, 8)}</span> : null
  const when = run ? (
    <>
      {formatRelativeTime(run.startedAt)}
      {start !== null && end !== null ? ` · took ${formatDuration(end - start)}` : ""}
    </>
  ) : null
  const summaryClass =
    "flex cursor-pointer list-none gap-2 px-3 py-2 outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 [&::-webkit-details-marker]:hidden"
  const chevron = "size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90"

  return (
    <details open={defaultOpen} className="group rounded-md border border-border">
      {layout === "page" ? (
        // In the half-width Runs card one line cannot hold the pill, the id,
        // when and how long, and the summary: the first line is the verdict
        // and the summary (not truncated), the second is when and how long.
        // Spans, since a summary only takes phrasing content.
        <summary className={cn(summaryClass, "items-start")}>
          <ChevronRightIcon className={cn(chevron, "mt-[3px]")} />
          <span className="min-w-0 flex-1 space-y-0.5">
            <span className="flex flex-wrap items-center gap-x-2 gap-y-0.5">
              {status}
              {runId}
              <span className="ml-auto text-xs text-muted-foreground">{summary}</span>
            </span>
            {when ? <span className="block text-xs text-muted-foreground">{when}</span> : null}
          </span>
        </summary>
      ) : (
        <summary className={cn(summaryClass, "items-center")}>
          <ChevronRightIcon className={chevron} />
          {status}
          {runId}
          {when ? <span className="text-xs text-muted-foreground">{when}</span> : null}
          <span className="ml-auto truncate pl-2 text-xs text-muted-foreground">{summary}</span>
        </summary>
      )}
      <div className="border-t border-border">
        {results.length === 0 ? (
          <p className="px-3 py-2 text-xs text-muted-foreground">
            {run && run.status === "failed" ? (
              <>
                This run failed before it reached any table. Its log on the{" "}
                <Link href="/pipelines/ingest_job" className="text-primary hover:underline">
                  ingest_job pipeline
                </Link>{" "}
                says why.
              </>
            ) : run && isActive(run) ? (
              "Waiting for the first table…"
            ) : (
              "No table results were recorded for this run."
            )}
          </p>
        ) : (
          <ul className="divide-y divide-border">
            {results.map((r) => (
              <li key={`${r.object}-${r.startedAt}`} className="space-y-0.5 px-3 py-1.5">
                <div className="flex items-center gap-2">
                  <span className="min-w-0 flex-1 truncate font-mono text-xs" title={r.object}>
                    {r.object}
                  </span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    {r.rows === null ? "rows not measured" : `${r.rows.toLocaleString()} rows`}
                  </span>
                  <Pill tone={RUN_TONE[r.status] ?? "neutral"}>{r.status}</Pill>
                </div>
                {r.error ? (
                  <p className="line-clamp-2 text-xs text-destructive" title={r.error}>
                    {r.error}
                  </p>
                ) : null}
              </li>
            ))}
          </ul>
        )}
      </div>
    </details>
  )
}

/**
 * Read-only `Debezium` `.properties` rendering for a `cdc` adapter's
 * captured table. `properties` holds ONLY `${ENV_VAR_NAME}` references —
 * labeled explicitly as such, never resolved here or anywhere in the
 * console.
 */
function DebeziumPanel({ connectorId, table }: { connectorId: string; table: string }) {
  const props = useService(
    (signal) => connectorService.getDebeziumProperties(connectorId, table, signal),
    [connectorId, table]
  )

  if (props.status === "loading") return <LoadingSkeleton rows={2} />
  if (props.status === "error") return <ErrorState error={props.error} onRetry={props.reload} />

  return (
    <div>
      <p className="text-xs font-medium text-muted-foreground">Debezium properties · {props.data.table}</p>
      <p className="mt-0.5 text-xs text-muted-foreground">{props.data.note}</p>
      <pre className="mt-1.5 overflow-x-auto rounded-md border bg-muted p-2 font-mono text-xs">
        {props.data.properties}
      </pre>
    </div>
  )
}
