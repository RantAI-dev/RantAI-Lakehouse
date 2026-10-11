"use client";

import * as React from "react";
import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { filterLabel, isActiveFilter } from "@/lib/dashboard-filter-state";
import { MEASURE_AGGREGATES, MAX_GROUPINGS, type Grouping, type MeasureRow } from "@/lib/chart-shape";
import { moveItem } from "@/lib/table-draft";
import { GRAIN_LABELS, grainChoices, type ColumnKind } from "@/lib/time-grain";
import type { FilterDef } from "@/services/clients/bi-store";
import { DateEditor, NumberEditor, TextEditor, blankFilter, type EditorProps } from "./filters/filter-editors";

/**
 * The chart builder's sections for `BI-6`: the list of measures (each with
 * an aggregate of its own), the chart's own filters, and the groupings of a
 * table. Plain controls over the builder's state; the builder owns the state
 * and the payload.
 */

const DEFAULT_AGGREGATE = "__default__";

export type ColumnChoice = { name: string; kind: ColumnKind };

/** The measures: one row each, with its column, its aggregate, and move / remove. */
export function MeasureRows({
  rows, onChange, labels, min, max, own, columns, defaultAggregate, disabled, placeholder, wholeMeasure, fieldLabel,
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
  /** The chart's aggregate, shown as the default of each row. */
  readonly defaultAggregate: string;
  readonly disabled: boolean;
  readonly placeholder: string;
  /** Calculated fields that are the whole measure (aggregate or table level): they take no aggregate. */
  readonly wholeMeasure: ReadonlySet<string>;
  readonly fieldLabel: (name: string) => React.ReactNode;
}) {
  const set = (i: number, patch: Partial<MeasureRow>) => onChange(rows.map((r, j) => (j === i ? { ...r, ...patch } : r)));
  const flexible = max > min;
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
            {own && max > 1 ? (
              <Select
                value={row.aggregate || DEFAULT_AGGREGATE}
                items={{ [DEFAULT_AGGREGATE]: `Default (${defaultAggregate})` }}
                onValueChange={(v) => set(i, { aggregate: v === DEFAULT_AGGREGATE || v === null ? "" : v })}
                disabled={disabled || wholeMeasure.has(row.column)}
              >
                <SelectTrigger className="w-36 shrink-0" aria-label={`Aggregation of ${row.column || labels(i)}`}><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={DEFAULT_AGGREGATE}>Default ({defaultAggregate})</SelectItem>
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
  value, onChange, columns, disabled,
}: {
  readonly value: readonly FilterDef[];
  readonly onChange: (next: FilterDef[]) => void;
  readonly columns: readonly ColumnChoice[];
  readonly disabled: boolean;
}) {
  // `null`: nothing open; a column: a new filter on it; a number: editing that one.
  const [open, setOpen] = React.useState<{ column: string } | { index: number } | null>(null);
  const kindOf = (column: string) => columns.find((c) => c.name === column)?.kind ?? "text";
  const unused = columns.filter((c) => !value.some((f) => f.column === c.name));
  const others = (column: string) => value.filter((f) => f.column !== column && isActiveFilter(f));
  const editing = open && "index" in open ? value[open.index] : undefined;
  const column = open ? ("column" in open ? open.column : editing?.column ?? "") : "";
  return (
    <div className="grid gap-1.5">
      <Label>Filters</Label>
      {value.length ? (
        <ul className="flex flex-wrap gap-1.5">
          {value.map((f, i) => (
            <li key={`${f.column}:${f.op ?? "in"}`} className="inline-flex h-7 items-center rounded-md border border-border bg-background text-sm">
              <Button
                type="button" variant="ghost" size="sm" className="h-7 max-w-64 rounded-r-none px-2 font-normal"
                title={filterLabel(f, kindOf(f.column))} onClick={() => setOpen(open && "index" in open && open.index === i ? null : { index: i })}
              >
                <span className="truncate">{filterLabel(f, kindOf(f.column))}</span>
              </Button>
              <Button type="button" variant="ghost" size="icon-sm" className="size-7 rounded-l-none" aria-label={`Remove ${f.column} filter`}
                onClick={() => { onChange(value.filter((_, j) => j !== i)); setOpen(null); }}>
                <X className="size-3.5" />
              </Button>
            </li>
          ))}
        </ul>
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

/** A table's groupings: 1 to five columns, in order, a date column optionally cut by a grain. */
export function GroupingsEditor({
  value, onChange, columns, disabled, placeholder, fieldLabel,
}: {
  readonly value: readonly Grouping[];
  readonly onChange: (next: Grouping[]) => void;
  readonly columns: readonly ColumnChoice[];
  readonly disabled: boolean;
  readonly placeholder: string;
  readonly fieldLabel: (name: string) => React.ReactNode;
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
            <li key={g.column} className="flex items-center gap-1 rounded-md border border-border px-2 py-1">
              <span className="min-w-0 flex-1 truncate text-sm">{fieldLabel(g.column)}</span>
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
        <Select value="" onValueChange={(v) => { if (v) onChange([...value, { column: v }]); }} disabled={disabled}>
          <SelectTrigger className="w-full" aria-label="Group by"><Plus className="size-3.5 text-muted-foreground" aria-hidden /><SelectValue placeholder={value.length ? "Add another column" : placeholder} /></SelectTrigger>
          <SelectContent>{free.map((c) => <SelectItem key={c.name} value={c.name}>{fieldLabel(c.name)}</SelectItem>)}</SelectContent>
        </Select>
      ) : null}
    </div>
  );
}
