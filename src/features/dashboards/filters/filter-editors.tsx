"use client";

import * as React from "react";
import { SelectField } from "@/components/patterns/select-field";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";
import {
  datePickToFilter, filterToDatePick, filterToNumberPick, isActiveFilter, numberPickToFilter, opOf,
  type DatePick, type NumberPick,
} from "@/lib/dashboard-filter-state";
import type { FilterDef, FilterKind, RelativeUnit } from "@/services/clients/bi-store";
import { ValueList } from "./value-list";
import type { OwnSource } from "./use-filter-values";

export type EditorProps = {
  board: string;
  column: string;
  /** The board's other filters, which narrow value lists. */
  others: FilterDef[];
  /**
   * `BI-47`: the chart's own mart (with its joins) or saved SQL source, whose
   * values the list reads; absent for a dashboard filter, which asks by board.
   */
  source?: OwnSource;
  /** The filter being edited; a fresh one for a column that has none yet. */
  initial: FilterDef;
  /** Commit; the caller closes the popover. */
  onApply: (next: FilterDef) => void;
};

/** What a new, empty filter on a column of this kind looks like. */
export function blankFilter(column: string, kind: FilterKind | string): FilterDef {
  if (kind === "number") return { column, values: [], op: "between" };
  if (kind === "date" || kind === "datetime") return { column, values: [], op: "relative", anchor: "last", unit: "day", n: 30 };
  return { column, values: [] };
}

function ApplyRow({ disabled, onApply }: { disabled: boolean; onApply: () => void }) {
  return (
    <div className="flex justify-end border-t border-border pt-2">
      <Button size="sm" disabled={disabled} onClick={onApply}>Apply</Button>
    </div>
  );
}

const TEXT_OPS = [
  ["contains", "contains"], ["not_contains", "does not contain"], ["starts_with", "starts with"], ["ends_with", "ends with"],
] as const;

export function TextEditor({ board, column, others, initial, onApply, source }: EditorProps) {
  const isText = ["contains", "not_contains", "starts_with", "ends_with"].includes(opOf(initial));
  const [tab, setTab] = React.useState<string>(isText ? "text" : "values");
  const [values, setValues] = React.useState<FilterDef>(isText ? { column, values: [] } : initial);
  const [match, setMatch] = React.useState<FilterDef>(isText ? initial : { column, values: [], op: "contains", text: "" });
  const current = tab === "values" ? values : match;
  return (
    <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
      <TabsList className="w-full">
        <TabsTrigger value="values">Values</TabsTrigger>
        <TabsTrigger value="text">Text</TabsTrigger>
      </TabsList>
      <TabsContent value="values">
        <ValueList board={board} column={column} others={others} draft={values} onDraft={setValues} source={source} />
      </TabsContent>
      <TabsContent value="text">
        <div className="flex flex-col gap-2">
          <div className="inline-flex w-fit flex-wrap rounded-md border border-border p-0.5 text-xs" role="group" aria-label="Match mode">
            {TEXT_OPS.map(([op, label]) => (
              <button
                key={op}
                type="button"
                aria-pressed={match.op === op}
                onClick={() => setMatch({ ...match, op })}
                className={cn("rounded px-2 py-0.5", match.op === op ? "bg-muted font-medium text-foreground" : "text-muted-foreground hover:text-foreground")}
              >
                {label}
              </button>
            ))}
          </div>
          <Input
            value={match.text ?? ""}
            maxLength={200}
            onChange={(e) => setMatch({ ...match, text: e.target.value })}
            placeholder="Text to match, any case"
            aria-label="Text to match"
          />
        </div>
      </TabsContent>
      <ApplyRow disabled={!isActiveFilter(current)} onApply={() => onApply(current)} />
    </Tabs>
  );
}

const NUMBER_MODES = [
  ["between", "between"], ["eq", "equal to"], ["ne", "not equal to"], ["gt", "greater than"], ["lt", "less than"],
] as const;

export function NumberEditor({ board, column, others, initial, onApply, source }: EditorProps) {
  const pick = filterToNumberPick(initial);
  const isList = !pick && ["in", "not_in"].includes(opOf(initial)) && initial.values.length > 0;
  const [tab, setTab] = React.useState<string>(isList ? "values" : "compare");
  const [mode, setMode] = React.useState<NumberPick["mode"]>(pick?.mode ?? "between");
  const [value, setValue] = React.useState(pick && pick.mode !== "between" ? pick.value : "");
  const [min, setMin] = React.useState(pick?.mode === "between" ? (pick.min ?? "") : "");
  const [max, setMax] = React.useState(pick?.mode === "between" ? (pick.max ?? "") : "");
  const [list, setList] = React.useState<FilterDef>(isList ? initial : { column, values: [] });
  const compare = numberPickToFilter(column, mode === "between" ? { mode, min, max } : { mode, value });
  const current = tab === "compare" ? compare : list;
  const field = (v: string, set: (v: string) => void, label: string, placeholder = "") => (
    <Input type="number" step="any" value={v} onChange={(e) => set(e.target.value)} placeholder={placeholder} aria-label={label} />
  );
  return (
    <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
      <TabsList className="w-full">
        <TabsTrigger value="compare">Compare</TabsTrigger>
        <TabsTrigger value="values">Values</TabsTrigger>
      </TabsList>
      <TabsContent value="compare">
        <div className="flex flex-col gap-2">
          <SelectField aria-label="Comparison" value={mode} className="w-full"
            options={NUMBER_MODES.map(([value, label]) => ({ value, label }))}
            onValueChange={(next) => setMode(next as NumberPick["mode"])} />
          {mode === "between" ? (
            <div className="flex gap-2">{field(min, setMin, "From", "From")}{field(max, setMax, "To", "To")}</div>
          ) : (
            field(value, setValue, "Number")
          )}
        </div>
      </TabsContent>
      <TabsContent value="values">
        <ValueList board={board} column={column} others={others} draft={list} onDraft={setList} source={source} />
      </TabsContent>
      <ApplyRow disabled={!current || !isActiveFilter(current)} onApply={() => current && onApply(current)} />
    </Tabs>
  );
}

type Preset = { label: string; pick: DatePick };
const PRESETS: Preset[] = [
  { label: "Last 7 days", pick: { mode: "last", unit: "day", n: 7 } },
  { label: "Last 30 days", pick: { mode: "last", unit: "day", n: 30 } },
  { label: "Last 90 days", pick: { mode: "last", unit: "day", n: 90 } },
  { label: "This month", pick: { mode: "this", unit: "month" } },
  { label: "This quarter", pick: { mode: "this", unit: "quarter" } },
  { label: "This year", pick: { mode: "this", unit: "year" } },
  { label: "Previous month", pick: { mode: "previous", unit: "month" } },
  { label: "Previous year", pick: { mode: "previous", unit: "year" } },
];
const UNITS: RelativeUnit[] = ["day", "week", "month", "quarter", "year"];

const DATE_MODES = [
  ["on", "On"], ["before", "Before"], ["after", "After"], ["range", "Between"], ["month", "Month"], ["quarter", "Quarter"],
] as const;
type DateMode = (typeof DATE_MODES)[number][0];
const MONTH_NAMES = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

export function DateEditor({ column, initial, onApply }: EditorProps) {
  const pick = filterToDatePick(initial);
  const relativeNow = pick && ["last", "next", "this", "previous"].includes(pick.mode);
  const [tab, setTab] = React.useState<string>(pick && !relativeNow ? "date" : "relative");
  // Relative: "Last / Next N units".
  const [dir, setDir] = React.useState<"last" | "next">(pick?.mode === "next" ? "next" : "last");
  const [n, setN] = React.useState(String(pick && (pick.mode === "last" || pick.mode === "next") ? pick.n : 30));
  const [unit, setUnit] = React.useState<RelativeUnit>(pick && "unit" in pick && (pick.mode === "last" || pick.mode === "next") ? pick.unit : "day");
  // Date: one mode at a time. Year starts empty rather than from a clock, so
  // the editor shows nothing it did not get from the stored filter.
  const [mode, setMode] = React.useState<DateMode>(pick && !relativeNow ? (pick.mode as DateMode) : "on");
  const [date, setDate] = React.useState(pick && "date" in pick ? pick.date : "");
  const [from, setFrom] = React.useState(pick?.mode === "range" ? (pick.from ?? "") : "");
  const [to, setTo] = React.useState(pick?.mode === "range" ? (pick.to ?? "") : "");
  const [year, setYear] = React.useState(pick && (pick.mode === "month" || pick.mode === "quarter") ? String(pick.year) : "");
  const [month, setMonth] = React.useState(pick?.mode === "month" ? pick.month : 1);
  const [quarter, setQuarter] = React.useState(pick?.mode === "quarter" ? pick.quarter : 1);
  const apply = (p: DatePick) => {
    const f = datePickToFilter(column, p);
    if (f) onApply(f);
  };
  const datePick: DatePick =
    mode === "range" ? { mode, from, to }
    : mode === "month" ? { mode, year: Number(year), month }
    : mode === "quarter" ? { mode, year: Number(year), quarter }
    : { mode, date };
  const datePickOk = datePickToFilter(column, datePick) !== null;
  const relativePick: DatePick = { mode: dir, n: Number(n), unit };
  const input = (v: string, set: (v: string) => void, label: string) => (
    <Input type="date" value={v} onChange={(e) => set(e.target.value)} aria-label={label} />
  );
  return (
    <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
      <TabsList className="w-full">
        <TabsTrigger value="relative">Relative</TabsTrigger>
        <TabsTrigger value="date">Date</TabsTrigger>
      </TabsList>
      <TabsContent value="relative">
        <div className="flex flex-col gap-2">
          <div className="grid grid-cols-2 gap-1">
            {PRESETS.map((p) => (
              <Button key={p.label} size="sm" variant="outline" onClick={() => apply(p.pick)}>{p.label}</Button>
            ))}
          </div>
          <div className="flex items-end gap-2 border-t border-border pt-2">
            <SelectField aria-label="Direction" value={dir} className="w-20"
              options={[{ value: "last", label: "Last" }, { value: "next", label: "Next" }]}
              onValueChange={(next) => setDir(next as "last" | "next")} />
            <Input type="number" min={1} max={3650} value={n} onChange={(e) => setN(e.target.value)} aria-label="How many" className="w-16" />
            <SelectField aria-label="Unit" value={unit}
              options={UNITS.map((u) => ({ value: u, label: `${u}s` }))}
              onValueChange={(next) => setUnit(next as RelativeUnit)} />
            <Button size="sm" disabled={datePickToFilter(column, relativePick) === null} onClick={() => apply(relativePick)}>Apply</Button>
          </div>
        </div>
      </TabsContent>
      <TabsContent value="date">
        <div className="flex flex-col gap-2">
          <SelectField aria-label="Date filter" value={mode} className="w-full"
            options={DATE_MODES.map(([value, label]) => ({ value, label }))}
            onValueChange={(next) => setMode(next as DateMode)} />
          {mode === "range" ? (
            <div className="flex gap-2">{input(from, setFrom, "From")}{input(to, setTo, "To")}</div>
          ) : mode === "month" || mode === "quarter" ? (
            <div className="flex gap-2">
              {mode === "month" ? (
                <SelectField aria-label="Month" value={String(month)} className="w-full"
                  options={MONTH_NAMES.map((name, i) => ({ value: String(i + 1), label: name }))}
                  onValueChange={(next) => setMonth(Number(next))} />
              ) : (
                <SelectField aria-label="Quarter" value={String(quarter)} className="w-full"
                  options={[1, 2, 3, 4].map((q) => ({ value: String(q), label: `Q${q}` }))}
                  onValueChange={(next) => setQuarter(Number(next))} />
              )}
              <Input type="number" min={1900} max={2299} value={year} onChange={(e) => setYear(e.target.value)} placeholder="Year" aria-label="Year" className="w-24" />
            </div>
          ) : (
            input(date, setDate, mode === "on" ? "On" : mode === "before" ? "Before" : "After")
          )}
          <ApplyRow disabled={!datePickOk} onApply={() => apply(datePick)} />
        </div>
      </TabsContent>
    </Tabs>
  );
}
