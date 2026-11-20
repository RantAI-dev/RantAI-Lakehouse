"use client"

import * as React from "react"
import Link from "next/link"
import { ArrowDownIcon, ArrowUpIcon, FileCodeIcon, PlusIcon, Trash2Icon, WandSparklesIcon } from "lucide-react"
import { CodeView } from "@/components/patterns/code-view"
import { SelectField } from "@/components/patterns/select-field"
import { SqlEditor } from "@/components/sql-editor"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RequiredMarker } from "@/components/ui/required"
import { Switch } from "@/components/ui/switch"
import { createSqlCompletionSource, sqlCompletionExtension } from "@/features/queries/sql-completion-source"
import { useService } from "@/hooks/use-service"
import {
  MAX_SQL_CHARS,
  formColumns,
  joinOutputColumns,
  measureName,
  readColumns,
  stepCompletion,
  type Column,
  type EditorInput,
  type EditorState,
  type EditorStep,
  type ProblemField,
  type StepRead,
} from "@/lib/pipeline-editor"
import type {
  PipelineJoinColumn,
  PipelineJoinPair,
  PipelineMeasure,
  PipelineStepCondition,
} from "@/services/contracts/pipelines"
import { formatSql } from "@/lib/sql-format"
import { lakehouseService } from "@/services"
import { COLUMNS_APPEAR_LATER, ColumnField, ZoneField } from "./pipeline-pickers"
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
      {/* UX-1-R8: `nameProblem` refuses a blank input/step name, so every
          call site of this field is a required name. */}
      <Label htmlFor={id} required>Name in this pipeline</Label>
      <Input
        id={id}
        aria-required
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
            required
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
          {/* UX-1-R8: an input's table is required whether typed or picked
              from the connector's targets (`editorProblems` field `table`). */}
          <Label htmlFor={tableControl ? `input-table-${input.id}` : undefined} required>Table</Label>
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
                aria-required
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
                aria-required
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
          Step {index + 1} ·{" "}
          {
            // BLD-16 / J9: the add buttons and this header say the same
            // words — SQL, Join, Aggregate, Filter, Operations.
            step.kind === "sql"
              ? "SQL"
              : step.kind === "transform"
                ? "Operations"
                : step.kind === "join"
                  ? "Join"
                  : step.kind === "aggregate"
                    ? "Aggregate"
                    : step.kind === "filter"
                      ? "Filter"
                      : step.stored.kind
          }
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
          {/* UX-1-R8: a transform step must read an input or an earlier step
              (`editorProblems` field `reads`). */}
          <Label htmlFor={`step-reads-${step.id}`} required>Reads</Label>
          {/* D6a: an input or an earlier step. The value is the item's id,
              so the choice follows a rename (R1-B2). */}
          <SelectField
            id={`step-reads-${step.id}`}
            className="w-full"
            aria-label="Reads"
            aria-required
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
          {/* UX-1-R8: an empty SELECT is refused by `editorProblems` field
              `sql`; the required state goes on CodeMirror's textbox. */}
          <p className="flex items-center gap-1.5 text-sm font-medium" id={`step-sql-label-${step.id}`}>
            SQL <RequiredMarker />
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
          <SqlEditor
            value={step.sql}
            onChange={onSql}
            minHeight="220px"
            wrap
            extensions={extensions}
            labelledBy={`step-sql-label-${step.id}`}
            required
          />
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

// ── BLD-16 (BLD-1 part 2) / J9: the three form panes ─────────────────────

/** What a save refusal named on a form step, while the step is unchanged (R1-S7's rule, kept for the form kinds). */
export type FailedRow = { field: string | null; row: number | null; message: string | null }

/** The `on`/`match` value a stored mode carries, for a two-choice picker. */
function modeValue(stored: string): string {
  return stored === "any" ? "any" : "all"
}

/**
 * The picker of what a form step reads: an input or an earlier step, by
 * id, so the choice follows a rename (R1-B2). One component because a
 * join's two sides, an aggregate's and a filter's read are the same
 * control with a different label.
 */
function ReadSideField({
  id,
  label,
  field,
  value,
  readable,
  invalid,
  required,
  onChange,
}: {
  id: string
  label: string
  /** Which control the problem about this read is on (`left`, `right`, `reads`). */
  field: ProblemField
  value: StepRead
  readable: { id: string; name: string }[]
  invalid?: Invalid
  required?: boolean
  onChange: (next: StepRead) => void
}) {
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id} required={required}>{label}</Label>
      <SelectField
        id={id}
        className="w-full"
        aria-label={label}
        aria-required={required}
        value={"id" in value ? value.id : ""}
        placeholder="Choose an input or an earlier step"
        options={readable.map((item) => ({ value: item.id, label: item.name }))}
        onValueChange={(id) => onChange({ id })}
        aria-invalid={tie(invalid, field)["aria-invalid"]}
        aria-describedby={tie(invalid, field)["aria-describedby"]}
      />
    </div>
  )
}

/** The six operators a condition may use (J2: the transform grammar's own list). */
const CONDITION_OPERATORS = [
  { value: "=", label: "=" },
  { value: "!=", label: "!=" },
  { value: "<", label: "<" },
  { value: ">", label: ">" },
  { value: "<=", label: "<=" },
  { value: ">=", label: ">=" },
]

/**
 * The rows of one condition list — a filter's conditions or an aggregate's
 * `having` (J9): a column, one of the six operators, and the value. The
 * same rows for both, because both are `StepCondition`s on the wire.
 */
function ConditionRows({
  id,
  label,
  conditions,
  columns,
  field,
  failed,
  invalid,
  describedBy,
  maxRows = 20,
  onChange,
}: {
  id: string
  label: string
  conditions: PipelineStepCondition[]
  /** The columns to offer, or `null` when they are not known (free text then). */
  columns: string[] | null
  field: ProblemField
  failed: FailedRow
  invalid?: Invalid
  describedBy?: string
  maxRows?: number
  onChange: (next: PipelineStepCondition[]) => void
}) {
  const setRow = (i: number, patch: Partial<PipelineStepCondition>) =>
    onChange(conditions.map((row, j) => (j === i ? { ...row, ...patch } : row)))
  return (
    <div className="flex flex-col gap-2">
      <p className="text-sm font-medium">{label}</p>
      {conditions.map((row, i) => {
        const refusedHere = failed.field === field && failed.row === i
        return (
          <div key={i} className="grid gap-2 rounded-md border border-border p-2 sm:grid-cols-[minmax(0,1fr)_6.5rem_minmax(0,1fr)_auto]">
            <ColumnField
              id={`${id}-column-${i}`}
              label="Column"
              required
              value={row.column}
              onChange={(column) => setRow(i, { column })}
              columns={columns}
              placeholder="column"
            />
            <div className="space-y-1.5">
              {i === 0 ? <Label htmlFor={`${id}-operator-${i}`} required>Is</Label> : null}
              <SelectField
                id={`${id}-operator-${i}`}
                className="w-full"
                aria-label={i === 0 ? undefined : "Operator"}
                aria-required
                value={row.operator}
                placeholder="="
                options={CONDITION_OPERATORS}
                onValueChange={(operator) => setRow(i, { operator })}
                aria-invalid={refusedHere || (invalid?.field === field && row.column.trim() === "") ? true : undefined}
                aria-describedby={[describedBy, refusedHere ? `${id}-failed-${i}` : undefined].filter(Boolean).join(" ") || undefined}
              />
            </div>
            <div className="space-y-1.5">
              {i === 0 ? (
                <Label htmlFor={`${id}-value-${i}`}>
                  Value <span className="text-muted-foreground">(compared as text)</span>
                </Label>
              ) : null}
              <Input
                id={`${id}-value-${i}`}
                aria-label={i === 0 ? undefined : "Value"}
                value={row.value}
                onChange={(e) => setRow(i, { value: e.target.value })}
                className="font-mono"
                spellCheck={false}
              />
            </div>
            <Button type="button" variant="ghost" size="icon-sm" className="self-end" aria-label={`Remove condition ${i + 1}`} onClick={() => onChange(conditions.filter((_, j) => j !== i))}>
              <Trash2Icon aria-hidden />
            </Button>
            {refusedHere && failed.message ? (
              <p id={`${id}-failed-${i}`} role="alert" className="text-xs text-destructive sm:col-span-4">
                {failed.message}
              </p>
            ) : null}
          </div>
        )
      })}
      {conditions.length < maxRows ? (
        <Button type="button" variant="outline" size="sm" className="w-fit" onClick={() => onChange([...conditions, { column: "", operator: "=", value: "" }])}>
          <PlusIcon aria-hidden />
          Add a condition
        </Button>
      ) : null}
    </div>
  )
}

/**
 * J3, F3: the sentence under the join type. An inner join says what it is;
 * an outer join also says what happens to a row with no match — the
 * engine's default, `0` or empty text, not a blank (F3), said here because
 * the compiler cannot add `join_use_nulls` (the orchestrator owns the
 * `SETTINGS`).
 */
const JOIN_TYPES = [
  { value: "inner", label: "inner", note: "Only rows that match." },
  { value: "left", label: "left", note: "Every row of the left, matched or not." },
  { value: "right", label: "right", note: "Every row of the right, matched or not." },
  { value: "full", label: "full", note: "Every row of both sides, matched or not." },
]

/** A `join` step: the two sides, the type, the pairs to match on, and the output columns (J9). */
/** A `join` step: the two sides, the type, the pairs to match on, and the output columns (J9). */
export function JoinStepPane({
  step,
  index,
  count,
  readable,
  leftName,
  rightName,
  inputColumns,
  stepColumns,
  failed,
  problem,
  invalid,
  onRename,
  onChange,
  onPreviewItem,
  onMove,
  onRemove,
}: {
  step: Extract<EditorStep, { kind: "join" }>
  index: number
  count: number
  /** The inputs and earlier steps each side may read, by id (R1-B2). */
  readable: { id: string; name: string }[]
  /** What each side reads now, by name, for the sentences. */
  leftName: string
  rightName: string
  /** Every input's and step's columns, by id — the sides' columns are read from here, so a side's CHANGE can fill the output with the columns of the item it now reads. */
  inputColumns: Readonly<Record<string, Column[] | null>>
  stepColumns: Readonly<Record<string, Column[] | null>>
  failed: FailedRow
  problem: React.ReactNode
  invalid?: Invalid
  onRename: (name: string) => void
  onChange: (patch: {
    left?: StepRead
    right?: StepRead
    joinType?: string
    on?: PipelineJoinPair[]
    columns?: PipelineJoinColumn[]
  }) => void
  /** Runs the preview of one read item, to load its columns (D16). */
  onPreviewItem: (itemId: string) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  // J9: a column the API would refuse is not offered, and the pane says why.
  const readItemColumns = (read: StepRead): Column[] | null => readColumns(read, inputColumns, stepColumns)
  const leftColumns = readItemColumns(step.left)
  const rightColumns = readItemColumns(step.right)
  const left = formColumns(leftColumns ?? [])
  const right = formColumns(rightColumns ?? [])
  const droppedColumns = left.dropped || right.dropped
  const joinType = JOIN_TYPES.find((t) => t.value === step.joinType) ?? JOIN_TYPES[0]
  const outer = joinType.value !== "inner"
  const itemName = (read: StepRead): string =>
    "id" in read ? (readable.find((item) => item.id === read.id)?.name ?? "") : ""
  /**
   * J9's fill rule: a change of side, when both sides' columns are then
   * known, fills the output with every column of the left, then every
   * column of the right (`joinOutputColumns`, in `lib/pipeline-editor.ts` —
   * the rule is tested there; this only decides WHEN it runs). The new
   * side's columns are read from the maps, because the pane's own props
   * still describe the state before the change.
   */
  const setSide = (side: "left" | "right", next: StepRead) => {
    const nextLeft = side === "left" ? next : step.left
    const nextRight = side === "right" ? next : step.right
    const nextLeftColumns = readItemColumns(nextLeft)
    const nextRightColumns = readItemColumns(nextRight)
    // BLD-16 review R1: the fill gets the SAME lists the pickers offer —
    // `formColumns` — or a column the API refuses (`order date`) would be
    // filled in and the save, the preview and Show as SQL would all fail.
    const columns =
      nextLeftColumns !== null && nextRightColumns !== null
        ? joinOutputColumns(
            itemName(nextLeft),
            formColumns(nextLeftColumns).columns,
            itemName(nextRight),
            formColumns(nextRightColumns).columns
          )
        : step.columns
    onChange(side === "left" ? { left: next, columns } : { right: next, columns })
  }
  const sideColumns = (side: "left" | "right"): string[] | null => {
    const known = (side === "left" ? left : right).columns
    return known.length === 0 ? null : known.map((column) => column.name)
  }
  const previewSide = (side: "left" | "right") => {
    const read = side === "left" ? step.left : step.right
    if ("id" in read) onPreviewItem(read.id)
  }
  const toggleColumn = (side: "left" | "right", column: string) => {
    const held = step.columns.find((out) => out.side === side && out.column === column)
    onChange({ columns: held ? step.columns.filter((out) => out !== held) : [...step.columns, { side, column }] })
  }
  const setOutputName = (i: number, name: string) =>
    onChange({ columns: step.columns.map((out, j) => (j === i ? { ...out, name } : out)) })

  return (
    <div className="flex flex-col gap-4">
      <StepHeader step={step} index={index} count={count} onMove={onMove} onRemove={onRemove} />
      <div className="grid gap-3 sm:grid-cols-2">
        <NameField id={`step-name-${step.id}`} value={step.name} onChange={onRename} invalid={invalid} />
        <div className="space-y-1.5">
          <Label htmlFor={`step-join-type-${step.id}`}>Join type</Label>
          <SelectField
            id={`step-join-type-${step.id}`}
            className="w-full"
            aria-label="Join type"
            value={step.joinType}
            options={JOIN_TYPES.map((t) => ({ value: t.value, label: t.label }))}
            onValueChange={(joinType) => onChange({ joinType })}
            aria-invalid={tie(invalid, "name")["aria-invalid"]}
          />
          {/* J3: one line per type, so the four are told apart without SQL. */}
          <p className="text-xs text-muted-foreground">{joinType.note}</p>
          {outer ? (
            // F3: the engine's default on the missing side — said here
            // because a run shows it and a save cannot change it.
            <p className="text-xs text-muted-foreground">
              A row with no match gets 0 or an empty text in the other side&apos;s columns, not a blank.
            </p>
          ) : null}
        </div>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <ReadSideField
          id={`step-left-${step.id}`}
          label="Left"
          field="left"
          value={step.left}
          readable={readable}
          invalid={invalid}
          required
          onChange={(next) => setSide("left", next)}
        />
        <ReadSideField
          id={`step-right-${step.id}`}
          label="Right"
          field="right"
          value={step.right}
          readable={readable}
          invalid={invalid}
          required
          onChange={(next) => setSide("right", next)}
        />
      </div>
      <div className="flex flex-col gap-2">
        <p className="text-sm font-medium">Match on</p>
        <p className="text-xs text-muted-foreground">Pairs of columns that must be equal.</p>
        {step.on.map((pair, i) => {
          const refusedHere = failed.field === "on" && failed.row === i
          return (
            <div key={i} className="grid gap-2 rounded-md border border-border p-2 sm:grid-cols-[minmax(0,1fr)_minmax(0,1fr)_auto]">
              <ColumnField
                id={`step-pair-left-${step.id}-${i}`}
                label={`${leftName || "The left side"}'s column`}
                required
                value={pair.left}
                onChange={(left) => onChange({ on: step.on.map((row, j) => (j === i ? { ...row, left } : row)) })}
                columns={sideColumns("left")}
                placeholder="column"
              />
              <ColumnField
                id={`step-pair-right-${step.id}-${i}`}
                label={`${rightName || "The right side"}'s column`}
                required
                value={pair.right}
                onChange={(right) => onChange({ on: step.on.map((row, j) => (j === i ? { ...row, right } : row)) })}
                columns={sideColumns("right")}
                placeholder="column"
              />
              <Button
                type="button"
                variant="ghost"
                size="icon-sm"
                className="self-end"
                aria-label={`Remove pair ${i + 1}`}
                onClick={() => onChange({ on: step.on.filter((_, j) => j !== i) })}
              >
                <Trash2Icon aria-hidden />
              </Button>
              {refusedHere && failed.message ? (
                <p id={`step-pair-failed-${step.id}-${i}`} role="alert" className="text-xs text-destructive sm:col-span-3">
                  {failed.message}
                </p>
              ) : null}
            </div>
          )
        })}
        {step.on.length < 10 ? (
          <Button type="button" variant="outline" size="sm" className="w-fit" onClick={() => onChange({ on: [...step.on, { left: "", right: "" }] })}>
            <PlusIcon aria-hidden />
            Add a pair
          </Button>
        ) : null}
      </div>
      <div className="flex flex-col gap-2">
        <p className="text-sm font-medium">Output columns</p>
        {droppedColumns ? (
          // J9: the honest sentence for what the pickers left out.
          <p className="text-xs text-muted-foreground">
            Columns whose names are not letters, digits and underscores are left out; use a SQL step to read them.
          </p>
        ) : null}
        {(["left", "right"] as const).map((side) => {
          const known = side === "left" ? left : right
          if (known.columns.length === 0) {
            return (
              <p key={side} className="text-xs text-muted-foreground">
                {side === "left" ? leftName || "The left side" : rightName || "The right side"}:&nbsp;
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-6 px-2"
                  onClick={() => previewSide(side)}
                >
                  Load columns
                </Button>
              </p>
            )
          }
          return (
            <div key={side} className="flex flex-col gap-1.5">
              <p className="text-xs font-medium">{side === "left" ? leftName || "The left side" : rightName || "The right side"}</p>
              <div className="flex flex-wrap gap-x-4 gap-y-1.5">
                {known.columns.map((column) => (
                  <label key={`${side}-${column.name}`} className="flex items-center gap-1.5 text-sm">
                    <input
                      type="checkbox"
                      checked={step.columns.some((out) => out.side === side && out.column === column.name)}
                      onChange={() => toggleColumn(side, column.name)}
                    />
                    <span className="font-mono">{column.name}</span>
                    <span className="text-xs text-muted-foreground">{column.type}</span>
                  </label>
                ))}
              </div>
            </div>
          )
        })}
        {step.columns.map((out, i) => (
          <div key={`${out.side}-${out.column}-${i}`} className="grid gap-2 rounded-md border border-border p-2 sm:grid-cols-[5rem_minmax(0,1fr)_minmax(0,1fr)_auto]">
            <p className="self-center font-mono text-xs text-muted-foreground">{out.side}</p>
            <p className="self-center font-mono text-sm">{out.column}</p>
            <div className="space-y-1.5">
              <Label htmlFor={`step-out-name-${step.id}-${i}`}>Output name</Label>
              <Input
                id={`step-out-name-${step.id}-${i}`}
                value={out.name ?? ""}
                onChange={(e) => setOutputName(i, e.target.value)}
                className="font-mono"
                spellCheck={false}
                placeholder={out.column}
                aria-invalid={failed.field === "columns" && failed.row === i ? true : undefined}
                aria-describedby={failed.field === "columns" && failed.row === i && failed.message ? `step-out-failed-${step.id}-${i}` : undefined}
              />
            </div>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="self-end"
              aria-label={`Remove output column ${out.column}`}
              onClick={() => onChange({ columns: step.columns.filter((_, j) => j !== i) })}
            >
              <Trash2Icon aria-hidden />
            </Button>
            {failed.field === "columns" && failed.row === i && failed.message ? (
              <p id={`step-out-failed-${step.id}-${i}`} role="alert" className="text-xs text-destructive sm:col-span-4">
                {failed.message}
              </p>
            ) : null}
          </div>
        ))}
      </div>
      {problem}
    </div>
  )
}


/** An `aggregate` step: group by columns, measures, and which groups to keep (J9). */
export function AggregateStepPane({
  step,
  index,
  count,
  readable,
  readsName,
  columns,
  failed,
  problem,
  invalid,
  onRename,
  onChange,
  onPreviewItem,
  onMove,
  onRemove,
}: {
  step: Extract<EditorStep, { kind: "aggregate" }>
  index: number
  count: number
  /** The inputs and earlier steps it may read, by id (R1-B2). */
  readable: { id: string; name: string }[]
  /** The name of what it reads now, for the sentences below. */
  readsName: string
  columns: Column[] | null
  failed: FailedRow
  problem: React.ReactNode
  invalid?: Invalid
  onRename: (name: string) => void
  onChange: (patch: {
    reads?: StepRead
    groupBy?: string[]
    measures?: PipelineMeasure[]
    havingMatch?: string
    having?: PipelineStepCondition[]
  }) => void
  /** Runs the preview of the read item, to load its columns (D16). */
  onPreviewItem: (itemId: string) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  // J9: a column the API would refuse is not offered, and the pane says why.
  const offered = formColumns(columns ?? [])
  const columnNames = offered.columns.map((column) => column.name)
  const measureFunctions = [
    { value: "count", label: "count" },
    { value: "countDistinct", label: "countDistinct" },
    { value: "sum", label: "sum" },
    { value: "avg", label: "avg" },
    { value: "min", label: "min" },
    { value: "max", label: "max" },
  ]
  /** The names a new measure's auto-name must not take (J9's `measureName`). */
  const takenNames = [...step.groupBy, ...step.measures.map((measure) => measure.name)]
  const setMeasure = (i: number, patch: Partial<PipelineMeasure>) => {
    const measures = step.measures.map((measure, j) => {
      if (j !== i) return measure
      const next = { ...measure, ...patch }
      // J9: a name the person has not typed is filled in —
      // `<function>_<column>`, `count` for a bare count, made unique.
      if (next.name === "") next.name = measureName(next.function, next.column ?? "", takenNames)
      return next
    })
    onChange({ measures })
  }
  const havingChoices = [...step.groupBy, ...step.measures.map((measure) => measure.name)]
  const readId = "id" in step.reads ? step.reads.id : null

  return (
    <div className="flex flex-col gap-4">
      <StepHeader step={step} index={index} count={count} onMove={onMove} onRemove={onRemove} />
      <div className="grid gap-3 sm:grid-cols-2">
        <NameField id={`step-name-${step.id}`} value={step.name} onChange={onRename} invalid={invalid} />
        <ReadSideField
          id={`step-reads-${step.id}`}
          label="Reads"
          field="reads"
          value={step.reads}
          readable={readable}
          invalid={invalid}
          required
          onChange={(reads) => onChange({ reads })}
        />
      </div>
      <div className="flex flex-col gap-2">
        <p className="text-sm font-medium">Group by</p>
        {offered.dropped ? (
          <p className="text-xs text-muted-foreground">
            Columns whose names are not letters, digits and underscores are left out; use a SQL step to read them.
          </p>
        ) : null}
        {columns === null || columnNames.length === 0 ? (
          <p className="text-xs text-muted-foreground">
            {readId !== null ? (
              <>
                <Button
                  type="button"
                  variant="outline"
                  size="sm"
                  className="h-6 px-2"
                  onClick={() => onPreviewItem(readId)}
                >
                  Load columns
                </Button>
                {" "}of {readsName || "what this step reads"}, or type the columns below.
              </>
            ) : (
              "Choose what this step reads first."
            )}
          </p>
        ) : (
          <div className="flex flex-wrap gap-x-4 gap-y-1.5">
            {offered.columns.map((column) => (
              <label key={column.name} className="flex items-center gap-1.5 text-sm">
                <input
                  type="checkbox"
                  checked={step.groupBy.includes(column.name)}
                  onChange={() =>
                    onChange({
                      groupBy: step.groupBy.includes(column.name)
                        ? step.groupBy.filter((name) => name !== column.name)
                        : [...step.groupBy, column.name],
                    })
                  }
                />
                <span className="font-mono">{column.name}</span>
                <span className="text-xs text-muted-foreground">{column.type}</span>
              </label>
            ))}
          </div>
        )}
        {step.groupBy.map((name, i) => (
          <div key={`${name}-${i}`} className="flex items-center gap-2">
            <p className="font-mono text-sm">{name}</p>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              aria-label={`Remove group-by column ${name}`}
              onClick={() => onChange({ groupBy: step.groupBy.filter((_, j) => j !== i) })}
            >
              <Trash2Icon aria-hidden />
            </Button>
          </div>
        ))}
        {step.groupBy.length === 0 ? (
          <p className="text-xs text-muted-foreground">No group-by column: the whole read becomes one row.</p>
        ) : null}
      </div>
      <div className="flex flex-col gap-2">
        <p className="text-sm font-medium">Measures</p>
        {step.measures.map((measure, i) => (
          <div key={i} className="grid gap-2 rounded-md border border-border p-2 sm:grid-cols-[minmax(0,8rem)_minmax(0,1fr)_minmax(0,1fr)_auto]">
            <div className="space-y-1.5">
              {i === 0 ? <Label htmlFor={`step-measure-function-${step.id}-${i}`}>Function</Label> : null}
              <SelectField
                id={`step-measure-function-${step.id}-${i}`}
                className="w-full"
                aria-label="Function"
                value={measure.function}
                options={measureFunctions}
                onValueChange={(fn) => setMeasure(i, { function: fn })}
                aria-invalid={failed.field === "measures" && failed.row === i ? true : undefined}
                aria-describedby={failed.field === "measures" && failed.row === i && failed.message ? `step-measure-failed-${step.id}-${i}` : undefined}
              />
            </div>
            <ColumnField
              id={`step-measure-column-${step.id}-${i}`}
              label="Column (count may leave it empty)"
              value={measure.column ?? ""}
              onChange={(column) => setMeasure(i, { column })}
              columns={columnNames.length === 0 ? null : columnNames}
              placeholder={measure.function === "count" ? "empty counts rows" : "column"}
            />
            <div className="space-y-1.5">
              {i === 0 ? (
                <Label htmlFor={`step-measure-name-${step.id}-${i}`} required>
                  Name
                </Label>
              ) : null}
              <Input
                id={`step-measure-name-${step.id}-${i}`}
                aria-label="Name"
                aria-required
                value={measure.name}
                onChange={(e) => setMeasure(i, { name: e.target.value })}
                className="font-mono"
                spellCheck={false}
              />
            </div>
            <Button
              type="button"
              variant="ghost"
              size="icon-sm"
              className="self-end"
              aria-label={`Remove measure ${i + 1}`}
              onClick={() => onChange({ measures: step.measures.filter((_, j) => j !== i) })}
            >
              <Trash2Icon aria-hidden />
            </Button>
            {failed.field === "measures" && failed.row === i && failed.message ? (
              <p id={`step-measure-failed-${step.id}-${i}`} role="alert" className="text-xs text-destructive sm:col-span-4">
                {failed.message}
              </p>
            ) : null}
          </div>
        ))}
        {step.measures.length < 50 ? (
          <Button
            type="button"
            variant="outline"
            size="sm"
            className="w-fit"
            onClick={() => onChange({ measures: [...step.measures, { function: "sum", column: "", name: "" }] })}
          >
            <PlusIcon aria-hidden />
            Add a measure
          </Button>
        ) : null}
      </div>
      <div>
        <div className="mb-2 grid gap-2 sm:grid-cols-[minmax(0,10rem)_minmax(0,1fr)]">
          <div className="space-y-1.5">
            <Label htmlFor={`step-having-match-${step.id}`}>Only keep groups where</Label>
            <SelectField
              id={`step-having-match-${step.id}`}
              className="w-full"
              aria-label="Only keep groups where"
              value={modeValue(step.havingMatch)}
              options={[
                { value: "all", label: "all hold" },
                { value: "any", label: "any holds" },
              ]}
              onValueChange={(mode) => onChange({ havingMatch: mode })}
            />
          </div>
          <p className="self-end text-xs text-muted-foreground">
            A condition names a group-by column or a measure.
            {step.measures.some((measure) => ["count", "countDistinct", "sum", "avg"].includes(measure.function))
              ? " A count, sum or average is compared with a number."
              : ""}
          </p>
        </div>
        <ConditionRows
          id={`step-having-${step.id}`}
          label="Conditions on the groups"
          conditions={step.having}
          columns={havingChoices.length === 0 ? null : havingChoices}
          field="having"
          failed={failed}
          invalid={invalid}
          describedBy={tie(invalid, "having")["aria-describedby"]}
          onChange={(having) => onChange({ having })}
        />
      </div>
      {problem}
    </div>
  )
}

/** A `filter` step: keep the rows of what it reads that meet all — or any — of its conditions (J9). */
export function FilterStepPane({
  step,
  index,
  count,
  readable,
  readsName,
  columns,
  failed,
  problem,
  invalid,
  onRename,
  onChange,
  onPreviewItem,
  onMove,
  onRemove,
}: {
  step: Extract<EditorStep, { kind: "filter" }>
  index: number
  count: number
  /** The inputs and earlier steps it may read, by id (R1-B2). */
  readable: { id: string; name: string }[]
  /** The name of what it reads now, for the sentences below. */
  readsName: string
  columns: Column[] | null
  failed: FailedRow
  problem: React.ReactNode
  invalid?: Invalid
  onRename: (name: string) => void
  onChange: (patch: { reads?: StepRead; match?: string; conditions?: PipelineStepCondition[] }) => void
  /** Runs the preview of the read item, to load its columns (D16). */
  onPreviewItem: (itemId: string) => void
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  // J9: a column the API would refuse is not offered, and the pane says why.
  const offered = formColumns(columns ?? [])
  const columnNames = offered.columns.map((column) => column.name)
  const readId = "id" in step.reads ? step.reads.id : null
  return (
    <div className="flex flex-col gap-4">
      <StepHeader step={step} index={index} count={count} onMove={onMove} onRemove={onRemove} />
      <div className="grid gap-3 sm:grid-cols-2">
        <NameField id={`step-name-${step.id}`} value={step.name} onChange={onRename} invalid={invalid} />
        <ReadSideField
          id={`step-reads-${step.id}`}
          label="Reads"
          field="reads"
          value={step.reads}
          readable={readable}
          invalid={invalid}
          required
          onChange={(reads) => onChange({ reads })}
        />
      </div>
      {offered.dropped ? (
        <p className="text-xs text-muted-foreground">
          Columns whose names are not letters, digits and underscores are left out; use a SQL step to read them.
        </p>
      ) : null}
      {columns === null || columnNames.length === 0 ? (
        <p className="text-xs text-muted-foreground">
          {readId !== null ? (
            <>
              <Button
                type="button"
                variant="outline"
                size="sm"
                className="h-6 px-2"
                onClick={() => onPreviewItem(readId)}
              >
                Load columns
              </Button>
              {" "}of {readsName || "what this step reads"}, or type them below.
            </>
          ) : (
            "Choose what this step reads first."
          )}
        </p>
      ) : null}
      <div className="grid gap-2 sm:grid-cols-[minmax(0,10rem)]">
        <div className="space-y-1.5">
          <Label htmlFor={`step-match-${step.id}`}>Keep rows where</Label>
          <SelectField
            id={`step-match-${step.id}`}
            className="w-full"
            aria-label="Keep rows where"
            value={modeValue(step.match)}
            options={[
              { value: "all", label: "all hold" },
              { value: "any", label: "any holds" },
            ]}
            onValueChange={(mode) => onChange({ match: mode })}
            aria-invalid={tie(invalid, "conditions")["aria-invalid"]}
            aria-describedby={tie(invalid, "conditions")["aria-describedby"]}
          />
        </div>
      </div>
      <ConditionRows
        id={`step-conditions-${step.id}`}
        label="Conditions"
        conditions={step.conditions}
        columns={columnNames.length === 0 ? null : columnNames}
        field="conditions"
        failed={failed}
        invalid={invalid}
        describedBy={tie(invalid, "conditions")["aria-describedby"]}
        onChange={(conditions) => onChange({ conditions })}
      />
      <p className="text-xs text-muted-foreground">
        Every column of {readsName || "what this step reads"} is kept; a condition only chooses the rows.
      </p>
      {problem}
    </div>
  )
}

/**
 * J6, on every form pane (the three new kinds and `transform`): the step as
 * the API compiles it. "Show as SQL" asks the API and shows the answer
 * read-only — formatted with `formatSql` for the eye only, never saved —
 * and the console renders no SQL of its own. "Switch to SQL step" is the
 * one-way take-over: its confirm lives in the page, and the step is
 * replaced with the route's own text, unformatted.
 */
export function StepSqlSection({
  text,
  stale,
  error,
  pending,
  onShow,
  onSwitch,
}: {
  /** The route's own text, or `null` while none was shown. */
  text: string | null
  /** Whether the definition changed since the text was shown (R1-S5's rule, as the preview's). */
  stale: boolean
  /** A refusal from the route, shown here (D7). */
  error: string | null
  pending: boolean
  onShow: () => void
  onSwitch: () => void
}) {
  return (
    <div className="flex flex-col gap-2 border-t border-border pt-3">
      <div className="flex flex-wrap gap-1">
        <Button type="button" variant="outline" size="sm" onClick={onShow} disabled={pending}>
          <FileCodeIcon aria-hidden />
          {pending ? "Building the SQL…" : text !== null ? "Show as SQL again" : "Show as SQL"}
        </Button>
        <Button type="button" variant="outline" size="sm" onClick={onSwitch} disabled={pending}>
          <WandSparklesIcon aria-hidden />
          Switch to SQL step
        </Button>
      </div>
      {error ? <PaneProblem message={error} tone="error" /> : null}
      {text !== null && stale ? (
        <p role="status" className="text-xs font-medium text-amber-700 dark:text-amber-400">
          Stale: the definition changed since this SQL was shown.
        </p>
      ) : null}
      {text !== null ? <CodeView text={formatSql(text)} language="sql" /> : null}
    </div>
  )
}

/**
 * A step of a kind this console does not know (BLD-1 part 1 S3 review
 * R1-S2; part 2 adds kinds). It is shown, it can be moved or removed as a
 * step of the pipeline, and it is saved back exactly as stored; nothing in
 * it is edited here, because its fields are not known here.
 *
 * BLD-16: a stored step whose fields do not parse to its kind's shape
 * (`knownStep`) also opens here, reading its own editor shape — shown, not
 * edited, exactly as `unknown` is.
 */
export function UnknownStepPane({
  step,
  index,
  count,
  problem,
  onMove,
  onRemove,
}: {
  step: EditorStep
  index: number
  count: number
  problem: React.ReactNode
  /** Which control the pane's problem is about (R1-S10). */
  invalid?: Invalid
  onMove: (delta: -1 | 1) => void
  onRemove: () => void
}) {
  const stored = step.kind === "unknown" ? step.stored : null
  const kind = step.kind === "unknown" ? step.stored.kind : step.kind
  return (
    <div className="flex flex-col gap-4">
      <StepHeader step={step} index={index} count={count} onMove={onMove} onRemove={onRemove} />
      <p className="text-sm text-muted-foreground">
        This step is of kind <span className="font-mono text-foreground">{kind}</span>, which this console
        cannot edit yet. It is saved back exactly as it is stored.
      </p>
      <CodeView text={JSON.stringify(stored ?? step, null, 2)} language="plain" />
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
          required
          value={targetZone}
          onChange={onZone}
          zones={zones}
          note={zoneNote}
          describedBy={tie(invalid, "output-zone")["aria-describedby"]}
        />
        <div className="space-y-1.5">
          {/* UX-1-R8: both output fields are required by `editorProblems`. */}
          <Label htmlFor="pipeline-target-table" required>Output table</Label>
          <Input
            id="pipeline-target-table"
            aria-required
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
