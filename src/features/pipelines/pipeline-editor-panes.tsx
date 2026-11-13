"use client"

import * as React from "react"
import Link from "next/link"
import { ArrowDownIcon, ArrowUpIcon, Trash2Icon, WandSparklesIcon } from "lucide-react"
import { CodeView } from "@/components/patterns/code-view"
import { SelectField } from "@/components/patterns/select-field"
import { SqlEditor } from "@/components/sql-editor"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Switch } from "@/components/ui/switch"
import { createSqlCompletionSource, sqlCompletionExtension } from "@/features/queries/sql-completion-source"
import { useService } from "@/hooks/use-service"
import {
  MAX_SQL_CHARS,
  stepCompletion,
  type Column,
  type EditorInput,
  type EditorState,
  type EditorStep,
  type ProblemField,
  type StepRead,
} from "@/lib/pipeline-editor"
import { formatSql } from "@/lib/sql-format"
import { lakehouseService } from "@/services"
import { COLUMNS_APPEAR_LATER, ZoneField } from "./pipeline-pickers"
import { BRONZE_ZONE } from "./pipeline-reupload-trigger"
import { TransformRows } from "./pipeline-transform-rows"

/**
 * The panes of the pipeline editor (BLD-1 part 1 D17): one for an input,
 * one per kind of step, and one for the output. Each pane only shows and
 * edits; the rules are in `lib/pipeline-editor.ts` and the state is the
 * editor's (`pipeline-editor.tsx`).
 */

/**
 * The control a pane's problem is about, and the id of the sentence that
 * says it (BLD-1 part 1 S3 review R1-S10: a hint was not tied to its field).
 */
export type Invalid = { field: ProblemField; describedBy: string } | null

/** The ARIA a control gets when the pane's problem is about it. */
function tie(invalid: Invalid | undefined, field: ProblemField): { "aria-invalid"?: true; "aria-describedby"?: string } {
  return invalid && invalid.field === field ? { "aria-invalid": true, "aria-describedby": invalid.describedBy } : {}
}

/** A problem or a refusal shown in the pane of the item it is about. */
export function PaneProblem({ id, message, tone }: { id?: string; message: string; tone: "error" | "hint" }) {
  return (
    <p
      id={id}
      role={tone === "error" ? "alert" : undefined}
      className={tone === "error" ? "text-sm text-destructive" : "text-xs text-amber-700 dark:text-amber-400"}
    >
      {message}
    </p>
  )
}

function NameField({
  id,
  value,
  onChange,
  invalid,
}: {
  id: string
  value: string
  onChange: (next: string) => void
  invalid?: Invalid
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>Name in this pipeline</Label>
      <Input
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="font-mono"
        spellCheck={false}
        {...tie(invalid, "name")}
      />
    </div>
  )
}

/** The input's columns with their types, as the lakehouse lists them. */
function ColumnList({ columns }: { columns: Column[] | null }) {
  if (columns === null) return <p className="text-xs text-muted-foreground">{COLUMNS_APPEAR_LATER}</p>
  return (
    <ul aria-label="Columns of this table" className="flex flex-wrap gap-1.5">
      {columns.map((column) => (
        <li key={column.name} className="rounded border border-border px-1.5 py-0.5 font-mono text-[11px]">
          {column.name} <span className="text-muted-foreground">{column.type}</span>
        </li>
      ))}
    </ul>
  )
}

export type ConnectorSource = {
  connectorId: string
  /** The connector's ingest targets; `null` while they are read or when the read failed. */
  targets: string[] | null
  failed: boolean
}

/**
 * An input: its zone and table, the name its steps read it by, and its
 * columns with their types (`getTableDetail` already returns them).
 *
 * The first input of a pipeline fed by a connector is that connector's
 * (`BLD-7` decision 5): its zone is shown, not editable, and its table is one
 * of the connector's ingest targets, never a typed name. The API holds the
 * same rule (`validate_connector_source`), so the pane and a 400 cannot
 * disagree.
 */
export function InputPane({
  input,
  shown,
  index,
  canRemove,
  connector,
  zones,
  zoneNote,
  icebergZones,
  columns,
  problem,
  invalid,
  onRename,
  onZone,
  onTable,
  onRemove,
}: {
  input: EditorInput
  /** The input as it will be saved (`effectiveInputs`): with a connector, the first one's zone and table. */
  shown: EditorInput
  index: number
  canRemove: boolean
  connector: ConnectorSource | null
  zones: string[] | null
  zoneNote?: string
  icebergZones: ReadonlySet<string>
  columns: Column[] | null
  problem: React.ReactNode
  /** Which control the pane's problem is about (R1-S10). */
  invalid?: Invalid
  onRename: (name: string) => void
  onZone: (zone: string) => void
  onTable: (table: string) => void
  onRemove: () => void
}) {
  const fromConnector = connector !== null && index === 0
  // DATA-19 D4: the zone's real table names as SUGGESTIONS only, for a
  // zone the lakehouse lists as an Iceberg namespace; a ClickHouse zone
  // has none to list, and typing stays allowed either way.
  const zone = input.zone.trim()
  const listable = !fromConnector && icebergZones.has(zone)
  const tables = useService(
    (signal) => (listable ? lakehouseService.listTables(zone, undefined, signal) : Promise.resolve([])),
    [zone, listable]
  )
  const suggestions = tables.status === "success" ? tables.data.map((table) => table.name).sort() : []
  const listId = `input-table-options-${input.id}`
  const tableControl =
    !fromConnector || (connector !== null && !connector.failed && connector.targets !== null && connector.targets.length > 0)

  return (
    <div className="flex flex-col gap-4">
      <div className="flex items-start justify-between gap-3">
        <div>
          <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-muted-foreground">Input {index + 1}</p>
          <p className="font-mono text-sm font-semibold">{input.name || "Unnamed input"}</p>
        </div>
        {canRemove ? (
          <Button type="button" variant="ghost" size="sm" onClick={onRemove}>
            <Trash2Icon aria-hidden />
            Remove input
          </Button>
        ) : null}
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        {fromConnector ? (
          <div className="space-y-1.5">
            <Label htmlFor={`input-zone-${input.id}`}>Zone</Label>
            {/* BLD-7 decision 5: shown, not editable. A connector loads into
                Bronze, so that is the only zone it can feed (`DEC-15`). */}
            <Input
              id={`input-zone-${input.id}`}
              value={BRONZE_ZONE}
              readOnly
              disabled
              aria-label="Zone, fixed to bronze by the connector"
            />
          </div>
        ) : (
          <ZoneField
            id={`input-zone-${input.id}`}
            label="Zone"
            value={input.zone}
            onChange={onZone}
            zones={zones}
            note={zoneNote}
            describedBy={tie(invalid, "zone")["aria-describedby"]}
          />
        )}
        <div className="space-y-1.5">
          {/* R1-S10: the label names a control only while there is one; while
              the connector's tables are read there is a sentence instead. */}
          <Label htmlFor={tableControl ? `input-table-${input.id}` : undefined}>Table</Label>
          {fromConnector ? (
            connector.failed ? (
              <p className="text-sm text-destructive">
                Could not read this connector&apos;s tables.{" "}
                <Link className="underline" href={`/connectors/${encodeURIComponent(connector.connectorId)}?tab=ingest`}>
                  Open its Ingest tab
                </Link>
                .
              </p>
            ) : connector.targets === null ? (
              <p className="text-sm text-muted-foreground">Reading this connector&apos;s tables…</p>
            ) : connector.targets.length === 0 ? (
              // Same sentence as the API's own refusal
              // (`routes::authored_pipelines::CONNECTOR_HAS_NO_TABLES`).
              <p className="text-sm text-muted-foreground">
                This connector has not loaded any table yet; run its ingest first.{" "}
                <Link className="underline" href={`/connectors/${encodeURIComponent(connector.connectorId)}?tab=ingest`}>
                  Open its Ingest tab
                </Link>
                .
              </p>
            ) : (
              <SelectField
                id={`input-table-${input.id}`}
                className="w-full"
                aria-label="Table, one of this connector's tables"
                value={shown.table}
                placeholder="Choose one of the connector's tables"
                options={connector.targets.map((target) => ({ value: target, label: target }))}
                onValueChange={onTable}
                aria-invalid={tie(invalid, "table")["aria-invalid"]}
                aria-describedby={tie(invalid, "table")["aria-describedby"]}
              />
            )
          ) : (
            <>
              {/* A `datalist`, not a picker, so a table the namespace listing
                  has not caught up with can still be named by hand. */}
              <Input
                id={`input-table-${input.id}`}
                value={input.table}
                onChange={(e) => onTable(e.target.value)}
                placeholder="orders_events"
                {...tie(invalid, "table")}
                list={suggestions.length > 0 ? listId : undefined}
                title={suggestions.length > 0 ? `Suggestions from ${zone}; any other name can be typed.` : undefined}
              />
              <datalist id={listId}>
                {suggestions.map((name) => (
                  <option key={name} value={name} />
                ))}
              </datalist>
            </>
          )}
        </div>
        <div className="sm:col-span-2">
          <NameField id={`input-name-${input.id}`} value={input.name} onChange={onRename} invalid={invalid} />
          <p className="mt-1 text-xs text-muted-foreground">Steps read this table by this name.</p>
        </div>
      </div>
      <div className="space-y-1.5">
        <p className="text-sm font-medium">Columns</p>
        <ColumnList columns={columns} />
      </div>
      {problem}
    </div>
  )
}

/** The step's header: its number and kind, and the controls that move or remove it. */
function StepHeader({
  step,
  index,
  count,
  onMove,
  onRemove,
}: {
  step: EditorStep
  index: number
  count: number
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  const label = step.name || `step ${index + 1}`
  return (
    <div className="flex items-start justify-between gap-3">
      <div>
        <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
          Step {index + 1} · {step.kind === "sql" ? "SQL" : step.kind === "transform" ? "Form" : step.stored.kind}
        </p>
        <p className="font-mono text-sm font-semibold">{step.name || "Unnamed step"}</p>
      </div>
      <div className="flex items-center gap-1">
        <Button type="button" variant="ghost" size="icon-sm" aria-label={`Move ${label} up`} disabled={index === 0} onClick={() => onMove(-1)}>
          <ArrowUpIcon aria-hidden />
        </Button>
        <Button
          type="button"
          variant="ghost"
          size="icon-sm"
          aria-label={`Move ${label} down`}
          disabled={index === count - 1}
          onClick={() => onMove(1)}
        >
          <ArrowDownIcon aria-hidden />
        </Button>
        <Button type="button" variant="ghost" size="sm" onClick={onRemove}>
          <Trash2Icon aria-hidden />
          Remove step
        </Button>
      </div>
    </div>
  )
}

/** A `transform` step: what it reads, and the five operations (moved as they are, D17). */
export function TransformStepPane({
  step,
  index,
  count,
  readable,
  readsName,
  columns,
  readsStep,
  failedTransform,
  failedMessage,
  failedMessageId,
  problem,
  invalid,
  onRename,
  onChange,
  onMove,
  onRemove,
}: {
  step: Extract<EditorStep, { kind: "transform" }>
  index: number
  count: number
  /** The inputs and earlier steps it may read, by id (R1-B2). */
  readable: { id: string; name: string }[]
  /** The name of what it reads now, for the sentences below. */
  readsName: string
  columns: string[] | null
  /** Whether it reads a step (whose columns are known only once that step is previewed). */
  readsStep: boolean
  failedTransform: number | null
  failedMessage: string | null
  failedMessageId?: string
  problem: React.ReactNode
  /** Which control the pane's problem is about (R1-S10). */
  invalid?: Invalid
  onRename: (name: string) => void
  onChange: (patch: { reads?: StepRead; transforms?: string[] }) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  return (
    <div className="flex flex-col gap-4">
      <StepHeader step={step} index={index} count={count} onMove={onMove} onRemove={onRemove} />
      <div className="grid gap-3 sm:grid-cols-2">
        <NameField id={`step-name-${step.id}`} value={step.name} onChange={onRename} invalid={invalid} />
        <div className="space-y-1.5">
          <Label htmlFor={`step-reads-${step.id}`}>Reads</Label>
          {/* D6a: an input or an earlier step. The value is the item's id,
              so the choice follows a rename (R1-B2). */}
          <SelectField
            id={`step-reads-${step.id}`}
            className="w-full"
            aria-label="Reads"
            value={"id" in step.reads ? step.reads.id : ""}
            placeholder="Choose an input or an earlier step"
            options={readable.map((item) => ({ value: item.id, label: item.name }))}
            onValueChange={(id) => onChange({ reads: { id } })}
            aria-invalid={tie(invalid, "reads")["aria-invalid"]}
            aria-describedby={tie(invalid, "reads")["aria-describedby"]}
          />
        </div>
      </div>
      <TransformRows
        transforms={step.transforms}
        onChange={(transforms) => onChange({ transforms })}
        columns={columns}
        columnsNote={readsStep ? `Preview ${readsName} to pick from its columns; until then, type them.` : undefined}
        failedIndex={failedTransform}
        failedMessage={failedMessage}
        failedMessageId={failedMessageId}
        describedBy={tie(invalid, "operations")["aria-describedby"]}
      />
      {problem}
    </div>
  )
}

/**
 * A SQL step: one `SELECT` over the inputs and the earlier steps (D5), in
 * the console's SQL editor, completing those names and the inputs' columns
 * (`sql-completion-source.ts`), with Format (`lib/sql-format.ts`).
 */
export function SqlStepPane({
  step,
  index,
  count,
  state,
  inputColumns,
  problem,
  invalid,
  hint,
  onRename,
  onSql,
  onMove,
  onRemove,
}: {
  step: Extract<EditorStep, { kind: "sql" }>
  index: number
  count: number
  state: EditorState
  inputColumns: Readonly<Record<string, Column[] | null>>
  problem: React.ReactNode
  /** Which control the pane's problem is about (R1-S10). */
  invalid?: Invalid
  /** R1-S1: a name the SQL reads that is not an input or an earlier step; does not block a save. */
  hint: string | null
  onRename: (name: string) => void
  onSql: (sql: string) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  const completion = React.useMemo(() => stepCompletion(state, index, inputColumns), [state, index, inputColumns])
  // The extension is rebuilt only when what it offers changes: `SqlEditor`
  // reconfigures CodeMirror whenever the array's identity changes, and a
  // new one on every keystroke would do that on every keystroke.
  const completionKey = JSON.stringify(completion)
  const extensions = React.useMemo(
    () => {
      const { tables, columns } = JSON.parse(completionKey) as ReturnType<typeof stepCompletion>
      return [sqlCompletionExtension(createSqlCompletionSource({ tables, columns: () => columns, parameters: () => [] }))]
    },
    [completionKey]
  )
  const readable = completion.tables.map((table) => table.reference)
  const hintId = `step-sql-hint-${step.id}`
  const length = step.sql.length

  return (
    <div className="flex flex-col gap-4">
      <StepHeader step={step} index={index} count={count} onMove={onMove} onRemove={onRemove} />
      <NameField id={`step-name-${step.id}`} value={step.name} onChange={onRename} invalid={invalid} />
      <div className="flex flex-col gap-1.5">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-sm font-medium" id={`step-sql-label-${step.id}`}>
            SQL
          </p>
          <Button type="button" variant="outline" size="sm" onClick={() => onSql(formatSql(step.sql))}>
            <WandSparklesIcon aria-hidden />
            Format
          </Button>
        </div>
        <p className="text-xs text-muted-foreground">
          One <span className="font-mono">SELECT</span>. You can read:{" "}
          {readable.length === 0 ? (
            "no input has a name yet"
          ) : (
            <span aria-label="Names this step can read">
              {readable.map((name, i) => (
                <React.Fragment key={name}>
                  {i > 0 ? ", " : null}
                  <span className="font-mono text-foreground">{name}</span>
                </React.Fragment>
              ))}
            </span>
          )}
        </p>
        <div
          role="group"
          aria-labelledby={`step-sql-label-${step.id}`}
          aria-describedby={[tie(invalid, "sql")["aria-describedby"], hint ? hintId : undefined].filter(Boolean).join(" ") || undefined}
        >
          <SqlEditor value={step.sql} onChange={onSql} minHeight="220px" wrap extensions={extensions} />
        </div>
        <p className={length > MAX_SQL_CHARS ? "text-xs text-destructive" : "text-xs text-muted-foreground"}>
          {length.toLocaleString("en-US")} of {MAX_SQL_CHARS.toLocaleString("en-US")} characters
        </p>
      </div>
      {hint ? <PaneProblem id={hintId} message={hint} tone="hint" /> : null}
      {problem}
    </div>
  )
}

/**
 * A step of a kind this console does not know (BLD-1 part 1 S3 review
 * R1-S2; part 2 adds kinds). It is shown, it can be moved or removed as a
 * step of the pipeline, and it is saved back exactly as stored; nothing in
 * it is edited here, because its fields are not known here.
 */
export function UnknownStepPane({
  step,
  index,
  count,
  problem,
  onMove,
  onRemove,
}: {
  step: Extract<EditorStep, { kind: "unknown" }>
  index: number
  count: number
  problem: React.ReactNode
  /** Which control the pane's problem is about (R1-S10). */
  invalid?: Invalid
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  return (
    <div className="flex flex-col gap-4">
      <StepHeader step={step} index={index} count={count} onMove={onMove} onRemove={onRemove} />
      <p className="text-sm text-muted-foreground">
        This step is of kind <span className="font-mono text-foreground">{step.stored.kind}</span>, which this console
        cannot edit yet. It is saved back exactly as it is stored.
      </p>
      <CodeView text={JSON.stringify(step.stored, null, 2)} language="plain" />
      {problem}
    </div>
  )
}

/**
 * The output table. Every run rebuilds it (decision 8: the user does not
 * choose how it is refreshed), and with no step it is a copy of the one
 * input, which is chosen here on purpose (BLD-10 D2) rather than falling out
 * of adding nothing.
 */
export function OutputPane({
  targetZone,
  targetTable,
  zones,
  zoneNote,
  writtenFrom,
  plainCopy,
  plainCopyAllowed,
  problem,
  invalid,
  onZone,
  onTable,
  onPlainCopy,
}: {
  targetZone: string
  targetTable: string
  zones: string[] | null
  zoneNote?: string
  /** What the output is written from, said for the state it is in (`outputSourceLine`). */
  writtenFrom: string
  plainCopy: boolean
  /** Only with no step is there anything to copy as it is. */
  plainCopyAllowed: boolean
  problem: React.ReactNode
  /** Which control the pane's problem is about (R1-S10). */
  invalid?: Invalid
  onZone: (zone: string) => void
  onTable: (table: string) => void
  onPlainCopy: (on: boolean) => void
}) {
  return (
    <div className="flex flex-col gap-4">
      <div>
        <p className="font-mono text-[11px] uppercase tracking-[0.14em] text-muted-foreground">Output</p>
        <p className="text-sm">{writtenFrom}</p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        {/* WEB-8 D8: the zone is one the lakehouse lists. The table stays
            free text: this pipeline creates it. */}
        <ZoneField
          id="pipeline-target-zone"
          label="Output zone"
          value={targetZone}
          onChange={onZone}
          zones={zones}
          note={zoneNote}
          describedBy={tie(invalid, "output-zone")["aria-describedby"]}
        />
        <div className="space-y-1.5">
          <Label htmlFor="pipeline-target-table">Output table</Label>
          <Input
            id="pipeline-target-table"
            value={targetTable}
            onChange={(e) => onTable(e.target.value)}
            placeholder="orders_hourly"
            {...tie(invalid, "output-table")}
          />
        </div>
      </div>
      <p className="text-sm text-muted-foreground">Every run rebuilds this table.</p>
      <div className="flex items-center justify-between rounded-lg border border-border px-3 py-2">
        <div>
          <p className="text-sm font-medium">Copy the input as it is</p>
          <p className="text-xs text-muted-foreground">No step: each run copies the input table unchanged.</p>
        </div>
        <Switch
          checked={plainCopy}
          onCheckedChange={onPlainCopy}
          disabled={!plainCopyAllowed}
          aria-label="Copy the input as it is"
          aria-describedby={tie(invalid, "plain-copy")["aria-describedby"]}
        />
      </div>
      {problem}
    </div>
  )
}
