"use client"

import * as React from "react"
import Link from "next/link"
import {
  CalendarClockIcon,
  ChevronRightIcon,
  DownloadIcon,
  LoaderIcon,
  PlayIcon,
  PlusIcon,
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
import { specWithTopic } from "@/lib/connectors/kafka-topic"
import { formatDateTime, formatDuration, formatRelativeTime, formatTimeUntil, parseTimestamp } from "@/lib/format"
import { notifySuccess, withNotify } from "@/lib/notify"
import { cn } from "@/lib/utils"
import { connectorService } from "@/services"
import { TableActionsMenu } from "./connector-table-actions"
import { Ga4ReportEditor } from "./ga4-report-editor"
import { WorkdayReportEditor } from "./workday-report-editor"
import { SharepointSitePicker } from "./sharepoint-site-picker"
import { FINDER_WORDS, finderKind, TableFinder } from "./table-finder"
import { useColumnPreviews, type PreviewEntry } from "./use-discovery"
import type {
  ConnectorRun,
  Dial,
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

/**
 * The Bronze table of a listed object that came with a label (`SRC-3` slice 5 review BLOCKER 14: a SharePoint
 * list is named by two ids and labelled `Site / List`): `<connector>_<label>`, the whole label slugged, because
 * a label is words and not a `schema.table` whose last part names the table. The source-object key stays the ids,
 * so renaming the list at the source changes neither the key nor the table already created.
 */
export function labelTarget(connectorName: string, label: string): string {
  const joined = [slug(connectorName), slug(label)].filter(Boolean).join("_") || "table"
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

type Selected = {
  name: string
  target: string
  loadMode: LoadMode
  incrementalKey?: string
  /** SRC-10: the columns that identify one row, for the two deduplicating modes. Kept whatever the mode, so a key
   * filled from discovery survives a switch to a mode that does not use it (the API ignores it there). */
  primaryKey?: string[]
  /** SRC-11: the columns left out of the load (SQL sources whose columns can be listed). */
  excludedColumns?: string[]
}
type Selection = Selected[]

/** What a table gets when it is ticked: one copy of the source per run. */
const DEFAULT_LOAD_MODE: LoadMode = "replace"

/** `SRC-4` decision S2-g: a change-capture table keeps the current row per key unless history is chosen. */
const defaultLoadMode = (adapter: string): LoadMode => (adapter === "cdc" ? "incremental_dedup" : DEFAULT_LOAD_MODE)

const LOAD_MODE_LABEL: Record<LoadMode, string> = {
  replace: "Replace all rows",
  incremental: "Add only new rows",
  append: "Add all rows again",
  replace_dedup: "Replace all rows, one per key",
  incremental_dedup: "Add only new rows, one per key",
  history: "Keep history",
}

/** `SRC-4` decision S2-g: the two modes change capture offers, in the words the product owner signed. */
const CHANGE_CAPTURE_LABEL: Partial<Record<LoadMode, string>> = {
  incremental_dedup: "Current row per key",
  history: "Keep history",
}

const modeLabel = (mode: LoadMode, changeCapture: boolean): string =>
  (changeCapture ? CHANGE_CAPTURE_LABEL[mode] : undefined) ?? LOAD_MODE_LABEL[mode]

/** What a mode does for change capture, in one line under the choice. */
function changeCaptureHint(mode: LoadMode): string {
  if (mode === "history") {
    return "Every change is kept as a row, with its operation (_op), the time it happened at the source (_source_ts_ms), whether it deleted the row (_deleted) and its position (_offset)."
  }
  return "The table holds the current row of each key: a change updates the row and a delete removes it. A first copy of the source is taken when capture starts."
}

/** What a mode does on each run, in one line under the choice, including what happens to the rows already there. */
function loadModeHint(mode: LoadMode, cursor: string): string {
  switch (mode) {
    case "replace":
      return "Each run replaces the rows already in the table with a fresh copy of the source."
    case "incremental":
      return `The first run replaces the table's rows with a copy of the source; later runs add rows whose ${
        cursor || "column"
      } is higher than the last one seen and keep the rows already there. A changed row is added again as a new version.`
    case "append":
      return "Each run adds the whole source below the rows already in the table, so the table grows by a full copy each time."
    case "replace_dedup":
      return "Each run replaces the rows already in the table with a fresh copy of the source, one row per key: the one with the highest value of the cursor column, or the last one read."
    case "incremental_dedup":
      return `The first run replaces the table's rows with a copy of the source, one row per key; later runs merge rows whose ${
        cursor || "column"
      } is higher than the last one seen into the rows already there, updating the row of a key it already holds.`
    case "history":
      return "Every version of a row is kept, with the time a load first saw it (_valid_from), the time it was replaced (_valid_to) and a mark on the current one (_is_current). Each run reads the whole table."
  }
}

/**
 * The modes a source offers. Replace, append, replace-and-deduplicate and history run on
 * every batch source (history reads the whole table, so it needs no cursor); the two incremental ones are pushed into a SQL query, so
 * no other adapter runs them (SRC-10 decision 2).
 */
function loadModesFor(adapter: string): LoadMode[] {
  // SRC-4 decision S2-g: change capture keeps the current row per key (the default) or every change.
  if (adapter === "cdc") return ["incremental_dedup", "history"]
  // SRC-3 AD6: a business-app object loads incrementally by the cursor its vendor declares (the finder shows
  // which objects have one). PR #216 final review BLOCKER 18: plain `incremental` appends a changed record as
  // a second row, so `saas` offers only the mode that merges on the vendor's key.
  if (adapter === "saas") return ["replace", "append", "replace_dedup", "incremental_dedup", "history"]
  return adapter === "sql"
    ? ["replace", "incremental", "append", "replace_dedup", "incremental_dedup", "history"]
    : ["replace", "append", "replace_dedup", "history"]
}

/** Why a source cannot offer the incremental modes, or `null` when it can. */
function missingModesNote(modes: LoadMode[], changeCapture = false): string | null {
  if (changeCapture) return null
  return modes.includes("incremental") || modes.includes("incremental_dedup")
    ? null
    : "Adding only new rows, with or without one row per key, is offered for SQL sources and business apps only: they can ask the source for just the new rows."
}

const isDedup = (mode: LoadMode) => mode === "replace_dedup" || mode === "incremental_dedup"
/** The modes that identify a row by its key: the deduplicating ones, and history (SRC-10 decisions 1 and 3). */
const needsKey = (mode: LoadMode) => isDedup(mode) || mode === "history"

/**
 * Said under the mode when the chosen mode and the saved one are on different sides of the history boundary
 * (SRC-10 decision 1). The console cannot know whether the table has rows, so it says what to do either way: a
 * history table's rows cannot be kept as another mode's, nor the reverse.
 */
const HISTORY_BOUNDARY_NOTE =
  "After saving, use Refresh, removing rows on this table: its rows must be rebuilt for the new mode."
const crossesHistoryBoundary = (saved: LoadMode | null, chosen: LoadMode) =>
  saved !== null && (saved === "history") !== (chosen === "history")

/** Why a table's load mode cannot be saved, or `null`. */
export function loadModeProblem(selected: {
  loadMode: LoadMode
  incrementalKey?: string
  primaryKey?: string[]
  /** The connector's adapter, when the caller knows it: `saas` cannot save plain `incremental`. */
  adapter?: string
}): string | null {
  // The API refuses each of these (`validate_load_modes`), so they are shown before saving (SRC-10 decision 3).
  // One at a time, the cursor first: it is the one the field next to the mode asks for.
  // PR #216 final review BLOCKER 18: plain `incremental` appends a changed business-app record as a second row.
  if (selected.adapter === "saas" && selected.loadMode === "incremental") {
    return "A business app can only add new rows one per key: choose “Add only new rows, one per key”."
  }
  if (
    selected.adapter !== "cdc" &&
    (selected.loadMode === "incremental" || selected.loadMode === "incremental_dedup") &&
    !(selected.incrementalKey ?? "").trim()
  ) {
    return "Pick the column that marks new rows, such as an id or an updated-at time."
  }
  if (needsKey(selected.loadMode) && (selected.primaryKey ?? []).length === 0) {
    if (selected.adapter === "cdc") {
      return "Name the column, or the columns together, that identify one row. Change capture cannot follow a table without a key."
    }
    return "Name the column, or the columns together, that identify one row. A mode with one row per key cannot be saved without a key."
  }
  return null
}

/** Said under the box an Oracle change-capture connector's tables are typed in (`SRC-4` Amendment 10, S10-c). */
export const ORACLE_BY_HAND_NOTE =
  "Type each table as SCHEMA.TABLE, as Oracle names it (upper case). The names are not checked against the source until you test the connection: a name that does not exist captures nothing."

/**
 * `SRC-11` decision 9: the sources whose columns the product lists before loading, so a column can be left
 * out: `sql` connectors on the PostgreSQL, MySQL (MariaDB uses it), SQL Server or Oracle driver (Oracle since
 * `SRC-4` T1.2: `SRC-13` lists its columns and the orchestrator maps their spelling to the reflected one, as
 * `supports_column_choice` in `ingest_spec.rs` says); files, REST, MongoDB, Kafka and SFTP have no declared column
 * list to choose from. Business apps (`saas`, `SRC-3`
 * AD6) are offered it too: a vendor that describes its fields lists them. The columns come from the
 * preview route (`useColumnPreviews`), also for a table saved in an earlier visit.
 */
export function supportsColumnChoice(adapter: string, dial: Dial | null | undefined): boolean {
  const driver = typeof dial?.driver === "string" ? dial.driver : ""
  return adapter === "saas" || (adapter === "sql" && (driver === "postgres" || driver === "mysql" || driver === "mssql" || driver === "oracle"))
}

/**
 * The columns a table cannot leave out, with the reason (`SRC-11` decision 11): the key of a table that
 * deduplicates or keeps history, and the cursor of an incremental one. The API answers 422 for either.
 */
export function lockedColumns(s: {
  loadMode: LoadMode
  incrementalKey?: string
  primaryKey?: string[]
}): Map<string, string> {
  const locked = new Map<string, string>()
  if (needsKey(s.loadMode)) {
    for (const column of s.primaryKey ?? []) locked.set(column, "Key column: it identifies a row of this table.")
  }
  if ((s.loadMode === "incremental" || s.loadMode === "incremental_dedup") && s.incrementalKey?.trim()) {
    locked.set(s.incrementalKey.trim(), "Cursor column: it marks which rows are new.")
  }
  return locked
}

/**
 * What is saved as left out: the user's choice minus the columns the table's current mode locks, so choosing
 * a mode or a key that needs a column the user had unticked never sends the API a combination it refuses.
 */
function effectiveExcluded(s: Selected): string[] {
  const locked = lockedColumns(s)
  return (s.excludedColumns ?? []).filter((c) => !locked.has(c))
}

function fromSpec(spec: IngestSpec): Selection {
  return spec.sourceObjects.map((o) => ({
    name: o.name,
    target: o.target,
    // Saved before load modes existed: the ingest job replaces; change capture keeps the current row per key.
    loadMode: o.loadMode ?? defaultLoadMode(spec.adapter ?? ""),
    incrementalKey: o.incrementalKey,
    primaryKey: o.primaryKey,
    excludedColumns: o.excludedColumns,
  }))
}

/** `withCursor: false` for change capture, whose tables follow the source's changes and read no cursor column. */
function toSourceObjects(selection: Selection, withCursor: boolean): SourceObject[] {
  return selection.map((s) => ({
    name: s.name,
    target: s.target,
    loadMode: s.loadMode,
    ...(withCursor && (s.loadMode === "incremental" || s.loadMode === "incremental_dedup") && s.incrementalKey?.trim()
      ? { incrementalKey: s.incrementalKey.trim() }
      : {}),
    ...(s.primaryKey && s.primaryKey.length > 0 ? { primaryKey: s.primaryKey } : {}),
    ...(effectiveExcluded(s).length > 0 ? { excludedColumns: effectiveExcluded(s) } : {}),
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
  pausedReason = null,
  waitingTables,
  onResumed,
}: {
  connectorId: string
  connectorName: string
  layout?: "page" | "embedded"
  onTableCount?: (count: number) => void
  /** Called after Resume lifted a pause (`SRC-11`), so the page reloads the connector and the pause goes. */
  onResumed?: () => void
  /** Why the connector is paused (`SRC-8`); the API refuses a run meanwhile. */
  pausedReason?: string | null
  /** Source tables that wait for a schema decision (`SRC-8`), from the Schema changes tab's one list. */
  waitingTables?: ReadonlySet<string>
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
      pausedReason={pausedReason}
      waitingTables={waitingTables}
      onResumed={onResumed}
    />
  )
}

function IngestEditor({
  connectorId,
  connectorName,
  spec,
  layout,
  onSaved,
  pausedReason,
  waitingTables,
  onResumed,
}: {
  connectorId: string
  connectorName: string
  spec: IngestSpec
  layout: "page" | "embedded"
  onSaved: (spec: IngestSpec) => void
  pausedReason: string | null
  waitingTables?: ReadonlySet<string>
  onResumed?: () => void
}) {
  const adapter = spec.adapter ?? ""
  const isCdc = adapter === "cdc"
  const isStream = adapter === "kafka"
  // SRC-13 D1 to D8: every type but Google Sheets has a Find flow; Kafka's picks its one topic (D4).
  const finder = finderKind(adapter, spec.dial)
  // Typed names stay possible next to a list that is cut or incomplete (and are the only way for a type
  // that cannot list); a SQL source and Kafka take no typed name.
  // Slice 3 review BLOCKER 9: a Google Analytics connector has no tables to type or tick; the person defines reports.
  const isGa4 = adapter === "saas" && spec.dial.vendor === "ga4"
  // Slice 5 review BLOCKER 14: a SharePoint connector's sites are picked, and the picks are the dial's `objects`.
  const isSharepoint = adapter === "saas" && spec.dial.vendor === "sharepoint"
  // Slice 6: a Workday connector's reports are named by owner and name, and are the dial's `objects` too.
  const isWorkday = adapter === "saas" && spec.dial.vendor === "workday"
  const keepsDialObjects = isGa4 || isSharepoint || isWorkday
  const manualAdd = finder !== "sql" && finder !== "kafka" && !isGa4 && !isWorkday
  const savedReports = React.useMemo(
    () => (Array.isArray(spec.dial.objects) ? spec.dial.objects.filter((o: unknown): o is string => typeof o === "string") : []),
    [spec.dial]
  )
  const [reports, setReports] = React.useState<string[]>(savedReports)
  // The labels the finder listed, to name a Bronze table and to show a picked object in words (BLOCKER 14).
  const [labels, setLabels] = React.useState<Record<string, string>>({})
  const rememberLabels = (found: Record<string, string>) =>
    setLabels((current) => (Object.entries(found).every(([k, v]) => current[k] === v) ? current : { ...current, ...found }))
  const previews = useColumnPreviews(connectorId)
  const columnChoice = !isCdc && supportsColumnChoice(adapter, spec.dial)
  // Owned here, not by the Runs card: the per-table actions are disabled while a run is going.
  const runState = useIngestRunState(connectorId)

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

  const [manualName, setManualName] = React.useState("")
  // `SRC-4` Amendment 10 (S10-c): an Oracle change-capture connector lists nothing; each table is typed as
  // SCHEMA.TABLE, in the upper case Oracle stores it in, and is not checked against the source until capture starts.
  const oracleByHand = isCdc && spec.dial.driver === "oracle"
  const save = useServiceAction(
    withNotify({ success: "Tables and schedule saved", error: "Could not save tables" }, (signal) =>
      connectorService.setIngestSpec(
        connectorId,
        {
          adapter,
          ingestMode: spec.ingestMode ?? "batch",
          dial: keepsDialObjects ? { ...spec.dial, objects: reports } : spec.dial,
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
  const modeProblems = selection.map((s) => loadModeProblem({ ...s, adapter }))
  const scheduleProblem = !isCdc && cronChoice === "custom" ? cronProblem(customCron) : null
  const dirty =
    JSON.stringify(toSourceObjects(selection, !isCdc)) !== JSON.stringify(toSourceObjects(saved, !isCdc)) ||
    (!isCdc && cron !== savedCron) ||
    (keepsDialObjects && JSON.stringify(reports) !== JSON.stringify(savedReports))
  const canSave =
    dirty && problems.every((p) => p === null) && modeProblems.every((p) => p === null) && scheduleProblem === null

  // The columns of the tables read through the preview route, to pick a cursor column from (SRC-13 D6).
  const columnsOf = new Map<string, string[]>()
  // The key each of them declares at the source (SQL drivers); absent when it declares none.
  const keysOf = new Map<string, string[]>()
  for (const [name, entry] of previews.entries) {
    if (entry.status !== "done" || entry.columns.length === 0) continue
    columnsOf.set(name, entry.columns.map((c) => c.name))
    if (entry.primaryKey.length > 0) keysOf.set(name, entry.primaryKey)
  }
  // Whether the columns of a table of this connector are read ahead, for a key or a cursor to pick from:
  // only where the API answers a preview in the call itself (SQL), so ticking a table does not start a job.
  const readsColumnsAhead = finder === "sql"
  const wantsColumns = (s: Selected) => s.loadMode !== "replace" && s.loadMode !== "append"
  // SRC-13 D6: the columns of a SQL table are read when it is ticked or its mode needs them, one request
  // at a time; `ensure` asks for a name once.
  React.useEffect(() => {
    if (!readsColumnsAhead) return
    for (const s of selection) if (wantsColumns(s)) previews.ensure(s.name)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [readsColumnsAhead, selection, previews.ensure])

  function toggle(name: string, label?: string) {
    if (label) rememberLabels({ [name]: label })
    setSelection((current) =>
      current.some((s) => s.name === name)
        ? current.filter((s) => s.name !== name)
        : [...current, newTable(name, label)]
    )
    // A ticked SQL table's key comes with its columns; the mode switch below fills it in (SRC-10 decision 3).
    if (readsColumnsAhead && !selectedNames.has(name)) previews.ensure(name)
  }

  /** Tick or untick every one of `names`: the button over the shown rows of the list. */
  function setMany(names: string[], on: boolean, found: Record<string, string> = {}) {
    rememberLabels(found)
    setSelection((current) => {
      if (!on) return current.filter((s) => !names.includes(s.name))
      const have = new Set(current.map((s) => s.name))
      return [...current, ...names.filter((n) => !have.has(n)).map((n) => newTable(n, found[n]))]
    })
  }

  /** A ticked table: one copy of the source per run, with the key the source declares when it declares one. */
  function newTable(name: string, label?: string): Selected {
    const detected = keysOf.get(name)
    return {
      name,
      target: label ? labelTarget(connectorName, label) : defaultTarget(connectorName, name),
      loadMode: defaultLoadMode(adapter),
      ...(detected ? { primaryKey: detected } : {}),
    }
  }

  function change(name: string, patch: Partial<Selected>) {
    setSelection((current) =>
      current.map((s) => {
        if (s.name !== name) return s
        const next = { ...s, ...patch }
        // Switching to a mode with one row per key fills the key from the source's own, when it has one and
        // none is set (SRC-10 decision 3). The user's own choice is never overwritten.
        const detected = keysOf.get(name)
        if (patch.loadMode && needsKey(patch.loadMode) && (next.primaryKey ?? []).length === 0 && detected) {
          next.primaryKey = detected
        }
        return next
      })
    )
  }

  /** A report defined in the editor: kept in the dial's `objects` and ticked as an object that continues from its date. */
  function addReport(text: string, cursor: string, key: string[]) {
    setReports((current) => (current.includes(text) ? current : [...current, text]))
    const label = text.slice(0, text.indexOf("=")).trim()
    setSelection((current) =>
      current.some((s) => s.name === text)
        ? current
        : [...current, { ...newTable(text), target: defaultTarget(connectorName, label), loadMode: "incremental_dedup", incrementalKey: cursor, primaryKey: key }]
    )
  }

  /** A Workday report: kept in the dial's `objects` and ticked, read in full on every run (the default mode replaces the table). */
  function addWorkdayReport(text: string) {
    setReports((current) => (current.includes(text) ? current : [...current, text]))
    setSelection((current) => (current.some((s) => s.name === text) ? current : [...current, newTable(text, text.slice(text.indexOf("/") + 1))]))
  }

  function removeReport(text: string) {
    setReports((current) => current.filter((r) => r !== text))
    setSelection((current) => current.filter((s) => s.name !== text))
  }

  function addManual() {
    const typed = manualName.trim()
    // Oracle stores unquoted names in upper case and the API requires that spelling (S10-c).
    const name = oracleByHand ? typed.toUpperCase() : typed
    if (!name || selectedNames.has(name)) return
    setSelection((current) => [...current, newTable(name)])
    setManualName("")
  }

  // SRC-13 D3: the endpoints a REST connector declares, to tell a table that names one from one saved before.
  const declaredEndpoints = Array.isArray(spec.dial.endpoints)
    ? spec.dial.endpoints.flatMap((e: unknown) =>
        e && typeof e === "object" && typeof (e as { path?: unknown }).path === "string"
          ? [(e as { path: string }).path]
          : []
      )
    : []

  /** Why a table cannot be refreshed or cleared now, or `null` (SRC-10 decision 5). */
  function tableActionBlock(name: string): string | null {
    if (!saved.some((t) => t.name === name) || dirty) return "Save your changes before refreshing or clearing a table."
    if (pausedReason) return `Paused: ${pausedReason}`
    if (runState.watching) return "A run of this connector is going; wait for it to finish."
    if (waitingTables?.has(name)) {
      return "This table waits for a schema decision; approve or resolve the change on the Schema changes tab first."
    }
    return null
  }

  const page = layout === "page"
  const tablesIntro = isStream
    ? "A Kafka connector reads its topic; each run takes one micro-batch."
    : isCdc
      ? "Each table here lands in its own Bronze table, kept in step with the source by following its changes. Nothing is captured until at least one table is saved."
      : "Each table here lands in its own Bronze table. A run replaces its rows, or adds to them, as chosen per table. Nothing is ingested until at least one table is saved."

  const cdcNotes = (
    <>
      {isCdc ? (
        <p className="rounded-md border border-border bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
          {backfillTriggerMessage(typeof spec.dial.driver === "string" ? spec.dial.driver : "cdc")}
        </p>
      ) : null}
    </>
  )

  const words = finder ? FINDER_WORDS[finder] : null
  const tablesBody = !isStream ? (
    <>
      {/* Selected tables: the list that is saved. */}
      <div className="space-y-2">
        <p className="text-xs font-medium text-muted-foreground">
          Selected · {selection.length} {selection.length === 1 ? "table" : "tables"}
        </p>
        {selection.length === 0 ? (
          <p className="rounded-md border border-dashed border-border px-3 py-4 text-center text-xs text-muted-foreground">
            No tables selected yet.{words ? ` Find ${words.many} below and tick the ones to copy.` : ""}
          </p>
        ) : (
          <>
          {!columnChoice && !isCdc ? (
            <p className="text-xs text-muted-foreground">{NO_COLUMN_CHOICE_NOTE}</p>
          ) : null}
          <ul className="divide-y divide-border rounded-md border border-border">
            {selection.map((s, i) => (
              <li key={s.name} className="space-y-1.5 px-3 py-2">
                <div className="flex items-center gap-2">
                  <span className={cn("min-w-0 flex-1 truncate text-xs", labels[s.name] ? "" : "font-mono")} title={s.name}>
                    {labels[s.name] ?? s.name}
                  </span>
                  {waitingTables?.has(s.name) ? (
                    <Pill
                      tone="warning"
                      title="A change at the source waits for approval; see Schema changes"
                    >
                      Waiting for a decision
                    </Pill>
                  ) : null}
                  {!isCdc ? (
                    <TableActionsMenu
                      connectorId={connectorId}
                      table={s.name}
                      target={s.target}
                      disabledReason={tableActionBlock(s.name)}
                      onStarted={(runId) => {
                        runState.launch(runId)
                        runState.refreshRuns()
                      }}
                    />
                  ) : null}
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
                <LoadModeFields
                  selected={s}
                  modes={loadModesFor(adapter)}
                  savedMode={saved.find((t) => t.name === s.name)?.loadMode ?? null}
                  columns={columnsOf.get(s.name) ?? null}
                  detectedKey={keysOf.get(s.name) ?? null}
                  problem={modeProblems[i]}
                  changeCapture={isCdc}
                  onChange={(patch) => change(s.name, patch)}
                />
                {columnChoice ? (
                  <ColumnChoice
                    selected={s}
                    preview={previews.entries.get(s.name)}
                    onOpen={() => previews.ensure(s.name)}
                    onChange={(excludedColumns) => change(s.name, { excludedColumns })}
                  />
                ) : null}
                {adapter === "rest" && !declaredEndpoints.includes(s.name) ? (
                  <p className="text-xs text-muted-foreground">{REST_OLD_OBJECT_NOTE}</p>
                ) : null}
              </li>
            ))}
          </ul>
          </>
        )}
      </div>

      {isSharepoint ? (
        <SharepointSitePicker
          connectorId={connectorId}
          picked={reports}
          savedPicked={savedReports}
          onToggle={(site) => setReports((current) => (current.includes(site) ? current.filter((n) => n !== site) : [...current, site]))}
        />
      ) : null}
      {/* Finding tables (SRC-13): one flow for every type that can list. */}
      {finder ? (
        <TableFinder
          connectorId={connectorId}
          adapter={adapter}
          dial={spec.dial}
          defaultSchema={defaultDiscoverSchema(spec.dial)}
          previews={previews}
          selectedNames={selectedNames}
          onToggle={toggle}
          onSetMany={setMany}
          wide={page}
        />
      ) : null}
      {isWorkday ? <WorkdayReportEditor reports={reports} onAdd={addWorkdayReport} onRemove={removeReport} /> : null}
      {isGa4 ? (
        <Ga4ReportEditor connectorId={connectorId} reports={reports} onAdd={addReport} onRemove={removeReport} />
      ) : null}
      {manualAdd ? (
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
              placeholder={
                oracleByHand
                  ? "SCHEMA.TABLE"
                  : adapter === "sftp" || adapter === "files"
                  ? "orders.csv"
                  : finder === "oracle"
                    ? "SCHEMA.TABLE"
                    : adapter === "rest"
                      ? "/orders"
                      : "object name"
              }
              className="h-8 font-mono text-xs"
              autoComplete="off"
            />
            <Button type="button" size="sm" variant="outline" disabled={!manualName.trim()} onClick={addManual}>
              <PlusIcon data-icon="inline-start" />
              Add
            </Button>
          </div>
          <p className="text-xs text-muted-foreground">
            {oracleByHand
              ? ORACLE_BY_HAND_NOTE
              : finder === null
              ? "This connector type cannot list its objects, so type each one as the source names it."
              : finder === "oracle"
                ? "Or type SCHEMA.TABLE as Oracle names it, for a table the list does not show."
                : adapter === "rest"
                  ? "Or type the path of an endpoint this connector declares."
                  : "Or type a name the list does not show."}
          </p>
        </div>
      ) : null}
    </>
  ) : null

  // SRC-13 D4: Kafka reads one topic, the dial's `topic`. Picking one from the broker's list saves the
  // spec with only that field changed (`specWithTopic`); the connector reads it at its next run.
  const topic = typeof spec.dial.topic === "string" ? spec.dial.topic : ""
  const pickTopic = useServiceAction(
    withNotify({ success: "Topic saved", error: "Could not save the topic" }, (signal, name: string) =>
      connectorService.setIngestSpec(connectorId, specWithTopic(spec, name), signal)
    )
  )
  const topicBody = isStream ? (
    <div className="space-y-2">
      <p className="text-xs text-muted-foreground">
        Topic:{" "}
        <span className="font-mono text-foreground" data-testid="kafka-topic">
          {topic || "none chosen"}
        </span>
      </p>
      <TableFinder
        connectorId={connectorId}
        adapter={adapter}
        dial={spec.dial}
        defaultSchema=""
        previews={previews}
        selectedNames={new Set(topic ? [topic] : [])}
        onToggle={async (name) => {
          const result = await pickTopic.run(name)
          if (result) onSaved(result)
        }}
        onSetMany={() => {}}
        single
        busy={pickTopic.status === "pending"}
        wide={page}
      />
      {pickTopic.status === "error" ? <p className="text-xs text-destructive">{pickTopic.error.message}</p> : null}
    </div>
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
      state={runState}
      pausedReason={pausedReason}
      onResumed={onResumed}
      resumeOnCaptureTab={isCdc}
      blockedReason={
        pausedReason
          ? `Paused: ${pausedReason}`
          : dirty
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
              {topicBody}
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
      {topicBody}
      {scheduleBlock}
      {saveRow}
      {runs}
    </div>
  )
}

/**
 * What a run does with one table's rows: replace them, add only new ones
 * (by a cursor column), add everything again, and for the two deduplicating
 * modes keep one row per key. The cursor and the key are picked from the
 * table's columns once it has been found, and typed otherwise.
 */
function LoadModeFields({
  selected,
  modes,
  savedMode,
  columns,
  detectedKey,
  problem,
  changeCapture,
  onChange,
}: {
  selected: Selected
  modes: LoadMode[]
  /** The mode this table is saved with, or null for a table not saved yet. */
  savedMode: LoadMode | null
  columns: string[] | null
  /** The key the source declares for this table, when it was found and declares one. */
  detectedKey: string[] | null
  problem: string | null
  /** `SRC-4`: a change-capture table reads no cursor column and has its own words for the two modes. */
  changeCapture: boolean
  onChange: (patch: Partial<Selected>) => void
}) {
  const cursor = selected.incrementalKey ?? ""
  // A saved mode this adapter no longer offers still shows, rather than
  // silently reading as another one.
  const offered = modes.includes(selected.loadMode) ? modes : [...modes, selected.loadMode]
  const fieldClass = "h-7 rounded-lg border border-input bg-transparent px-2 text-xs"
  const note = missingModesNote(modes, changeCapture)
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-1.5">
        <span className="shrink-0 text-xs text-muted-foreground">{changeCapture ? "Keep" : "Each run"}</span>
        <select
          aria-label={`Load mode for ${selected.name}`}
          className={fieldClass}
          value={selected.loadMode}
          onChange={(e) => onChange({ loadMode: e.target.value as LoadMode })}
        >
          {offered.map((mode) => (
            <option key={mode} value={mode}>
              {modeLabel(mode, changeCapture)}
            </option>
          ))}
        </select>
        {!changeCapture && (selected.loadMode === "incremental" || selected.loadMode === "incremental_dedup") ? (
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
      {note ? <p className="text-xs text-muted-foreground">{note}</p> : null}
      {needsKey(selected.loadMode) ? (
        <KeyFields
          name={selected.name}
          value={selected.primaryKey ?? []}
          columns={columns}
          detected={detectedKey}
          invalid={problem !== null && (selected.primaryKey ?? []).length === 0}
          onChange={(primaryKey) => onChange({ primaryKey })}
        />
      ) : null}
      <p className={problem ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>
        {problem ?? (changeCapture ? changeCaptureHint(selected.loadMode) : loadModeHint(selected.loadMode, cursor.trim()))}
      </p>
      {!changeCapture && crossesHistoryBoundary(savedMode, selected.loadMode) ? (
        <p className="text-xs text-muted-foreground">{HISTORY_BOUNDARY_NOTE}</p>
      ) : null}
    </div>
  )
}

/** `SRC-11` decision 9: said when the source type cannot list its columns before loading. */
export const NO_COLUMN_CHOICE_NOTE = "Column choice is not available for this source type."

/**
 * `SRC-13` decision D3: a REST table now loads the one endpoint it names. A table saved before names a target,
 * not an endpoint, and the loader keeps reading every endpoint for it until it is picked again.
 */
export const REST_OLD_OBJECT_NOTE =
  "Saved before endpoints could be picked: this table loads every endpoint of the connector. Pick one endpoint in Find endpoints to load only that one."

/**
 * Which columns of a table a load reads (`SRC-11` decisions 9 to 11). Opened per table; the columns come from
 * the preview route (`SRC-13`), read when the section is opened, so a table saved in an earlier visit lists them
 * too. Unticking adds the column to `excludedColumns`. A key column of a table that deduplicates or keeps
 * history, and the cursor column, are ticked and locked with the reason. A column left out earlier that the
 * source no longer lists still shows, so it can be ticked again.
 */
function ColumnChoice({
  selected,
  preview,
  onOpen,
  onChange,
}: {
  selected: Selected
  /** What the preview route said about this table's columns, or `undefined` before they were asked for. */
  preview: PreviewEntry | undefined
  /** Asks for the columns (once): called when the section is opened. */
  onOpen: () => void
  onChange: (excluded: string[]) => void
}) {
  const locked = lockedColumns(selected)
  const excluded = effectiveExcluded(selected)
  const columns = preview?.status === "done" && preview.columns.length > 0 ? preview.columns.map((c) => c.name) : null
  const shown = columns ? [...columns, ...excluded.filter((c) => !columns.includes(c))] : null
  return (
    <details className="group">
      <summary
        onClick={onOpen}
        className="flex cursor-pointer list-none items-center gap-1.5 text-xs text-muted-foreground outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 [&::-webkit-details-marker]:hidden"
      >
        <ChevronRightIcon className="size-3.5 shrink-0 transition-transform group-open:rotate-90" />
        Columns
        {excluded.length > 0 ? ` · ${excluded.length} left out` : ""}
      </summary>
      <div className="mt-1 space-y-1 pl-5">
        {shown === null ? (
          <p className="text-xs text-muted-foreground">
            {excluded.length > 0 ? `Left out: ${excluded.join(", ")}. ` : ""}
            {preview === undefined || preview.status === "loading"
              ? "Reading the columns…"
              : preview.status === "failed"
                ? `Cannot read the columns · ${preview.reason}`
                : "The source reports no columns for this table."}
          </p>
        ) : (
          <>
            <ul className="space-y-0.5">
              {shown.map((column) => {
                const reason = locked.get(column)
                return (
                  <li key={column}>
                    <label className="flex items-center gap-1.5 font-mono text-xs">
                      <input
                        type="checkbox"
                        aria-label={`Load column ${column} of ${selected.name}`}
                        checked={reason !== undefined || !excluded.includes(column)}
                        disabled={reason !== undefined}
                        onChange={(e) =>
                          onChange(e.target.checked ? excluded.filter((c) => c !== column) : [...excluded, column])
                        }
                      />
                      {column}
                      {reason ? <span className="font-sans text-muted-foreground">{reason}</span> : null}
                    </label>
                  </li>
                )
              })}
            </ul>
            <p className="text-xs text-muted-foreground">
              An unticked column is not read from the source on the next load.
            </p>
          </>
        )}
      </div>
    </details>
  )
}

/**
 * The columns that identify one row of a deduplicating table (SRC-10 decision 3).
 * A table found by discovery lists its columns to tick, with the source's own key
 * already ticked; a table typed by hand takes the names, separated by commas.
 */
function KeyFields({
  name,
  value,
  columns,
  detected,
  invalid,
  onChange,
}: {
  name: string
  value: string[]
  columns: string[] | null
  detected: string[] | null
  invalid: boolean
  onChange: (key: string[]) => void
}) {
  const [typed, setTyped] = React.useState(value.join(", "))
  const origin =
    detected && detected.length > 0
      ? sameColumns(value, detected)
        ? `Filled in from the source, which declares ${detected.join(", ")} as this table's key.`
        : `The source declares ${detected.join(", ")} as this table's key; this table uses another.`
      : null
  return (
    <div className="space-y-1">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
        <span className="shrink-0 text-xs text-muted-foreground">Key</span>
        {columns && columns.length > 0 ? (
          <fieldset aria-label={`Key columns for ${name}`} className="flex min-w-0 flex-wrap gap-x-3 gap-y-0.5">
            {(value.every((c) => columns.includes(c)) ? columns : [...value.filter((c) => !columns.includes(c)), ...columns]).map(
              (column) => (
                <label key={column} className="flex items-center gap-1 font-mono text-xs">
                  <input
                    type="checkbox"
                    aria-label={`${column} is part of the key of ${name}`}
                    checked={value.includes(column)}
                    onChange={(e) =>
                      onChange(e.target.checked ? [...value, column] : value.filter((c) => c !== column))
                    }
                  />
                  {column}
                </label>
              )
            )}
          </fieldset>
        ) : (
          <Input
            aria-label={`Key columns for ${name}`}
            value={typed}
            onChange={(e) => {
              setTyped(e.target.value)
              onChange(
                e.target.value
                  .split(",")
                  .map((c) => c.trim())
                  .filter((c) => c !== "")
              )
            }}
            placeholder="id, or id, region"
            className="h-7 min-w-0 flex-1 font-mono text-xs"
            aria-invalid={invalid}
            autoComplete="off"
          />
        )}
      </div>
      {origin ? <p className="text-xs text-muted-foreground">{origin}</p> : null}
    </div>
  )
}

/** Whether two column lists name the same columns, in any order. */
function sameColumns(a: string[], b: string[]): boolean {
  return a.length === b.length && [...a].sort().join("\u0000") === [...b].sort().join("\u0000")
}

/** How often a run in progress is checked on. */
const POLL_MS = 3_000

/** How long a run just launched is waited for before the run record lists it. */
const LAUNCH_GRACE_MS = 60_000

/**
 * How long a run whose record still says `running` is believed. A table holds
 * a run open for at most about 21 minutes of waiting between its three retries
 * (`SRC-11` decision 1) plus the load itself, so a run with no end reported
 * after six hours was stopped without reporting one: a worker killed or a run
 * cancelled in the orchestrator fires neither of the reports that close a
 * record (the Rust handoff of tasks 1 to 4 says so). It is shown as "no end
 * reported", not as active forever, and it no longer disables Run now.
 */
export const STALE_RUN_MS = 6 * 60 * 60 * 1000

function startedMillis(run: ConnectorRun): number {
  return parseTimestamp(run.startedAt).getTime()
}

/** Whether the run's record says it is going and the bound above has not passed. */
export function isActive(run: ConnectorRun, now = Date.now()): boolean {
  if (run.status !== "running") return false
  const started = startedMillis(run)
  return Number.isNaN(started) || now - started < STALE_RUN_MS
}

/** A record that says `running` for longer than `STALE_RUN_MS`: its end was never reported. */
export function isNoEndReported(run: ConnectorRun, now = Date.now()): boolean {
  return run.status === "running" && !isActive(run, now)
}

export function runStatusLabel(run: ConnectorRun, now = Date.now()): string {
  if (isNoEndReported(run, now)) return "No end reported"
  switch (run.status) {
    case "running":
      return "Running"
    case "succeeded":
      return "Succeeded"
    case "failed":
      return "Failed"
    case "paused":
      return "Paused"
    default:
      return run.status
  }
}

export function runStatusTone(
  run: ConnectorRun,
  now = Date.now()
): "success" | "destructive" | "warning" | "neutral" | "info" {
  if (isNoEndReported(run, now)) return "neutral"
  switch (run.status) {
    case "running":
      return "info"
    case "succeeded":
      return "success"
    case "failed":
      return "destructive"
    case "paused":
      return "warning"
    default:
      return "neutral"
  }
}

const TRIGGER_LABEL: Record<string, string> = {
  schedule: "Schedule",
  user: "Started by a user",
  refresh: "Refresh",
  clear: "Clear",
  other: "Started from the orchestrator",
}

const END_REASON_LABEL: Record<string, string> = {
  completed: "Every table was loaded.",
  table_failed: "At least one table could not be loaded.",
  connector_paused: "The connector was paused, so nothing was loaded.",
  other: "The run ended for another reason.",
}

/** What an attempt's closed error class says (`SRC-11` decision 2); never the source's own text. */
const ERROR_CLASS_LABEL: Record<string, string> = {
  unreachable: "The source could not be reached",
  timeout: "The source took too long to answer",
  source_busy: "The source was busy",
  api_unreachable: "The product's own API could not be reached",
  sign_in_refused: "The source refused the sign-in",
  address_refused: "The address was refused",
  unsupported: "The source type or load mode is not supported",
  setting_rejected: "A setting was rejected",
  schema_decision_pending: "The table is waiting for a schema decision",
  other: "Failed for another reason",
  permission_missing: "The account lacks a permission at the source",
  rate_limited: "The source limited the rate of requests",
}

/** The retried classes, for the word beside them; the others fail at once. */
const RETRIED_CLASSES = new Set(["unreachable", "timeout", "source_busy", "api_unreachable", "rate_limited"])

const EVENT_LABEL: Record<string, string> = {
  run_started: "Run started",
  attempt: "Attempt",
  retry_wait: "Waiting to retry",
  schema_decision_pending: "Waiting for a schema decision",
  columns_left_out: "Columns left out",
  report_gap: "Log may be incomplete",
  run_ended: "Run ended",
}

/**
 * The run record of a connector and what is known from it: whether a run is
 * going, including one just launched that the record does not list yet. Checked
 * every few seconds while a run is going. Used by the editor (the per-table
 * actions are disabled meanwhile) and handed to the Runs card.
 */
function useIngestRunState(connectorId: string) {
  const list = useRefreshable((s) => connectorService.listConnectorRuns(connectorId, s), connectorId)
  // A run just launched, until the record lists it.
  const [launchedRunId, setLaunchedRunId] = React.useState<string | null>(null)

  const runs = list.data ?? []
  const activeRun = runs.find((r) => isActive(r)) ?? null
  const awaitingLaunch = launchedRunId !== null && !runs.some((r) => r.runId === launchedRunId)
  const watching = activeRun !== null || awaitingLaunch
  const { refresh: refreshRuns } = list

  React.useEffect(() => {
    if (!watching) return
    const timer = window.setInterval(refreshRuns, POLL_MS)
    return () => window.clearInterval(timer)
  }, [watching, refreshRuns])

  // Stop waiting for a launched run the record never lists.
  React.useEffect(() => {
    if (!launchedRunId) return
    const timer = window.setTimeout(() => setLaunchedRunId(null), LAUNCH_GRACE_MS)
    return () => window.clearTimeout(timer)
  }, [launchedRunId])

  // A run that just finished: one last read, so its final numbers show.
  const wasWatching = React.useRef(false)
  React.useEffect(() => {
    if (wasWatching.current && !watching) refreshRuns()
    wasWatching.current = watching
  }, [watching, refreshRuns])

  return {
    list,
    runs,
    activeRun,
    awaitingLaunch,
    launchedRunId,
    watching,
    launch: setLaunchedRunId,
    refreshRuns,
  }
}

type IngestRunState = ReturnType<typeof useIngestRunState>

/**
 * `SRC-11` decision 3, mirrored from `AUTO_DISABLE_REASON_HEAD` / `_TAIL` in
 * `rust/crates/lakehouse-store/src/connectors.rs` (`is_auto_disable_pause_reason`):
 * "Paused after N failed runs in a row", N from the API's setting. Resume lifts
 * only this pause; a schema-change pause is lifted by approving the change.
 */
export function isAutoDisablePause(reason: string | null | undefined): boolean {
  return /^Paused after \d+ failed runs in a row$/.test(reason ?? "")
}

/** The reason a person's own pause carries (`PERSON_PAUSE_REASON` in `connectors.rs`, `SRC-4`); Resume lifts it too. */
export const PERSON_PAUSE_REASON = "Paused by a person"

/** A pause Resume can lift: repeated failures set it, or a person did. */
export const isLiftablePause = (reason: string | null | undefined): boolean =>
  isAutoDisablePause(reason) || reason === PERSON_PAUSE_REASON

/**
 * The connector's runs from the product's own record (newest 50), each opening
 * to its log, and a Run now that stays disabled while a run is going: Bronze is
 * append-only, so a second run would copy every table in again. While the
 * connector is paused the card says so, with Resume when the pause is the
 * auto-disable one.
 */
function IngestRuns({
  layout,
  connectorId,
  state,
  blockedReason,
  pausedReason,
  onResumed,
  resumeOnCaptureTab = false,
}: {
  layout: "page" | "embedded"
  connectorId: string
  state: IngestRunState
  blockedReason: string | null
  pausedReason: string | null
  onResumed?: () => void
  /** A change-capture connector is paused and resumed on the Change capture tab (`SRC-4` S4-g), not here. */
  resumeOnCaptureTab?: boolean
}) {
  const { list, runs, activeRun, awaitingLaunch, launchedRunId, watching, refreshRuns } = state
  const runNow = useServiceAction((signal, id: string) => connectorService.runIngest(id, signal))

  const runNowButton = (
    <Button
      type="button"
      size="sm"
      variant="outline"
      disabled={runNow.status === "pending" || watching || blockedReason !== null}
      onClick={async () => {
        const result = await runNow.run(connectorId)
        if (result?.runId) state.launch(result.runId)
        refreshRuns()
      }}
    >
      <PlayIcon data-icon="inline-start" />
      {runNow.status === "pending" ? "Starting…" : "Run now"}
    </Button>
  )

  const body = (
    <>
      {pausedReason ? (
        <PausedNote
          connectorId={connectorId}
          reason={pausedReason}
          onResumed={onResumed}
          resumeOnCaptureTab={resumeOnCaptureTab}
        />
      ) : blockedReason && !watching ? (
        <p className="text-xs text-muted-foreground">{blockedReason}</p>
      ) : null}
      {activeRun ? (
        <p className="flex items-center gap-1.5 text-xs text-primary">
          <LoaderIcon className="size-3.5 animate-spin" />
          Run {activeRun.runId.slice(0, 8)} is running. Its log shows each attempt as it ends.
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

      {list.error ? (
        <ErrorState error={list.error} onRetry={refreshRuns} />
      ) : list.data === null ? (
        <LoadingSkeleton rows={2} />
      ) : runs.length === 0 ? (
        <p className="text-xs text-muted-foreground">No runs yet.</p>
      ) : (
        <div className="space-y-2">
          {runs.map((run, index) => (
            <RunCard
              key={run.runId}
              layout={layout}
              connectorId={connectorId}
              run={run}
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

/**
 * The pause, in the Runs card. Resume (`SRC-11` decision 4) only for the pause
 * repeated failures set; the API answers any other with a fixed 409 and nothing
 * changes, which is shown as it came if it ever happens.
 */
function PausedNote({
  connectorId,
  reason,
  onResumed,
  resumeOnCaptureTab = false,
}: {
  connectorId: string
  reason: string
  onResumed?: () => void
  resumeOnCaptureTab?: boolean
}) {
  const resume = useServiceAction(async (signal) => {
    const result = await connectorService.resumeConnector(connectorId, signal)
    if (!signal.aborted) notifySuccess("Resumed; the connector runs on its schedule again")
    return result
  })
  return (
    <div className="space-y-1.5 rounded-md border border-border bg-muted/30 px-3 py-2">
      <p className="text-xs">Paused: {reason}</p>
      {isLiftablePause(reason) && resumeOnCaptureTab ? (
        <p className="text-xs text-muted-foreground">Resume it on the Change capture tab.</p>
      ) : isLiftablePause(reason) ? (
        <>
          <p className="text-xs text-muted-foreground">
            {isAutoDisablePause(reason)
              ? "Fix the cause, then resume. The count of failed runs starts again from zero."
              : "Resume when you want it to run again."}
          </p>
          <Button
            type="button"
            size="sm"
            disabled={resume.status === "pending"}
            onClick={async () => {
              if (await resume.run()) onResumed?.()
            }}
          >
            {resume.status === "pending" ? "Resuming…" : "Resume"}
          </Button>
        </>
      ) : null}
      {resume.status === "error" ? (
        <p role="alert" className="text-xs text-destructive">
          {resume.error.message}
        </p>
      ) : null}
    </div>
  )
}

function RunCard({
  layout,
  connectorId,
  run,
  defaultOpen,
}: {
  layout: "page" | "embedded"
  connectorId: string
  run: ConnectorRun
  defaultOpen: boolean
}) {
  const [logOpen, setLogOpen] = React.useState(false)
  const noEnd = isNoEndReported(run)
  const attempts = `${run.attempts} ${run.attempts === 1 ? "attempt" : "attempts"}`
  const summary = noEnd
    ? "No end reported"
    : `${run.rowsLoaded.toLocaleString()} rows · ${attempts}`

  const status = <Pill tone={runStatusTone(run)}>{runStatusLabel(run)}</Pill>
  const runId = (
    <>
      <span className="font-mono text-xs">{run.runId.slice(0, 8)}</span>
      <span className="max-w-full truncate text-xs font-medium">
        {`${TRIGGER_LABEL[run.trigger] ?? run.trigger}${run.table ? ` · ${run.table}` : ""}`}
      </span>
    </>
  )
  const when = (
    <>
      {formatRelativeTime(run.startedAt)}
      {run.durationSeconds !== null ? ` · took ${formatDuration(run.durationSeconds * 1000)}` : ""}
    </>
  )
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
            <span className="block text-xs text-muted-foreground">{when}</span>
          </span>
        </summary>
      ) : (
        <summary className={cn(summaryClass, "items-center")}>
          <ChevronRightIcon className={chevron} />
          {status}
          {runId}
          <span className="text-xs text-muted-foreground">{when}</span>
          <span className="ml-auto truncate pl-2 text-xs text-muted-foreground">{summary}</span>
        </summary>
      )}
      <div className="space-y-2 border-t border-border px-3 py-2">
        <p className="text-xs text-muted-foreground">
          {noEnd
            ? "This run never reported its end. It may have been stopped before it finished."
            : run.endReason
              ? (END_REASON_LABEL[run.endReason] ?? run.endReason)
              : run.status === "running"
                ? "Still going."
                : null}
        </p>
        {logOpen ? (
          <RunLog connectorId={connectorId} runId={run.runId} />
        ) : (
          <Button type="button" size="xs" variant="outline" onClick={() => setLogOpen(true)}>
            Show log
          </Button>
        )}
      </div>
    </details>
  )
}

/** "90" → "2 min": the wait before a retry, rounded up to whole minutes. */
function waitLabel(seconds: number): string {
  return seconds < 60 ? `${seconds} s` : `${Math.ceil(seconds / 60)} min`
}

/**
 * One run's log (`SRC-11` decision 5): its attempts, table by table, then the
 * product's own lines, with a download of the same lines as text. The API
 * writes the lines from a closed list and never carries the source's or the
 * driver's error text; this view only words the closed event and error class.
 */
function RunLog({ connectorId, runId }: { connectorId: string; runId: string }) {
  const log = useService((s) => connectorService.getConnectorRunLog(connectorId, runId, s), [connectorId, runId])
  const download = useServiceAction(async (signal) => {
    const text = await connectorService.downloadConnectorRunLog(connectorId, runId, signal)
    if (signal.aborted) return text
    const url = URL.createObjectURL(new Blob([text], { type: "text/plain" }))
    const a = document.createElement("a")
    a.href = url
    a.download = `run-${runId}.log`
    a.click()
    URL.revokeObjectURL(url)
    return text
  })

  if (log.status === "loading") return <LoadingSkeleton rows={2} />
  if (log.status === "error") return <ErrorState error={log.error} onRetry={log.reload} />
  const { attempts, lines } = log.data
  return (
    <div className="space-y-2">
      <div className="flex items-center gap-2">
        <p className="text-xs font-medium text-muted-foreground">Log</p>
        <Button
          type="button"
          size="xs"
          variant="outline"
          className="ml-auto"
          disabled={download.status === "pending"}
          onClick={() => void download.run()}
        >
          <DownloadIcon data-icon="inline-start" />
          Download log
        </Button>
      </div>
      {download.status === "error" ? (
        <p role="alert" className="text-xs text-destructive">
          {download.error.message}
        </p>
      ) : null}
      {attempts.length === 0 && lines.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          This run has no log lines. Logs are kept for 30 days, and a run shows lines once the orchestrator reports them.
        </p>
      ) : (
        <>
          {attempts.length > 0 ? (
            <ul className="divide-y divide-border rounded-md border border-border">
              {attempts.map((a) => (
                <li key={`${a.table}-${a.attempt}`} className="space-y-0.5 px-3 py-1.5">
                  <div className="flex items-center gap-2">
                    <span className="min-w-0 flex-1 truncate font-mono text-xs" title={a.table}>
                      {a.table}
                    </span>
                    <span className="shrink-0 text-xs text-muted-foreground">attempt {a.attempt}</span>
                    <span className="shrink-0 text-xs text-muted-foreground">{a.rowsLoaded.toLocaleString()} rows</span>
                    <Pill tone={a.result === "succeeded" ? "success" : "destructive"}>
                      {a.result === "succeeded" ? "Succeeded" : "Failed"}
                    </Pill>
                  </div>
                  {a.result !== "succeeded" ? (
                    <p className="text-xs text-destructive">
                      {a.errorClass ? (ERROR_CLASS_LABEL[a.errorClass] ?? a.errorClass) : "Failed"}
                      {a.errorClass && RETRIED_CLASSES.has(a.errorClass) && a.waitSeconds === null
                        ? ". Retries are used up."
                        : ""}
                      {a.waitSeconds !== null ? `. Next attempt in ${waitLabel(a.waitSeconds)}.` : ""}
                    </p>
                  ) : null}
                </li>
              ))}
            </ul>
          ) : null}
          {lines.length > 0 ? (
            <ol className="space-y-0.5 rounded-md border border-border bg-muted/30 px-3 py-2">
              {lines.map((l, i) => (
                <li key={`${l.at}-${i}`} className="flex flex-wrap gap-x-2 text-xs">
                  <span className="shrink-0 text-muted-foreground" title={formatDateTime(l.at)}>
                    {formatDateTime(l.at)}
                  </span>
                  <span className="shrink-0 font-medium">{EVENT_LABEL[l.event] ?? l.event}</span>
                  <span className="min-w-0 break-words text-muted-foreground">{l.text}</span>
                </li>
              ))}
            </ol>
          ) : null}
        </>
      )}
    </div>
  )
}
