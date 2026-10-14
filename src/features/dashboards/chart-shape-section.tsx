"use client";

import * as React from "react";
import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { RowsTable } from "@/components/patterns/rows-table";
import { filterLabel, isActiveFilter } from "@/lib/dashboard-filter-state";
import {
  MEASURE_AGGREGATES, MAX_GROUPINGS, MAX_STEPS, columnsAfterSteps, isMetricColumn, storedAggregate, visibleColumns,
  type Grouping, type MeasureRow, type ResultColumns, type StepCondition, type StepDef,
} from "@/lib/chart-shape";
import { binDraftFromDef, binPayload, type BinDef } from "@/lib/chart-bins";
import { moveItem } from "@/lib/table-draft";
import { GRAIN_LABELS, grainChoices, type ColumnKind } from "@/lib/time-grain";
import type { FilterDef } from "@/services/clients/bi-store";
import { BinPicker } from "./bin-picker";
import { ColumnList } from "./table-settings";
import { DateEditor, NumberEditor, TextEditor, blankFilter, type EditorProps } from "./filters/filter-editors";

/**
 * The chart builder's sections for `BI-6`: the list of measures (each with
 * an aggregate of its own), the chart's own filters, and the groupings of a
 * table. Plain controls over the builder's state; the builder owns the state
 * and the payload.
 */

export type ColumnChoice = { name: string; kind: ColumnKind };

/**
 * The measures: one row each, with its column, its aggregation, and move / remove.
 * Every row shows the aggregation it is computed with (phase 2 QA R4), so there is no
 * second, chart-wide control to reconcile: a row that stores none shows the chart's
 * (`defaultAggregate`), and choosing that same aggregate again stores none, which
 * keeps the result name a chart saved without an aggregate of its own always had.
 * A kind that takes the chart's one aggregate (`aggregateMode: "chart"`) writes it
 * through `onDefaultAggregate`; a box plot has a fixed one (`"fixed"`).
 */
export function MeasureRows({
  rows, onChange, labels, min, max, own, columns, defaultAggregate, disabled, placeholder, wholeMeasure, fieldLabel,
  onDefaultAggregate, fixedNote,
}: {
  readonly rows: readonly MeasureRow[];
  readonly onChange: (next: MeasureRow[]) => void;
  /** The label of each row by position, for the fixed roles ("X metric"). */
  readonly labels: (index: number) => string;
  readonly min: number;
  readonly max: number;
  /** Whether a row may carry an aggregate of its own. */
  readonly own: boolean;
  readonly columns: readonly string[];
  /** The aggregate of a row that stores none: the chart's. */
  readonly defaultAggregate: string;
  readonly disabled: boolean;
  readonly placeholder: string;
  /** Calculated fields that are the whole measure (aggregate or table level): they take no aggregate. */
  readonly wholeMeasure: ReadonlySet<string>;
  readonly fieldLabel: (name: string) => React.ReactNode;
  /** For a kind with one chart-wide aggregate and no aggregate per row: where the pick goes. */
  readonly onDefaultAggregate?: (aggregate: string) => void;
  /** For a kind whose aggregation is fixed: the words that say so, in place of the control. */
  readonly fixedNote?: string;
}) {
  const set = (i: number, patch: Partial<MeasureRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const flexible = max > min;
  const aggregateItems = Object.fromEntries(MEASURE_AGGREGATES.map((a) => [a, a]));
  return (
    <div className="grid gap-2">
      {rows.map((row, i) => (
        <div key={i} className="grid gap-1.5">
          <Label>{labels(i)} {i < min ? <span aria-hidden className="text-destructive">*</span> : null}</Label>
          <div className="flex items-center gap-1.5">
            <Select value={row.column} onValueChange={(v) => set(i, { column: v ?? "" })} disabled={disabled}>
              <SelectTrigger className="w-full min-w-0 flex-1" aria-label={labels(i)}><SelectValue placeholder={placeholder} /></SelectTrigger>
              <SelectContent>{columns.map((m) => <SelectItem key={m} value={m}>{fieldLabel(m)}</SelectItem>)}</SelectContent>
            </Select>
            {fixedNote ? (
              <span className="shrink-0 text-xs text-muted-foreground">{fixedNote}</span>
            ) : (own || onDefaultAggregate) && !isMetricColumn(row.column) && !wholeMeasure.has(row.column) ? (
              <Select
                value={row.aggregate || defaultAggregate}
                items={aggregateItems}
                onValueChange={(v) => {
                  if (v === null) return;
                  if (own) set(i, { aggregate: storedAggregate(v, defaultAggregate) });
                  else onDefaultAggregate?.(v);
                }}
                disabled={disabled}
              >
                <SelectTrigger className="w-28 shrink-0" aria-label={`Aggregation of ${row.column || labels(i)}`}><SelectValue /></SelectTrigger>
                <SelectContent>
                  {MEASURE_AGGREGATES.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}
                </SelectContent>
              </Select>
            ) : null}
            {flexible ? (
              <>
                <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move measure ${i + 1} up`} disabled={i === 0} onClick={() => onChange(moveItem(rows, i, -1))}><ArrowUp /></Button>
                <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move measure ${i + 1} down`} disabled={i === rows.length - 1} onClick={() => onChange(moveItem(rows, i, 1))}><ArrowDown /></Button>
                <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove measure ${i + 1}`} disabled={rows.length <= min} onClick={() => onChange(rows.filter((_, j) => j !== i))}><X /></Button>
              </>
            ) : null}
          </div>
        </div>
      ))}
      {flexible && rows.length < max ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={disabled} onClick={() => onChange([...rows, { column: "", aggregate: "" }])}>
            <Plus className="size-3" aria-hidden /> Add measure
          </Button>
        </div>
      ) : null}
    </div>
  );
}

/** The editor for one filter, by the kind of its column. */
function FilterEditor({ kind, ...props }: EditorProps & { kind: string }) {
  if (kind === "number") return <NumberEditor {...props} />;
  if (kind === "date" || kind === "datetime") return <DateEditor {...props} />;
  return <TextEditor {...props} />;
}

/**
 * The chart's own filters (`BI-6`): the same editors as the dashboard's
 * filters, on the columns of the chart's data, shown inline (a popup over a
 * dialog would be outside its focus trap). They are always applied, and a
 * dashboard filter can only narrow them further.
 */
export function ChartFilters({
  value, onChange, columns, disabled, segments = [],
}: {
  readonly value: readonly FilterDef[];
  readonly onChange: (next: FilterDef[]) => void;
  readonly columns: readonly ColumnChoice[];
  readonly disabled: boolean;
  /** BI-2: the saved segments of the chart's table; one is added as a reference by name and stands for its filters. */
  readonly segments?: readonly { name: string; label: string }[];
}) {
  // `null`: nothing open; a column: a new filter on it; a number: editing that one.
  const [open, setOpen] = React.useState<{ column: string } | { index: number } | null>(null);
  const kindOf = (column: string) => columns.find((c) => c.name === column)?.kind ?? "text";
  const unused = columns.filter((c) => !value.some((f) => !f.segment && f.column === c.name));
  const others = (column: string) => value.filter((f) => !f.segment && f.column !== column && isActiveFilter(f));
  const editing = open && "index" in open ? value[open.index] : undefined;
  const column = open ? ("column" in open ? open.column : editing?.column ?? "") : "";
  return (
    <div className="grid gap-1.5">
      <Label>Filters</Label>
      {value.length ? (
        <ul className="flex flex-wrap gap-1.5">
          {value.map((f, i) => (
            <li key={f.segment ? `segment:${f.segment}` : `${f.column}:${f.op ?? "in"}`} className="inline-flex h-7 items-center rounded-md border border-border bg-background text-sm">
              <Button
                type="button" variant="ghost" size="sm" className="h-7 max-w-64 rounded-r-none px-2 font-normal" disabled={!!f.segment}
                title={f.segment ? `Segment ${segments.find((s) => s.name === f.segment)?.label ?? f.segment}` : filterLabel(f, kindOf(f.column))} onClick={() => setOpen(open && "index" in open && open.index === i ? null : { index: i })}
              >
                <span className="truncate">{f.segment ? `Segment: ${segments.find((s) => s.name === f.segment)?.label ?? f.segment}` : filterLabel(f, kindOf(f.column))}</span>
              </Button>
              <Button type="button" variant="ghost" size="icon-sm" className="size-7 rounded-l-none" aria-label={f.segment ? `Remove segment ${f.segment}` : `Remove ${f.column} filter`}
                onClick={() => { onChange(value.filter((_, j) => j !== i)); setOpen(null); }}>
                <X className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
      ) : null}
      {!open && segments.some((s) => !value.some((f) => f.segment === s.name)) ? (
        <Select value="" onValueChange={(v) => { if (v) onChange([...value, { column: "", values: [], segment: v }]); }} disabled={disabled}>
          <SelectTrigger className="w-full" aria-label="Add a segment"><Plus className="size-3.5 text-muted-foreground" aria-hidden /><SelectValue placeholder="Add a segment" /></SelectTrigger>
          <SelectContent>{segments.filter((s) => !value.some((f) => f.segment === s.name)).map((s) => <SelectItem key={s.name} value={s.name}>{s.label || s.name}</SelectItem>)}</SelectContent>
        </Select>
      ) : null}
      {open ? (
        <div className="rounded-md border border-border p-2.5" aria-label={`Filter on ${column}`}>
          <p className="pb-2 text-xs font-medium text-muted-foreground">{column}</p>
          <FilterEditor
            // A value list of a mart's column reads every mart that has it: the chart has no dashboard yet.
            board="" column={column} others={others(column)} kind={kindOf(column)}
            initial={editing ?? blankFilter(column, kindOf(column))}
            onApply={(next) => {
              onChange(editing && open && "index" in open ? value.map((f, j) => (j === open.index ? next : f)) : [...value, next]);
              setOpen(null);
            }}
          />
        </div>
      ) : null}
      {!open && unused.length ? (
        <Select value="" onValueChange={(v) => { if (v) setOpen({ column: v }); }} disabled={disabled}>
          <SelectTrigger className="w-full" aria-label="Add a filter"><Plus className="size-3.5 text-muted-foreground" aria-hidden /><SelectValue placeholder="Add a filter on a column" /></SelectTrigger>
          <SelectContent>{unused.map((c) => <SelectItem key={c.name} value={c.name}>{c.name}</SelectItem>)}</SelectContent>
        </Select>
      ) : null}
    </div>
  );
}

const NO_GRAIN = "__no_grain__";

/** A bin as the grouping keeps it while it is edited: a width still being typed stays as typed. */
function asBinDef(draft: { mode: "auto" | "count" | "width"; count: string; width: string }): BinDef {
  if (draft.mode === "auto") return { mode: "auto" };
  if (draft.mode === "count") return { mode: "count", count: draft.count };
  return binPayload(draft) ?? { mode: "width", width: draft.width };
}

/** A table's groupings: 1 to five columns, in order, a date column optionally cut by a grain. */
export function GroupingsEditor({
  value, onChange, columns, disabled, placeholder, fieldLabel, sqlSource = false,
}: {
  readonly value: readonly Grouping[];
  readonly onChange: (next: Grouping[]) => void;
  readonly columns: readonly ColumnChoice[];
  readonly disabled: boolean;
  readonly placeholder: string;
  readonly fieldLabel: (name: string) => React.ReactNode;
  /** A SQL source takes a bin width only (BI-10). */
  readonly sqlSource?: boolean;
}) {
  const free = columns.filter((c) => !value.some((g) => g.column === c.name));
  return (
    <div className="grid gap-1.5">
      <Label>Group by <span aria-hidden className="text-destructive">*</span></Label>
      <ul className="grid gap-1">
        {value.map((g, i) => {
          const kind = columns.find((c) => c.name === g.column)?.kind;
          const grains = grainChoices("bar", kind);
          return (
            <li key={g.column} className="flex flex-wrap items-center gap-1 rounded-md border border-border px-2 py-1">
              <span className="min-w-0 flex-1 truncate text-sm">{fieldLabel(g.column)}</span>
              {kind === "number" ? (
                <div className="w-56 shrink-0">
                  <BinPicker
                    value={binDraftFromDef(g.bin)} sqlSource={sqlSource} aria={`Bins of ${g.column}`}
                    onChange={(draft) => onChange(value.map((x, j) => (j === i ? { column: x.column, ...(draft ? { bin: asBinDef(draft) } : {}) } : x)))}
                  />
                </div>
              ) : null}
              {grains.length ? (
                <Select
                  value={g.grain || NO_GRAIN}
                  items={{ [NO_GRAIN]: "As is", ...GRAIN_LABELS }}
                  onValueChange={(v) => onChange(value.map((x, j) => (j === i ? { column: x.column, ...(v && v !== NO_GRAIN ? { grain: v } : {}) } : x)))}
                >
                  <SelectTrigger className="h-7 w-32 shrink-0" aria-label={`Group ${g.column} by`}><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value={NO_GRAIN}>As is</SelectItem>
                    {grains.map((gr) => <SelectItem key={gr} value={gr}>{GRAIN_LABELS[gr]}</SelectItem>)}
                  </SelectContent>
                </Select>
              ) : null}
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move ${g.column} up`} disabled={i === 0} onClick={() => onChange(moveItem(value, i, -1))}><ArrowUp /></Button>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move ${g.column} down`} disabled={i === value.length - 1} onClick={() => onChange(moveItem(value, i, 1))}><ArrowDown /></Button>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove ${g.column}`} onClick={() => onChange(value.filter((_, j) => j !== i))}><X /></Button>
            </li>
          );
        })}
      </ul>
      {value.length < MAX_GROUPINGS && free.length ? (
        <Select value="" onValueChange={(v) => {
          // A number column is grouped in bins, as a distinct value per number would be unreadable (BI-10).
          if (v) onChange([...value, free.find((c) => c.name === v)?.kind === "number" ? { column: v, bin: { mode: "auto" } } : { column: v }]);
        }} disabled={disabled}>
          <SelectTrigger className="w-full" aria-label="Group by"><Plus className="size-3.5 text-muted-foreground" aria-hidden /><SelectValue placeholder={value.length ? "Add another column" : placeholder} /></SelectTrigger>
          <SelectContent>{free.map((c) => <SelectItem key={c.name} value={c.name}>{fieldLabel(c.name)}</SelectItem>)}</SelectContent>
        </Select>
      ) : null}
    </div>
  );
}

const STEP_OPS: Record<string, string> = {
  eq: "is", ne: "is not", gt: "is greater than", gte: "is at least", lt: "is less than", lte: "is at most", between: "is between",
};
const STEP_TYPES: Record<string, string> = { filter: "Filter", summarise: "Summarise" };

/** One condition of a filter step: a column of the result before it, a comparison, and the value(s). */
function ConditionRow({
  value, onChange, onRemove, columns, kindOf, label,
}: {
  readonly value: StepCondition;
  readonly onChange: (next: StepCondition) => void;
  readonly onRemove: () => void;
  readonly columns: readonly string[];
  readonly kindOf: (column: string) => string;
  readonly label: (column: string) => string;
}) {
  const kind = kindOf(value.column);
  const type = kind === "number" ? "number" : kind === "date" || kind === "datetime" ? "date" : "text";
  const ops = type === "text" ? ["eq", "ne"] : Object.keys(STEP_OPS);
  const input = (v: string | undefined, set: (v: string) => void, aria: string) => (
    <Input type={type} step="any" value={v ?? ""} onChange={(e) => set(e.target.value)} aria-label={aria} className="min-w-0 flex-1" />
  );
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Select value={value.column} items={Object.fromEntries(columns.map((c) => [c, label(c)]))} onValueChange={(v) => onChange({ ...value, column: v ?? "" })}>
        <SelectTrigger className="w-40 shrink-0" aria-label="Condition column"><SelectValue placeholder="column" /></SelectTrigger>
        <SelectContent>{columns.map((c) => <SelectItem key={c} value={c}>{label(c)}</SelectItem>)}</SelectContent>
      </Select>
      <Select value={ops.includes(value.op) ? value.op : ops[0]} items={STEP_OPS} onValueChange={(v) => onChange({ ...value, op: v ?? "eq" })}>
        <SelectTrigger className="w-40 shrink-0" aria-label="Comparison"><SelectValue /></SelectTrigger>
        <SelectContent>{ops.map((o) => <SelectItem key={o} value={o}>{STEP_OPS[o]}</SelectItem>)}</SelectContent>
      </Select>
      {value.op === "between" ? (
        <>
          {input(value.min, (min) => onChange({ ...value, min }), "From")}
          {input(value.max, (max) => onChange({ ...value, max }), "To")}
        </>
      ) : input(value.value, (v) => onChange({ ...value, value: v }), "Value")}
      <Button type="button" size="icon-xs" variant="ghost" aria-label="Remove condition" onClick={onRemove}><X /></Button>
    </div>
  );
}

/**
 * The steps of a chart (`BI-6` part 2): after the summary, filter its result
 * (every condition must hold) or summarise it again, up to two. Each step reads
 * the columns of the one before it, so the choices follow what the step before
 * left.
 */
export function StepsEditor({
  value, onChange, base, disabled,
}: {
  readonly value: readonly StepDef[];
  readonly onChange: (next: StepDef[]) => void;
  /** The chart's grouped result, which the first step reads. */
  readonly base: ResultColumns;
  readonly disabled: boolean;
}) {
  const before = (i: number) => columnsAfterSteps(base, value.slice(0, i));
  const set = (i: number, step: StepDef) => onChange(value.map((s, j) => (j === i ? step : s)));
  const blankFilter = (cols: ResultColumns): StepDef => ({ filter: [{ column: cols.measures[0] ?? cols.groups[0]?.name ?? "", op: cols.measures.length ? "gt" : "eq", value: "" }] });
  const blankSummary = (cols: ResultColumns): StepDef => ({
    summarise: { by: cols.groups[0] ? [cols.groups[0].name] : [], measures: cols.measures[0] ? [{ column: cols.measures[0], aggregate: "sum" }] : [] },
  });
  return (
    <div className="grid gap-2">
      <Label>Steps</Label>
      {value.map((step, i) => {
        const cols = before(i);
        const names = [...cols.groups.map((g) => g.name), ...cols.measures];
        const label = (c: string) => cols.labels[c] ?? c;
        const kindOf = (c: string) => (cols.measures.includes(c) ? "number" : cols.groups.find((g) => g.name === c)?.kind ?? "text");
        return (
          <div key={i} className="grid gap-2 rounded-md border border-border p-2.5" aria-label={`Step ${i + 1}`}>
            <div className="flex items-center gap-2">
              <span className="text-xs font-medium text-muted-foreground">Step {i + 1}</span>
              <Select
                value={step.summarise ? "summarise" : "filter"} items={STEP_TYPES}
                onValueChange={(v) => set(i, v === "summarise" ? blankSummary(cols) : blankFilter(cols))}
              >
                <SelectTrigger className="h-7 w-36" aria-label={`Step ${i + 1} type`}><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="filter">Filter the result</SelectItem>
                  <SelectItem value="summarise">Summarise again</SelectItem>
                </SelectContent>
              </Select>
              <Button type="button" size="icon-xs" variant="ghost" className="ml-auto" aria-label={`Remove step ${i + 1}`} onClick={() => onChange(value.filter((_, j) => j !== i))}><X /></Button>
            </div>
            {step.summarise ? (
              <>
                {cols.groups.length ? (
                  <ColumnList
                    label="Keep" value={step.summarise.by} options={cols.groups.map((g) => g.name)} max={MAX_GROUPINGS}
                    onChange={(by) => set(i, { summarise: { ...step.summarise!, by } })} placeholder="Add a column"
                  />
                ) : (
                  // Phase 2 QA R8: nothing to keep is said once, with no empty list to open.
                  <p className="text-xs text-muted-foreground">There is no grouping to keep: group the chart by a column first.</p>
                )}
                <MeasureRows
                  rows={step.summarise.measures.map((m) => ({ column: m.column, aggregate: m.aggregate }))}
                  onChange={(rows) => set(i, { summarise: { ...step.summarise!, measures: rows.map((r) => ({ column: r.column, aggregate: r.aggregate || "sum" })) } })}
                  labels={(j) => `Measure ${j + 1}`} min={1} max={6} own columns={cols.measures} defaultAggregate="sum"
                  disabled={disabled} placeholder="pick a measure" wholeMeasure={new Set()} fieldLabel={label}
                />
              </>
            ) : (
              <div className="grid gap-1.5">
                {(step.filter ?? []).map((c, j) => (
                  <ConditionRow
                    key={j} value={c} columns={names} kindOf={kindOf} label={label}
                    onChange={(next) => set(i, { filter: (step.filter ?? []).map((x, k) => (k === j ? next : x)) })}
                    onRemove={() => set(i, { filter: (step.filter ?? []).filter((_, k) => k !== j) })}
                  />
                ))}
                <div>
                  <Button
                    type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={disabled}
                    onClick={() => set(i, { filter: [...(step.filter ?? []), blankFilter(cols).filter![0]] })}
                  >
                    <Plus className="size-3" aria-hidden /> Add condition
                  </Button>
                </div>
              </div>
            )}
          </div>
        );
      })}
      {value.length < MAX_STEPS ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={disabled}
            onClick={() => onChange([...value, blankFilter(before(value.length))])}>
            <Plus className="size-3" aria-hidden /> Add a step
          </Button>
        </div>
      ) : null}
    </div>
  );
}

/**
 * Up to 2,000 rows of the chart's last result, under the chart in the builder's
 * preview (`BI-6` part 2): what the steps left, before the chart's own limit.
 * Open for a chart with steps, folded for any other. Internal columns are not
 * listed.
 */
export function PreviewRowsTable({
  data, open,
}: {
  readonly data: { columns: string[]; rows: Record<string, unknown>[]; truncated: boolean; limit: number };
  readonly open: boolean;
}) {
  const columns = visibleColumns(data.columns);
  return (
    <details open={open} className="rounded-md border border-border text-xs">
      <summary className="cursor-pointer px-2 py-1.5 font-medium">
        Rows ({data.rows.length.toLocaleString("en-US")}{data.truncated ? ` of more than ${data.limit.toLocaleString("en-US")}` : ""})
      </summary>
      {data.truncated ? <p className="px-2 pb-1 text-muted-foreground">Only the first {data.limit.toLocaleString("en-US")} rows are shown.</p> : null}
      <div className="max-h-56 overflow-auto">
        <RowsTable columns={columns} rows={data.rows} />
      </div>
    </details>
  );
}
