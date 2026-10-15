"use client";

import * as React from "react";
import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { moveItem, PICK_LIMITS, type TableDraft } from "@/lib/table-draft";
import { AGGREGATES, COLUMN_FORMATS, COMPARE_PERIODS, type ColumnSetting, type PivotTotals } from "@/lib/table-types";

/** The columns the picked source offers, and what each holds. */
export type SourceColumns = { all: readonly string[]; dates: readonly string[] };

const FORMAT_LABELS: Record<string, string> = {
  auto: "Automatic", number: "Number", percent: "Percent", currency: "Currency (Rp)", date: "Date", link: "Link", image: "Image",
};
const TOTALS_LABELS: Record<PivotTotals, string> = { none: "No totals", grand: "Grand totals", all: "Totals and subtotals" };
const PERIOD_LABELS: Record<string, string> = { day: "Day", week: "Week", month: "Month", quarter: "Quarter", year: "Year" };
const DIR_LABELS: Record<string, string> = { asc: "Ascending", desc: "Descending" };
const GOOD_LABELS: Record<string, string> = { up: "Up is good", down: "Down is good" };
const NO_SORT = "__none__";
const DECIMALS = ["0", "1", "2", "3", "4", "5", "6"];

/** Format, label, digits, width, wrap and hide for one column. */
export function ColumnSettingsFields({
  column, setting, onChange, canHide,
}: {
  readonly column: string;
  readonly setting: ColumnSetting | undefined;
  readonly onChange: (next: ColumnSetting) => void;
  readonly canHide?: boolean;
}) {
  const s = setting ?? {};
  const numeric = ["number", "percent", "currency"].includes(s.format ?? "auto");
  return (
    <div className="grid grid-cols-2 gap-2 pt-2">
      <div className="grid gap-1">
        <Label htmlFor={`cs-label-${column}`} className="text-xs">Label</Label>
        <Input id={`cs-label-${column}`} value={s.label ?? ""} placeholder={column} onChange={(e) => onChange({ ...s, label: e.target.value || undefined })} />
      </div>
      <div className="grid gap-1">
        <Label className="text-xs">Format</Label>
        <Select value={s.format ?? "auto"} items={FORMAT_LABELS} onValueChange={(v) => onChange({ ...s, format: v ?? "auto" })}>
          <SelectTrigger className="w-full" aria-label={`Format of ${column}`}><SelectValue /></SelectTrigger>
          <SelectContent>{COLUMN_FORMATS.map((f) => <SelectItem key={f} value={f}>{FORMAT_LABELS[f]}</SelectItem>)}</SelectContent>
        </Select>
      </div>
      {numeric ? (
        <div className="grid gap-1">
          <Label className="text-xs">Decimals</Label>
          <Select value={s.decimals === undefined ? "auto" : String(s.decimals)} items={{ auto: "Automatic" }} onValueChange={(v) => onChange({ ...s, decimals: v === "auto" || v === null ? undefined : Number(v) })}>
            <SelectTrigger className="w-full" aria-label={`Decimals of ${column}`}><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="auto">Automatic</SelectItem>
              {DECIMALS.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
      ) : null}
      <div className="grid gap-1">
        <Label htmlFor={`cs-width-${column}`} className="text-xs">Width (px)</Label>
        <Input
          id={`cs-width-${column}`} type="number" min={60} max={800} value={s.width ?? ""} placeholder="auto"
          onChange={(e) => onChange({ ...s, width: e.target.value === "" ? undefined : Math.max(60, Math.min(800, Number(e.target.value) || 60)) })}
        />
      </div>
      <div className="col-span-2 flex flex-wrap items-center gap-4 text-xs">
        <Label className="gap-1.5 text-xs font-normal"><Checkbox checked={!!s.wrap} onCheckedChange={(c) => onChange({ ...s, wrap: c === true })} /> Wrap text</Label>
        {canHide ? <Label className="gap-1.5 text-xs font-normal"><Checkbox checked={!!s.hidden} onCheckedChange={(c) => onChange({ ...s, hidden: c === true })} /> Hide</Label> : null}
      </div>
    </div>
  );
}

/** An ordered list of columns: add from the source's, move, remove. */
export function ColumnList({
  label, value, options, max, onChange, renderExtra, placeholder = "Add a column", required,
}: {
  readonly label: string;
  readonly value: readonly string[];
  readonly options: readonly string[];
  readonly max: number;
  readonly onChange: (next: string[]) => void;
  readonly renderExtra?: (column: string) => React.ReactNode;
  readonly placeholder?: string;
  readonly required?: boolean;
}) {
  const free = options.filter((o) => !value.includes(o));
  return (
    <div className="grid gap-1.5">
      <Label>{label} {required ? <span aria-hidden className="text-destructive">*</span> : null}</Label>
      <ul className="grid gap-1">
        {value.map((column, i) => (
          <li key={column} className="rounded-md border border-border px-2 py-1">
            <div className="flex items-center gap-1">
              <span className="min-w-0 flex-1 truncate text-sm">{column}</span>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move ${column} up`} disabled={i === 0} onClick={() => onChange(moveItem(value, i, -1))}><ArrowUp /></Button>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move ${column} down`} disabled={i === value.length - 1} onClick={() => onChange(moveItem(value, i, 1))}><ArrowDown /></Button>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove ${column}`} onClick={() => onChange(value.filter((c) => c !== column))}><X /></Button>
            </div>
            {renderExtra?.(column)}
          </li>
        ))}
      </ul>
      {value.length < max && free.length ? (
        <Select value="" onValueChange={(v) => { if (v) onChange([...value, v]); }}>
          <SelectTrigger className="w-full" aria-label={label}><Plus className="size-3.5 text-muted-foreground" aria-hidden /><SelectValue placeholder={placeholder} /></SelectTrigger>
          <SelectContent>{free.map((o) => <SelectItem key={o} value={o}>{o}</SelectItem>)}</SelectContent>
        </Select>
      ) : null}
    </div>
  );
}

/** One column's settings behind a disclosure, so a long list stays short. */
function SettingsDisclosure({ column, draft, onChange, canHide }: { column: string; draft: TableDraft; onChange: (d: TableDraft) => void; canHide?: boolean }) {
  return (
    <details className="text-xs">
      <summary className="cursor-pointer select-none text-muted-foreground">Format</summary>
      <ColumnSettingsFields
        column={column} setting={draft.settings[column]} canHide={canHide}
        onChange={(next) => onChange({ ...draft, settings: { ...draft.settings, [column]: next } })}
      />
    </details>
  );
}

/** Table: grouped summary or raw rows, with the columns, their order, the sort and each one's format. */
export function TableSection({ draft, onChange, columns, groupedColumns }: {
  readonly draft: TableDraft;
  readonly onChange: (d: TableDraft) => void;
  readonly columns: SourceColumns;
  /** The columns a grouped table shows (dimension and measure), whose formats can be set. */
  readonly groupedColumns: readonly string[];
}) {
  const rows = draft.mode === "rows";
  const sortItems: Record<string, string> = { [NO_SORT]: "Natural order", ...Object.fromEntries(draft.columns.map((c) => [c, c])) };
  return (
    <div className="grid gap-3">
      <div className="grid gap-1.5">
        <Label>Rows</Label>
        <Select value={draft.mode} items={{ grouped: "Grouped summary", rows: "Raw rows" }} onValueChange={(v) => onChange({ ...draft, mode: v === "rows" ? "rows" : "grouped" })}>
          <SelectTrigger className="w-full" aria-label="Table mode"><SelectValue /></SelectTrigger>
          <SelectContent>
            <SelectItem value="grouped">Grouped summary</SelectItem>
            <SelectItem value="rows">Raw rows</SelectItem>
          </SelectContent>
        </Select>
      </div>
      {rows ? (
        <>
          <ColumnList
            label="Columns" required value={draft.columns} options={columns.all} max={PICK_LIMITS.columns}
            onChange={(next) => onChange({ ...draft, columns: next, sortColumn: next.includes(draft.sortColumn) ? draft.sortColumn : "" })}
            renderExtra={(c) => <SettingsDisclosure column={c} draft={draft} onChange={onChange} canHide />}
          />
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Sort by</Label>
              <Select value={draft.sortColumn || NO_SORT} items={sortItems} onValueChange={(v) => onChange({ ...draft, sortColumn: v === NO_SORT || !v ? "" : v })}>
                <SelectTrigger className="w-full" aria-label="Sort by"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={NO_SORT}>Natural order</SelectItem>
                  {draft.columns.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Direction</Label>
              <Select value={draft.sortDir} items={DIR_LABELS} disabled={!draft.sortColumn} onValueChange={(v) => onChange({ ...draft, sortDir: v === "desc" ? "desc" : "asc" })}>
                <SelectTrigger className="w-full" aria-label="Sort direction"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="asc">{DIR_LABELS.asc}</SelectItem>
                  <SelectItem value="desc">{DIR_LABELS.desc}</SelectItem>
                </SelectContent>
              </Select>
            </div>
          </div>
        </>
      ) : groupedColumns.length ? (
        <div className="grid gap-1">
          {groupedColumns.map((c) => (
            <div key={c} className="rounded-md border border-border px-2 py-1">
              <p className="text-sm">{c}</p>
              <SettingsDisclosure column={c} draft={draft} onChange={onChange} canHide />
            </div>
          ))}
        </div>
      ) : null}
    </div>
  );
}

/** Pivot: row and column fields, up to five values, totals. */
export function PivotSection({ draft, onChange, columns }: {
  readonly draft: TableDraft;
  readonly onChange: (d: TableDraft) => void;
  readonly columns: SourceColumns;
}) {
  const values = draft.pivotValues;
  const setValue = (i: number, patch: Partial<{ column: string; aggregate: string }>) =>
    onChange({ ...draft, pivotValues: values.map((v, j) => (j === i ? { ...v, ...patch } : v)) });
  return (
    <div className="grid gap-3">
      <ColumnList
        label="Rows" required value={draft.pivotRows} max={PICK_LIMITS.pivotRows}
        options={columns.all.filter((c) => !draft.pivotColumns.includes(c))}
        onChange={(next) => onChange({ ...draft, pivotRows: next })}
      />
      <ColumnList
        label="Columns" value={draft.pivotColumns} max={PICK_LIMITS.pivotColumns} placeholder="Add a column field"
        options={columns.all.filter((c) => !draft.pivotRows.includes(c))}
        onChange={(next) => onChange({ ...draft, pivotColumns: next })}
      />
      <div className="grid gap-1.5">
        <Label>Values <span aria-hidden className="text-destructive">*</span></Label>
        <ul className="grid gap-1">
          {values.map((v, i) => (
            <li key={i} className="rounded-md border border-border px-2 py-1">
              <div className="flex items-center gap-1">
                <Select value={v.aggregate} onValueChange={(a) => setValue(i, { aggregate: a ?? "sum" })}>
                  <SelectTrigger className="w-24" aria-label={`Aggregate ${i + 1}`}><SelectValue /></SelectTrigger>
                  <SelectContent>{AGGREGATES.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                </Select>
                <Select value={v.column} onValueChange={(c) => setValue(i, { column: c ?? "" })}>
                  <SelectTrigger className="w-full min-w-0 flex-1" aria-label={`Value column ${i + 1}`}><SelectValue placeholder="pick a column" /></SelectTrigger>
                  <SelectContent>{columns.all.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
                </Select>
                {values.length > 1 ? <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove value ${i + 1}`} onClick={() => onChange({ ...draft, pivotValues: values.filter((_, j) => j !== i) })}><X /></Button> : null}
              </div>
              {v.column ? <SettingsDisclosure column={v.column} draft={draft} onChange={onChange} /> : null}
            </li>
          ))}
        </ul>
        {values.length < PICK_LIMITS.pivotValues ? (
          <Button type="button" size="sm" variant="outline" onClick={() => onChange({ ...draft, pivotValues: [...values, { column: "", aggregate: "sum" }] })}>
            <Plus className="size-3.5" /> Add a value
          </Button>
        ) : null}
      </div>
      <div className="grid gap-1.5">
        <Label>Totals</Label>
        <Select value={draft.totals} items={TOTALS_LABELS} onValueChange={(v) => onChange({ ...draft, totals: (v as PivotTotals) ?? "none" })}>
          <SelectTrigger className="w-full" aria-label="Totals"><SelectValue /></SelectTrigger>
          <SelectContent>
            {(Object.keys(TOTALS_LABELS) as PivotTotals[]).map((t) => <SelectItem key={t} value={t}>{TOTALS_LABELS[t]}</SelectItem>)}
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}

/** KPI: compare with the previous period of a date column, or with a goal. */
export function KpiCompareSection({ draft, onChange, columns }: {
  readonly draft: TableDraft;
  readonly onChange: (d: TableDraft) => void;
  readonly columns: SourceColumns;
}) {
  const kinds = { none: "No comparison", previous: "Previous period", goal: "Goal" };
  return (
    <div className="grid gap-3">
      <div className="grid gap-1.5">
        <Label>Compare with</Label>
        <Select value={draft.compareKind} items={kinds} onValueChange={(v) => onChange({ ...draft, compareKind: (v as TableDraft["compareKind"]) ?? "none" })}>
          <SelectTrigger className="w-full" aria-label="Compare with"><SelectValue /></SelectTrigger>
          <SelectContent>
            {(Object.keys(kinds) as (keyof typeof kinds)[]).map((k) => <SelectItem key={k} value={k}>{kinds[k]}</SelectItem>)}
          </SelectContent>
        </Select>
      </div>
      {draft.compareKind === "previous" ? (
        <div className="grid grid-cols-2 gap-3">
          <div className="grid gap-1.5">
            <Label>Date column <span aria-hidden className="text-destructive">*</span></Label>
            <Select value={draft.compareColumn} onValueChange={(v) => onChange({ ...draft, compareColumn: v ?? "" })}>
              <SelectTrigger className="w-full" aria-label="Date column"><SelectValue placeholder={columns.dates.length ? "pick a column" : "no date column"} /></SelectTrigger>
              <SelectContent>{columns.dates.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
            </Select>
          </div>
          <div className="grid gap-1.5">
            <Label>Period</Label>
            <Select value={draft.comparePeriod} items={PERIOD_LABELS} onValueChange={(v) => onChange({ ...draft, comparePeriod: (v as TableDraft["comparePeriod"]) ?? "month" })}>
              <SelectTrigger className="w-full" aria-label="Period"><SelectValue /></SelectTrigger>
              <SelectContent>{COMPARE_PERIODS.map((p) => <SelectItem key={p} value={p}>{PERIOD_LABELS[p]}</SelectItem>)}</SelectContent>
            </Select>
          </div>
        </div>
      ) : null}
      {draft.compareKind === "goal" ? (
        <div className="grid gap-1.5">
          <Label htmlFor="kpi-goal">Goal <span aria-hidden className="text-destructive">*</span></Label>
          <Input id="kpi-goal" type="number" value={draft.goal} onChange={(e) => onChange({ ...draft, goal: e.target.value })} />
        </div>
      ) : null}
      {draft.compareKind !== "none" ? (
        <div className="grid gap-1.5">
          <Label>Better when</Label>
          <Select value={draft.goodDirection} items={GOOD_LABELS} onValueChange={(v) => onChange({ ...draft, goodDirection: v === "down" ? "down" : "up" })}>
            <SelectTrigger className="w-full" aria-label="Better when"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value="up">{GOOD_LABELS.up}</SelectItem>
              <SelectItem value="down">{GOOD_LABELS.down}</SelectItem>
            </SelectContent>
          </Select>
        </div>
      ) : null}
    </div>
  );
}
