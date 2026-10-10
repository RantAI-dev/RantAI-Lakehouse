"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Tabs, TabsList, TabsTrigger, TabsContent } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";
import { isActiveFilter, opOf } from "@/lib/dashboard-filter-state";
import type { FilterDef, FilterKind, RelativeUnit } from "@/services/clients/bi-store";
import { ValueList } from "./value-list";

export type EditorProps = {
  board: string;
  column: string;
  /** The board's other filters, which narrow value lists. */
  others: FilterDef[];
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

const TEXT_OPS = [["contains", "contains"], ["starts_with", "starts with"], ["ends_with", "ends with"]] as const;

export function TextEditor({ board, column, others, initial, onApply }: EditorProps) {
  const isText = ["contains", "starts_with", "ends_with"].includes(opOf(initial));
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
        <ValueList board={board} column={column} others={others} draft={values} onDraft={setValues} />
      </TabsContent>
      <TabsContent value="text">
        <div className="flex flex-col gap-2">
          <div className="inline-flex w-fit rounded-md border border-border p-0.5 text-xs" role="group" aria-label="Match mode">
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

export function NumberEditor({ board, column, others, initial, onApply }: EditorProps) {
  const isList = ["in", "not_in"].includes(opOf(initial)) && initial.values.length > 0;
  const [tab, setTab] = React.useState<string>(isList ? "values" : "range");
  const [range, setRange] = React.useState<FilterDef>(isList ? { column, values: [], op: "between" } : { ...initial, op: "between" });
  const [list, setList] = React.useState<FilterDef>(isList ? initial : { column, values: [] });
  const current = tab === "range" ? range : list;
  const bound = (key: "min" | "max", label: string) => (
    <label className="flex flex-1 flex-col gap-1 text-xs text-muted-foreground">
      {label}
      <Input
        type="number"
        step="any"
        value={range[key] ?? ""}
        onChange={(e) => setRange({ ...range, [key]: e.target.value || undefined })}
        placeholder="No limit"
      />
    </label>
  );
  return (
    <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
      <TabsList className="w-full">
        <TabsTrigger value="range">Range</TabsTrigger>
        <TabsTrigger value="values">Values</TabsTrigger>
      </TabsList>
      <TabsContent value="range">
        <div className="flex gap-2">{bound("min", "From (≥)")}{bound("max", "To (≤)")}</div>
      </TabsContent>
      <TabsContent value="values">
        <ValueList board={board} column={column} others={others} draft={list} onDraft={setList} />
      </TabsContent>
      <ApplyRow disabled={!isActiveFilter(current)} onApply={() => onApply(current)} />
    </Tabs>
  );
}

type Preset = { label: string; f: Pick<FilterDef, "anchor" | "unit" | "n"> };
const PRESETS: Preset[] = [
  { label: "Last 7 days", f: { anchor: "last", unit: "day", n: 7 } },
  { label: "Last 30 days", f: { anchor: "last", unit: "day", n: 30 } },
  { label: "Last 90 days", f: { anchor: "last", unit: "day", n: 90 } },
  { label: "This month", f: { anchor: "this", unit: "month" } },
  { label: "This quarter", f: { anchor: "this", unit: "quarter" } },
  { label: "This year", f: { anchor: "this", unit: "year" } },
  { label: "Previous month", f: { anchor: "previous", unit: "month" } },
  { label: "Previous year", f: { anchor: "previous", unit: "year" } },
];
const UNITS: RelativeUnit[] = ["day", "week", "month", "quarter", "year"];

export function DateEditor({ column, initial, onApply }: EditorProps) {
  const isRange = opOf(initial) === "between";
  const [tab, setTab] = React.useState<string>(isRange ? "range" : "relative");
  const [range, setRange] = React.useState<FilterDef>(isRange ? initial : { column, values: [], op: "between" });
  const [n, setN] = React.useState(String(initial.anchor === "last" && initial.n ? initial.n : 30));
  const [unit, setUnit] = React.useState<RelativeUnit>(initial.anchor === "last" && initial.unit ? initial.unit : "day");
  const relative = (f: Preset["f"]): FilterDef => ({ column, values: [], op: "relative", ...f });
  const count = Number(n);
  const customOk = Number.isInteger(count) && count >= 1 && count <= 3650;
  const date = (key: "min" | "max", label: string) => (
    <label className="flex flex-1 flex-col gap-1 text-xs text-muted-foreground">
      {label}
      <Input type="date" value={range[key] ?? ""} onChange={(e) => setRange({ ...range, [key]: e.target.value || undefined })} />
    </label>
  );
  return (
    <Tabs value={tab} onValueChange={(v) => setTab(String(v))}>
      <TabsList className="w-full">
        <TabsTrigger value="relative">Relative</TabsTrigger>
        <TabsTrigger value="range">Range</TabsTrigger>
      </TabsList>
      <TabsContent value="relative">
        <div className="flex flex-col gap-2">
          <div className="grid grid-cols-2 gap-1">
            {PRESETS.map((p) => (
              <Button key={p.label} size="sm" variant="outline" onClick={() => onApply(relative(p.f))}>{p.label}</Button>
            ))}
          </div>
          <div className="flex items-end gap-2 border-t border-border pt-2">
            <label className="flex w-20 flex-col gap-1 text-xs text-muted-foreground">
              Last
              <Input type="number" min={1} max={3650} value={n} onChange={(e) => setN(e.target.value)} />
            </label>
            <select
              aria-label="Unit"
              value={unit}
              onChange={(e) => setUnit(e.target.value as RelativeUnit)}
              className="h-8 flex-1 rounded-lg border border-input bg-transparent px-2 text-sm dark:bg-input/30"
            >
              {UNITS.map((u) => <option key={u} value={u}>{u}s</option>)}
            </select>
            <Button size="sm" disabled={!customOk} onClick={() => onApply(relative({ anchor: "last", unit, n: count }))}>Apply</Button>
          </div>
        </div>
      </TabsContent>
      <TabsContent value="range">
        <div className="flex flex-col gap-2">
          <div className="flex gap-2">{date("min", "From")}{date("max", "To")}</div>
          <ApplyRow disabled={!isActiveFilter(range)} onApply={() => onApply(range)} />
        </div>
      </TabsContent>
    </Tabs>
  );
}
