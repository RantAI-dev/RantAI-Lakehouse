"use client";

import * as React from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { clickProblem } from "@/lib/click-destination";
import { useService } from "@/hooks/use-service";
import { queryService } from "@/services";
import type { ChartClick } from "@/services/clients/bi-store";
import { apiFetch } from "@/services/http";

type BoardOpt = { id: string; name: string };
type ClickKind = "menu" | ChartClick["kind"];

const KIND_LABELS: Record<ClickKind, string> = {
  menu: "Drill menu",
  dashboard: "Open a dashboard",
  query: "Open a saved query",
  url: "Open a URL",
};

/** The columns a dashboard can be filtered by, as its own payload lists them. */
async function filterColumns(board: string, signal: AbortSignal): Promise<string[]> {
  if (!board) return [];
  const res = await apiFetch(`/api/dashboard?${new URLSearchParams({ board }).toString()}`, { signal });
  if (!res.ok) throw new Error("The dashboard's columns could not be loaded.");
  const json = (await res.json()) as { filterFields?: { column: string }[] };
  return (json.filterFields ?? []).map((f) => f.column);
}

/**
 * What a click on the chart does (BI-18·B): the drill menu, or another
 * dashboard, a saved query or a URL. The server checks the shape and the URL
 * rule at save; it does not check that the target exists, so the form lists
 * only what exists now and keeps a saved value that no longer does visible.
 */
export function ClickSetting({
  value, onChange, boards,
}: {
  readonly value: ChartClick | undefined;
  readonly onChange: (next: ChartClick | undefined) => void;
  readonly boards: readonly BoardOpt[];
}) {
  const kind: ClickKind = value?.kind ?? "menu";
  const problem = clickProblem(value);
  // Only fields the user has touched complain: a fresh choice is incomplete by definition.
  const [touched, setTouched] = React.useState(false);

  function pickKind(next: ClickKind) {
    setTouched(false);
    if (next === "menu") onChange(undefined);
    else if (next === "dashboard") onChange({ kind: "dashboard", board: "", column: "" });
    else if (next === "query") onChange({ kind: "query", id: "" });
    else onChange({ kind: "url", url: "" });
  }

  return (
    <div className="grid gap-1.5">
      <Label>On click</Label>
      <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
        <Select value={kind} items={KIND_LABELS} onValueChange={(v) => pickKind((v as ClickKind) ?? "menu")}>
          <SelectTrigger className="w-full" aria-label="On click"><SelectValue /></SelectTrigger>
          <SelectContent>
            {(Object.keys(KIND_LABELS) as ClickKind[]).map((k) => <SelectItem key={k} value={k}>{KIND_LABELS[k]}</SelectItem>)}
          </SelectContent>
        </Select>
        {value?.kind === "dashboard" ? <DashboardFields value={value} onChange={onChange} boards={boards} /> : null}
        {value?.kind === "query" ? <QueryField value={value} onChange={onChange} /> : null}
        {value?.kind === "url" ? (
          <Input
            aria-label="URL" value={value.url} placeholder="https://example.com/?q={value}"
            onChange={(e) => { setTouched(true); onChange({ kind: "url", url: e.target.value }); }}
          />
        ) : null}
      </div>
      {value?.kind === "url" ? (
        <p className="text-xs text-muted-foreground">{"{value}"} is replaced by the clicked value.</p>
      ) : null}
      {problem && (touched || value?.kind === "url") && (value?.kind !== "url" || value.url) ? (
        <p role="alert" className="text-xs text-destructive">{problem}</p>
      ) : null}
    </div>
  );
}

function DashboardFields({
  value, onChange, boards,
}: {
  readonly value: Extract<ChartClick, { kind: "dashboard" }>;
  readonly onChange: (next: ChartClick) => void;
  readonly boards: readonly BoardOpt[];
}) {
  const known = boards.some((b) => b.id === value.board);
  const labels: Record<string, string> = Object.fromEntries(boards.map((b) => [b.id, b.name]));
  // A saved board that has since been deleted stays visible, said plainly.
  if (value.board && !known) labels[value.board] = `${value.board} (no longer exists)`;
  const columns = useService((signal) => filterColumns(value.board, signal), [value.board]);
  const offered = columns.data ?? [];
  const columnLabels: Record<string, string> = Object.fromEntries(offered.map((c) => [c, c]));
  if (value.column && !offered.includes(value.column)) columnLabels[value.column] = `${value.column} (not on that dashboard)`;
  return (
    <>
      <Select value={value.board} items={labels} onValueChange={(v) => onChange({ kind: "dashboard", board: v ?? "", column: "" })}>
        <SelectTrigger className="w-full" aria-label="Dashboard"><SelectValue placeholder="Dashboard" /></SelectTrigger>
        <SelectContent>
          {boards.map((b) => <SelectItem key={b.id} value={b.id}>{b.name}</SelectItem>)}
          {value.board && !known ? <SelectItem value={value.board}>{labels[value.board]}</SelectItem> : null}
        </SelectContent>
      </Select>
      <Select
        value={value.column} items={columnLabels} disabled={!value.board}
        onValueChange={(v) => onChange({ kind: "dashboard", board: value.board, column: v ?? "" })}
      >
        <SelectTrigger className="w-full sm:col-start-2" aria-label="Column">
          <SelectValue placeholder={columns.status === "loading" && value.board ? "Loading columns…" : "Column to filter"} />
        </SelectTrigger>
        <SelectContent>
          {Object.entries(columnLabels).map(([c, label]) => <SelectItem key={c} value={c}>{label}</SelectItem>)}
        </SelectContent>
      </Select>
      {columns.status === "error" ? (
        <p role="alert" className="text-xs text-destructive sm:col-span-2">{columns.error.message}</p>
      ) : null}
    </>
  );
}

function QueryField({
  value, onChange,
}: {
  readonly value: Extract<ChartClick, { kind: "query" }>;
  readonly onChange: (next: ChartClick) => void;
}) {
  const saved = useService((signal) => queryService.listSaved(signal), []);
  const list = saved.data ?? [];
  const labels: Record<string, string> = Object.fromEntries(list.map((q) => [q.id, q.title]));
  if (value.id && !labels[value.id]) labels[value.id] = saved.status === "success" ? `${value.id} (no longer exists)` : value.id;
  return (
    <Select value={value.id} items={labels} onValueChange={(v) => onChange({ kind: "query", id: v ?? "" })}>
      <SelectTrigger className="w-full" aria-label="Saved query">
        <SelectValue placeholder={saved.status === "loading" ? "Loading queries…" : "Saved query"} />
      </SelectTrigger>
      <SelectContent>
        {Object.entries(labels).map(([id, label]) => <SelectItem key={id} value={id}>{label}</SelectItem>)}
      </SelectContent>
    </Select>
  );
}
